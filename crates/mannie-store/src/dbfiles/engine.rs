//! The engine: hardened connections of their own to the owner's SQLite
//! files (ADR-0056, decision 3), and the reads, classifications and writes
//! that the service runs on them (decisions 4 and 6).
//!
//! Each call opens its own connection and closes it when it returns, so no
//! statement stays open between calls and the owner's applications keep
//! their locks (spike S4). A connection opens read-only, or read-write
//! without create, without URI filenames, after the file's checks. It turns
//! on `SQLITE_DBCONFIG_DEFENSIVE`, turns off trusted schema, sets the limits
//! below SQLite's defaults with the attach limit at 0, and runs every
//! statement under the authorizer. A write runs its statements in one
//! transaction of the engine's own, which it begins and ends without the
//! authorizer, so the statements themselves can never begin or end one.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use mannie_types::tools::StatementClass;
use rusqlite::config::DbConfig;
use rusqlite::limits::Limit;
use rusqlite::{Connection, ErrorCode, InterruptHandle, OpenFlags, Statement};

use super::authorizer::{self, Actions};
use super::files::{self, FileRefusal};
use super::values::Value;

/// How long a statement waits for another program's lock before it fails
/// with [`EngineError::Busy`] (spike S4).
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);

/// How many steps of SQLite's virtual machine pass between two checks of a
/// statement's deadline and of its caller (spike S3).
const PROGRESS_STEPS: i32 = 10_000;

/// The most statements of one batch, which run in one transaction.
pub const MAX_BATCH_STATEMENTS: usize = 16;

/// The start of SQLite's messages for what the authorizer denied.
const NOT_AUTHORIZED: &str = "not authorized";

/// SQLite's limits for the engine's connections: lower than SQLite's
/// defaults, and high enough for real databases. The attach limit of 0
/// refuses `ATTACH` and every form of `VACUUM` (spike S1).
const LIMITS: &[(Limit, i32)] = &[
    (Limit::SQLITE_LIMIT_ATTACHED, 0),
    (Limit::SQLITE_LIMIT_LENGTH, 128 * 1024 * 1024),
    (Limit::SQLITE_LIMIT_SQL_LENGTH, 1024 * 1024),
    (Limit::SQLITE_LIMIT_EXPR_DEPTH, 500),
    (Limit::SQLITE_LIMIT_COMPOUND_SELECT, 250),
    (Limit::SQLITE_LIMIT_TRIGGER_DEPTH, 100),
    (Limit::SQLITE_LIMIT_LIKE_PATTERN_LENGTH, 10_000),
    (Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 10_000),
];

/// Why the engine could not do what it was asked. SQLite's messages are
/// content: the UI shows them under the catalog's wording, and nothing logs
/// them.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EngineError {
    /// The file failed its checks.
    #[error(transparent)]
    File(#[from] FileRefusal),
    /// The file is not a database, or it is damaged.
    #[error("the file is not a database, or it is damaged")]
    NotADatabase,
    /// The text holds no statement, or more than one.
    #[error("the text must hold exactly one statement")]
    NotOneStatement,
    /// The statement takes another number of parameters.
    #[error("the statement takes {expected} parameters, not {given}")]
    Parameters {
        /// The parameters that the statement takes.
        expected: usize,
        /// The parameters that the call gave.
        given: usize,
    },
    /// SQLite refused the statement, such as for a syntax error or an
    /// unknown table.
    #[error("the statement is not valid")]
    Invalid {
        /// SQLite's message.
        message: String,
    },
    /// The statement does something that the engine never runs, such as
    /// `ATTACH`, a transaction or a pragma that writes.
    #[error("the statement does something that the engine never runs")]
    Refused {
        /// SQLite's message.
        message: String,
    },
    /// A read was asked of a statement that changes the database.
    #[error("the statement is not a read")]
    NotARead {
        /// What the statement does.
        class: StatementClass,
    },
    /// The statement does something other than its approval named.
    #[error("the statement is a {found:?}, not a {expected:?}")]
    ClassChanged {
        /// The class that the approval named.
        expected: StatementClass,
        /// The class that the statement has now.
        found: StatementClass,
    },
    /// Another program held the file's lock for longer than the engine
    /// waits.
    #[error("another program is using the file")]
    Busy,
    /// The statement ran for longer than its limit.
    #[error("the statement took too long")]
    TimedOut,
    /// The caller stopped waiting, and the engine interrupted the statement.
    #[error("the statement was stopped")]
    Stopped,
    /// A write broke a constraint, such as a foreign key or a unique index.
    #[error("the write broke a constraint")]
    Constraint {
        /// SQLite's message.
        message: String,
    },
    /// A row that an edit or a deletion names no longer has the values
    /// that its view showed.
    #[error("the row changed since it was read")]
    RowChanged,
    /// The file cannot be written, such as a file that the system marks
    /// read-only.
    #[error("the file cannot be written")]
    ReadOnlyFile,
    /// A batch holds no statement, or more than [`MAX_BATCH_STATEMENTS`].
    #[error("a batch holds 1 to {MAX_BATCH_STATEMENTS} statements")]
    BatchSize,
    /// The table is unknown, or has no key that names one row.
    #[error("the table has no key that names one row")]
    NoKey,
    /// The database could not be used; the log has the cause.
    #[error("the database could not be used")]
    Failed,
}

/// How long a call may run and how much it may return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most rows that a result holds.
    pub rows: usize,
    /// The most bytes that a result's values hold, as [`Value::size`]
    /// counts them.
    pub bytes: usize,
    /// How long the call's statements may run.
    pub time: Duration,
}

/// A column of a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    /// The column's name, as the statement names it.
    pub name: String,
    /// The column's declared type, for a column of a table.
    pub declared_type: Option<String>,
}

/// Why a result holds fewer rows than its statement returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cut {
    /// It reached the most rows of its limits.
    Rows,
    /// It reached the most bytes of its limits.
    Bytes,
}

/// The rows that a statement returned.
#[derive(Debug, Clone, PartialEq)]
pub struct Returned {
    /// The columns, in the statement's order.
    pub columns: Vec<Column>,
    /// The rows, each with one value for each column.
    pub rows: Vec<Vec<Value>>,
    /// Why the rows stop early, or `None` when they are all there.
    pub cut: Option<Cut>,
}

/// A statement with its positional parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct Sql {
    /// The statement's text.
    pub text: String,
    /// The values of its parameters, `?1` first.
    pub parameters: Vec<Value>,
}

/// What one statement of a write did.
#[derive(Debug, Clone, PartialEq)]
pub struct Written {
    /// The rows that it inserted, updated or deleted.
    pub changes: u64,
    /// The rows that it returned, such as through `RETURNING`.
    pub rows: Option<Returned>,
}

/// Whether a connection reads or writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Read,
    Write,
}

/// Stops a call's statements: when the caller stops waiting, or when the
/// call runs past its deadline.
#[derive(Clone)]
pub struct Cancel {
    stopped: Arc<AtomicBool>,
    interrupt: Arc<Mutex<Option<InterruptHandle>>>,
}

impl std::fmt::Debug for Cancel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Cancel")
            .field("stopped", &self.is_stopped())
            .finish_non_exhaustive()
    }
}

impl Cancel {
    /// A call that runs until it is stopped.
    pub fn new() -> Self {
        Self {
            stopped: Arc::new(AtomicBool::new(false)),
            interrupt: Arc::new(Mutex::new(None)),
        }
    }

    /// Stops the call's statement now and every later one.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        let handle = self
            .interrupt
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(handle) = handle.as_ref() {
            handle.interrupt();
        }
    }

    /// Whether the call was stopped.
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// Follows a connection, so that stopping interrupts its statement.
    fn follow(&self, connection: &Connection) {
        *self
            .interrupt
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(connection.get_interrupt_handle());
    }
}

impl Default for Cancel {
    fn default() -> Self {
        Self::new()
    }
}

/// An open, hardened connection, with what its authorizer saw.
pub(super) struct Opened {
    pub(super) connection: Connection,
    actions: Option<Actions>,
    cancel: Cancel,
}

impl std::fmt::Debug for Opened {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Opened").finish_non_exhaustive()
    }
}

/// Maps an error of SQLite to the engine's error. The messages of the
/// statement's own errors are kept as content, and the code of any other
/// error is logged.
pub(super) fn engine_error(error: rusqlite::Error, cancel: &Cancel) -> EngineError {
    match error {
        rusqlite::Error::MultipleStatement => EngineError::NotOneStatement,
        rusqlite::Error::InvalidParameterCount(given, expected) => {
            EngineError::Parameters { expected, given }
        }
        // SQLite reports a function that the authorizer denied as an error of
        // the statement, with the authorizer's message.
        rusqlite::Error::SqlInputError { error, msg, .. }
            if error.code == ErrorCode::AuthorizationForStatementDenied
                || msg.starts_with(NOT_AUTHORIZED) =>
        {
            EngineError::Refused { message: msg }
        }
        rusqlite::Error::SqlInputError { msg, .. } => EngineError::Invalid { message: msg },
        rusqlite::Error::SqliteFailure(failure, text) => failure_error(failure.code, text, cancel),
        other => {
            tracing::warn!(
                kind = %error_kind(&other),
                "a database file failed in its binding"
            );
            EngineError::Failed
        }
    }
}

/// Maps a failure of SQLite, with its message, to the engine's error.
fn failure_error(code: ErrorCode, text: Option<String>, cancel: &Cancel) -> EngineError {
    let denied = text
        .as_deref()
        .is_some_and(|text| text.starts_with(NOT_AUTHORIZED));
    let message = text.unwrap_or_default();
    match code {
        ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => EngineError::Busy,
        ErrorCode::OperationInterrupted if cancel.is_stopped() => EngineError::Stopped,
        ErrorCode::OperationInterrupted => EngineError::TimedOut,
        ErrorCode::AuthorizationForStatementDenied => EngineError::Refused { message },
        ErrorCode::Unknown if denied => EngineError::Refused { message },
        ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt => EngineError::NotADatabase,
        ErrorCode::ConstraintViolation => EngineError::Constraint { message },
        ErrorCode::ReadOnly => EngineError::ReadOnlyFile,
        ErrorCode::Unknown | ErrorCode::TooBig | ErrorCode::ParameterOutOfRange => {
            EngineError::Invalid { message }
        }
        ErrorCode::CannotOpen => EngineError::File(FileRefusal::NotFound),
        other => {
            tracing::warn!(code = ?other, "a database file failed in SQLite");
            EngineError::Failed
        }
    }
}

/// The kind of a binding's error, without any of its text, which may hold
/// a statement's values.
fn error_kind(error: &rusqlite::Error) -> &'static str {
    match error {
        rusqlite::Error::FromSqlConversionFailure(..) => "conversion from SQL",
        rusqlite::Error::Utf8Error(..) => "UTF-8",
        rusqlite::Error::NulError(..) => "NUL character",
        rusqlite::Error::InvalidQuery => "invalid query",
        _other => "other",
    }
}

/// Turns on the hardening of SQLite's guidance on a new connection, before
/// the authorizer, which refuses the pragmas that set it.
fn harden(connection: &Connection, mode: Mode) -> rusqlite::Result<()> {
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
    for &(limit, value) in LIMITS {
        connection.set_limit(limit, value)?;
    }
    connection.busy_timeout(BUSY_TIMEOUT)?;
    // Foreign keys are enforced on writes, as SQLite's default in this build
    // sets them (spike S2).
    connection.execute_batch(
        "PRAGMA cell_size_check = ON; PRAGMA mmap_size = 0; PRAGMA foreign_keys = ON;",
    )?;
    if mode == Mode::Read {
        connection.execute_batch("PRAGMA query_only = ON;")?;
    }
    Ok(())
}

impl Opened {
    /// Opens a registered file after its checks, hardens the connection and
    /// installs the progress handler, which stops its statements at the
    /// deadline and when the call is stopped.
    pub(super) fn open(
        path: &Path,
        identity: &str,
        mode: Mode,
        time: Duration,
        cancel: &Cancel,
    ) -> Result<Self, EngineError> {
        files::check_registered(path, identity)?;
        let access = match mode {
            Mode::Read => OpenFlags::SQLITE_OPEN_READ_ONLY,
            Mode::Write => OpenFlags::SQLITE_OPEN_READ_WRITE,
        };
        // Without SQLITE_OPEN_CREATE and SQLITE_OPEN_URI: the engine never
        // creates a database, and the path is no URI.
        let flags = access | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let connection = Connection::open_with_flags(path, flags)
            .map_err(|error| engine_error(error, cancel))?;
        harden(&connection, mode).map_err(|error| engine_error(error, cancel))?;
        // The file is checked again, so that a file replaced while it opened
        // is refused.
        files::check_registered(path, identity)?;
        let deadline = Instant::now() + time;
        let watched = cancel.clone();
        connection
            .progress_handler(
                PROGRESS_STEPS,
                Some(move || watched.is_stopped() || Instant::now() >= deadline),
            )
            .map_err(|error| engine_error(error, cancel))?;
        cancel.follow(&connection);
        Ok(Self {
            connection,
            actions: None,
            cancel: cancel.clone(),
        })
    }

    /// Installs the authorizer for the statements that follow.
    pub(super) fn confine(&mut self) -> Result<(), EngineError> {
        let actions = authorizer::install(&self.connection).map_err(|error| self.error(error))?;
        self.actions = Some(actions);
        Ok(())
    }

    /// Removes the authorizer, before the engine ends its own transaction.
    pub(super) fn release(&mut self) -> Result<(), EngineError> {
        self.connection
            .authorizer(
                None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>,
            )
            .map_err(|error| self.error(error))?;
        self.actions = None;
        Ok(())
    }

    /// The engine's error for an error of SQLite on this connection.
    pub(super) fn error(&self, error: rusqlite::Error) -> EngineError {
        engine_error(error, &self.cancel)
    }

    /// Prepares exactly one statement under the authorizer, and returns it
    /// with its class: from the actions that SQLite reported, and a read for
    /// `EXPLAIN`, which runs nothing that it explains.
    pub(super) fn prepare<'c>(
        &'c self,
        text: &str,
    ) -> Result<(Statement<'c>, StatementClass), EngineError> {
        let actions = self.actions.as_ref().ok_or(EngineError::Failed)?;
        actions.clear();
        let statement = self
            .connection
            .prepare(text)
            .map_err(|error| self.error(error))?;
        if statement.expanded_sql().is_none() {
            return Err(EngineError::NotOneStatement);
        }
        let class = if statement.is_explain() > 0 {
            StatementClass::Read
        } else {
            actions.class()
        };
        Ok((statement, class))
    }
}

/// Binds positional parameters to a statement.
pub(super) fn bind(statement: &mut Statement<'_>, parameters: &[Value]) -> Result<(), EngineError> {
    let expected = statement.parameter_count();
    if expected != parameters.len() {
        return Err(EngineError::Parameters {
            expected,
            given: parameters.len(),
        });
    }
    for (index, value) in parameters.iter().enumerate() {
        statement
            .raw_bind_parameter(index + 1, value)
            .map_err(|_error| EngineError::Parameters {
                expected,
                given: parameters.len(),
            })?;
    }
    Ok(())
}

/// The columns of a prepared statement.
pub(super) fn columns_of(statement: &Statement<'_>) -> Vec<Column> {
    statement
        .columns()
        .iter()
        .map(|column| Column {
            name: column.name().to_owned(),
            declared_type: column.decl_type().map(str::to_owned),
        })
        .collect()
}

/// Steps a statement to its end, and keeps its rows up to the limits.
pub(super) fn collect(
    opened: &Opened,
    statement: &mut Statement<'_>,
    limits: Limits,
) -> Result<Returned, EngineError> {
    let columns = columns_of(statement);
    let width = columns.len();
    let mut rows = Vec::new();
    let mut bytes = 0_usize;
    let mut cut = None;
    let mut stepped = statement.raw_query();
    while let Some(row) = stepped.next().map_err(|error| opened.error(error))? {
        if rows.len() == limits.rows {
            cut = Some(Cut::Rows);
            break;
        }
        let mut values = Vec::with_capacity(width);
        for index in 0..width {
            let value = row.get_ref(index).map_err(|error| opened.error(error))?;
            values.push(Value::read(value));
        }
        let size: usize = values.iter().map(Value::size).sum();
        if bytes + size > limits.bytes {
            cut = Some(Cut::Bytes);
            break;
        }
        bytes += size;
        rows.push(values);
    }
    Ok(Returned { columns, rows, cut })
}

/// Classifies one statement on a registered file, without running it.
pub fn classify(path: &Path, identity: &str, text: &str) -> Result<StatementClass, EngineError> {
    let cancel = Cancel::new();
    let mut opened = Opened::open(path, identity, Mode::Read, Duration::from_secs(5), &cancel)?;
    opened.confine()?;
    let (_statement, class) = opened.prepare(text)?;
    Ok(class)
}

/// Classifies one statement against a schema, as its `CREATE` statements
/// make it in a database in memory, for a database that is not on this
/// computer (ADR-0056, decision 6). A statement whose object the copy lacks
/// fails as on the file.
pub fn classify_on_schema(schema: &[String], text: &str) -> Result<StatementClass, EngineError> {
    let cancel = Cancel::new();
    let connection = Connection::open_in_memory().map_err(|error| engine_error(error, &cancel))?;
    for create in schema {
        // A virtual table's own `CREATE` already made its shadow tables, whose
        // `CREATE` fails in the copy (spike S1); the copy classifies alike.
        if let Err(error) = connection.execute_batch(create) {
            tracing::debug!(%error, "a statement of a schema copy did not run");
        }
    }
    harden(&connection, Mode::Read).map_err(|error| engine_error(error, &cancel))?;
    let mut opened = Opened {
        connection,
        actions: None,
        cancel,
    };
    opened.confine()?;
    let (_statement, class) = opened.prepare(text)?;
    Ok(class)
}

/// Runs a read on a registered file: one statement that changes nothing,
/// with its parameters, within the limits.
pub fn read(
    path: &Path,
    identity: &str,
    sql: &Sql,
    limits: Limits,
    cancel: &Cancel,
) -> Result<Returned, EngineError> {
    let mut opened = Opened::open(path, identity, Mode::Read, limits.time, cancel)?;
    opened.confine()?;
    let (mut statement, class) = opened.prepare(&sql.text)?;
    // `readonly()` is also true for transactions, ATTACH and some pragmas,
    // which the authorizer refused already (spike S1).
    if class != StatementClass::Read || !statement.readonly() {
        return Err(EngineError::NotARead { class });
    }
    bind(&mut statement, &sql.parameters)?;
    collect(&opened, &mut statement, limits)
}

/// Writes to a registered file: statements that run in order in one
/// transaction of the engine's, each of the class that their approval named
/// at most. A statement of another class rolls the transaction back.
pub fn write(
    path: &Path,
    identity: &str,
    statements: &[Sql],
    class: StatementClass,
    limits: Limits,
    cancel: &Cancel,
) -> Result<Vec<Written>, EngineError> {
    if statements.is_empty() || statements.len() > MAX_BATCH_STATEMENTS {
        return Err(EngineError::BatchSize);
    }
    in_transaction(path, identity, limits.time, cancel, |opened| {
        let mut done = Vec::with_capacity(statements.len());
        for sql in statements {
            done.push(run_one(opened, sql, class, limits)?);
        }
        Ok(done)
    })
}

/// Runs one statement of a write, after it checks its class.
pub(super) fn run_one(
    opened: &Opened,
    sql: &Sql,
    class: StatementClass,
    limits: Limits,
) -> Result<Written, EngineError> {
    let (mut statement, found) = opened.prepare(&sql.text)?;
    // A write's statements are writes or schema changes, never reads, which
    // run on read connections; one stronger than its approval goes back to
    // the policy.
    let fits = found != StatementClass::Read && found <= class;
    if !fits {
        return Err(EngineError::ClassChanged {
            expected: class,
            found,
        });
    }
    bind(&mut statement, &sql.parameters)?;
    let returns_rows = statement.column_count() > 0;
    let rows = collect(opened, &mut statement, limits)?;
    // Finalizing the statement completes it, also one whose returned rows
    // reached the limits, since SQLite makes every change of RETURNING at
    // its first step.
    drop(statement);
    // SQLite keeps the count of the last insert, update or delete through a
    // schema change, which changes no rows of its own.
    let changes = if found == StatementClass::Write {
        opened.connection.changes()
    } else {
        0
    };
    Ok(Written {
        changes,
        rows: returns_rows.then_some(rows),
    })
}

/// Runs `work` in a transaction of the engine's on a read-write connection,
/// under the authorizer, and commits it when `work` succeeds or rolls it
/// back.
pub(super) fn in_transaction<T>(
    path: &Path,
    identity: &str,
    time: Duration,
    cancel: &Cancel,
    work: impl FnOnce(&Opened) -> Result<T, EngineError>,
) -> Result<T, EngineError> {
    let mut opened = Opened::open(path, identity, Mode::Write, time, cancel)?;
    opened
        .connection
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| opened.error(error))?;
    let worked = opened.confine().and_then(|()| work(&opened));
    let released = opened.release();
    match worked.and_then(|value| released.map(|()| value)) {
        Ok(value) => {
            opened
                .connection
                .execute_batch("COMMIT")
                .map_err(|error| opened.error(error))?;
            Ok(value)
        }
        Err(error) => {
            if let Err(rollback) = opened.connection.execute_batch("ROLLBACK") {
                tracing::warn!(
                    code = ?rollback.sqlite_error_code(),
                    "a write's transaction did not roll back; SQLite ends it as the connection closes"
                );
            }
            Err(error)
        }
    }
}

/// Counts the rows of a table or a view.
pub fn count(
    path: &Path,
    identity: &str,
    table: &str,
    limits: Limits,
    cancel: &Cancel,
) -> Result<u64, EngineError> {
    let mut opened = Opened::open(path, identity, Mode::Read, limits.time, cancel)?;
    opened.confine()?;
    let text = format!("SELECT count(*) FROM {}", quote(table));
    let (mut statement, _class) = opened.prepare(&text)?;
    let rows = collect(&opened, &mut statement, limits)?;
    match rows.rows.first().and_then(|row| row.first()) {
        Some(Value::Integer(count)) => Ok(u64::try_from(*count).unwrap_or(0)),
        _other => Err(EngineError::Failed),
    }
}

/// An identifier as SQL quotes it, with each double quote doubled.
pub(super) fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}
