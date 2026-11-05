//! The service `sqlite-files` (ADR-0056): the owner's own SQLite files on
//! this computer, which the owner chooses in the system's file dialog and
//! registers for the plugin. The plugin and its view name a file by its key,
//! never by its path, and the core keeps the registration, opens the file
//! with hardened connections of its own and runs what the plugin asks.
//!
//! Reads run for the plugin's view and for its tools, whose calls the
//! policy allowed. A change from the plugin's view is an owner operation,
//! which the core journals before it runs and never runs twice; the view
//! asks the owner to confirm a change of the schema and a deletion of rows
//! before it sends them. A write of a tool runs only what the policy or the
//! owner allowed for its call, through the call's ticket (decisions 6 and
//! 8).
//!
//! Values keep SQLite's storage classes. Integers cross JSON as decimal
//! text, since IPC's bindings refuse 64-bit integers, an infinite real has a
//! value of its own, and bytes cross as base64. SQLite's messages are
//! content: the plugin's view shows them under its catalog's wording, and
//! nothing logs them.
//!
//! A database on a worker's host is the worker's to run (ADR-0057): the
//! plugin reaches it through its own link, and the core classifies its
//! statements against the copy of the schema that the worker reported,
//! decides on them under the master's policy and states what lets a write
//! run in the write's evidence, which the plugin sends to the worker as it
//! is (decisions 6 and 7).

use std::fmt;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::BoxFuture;
use crate::contracts::{
    CeilingMode, DatabaseKey, DatabaseOperationId, StatementClass, WorkerId, WriteAuthority,
    WriteOrigin,
};
use crate::service::Service;

/// The service `sqlite-files`, major 1. It reads and changes the owner's
/// files, so it is sensitive.
#[derive(Debug)]
pub enum SqliteFiles {}

impl Service for SqliteFiles {
    const NAME: &'static str = "sqlite-files";
    const MAJOR: u16 = 1;
    const SENSITIVE: bool = true;
    type Api = dyn SqliteFilesApi;
}

/// The most statements of one change, which run in one transaction.
pub const MAX_STATEMENTS: usize = 16;

/// The most rows of a page of a table.
pub const MAX_PAGE_ROWS: u32 = 500;

/// The most characters of the owner's name for a database.
pub const MAX_NAME_CHARS: usize = 100;

/// The most statements of a copy of a database's schema.
pub const MAX_SCHEMA_STATEMENTS: usize = 20_000;

/// The most bytes of the statements of a copy of a database's schema.
pub const MAX_SCHEMA_BYTES: usize = 16 * 1024 * 1024;

/// The owner's SQLite files, as one plugin reaches them.
pub trait SqliteFilesApi: Send + Sync + fmt::Debug {
    /// Shows the system's file dialog, checks the file that the owner chose
    /// and registers it. Returns `None` when the owner closed the dialog.
    fn choose(
        &self,
        request: ChooseRequest,
    ) -> BoxFuture<'static, Result<Option<DatabaseFile>, SqliteFilesError>>;

    /// The files that the owner registered for the plugin, by name, each
    /// with what the core finds at its path now.
    fn files(&self) -> BoxFuture<'static, Result<Vec<DatabaseFile>, SqliteFilesError>>;

    /// Changes the owner's name for a file.
    fn rename(
        &self,
        key: DatabaseKey,
        name: String,
    ) -> BoxFuture<'static, Result<(), SqliteFilesError>>;

    /// Marks a file read-only, which refuses every write and change of its
    /// schema, or lifts the mark.
    fn mark(
        &self,
        key: DatabaseKey,
        read_only: bool,
    ) -> BoxFuture<'static, Result<(), SqliteFilesError>>;

    /// Accepts the file that is at a registration's path now, such as after
    /// a program saved it by replacing it, once it passes the checks.
    fn accept(
        &self,
        key: DatabaseKey,
    ) -> BoxFuture<'static, Result<DatabaseFile, SqliteFilesError>>;

    /// Forgets a file. The file stays as it is.
    fn forget(&self, key: DatabaseKey) -> BoxFuture<'static, Result<(), SqliteFilesError>>;

    /// The structure of a database: its tables, views, indexes and triggers.
    fn structure(
        &self,
        key: DatabaseKey,
    ) -> BoxFuture<'static, Result<DatabaseStructure, SqliteFilesError>>;

    /// Counts the rows of a table or a view, within a time limit.
    fn count(
        &self,
        key: DatabaseKey,
        table: String,
    ) -> BoxFuture<'static, Result<u64, SqliteFilesError>>;

    /// One page of a table, in the order of its key or of a column.
    fn page(
        &self,
        key: DatabaseKey,
        request: PageRequest,
    ) -> BoxFuture<'static, Result<Page, SqliteFilesError>>;

    /// What a statement does, read from SQLite without running it.
    fn classify(
        &self,
        key: DatabaseKey,
        statement: String,
    ) -> BoxFuture<'static, Result<StatementClass, SqliteFilesError>>;

    /// Runs one statement that changes nothing, within the limits.
    fn read(
        &self,
        key: DatabaseKey,
        statement: Statement,
        limits: ReadLimits,
    ) -> BoxFuture<'static, Result<StatementRows, SqliteFilesError>>;

    /// Runs the owner's change from the plugin's view as an owner operation:
    /// the core journals it before it runs, and an ID that the journal has
    /// refuses it.
    fn change(
        &self,
        key: DatabaseKey,
        change: OwnerChange,
    ) -> BoxFuture<'static, Result<ChangeOutcome, SqliteFilesError>>;

    /// Runs the statements that the policy or the owner allowed on one
    /// database for a call of the plugin's tool, which its ticket names,
    /// exactly as they were allowed and in their order, in one transaction.
    /// The ticket writes once to each database that the call's accesses
    /// name.
    fn write(
        &self,
        ticket: DatabaseTicket,
        key: DatabaseKey,
        statements: Vec<Statement>,
    ) -> BoxFuture<'static, Result<Vec<StatementOutcome>, SqliteFilesError>>;

    /// What a statement does on a database of a worker's host, read from
    /// SQLite against the copy of the schema that the worker reported,
    /// without running it (ADR-0056, decision 6). The worker classifies it
    /// again before it runs it.
    fn classify_copy(
        &self,
        schema: DatabaseSchema,
        statement: String,
    ) -> BoxFuture<'static, Result<StatementClass, SqliteFilesError>>;

    /// The evidence of the statements on a worker's database that the
    /// policy or the owner allowed for a call of the plugin's tool, which its
    /// ticket names, exactly as they were allowed and in their order
    /// (ADR-0057, decision 7). The plugin sends it to the worker, which
    /// journals the write and runs it under its ceiling. The ticket gives
    /// evidence once for each worker's database that the call's accesses
    /// name.
    fn evidence(
        &self,
        ticket: DatabaseTicket,
        key: DatabaseKey,
        statements: Vec<Statement>,
    ) -> BoxFuture<'static, Result<WriteEvidence, SqliteFilesError>>;

    /// Begins the owner's change from the plugin's view on a worker's
    /// database as an owner operation (ADR-0057, decisions 7 and 8): the
    /// core checks it against what the worker reported of the database,
    /// journals it as executing and returns it with its evidence, which the
    /// plugin sends to the worker. An ID that the journal has refuses it.
    fn begin_remote_change(
        &self,
        key: DatabaseKey,
        database: WorkerDatabase,
        change: OwnerChange,
    ) -> BoxFuture<'static, Result<Box<dyn RemoteChange>, SqliteFilesError>>;
}

/// The `CREATE` statements of a database's tables, indexes, views and
/// triggers, as the database's machine reported them, at most
/// [`MAX_SCHEMA_STATEMENTS`] of them and [`MAX_SCHEMA_BYTES`] in all. Clones
/// share the statements, so every statement of a call can name the same
/// copy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct DatabaseSchema(Arc<[String]>);

impl DatabaseSchema {
    /// A copy of a schema with these statements.
    pub fn new(statements: Vec<String>) -> Self {
        Self(Arc::from(statements))
    }

    /// The statements.
    pub fn statements(&self) -> &[String] {
        &self.0
    }

    /// Whether the copy keeps to [`MAX_SCHEMA_STATEMENTS`] and
    /// [`MAX_SCHEMA_BYTES`].
    pub fn fits(&self) -> bool {
        let bytes: usize = self.0.iter().map(String::len).sum();
        self.0.len() <= MAX_SCHEMA_STATEMENTS && bytes <= MAX_SCHEMA_BYTES
    }
}

impl Serialize for DatabaseSchema {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DatabaseSchema {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::<String>::deserialize(deserializer).map(Self::new)
    }
}

/// A worker's host as the worker reported it when it last connected: the
/// worker, its name and its ceiling (ADR-0057, decision 6).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct WorkerHost {
    /// The worker.
    pub worker: WorkerId,
    /// The worker's name, for display.
    pub name: String,
    /// The mode of the host's ceiling.
    pub mode: CeilingMode,
    /// Whether the host accepts the master's full access (ADR-0029,
    /// decision 5).
    pub accepts_full_access: bool,
}

impl WorkerHost {
    /// A worker's host in this mode.
    pub fn new(
        worker: WorkerId,
        name: impl Into<String>,
        mode: CeilingMode,
        accepts_full_access: bool,
    ) -> Self {
        Self {
            worker,
            name: name.into(),
            mode,
            accepts_full_access,
        }
    }
}

/// A database on a worker's host, as the worker reported it: what a
/// plugin's tool names for a statement there, and what the owner's change
/// there begins from (ADR-0057, decision 6). The core classifies the
/// statement against the copy of the schema and narrows its decision to the
/// host, and the worker checks every write again under its ceiling.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct WorkerDatabase {
    /// The host.
    pub host: WorkerHost,
    /// The owner's name for the database, at most [`MAX_NAME_CHARS`].
    pub name: String,
    /// The file's identity on the host.
    pub identity: String,
    /// Whether the owner marked it read-only.
    pub read_only: bool,
    /// The copy of its schema.
    pub schema: DatabaseSchema,
}

impl WorkerDatabase {
    /// A database on a worker's host, as the worker reported it.
    pub fn new(
        host: WorkerHost,
        name: impl Into<String>,
        identity: impl Into<String>,
        read_only: bool,
        schema: DatabaseSchema,
    ) -> Self {
        Self {
            host,
            name: name.into(),
            identity: identity.into(),
            read_only,
            schema,
        }
    }
}

/// A write on a database of a worker's host with what lets it run, as the
/// core states it from a call's ticket or an owner operation, never the
/// plugin (ADR-0057, decision 7). The plugin sends it to the worker as it
/// is: the worker computes the write's digest again from its own registry
/// and the write, checks the authority against its ceiling, and journals
/// the write under its operation ID before it runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct WriteEvidence {
    /// The operation's ID, good once: a worker's journal that has it refuses
    /// the write.
    pub operation: DatabaseOperationId,
    /// The database's key.
    pub key: DatabaseKey,
    /// The file's identity that the master saw.
    pub identity: String,
    /// The write.
    pub write: EvidencedWrite,
    /// Where it comes from.
    pub origin: WriteOrigin,
    /// What lets it run.
    pub authority: WriteAuthority,
}

impl WriteEvidence {
    /// The evidence of a write. Only the core makes one.
    #[cfg(feature = "host")]
    pub fn new(
        operation: DatabaseOperationId,
        key: DatabaseKey,
        identity: String,
        write: EvidencedWrite,
        origin: WriteOrigin,
        authority: WriteAuthority,
    ) -> Self {
        Self {
            operation,
            key,
            identity,
            write,
            origin,
            authority,
        }
    }
}

/// The write of a piece of evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EvidencedWrite {
    /// The statements that a call of a plugin's tool writes, in order, in
    /// one transaction.
    Statements {
        /// The statements.
        statements: Vec<Statement>,
    },
    /// The owner's change from the plugin's view.
    Change {
        /// The change.
        change: Change,
    },
}

/// An owner operation on a worker's database, which the core journaled as
/// executing before the plugin sends it (ADR-0057, decision 8). Dropping it
/// before it ends records the operation as uncertain, since the worker may
/// have run it.
pub trait RemoteChange: Send + Sync + fmt::Debug {
    /// The evidence that the plugin sends with the change.
    fn evidence(&self) -> &WriteEvidence;

    /// Records how the change ended on the worker.
    fn end(self: Box<Self>, outcome: RemoteOutcome) -> BoxFuture<'static, ()>;
}

/// How a write on a worker's database ended, as the plugin learned it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RemoteOutcome {
    /// It completed.
    Completed {
        /// The rows that it changed.
        changes: u64,
    },
    /// It failed without a change.
    Failed {
        /// The stable code of the failure: 1 to 64 lower-case letters,
        /// digits, dots and underscores.
        code: String,
    },
    /// Its answer was lost, such as when the link dropped, so it may have
    /// taken effect. The worker's journal holds how it ended.
    Uncertain,
}

/// The most characters of a filter's name in the file dialog.
pub const MAX_FILTER_CHARS: usize = 100;

/// What the file dialog shows, and how the chosen file is registered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ChooseRequest {
    /// The dialog's name for SQLite files, from the plugin's catalog, such
    /// as "SQLite databases".
    pub databases_filter: String,
    /// The dialog's name for every file, from the plugin's catalog, such as
    /// "All files", since a SQLite file may have any name.
    pub all_files_filter: String,
    /// The owner's name for the database, or `None` for the file's name.
    pub name: Option<String>,
    /// Whether the owner marks it read-only.
    pub read_only: bool,
}

/// The ticket of a call of a plugin's tool whose authorized accesses hold
/// writes or schema changes of databases, good for one write to each of
/// those databases while the call executes. Only the core makes one.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DatabaseTicket(String);

impl DatabaseTicket {
    /// A ticket with this text. Only the core makes one.
    #[cfg(feature = "host")]
    pub fn new(text: String) -> Self {
        Self(text)
    }

    /// The ticket's text.
    #[cfg(feature = "host")]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A value of SQLite as it crosses JSON. Two reals are equal when their
/// bits are, so a value equals itself and can name a call's access.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SqlValue {
    /// `NULL`.
    Null,
    /// A 64-bit integer, as decimal text.
    Integer {
        /// The integer's decimal text.
        value: String,
    },
    /// A finite real number.
    Real {
        /// The number.
        value: f64,
    },
    /// An infinite real number.
    Infinity {
        /// Whether it is negative infinity.
        negative: bool,
    },
    /// Text.
    Text {
        /// The text.
        value: String,
    },
    /// Bytes.
    Blob {
        /// The bytes as standard base64.
        base64: String,
    },
}

impl PartialEq for SqlValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,
            (Self::Integer { value }, Self::Integer { value: other })
            | (Self::Text { value }, Self::Text { value: other })
            | (Self::Blob { base64: value }, Self::Blob { base64: other }) => value == other,
            (Self::Real { value }, Self::Real { value: other }) => {
                value.to_bits() == other.to_bits()
            }
            (Self::Infinity { negative }, Self::Infinity { negative: other }) => negative == other,
            _different => false,
        }
    }
}

impl Eq for SqlValue {}

impl std::hash::Hash for SqlValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Null => {}
            Self::Integer { value } | Self::Text { value } => value.hash(state),
            Self::Real { value } => value.to_bits().hash(state),
            Self::Infinity { negative } => negative.hash(state),
            Self::Blob { base64 } => base64.hash(state),
        }
    }
}

impl SqlValue {
    /// An integer.
    pub fn integer(value: i64) -> Self {
        Self::Integer {
            value: value.to_string(),
        }
    }

    /// A real number, infinite or finite. SQLite stores no NaN, which
    /// becomes `NULL`, as SQLite makes it.
    pub fn real(value: f64) -> Self {
        if value.is_nan() {
            Self::Null
        } else if value.is_infinite() {
            Self::Infinity {
                negative: value < 0.0,
            }
        } else {
            Self::Real { value }
        }
    }

    /// Text.
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text {
            value: value.into(),
        }
    }

    /// Bytes.
    pub fn blob(bytes: &[u8]) -> Self {
        Self::Blob {
            base64: BASE64.encode(bytes),
        }
    }

    /// The integer of an integer value, or `None` for another value or text
    /// that is no 64-bit integer.
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Self::Integer { value } => value.parse().ok(),
            _other => None,
        }
    }

    /// The bytes of a blob value, or `None` for another value or invalid
    /// base64.
    pub fn as_blob(&self) -> Option<Vec<u8>> {
        match self {
            Self::Blob { base64 } => BASE64.decode(base64).ok(),
            _other => None,
        }
    }

    /// The value as SQL writes it, such as `42`, `'text'` or `X'0A'`, cut
    /// to `max_chars` characters with an ellipsis. Agents' results and the
    /// owner's approvals show values this way.
    pub fn display(&self, max_chars: usize) -> String {
        let full = match self {
            Self::Null => "NULL".to_owned(),
            Self::Integer { value } => value.clone(),
            Self::Real { value } => format!("{value:?}"),
            Self::Infinity { negative: false } => "Inf".to_owned(),
            Self::Infinity { negative: true } => "-Inf".to_owned(),
            Self::Text { value } => format!("'{}'", value.replace('\'', "''")),
            Self::Blob { .. } => {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                let bytes = self.as_blob().unwrap_or_default();
                let mut literal = String::with_capacity(bytes.len() * 2 + 3);
                literal.push_str("X'");
                for byte in bytes {
                    literal.push(char::from(HEX[usize::from(byte >> 4)]));
                    literal.push(char::from(HEX[usize::from(byte & 0x0f)]));
                }
                literal.push('\'');
                literal
            }
        };
        cut_chars(full, max_chars)
    }
}

/// A text cut to `max_chars` characters, with an ellipsis in place of the
/// rest.
fn cut_chars(text: String, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((end, _)) => {
            let mut cut = text[..end].to_owned();
            cut.push('…');
            cut
        }
        None => text,
    }
}

/// A statement with its positional parameters, `?1` first.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Statement {
    /// The statement's text.
    pub text: String,
    /// The values of its parameters.
    #[serde(default)]
    pub parameters: Vec<SqlValue>,
}

impl Statement {
    /// A statement with its parameters.
    pub fn new(text: impl Into<String>, parameters: Vec<SqlValue>) -> Self {
        Self {
            text: text.into(),
            parameters,
        }
    }
}

/// A file that the owner registered for the plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseFile {
    /// The plugin's key for it.
    pub key: DatabaseKey,
    /// The owner's name for it.
    pub name: String,
    /// The file's name, without its folder, for the owner to recognize it.
    pub file_name: String,
    /// Whether the owner marked it read-only.
    pub read_only: bool,
    /// What the core found at its path when it listed it.
    pub state: FileState,
}

/// What the core finds at a registered file's path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum FileState {
    /// The registered file is there.
    Available,
    /// Nothing is there.
    Missing,
    /// Another file is there, or the path reaches it through a link now,
    /// such as after a program saved it by replacing it. The owner accepts
    /// it, or forgets the registration.
    Changed,
    /// The file could not be read; the log has the cause.
    Unreadable,
}

/// The limits of a read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ReadLimits {
    /// The most rows of the result.
    pub rows: u32,
    /// The most bytes of the result's values.
    pub bytes: u32,
    /// How long the statement may run, in milliseconds.
    pub milliseconds: u32,
}

impl ReadLimits {
    /// Limits for the owner's view: 1,000 rows, 4 MiB and 10 seconds.
    pub const VIEW: Self = Self {
        rows: 1_000,
        bytes: 4 * 1024 * 1024,
        milliseconds: 10_000,
    };
}

/// A column of a statement's result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ResultColumn {
    /// The column's name.
    pub name: String,
    /// Its declared type, for a column of a table.
    pub declared_type: Option<String>,
}

/// Why a result holds fewer rows than its statement returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum RowsCut {
    /// It reached the most rows of its limits.
    Rows,
    /// It reached the most bytes of its limits.
    Bytes,
}

/// The rows that a statement returned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct StatementRows {
    /// The columns.
    pub columns: Vec<ResultColumn>,
    /// The rows, each with one value for each column.
    pub rows: Vec<Vec<SqlValue>>,
    /// Why the rows stop early, or `None` when they are all there.
    pub cut: Option<RowsCut>,
}

/// What kind of table an object of a structure is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum TableKind {
    /// An ordinary table.
    Table,
    /// A view.
    View,
    /// A virtual table, such as an FTS5 index.
    Virtual,
    /// A table that a virtual table keeps its data in.
    Shadow,
}

/// A column of a table or a view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TableColumn {
    /// The column's name.
    pub name: String,
    /// Its declared type, or an empty text for none.
    pub declared_type: String,
    /// Whether it is `NOT NULL`.
    pub not_null: bool,
    /// Its default as SQL.
    pub default: Option<String>,
    /// Its place in the primary key from 1, or 0 outside it.
    pub primary_key: u32,
    /// Whether it is hidden or generated.
    pub hidden: bool,
}

/// A foreign key of a table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKey {
    /// The table's columns.
    pub columns: Vec<String>,
    /// The table it refers to.
    pub parent: String,
    /// The parent's columns, or `None` for the parent's primary key.
    pub parent_columns: Vec<Option<String>>,
    /// What an update of the parent does.
    pub on_update: String,
    /// What a deletion of the parent does.
    pub on_delete: String,
}

/// A table, a view or a virtual table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    /// Its name.
    pub name: String,
    /// What kind of table it is.
    pub kind: TableKind,
    /// The statement that created it.
    pub sql: Option<String>,
    /// Its columns.
    pub columns: Vec<TableColumn>,
    /// Whether it is a table `WITHOUT ROWID`.
    pub without_rowid: bool,
    /// Whether it is `STRICT`.
    pub strict: bool,
    /// Its foreign keys.
    pub foreign_keys: Vec<ForeignKey>,
}

/// An index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    /// Its name.
    pub name: String,
    /// Its table.
    pub table: String,
    /// Whether it is unique.
    pub unique: bool,
    /// Whether a constraint made it, without a statement of its own.
    pub automatic: bool,
    /// Whether it is partial.
    pub partial: bool,
    /// Its columns, `None` for an expression.
    pub columns: Vec<Option<String>>,
    /// The statement that created it.
    pub sql: Option<String>,
}

/// A trigger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TriggerInfo {
    /// Its name.
    pub name: String,
    /// Its table or view.
    pub table: String,
    /// The statement that created it.
    pub sql: Option<String>,
}

/// The structure of a database.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseStructure {
    /// Its tables, views and virtual tables, by name.
    pub tables: Vec<TableInfo>,
    /// Its indexes.
    pub indexes: Vec<IndexInfo>,
    /// Its triggers.
    pub triggers: Vec<TriggerInfo>,
}

impl DatabaseStructure {
    /// The copy of the schema that the structure's statements make, in an
    /// order that recreates it: tables and views by name, then indexes, then
    /// triggers, as the core reads them.
    pub fn schema(&self) -> DatabaseSchema {
        let tables = self.tables.iter().filter_map(|table| table.sql.clone());
        let indexes = self.indexes.iter().filter_map(|index| index.sql.clone());
        let triggers = self
            .triggers
            .iter()
            .filter_map(|trigger| trigger.sql.clone());
        DatabaseSchema::new(tables.chain(indexes).chain(triggers).collect())
    }
}

/// How a filter compares a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum FilterOperator {
    /// `=`.
    Equal,
    /// `<>`.
    NotEqual,
    /// `<`.
    Less,
    /// `<=`.
    LessOrEqual,
    /// `>`.
    Greater,
    /// `>=`.
    GreaterOrEqual,
    /// `LIKE`.
    Like,
    /// `IS NULL`.
    IsNull,
    /// `IS NOT NULL`.
    IsNotNull,
}

/// A condition on one column of a page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    /// The column.
    pub column: String,
    /// How it compares.
    pub operator: FilterOperator,
    /// The value, which `IsNull` and `IsNotNull` ignore.
    pub value: SqlValue,
}

/// The column that pages sort by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Order {
    /// The column.
    pub column: String,
    /// Whether the largest values come first.
    pub descending: bool,
}

/// Where a page starts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Cursor {
    /// After the row with this key and, for sorted pages, this value of the
    /// column.
    After {
        /// The value of the column that the pages sort by.
        order: Option<SqlValue>,
        /// The row's key.
        key: Vec<SqlValue>,
    },
    /// After this many rows of a view, which has no key.
    Offset {
        /// The rows before the page.
        rows: u32,
    },
}

/// What a page asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PageRequest {
    /// The table or view.
    pub table: String,
    /// The column that the pages sort by, or `None` for the key's order.
    pub order: Option<Order>,
    /// The conditions that every row meets.
    #[serde(default)]
    pub filters: Vec<Filter>,
    /// Where the page starts, or `None` for the first page.
    pub after: Option<Cursor>,
    /// The most rows of the page, at most [`MAX_PAGE_ROWS`].
    pub rows: u32,
}

/// One row of a page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PageRow {
    /// The row's key, which names the row for an edit or a deletion; none
    /// for a view's row.
    pub key: Vec<SqlValue>,
    /// The row's values, one for each column.
    pub values: Vec<SqlValue>,
}

/// A page of a table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    /// The columns of the key.
    pub key: Vec<String>,
    /// The columns of the rows.
    pub columns: Vec<ResultColumn>,
    /// The rows.
    pub rows: Vec<PageRow>,
    /// Where the next page starts, or `None` after the last.
    pub next: Option<Cursor>,
}

/// A column with a value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ColumnValue {
    /// The column.
    pub column: String,
    /// The value.
    pub value: SqlValue,
}

/// A row as the view showed it: its key, and the values of the columns that
/// a change checks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RowMatch {
    /// The row's key, in the order of the page's key.
    pub key: Vec<SqlValue>,
    /// The values that the row must still have.
    #[serde(default)]
    pub expected: Vec<ColumnValue>,
}

/// What the owner changes from the plugin's view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Change {
    /// Statements that run in order in one transaction, at most
    /// [`MAX_STATEMENTS`]. A change of the schema runs only once the owner
    /// confirmed it.
    Statements {
        /// The statements.
        statements: Vec<Statement>,
        /// Whether the owner confirmed a change of the schema.
        schema_confirmed: bool,
    },
    /// Sets columns of one row that still has the values that the view
    /// showed.
    UpdateRow {
        /// The table.
        table: String,
        /// The row.
        row: RowMatch,
        /// The columns and their new values.
        set: Vec<ColumnValue>,
    },
    /// Inserts one row.
    InsertRow {
        /// The table.
        table: String,
        /// The columns and their values; the others take their defaults.
        values: Vec<ColumnValue>,
    },
    /// Deletes rows that still have the values that the view showed, all or
    /// none.
    DeleteRows {
        /// The table.
        table: String,
        /// The rows, at most [`MAX_PAGE_ROWS`].
        rows: Vec<RowMatch>,
    },
}

/// A change of the owner, with the ID that the view made for it, so a
/// request that arrives twice runs once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OwnerChange {
    /// The operation's ID.
    pub id: DatabaseOperationId,
    /// The change.
    pub change: Change,
}

/// What a change did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeOutcome {
    /// The rows that it changed.
    pub changes: u64,
    /// The rowid of an inserted row, for a table with rowids.
    pub rowid: Option<i64>,
    /// The rows that the last statement returned, such as through
    /// `RETURNING`.
    pub rows: Option<StatementRows>,
}

/// What one statement of a tool's write did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatementOutcome {
    /// The rows that it changed.
    pub changes: u64,
    /// The rows that it returned.
    pub rows: Option<StatementRows>,
}

/// Why the core refused a file when the owner chose it or accepted it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum FileRefusal {
    /// The file is a link.
    Link,
    /// The file is on another computer.
    Network,
    /// The file is one that SQLite keeps beside a database.
    Companion,
    /// The file is inside Mannie's own folders.
    MannieFolder,
    /// The file is not a SQLite database.
    NotADatabase,
    /// The path names no file.
    NotAFile,
}

/// Why a call of the service failed. A plugin's view words each code from
/// its catalog; the core logs only codes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum SqliteFilesError {
    /// No file is registered for the plugin with this key.
    #[error("no file is registered with this key")]
    NotRegistered,
    /// The plugin registered this file already, under this key.
    #[error("the file is registered already")]
    AlreadyRegistered {
        /// The key of its registration.
        key: DatabaseKey,
    },
    /// The plugin registered as many files as it may.
    #[error("the plugin registered as many files as it may")]
    TooManyFiles,
    /// The core refused the file that the owner chose.
    #[error("the file was refused")]
    FileRefused {
        /// Why.
        reason: FileRefusal,
    },
    /// The registered file is gone.
    #[error("the file is gone")]
    FileMissing,
    /// Another file is at the registration's path now.
    #[error("the file changed")]
    FileChanged,
    /// The file is not a database, or it is damaged.
    #[error("the file is not a database, or it is damaged")]
    NotADatabase,
    /// The owner marked the file read-only.
    #[error("the file is marked read-only")]
    MarkedReadOnly,
    /// The file cannot be written, such as one that the system marks
    /// read-only.
    #[error("the file cannot be written")]
    FileReadOnly,
    /// A name is empty, too long or holds control characters.
    #[error("the name is not valid")]
    InvalidName,
    /// A filter's name for the file dialog is empty, too long or holds
    /// control characters.
    #[error("the filter's name is not valid")]
    InvalidFilter,
    /// The text holds no statement, or more than one.
    #[error("the text must hold exactly one statement")]
    NotOneStatement,
    /// A change holds no statement, or more than [`MAX_STATEMENTS`].
    #[error("a change holds 1 to {MAX_STATEMENTS} statements")]
    TooManyStatements,
    /// The statement takes another number of parameters.
    #[error("the statement takes {expected} parameters, not {given}")]
    Parameters {
        /// The parameters that it takes.
        expected: u32,
        /// The parameters that it got.
        given: u32,
    },
    /// A value is not valid, such as an integer that is no 64-bit integer.
    #[error("a value is not valid")]
    InvalidValue,
    /// SQLite refused the statement, such as for a syntax error.
    #[error("the statement is not valid")]
    InvalidStatement {
        /// SQLite's message.
        message: String,
    },
    /// The statement does something that the core never runs, such as
    /// `ATTACH` or a transaction.
    #[error("the statement does something that the core never runs")]
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
    /// A statement changes the schema, and the owner has not confirmed it.
    #[error("the change of the schema needs the owner's confirmation")]
    SchemaUnconfirmed,
    /// Another program holds the file's lock.
    #[error("another program is using the file")]
    Busy,
    /// The statement ran for longer than its limit.
    #[error("the statement took too long")]
    TimedOut,
    /// A write broke a constraint, such as a foreign key.
    #[error("the write broke a constraint")]
    Constraint {
        /// SQLite's message.
        message: String,
    },
    /// A row changed since the view showed it.
    #[error("the row changed since it was read")]
    RowChanged,
    /// The table has no key that names one row, such as a view.
    #[error("the table has no key that names one row")]
    NoKey,
    /// The ticket names no write of this call, or not these statements.
    #[error("the write was not allowed for this call")]
    NotAllowed,
    /// The journal has an operation with this ID, which ran or may have
    /// run, and never runs again.
    #[error("the operation has run already")]
    AlreadyRun,
    /// This computer has no file dialog, such as a worker.
    #[error("this computer has no file dialog")]
    NoDialog,
    /// The worker's host lets it only read, so it refuses every write and
    /// change of a schema (ADR-0057, decision 6).
    #[error("the worker's host only reads")]
    ReadOnlyHost,
    /// The copy of a schema that a worker reported holds more than
    /// [`MAX_SCHEMA_STATEMENTS`] statements or [`MAX_SCHEMA_BYTES`].
    #[error("the copy of the schema is too large")]
    SchemaTooLarge,
    /// The core could not do it; the log has the cause.
    #[error("the core could not do it")]
    Failed,
}

impl SqliteFilesError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotRegistered => "sqlite_files.not_registered",
            Self::AlreadyRegistered { .. } => "sqlite_files.already_registered",
            Self::TooManyFiles => "sqlite_files.too_many_files",
            Self::FileRefused { .. } => "sqlite_files.file_refused",
            Self::FileMissing => "sqlite_files.file_missing",
            Self::FileChanged => "sqlite_files.file_changed",
            Self::NotADatabase => "sqlite_files.not_a_database",
            Self::MarkedReadOnly => "sqlite_files.marked_read_only",
            Self::FileReadOnly => "sqlite_files.file_read_only",
            Self::InvalidName => "sqlite_files.invalid_name",
            Self::InvalidFilter => "sqlite_files.invalid_filter",
            Self::NotOneStatement => "sqlite_files.not_one_statement",
            Self::TooManyStatements => "sqlite_files.too_many_statements",
            Self::Parameters { .. } => "sqlite_files.parameters",
            Self::InvalidValue => "sqlite_files.invalid_value",
            Self::InvalidStatement { .. } => "sqlite_files.invalid_statement",
            Self::Refused { .. } => "sqlite_files.refused",
            Self::NotARead { .. } => "sqlite_files.not_a_read",
            Self::SchemaUnconfirmed => "sqlite_files.schema_unconfirmed",
            Self::Busy => "sqlite_files.busy",
            Self::TimedOut => "sqlite_files.timed_out",
            Self::Constraint { .. } => "sqlite_files.constraint",
            Self::RowChanged => "sqlite_files.row_changed",
            Self::NoKey => "sqlite_files.no_key",
            Self::NotAllowed => "sqlite_files.not_allowed",
            Self::AlreadyRun => "sqlite_files.already_run",
            Self::NoDialog => "sqlite_files.no_dialog",
            Self::ReadOnlyHost => "sqlite_files.read_only_host",
            Self::SchemaTooLarge => "sqlite_files.schema_too_large",
            Self::Failed => "sqlite_files.failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DatabaseSchema, MAX_SCHEMA_BYTES, MAX_SCHEMA_STATEMENTS, SqlValue, SqliteFilesError,
        Statement,
    };

    #[test]
    fn a_schema_crosses_json_as_its_statements_and_keeps_to_its_limits() {
        let schema = DatabaseSchema::new(vec!["CREATE TABLE parts (name TEXT)".to_owned()]);

        let json = serde_json::to_value(&schema).unwrap();
        assert_eq!(json, serde_json::json!(["CREATE TABLE parts (name TEXT)"]));
        assert_eq!(
            serde_json::from_value::<DatabaseSchema>(json).unwrap(),
            schema
        );
        assert!(schema.fits());
        assert!(!DatabaseSchema::new(vec![String::new(); MAX_SCHEMA_STATEMENTS + 1]).fits());
        assert!(!DatabaseSchema::new(vec!["x".repeat(MAX_SCHEMA_BYTES + 1)]).fits());
    }

    #[test]
    fn values_cross_json_with_integers_as_text_and_infinity_of_its_own() {
        let values = [
            SqlValue::Null,
            SqlValue::integer(i64::MAX),
            SqlValue::real(1.5),
            SqlValue::real(f64::NEG_INFINITY),
            SqlValue::text("a 'quote'"),
            SqlValue::blob(&[0, 255]),
        ];

        let json = serde_json::to_value(&values).unwrap();
        let back: Vec<SqlValue> = serde_json::from_value(json.clone()).unwrap();

        assert_eq!(back, values);
        assert_eq!(
            json[1],
            serde_json::json!({ "type": "integer", "value": "9223372036854775807" })
        );
        assert_eq!(
            json[3],
            serde_json::json!({ "type": "infinity", "negative": true })
        );
        assert_eq!(SqlValue::real(f64::NAN), SqlValue::Null);
        assert_eq!(values[1].as_integer(), Some(i64::MAX));
        assert_eq!(values[5].as_blob(), Some(vec![0, 255]));
    }

    #[test]
    fn a_value_shows_as_sql_writes_it_cut_to_its_limit() {
        assert_eq!(SqlValue::Null.display(10), "NULL");
        assert_eq!(SqlValue::integer(-3).display(10), "-3");
        assert_eq!(SqlValue::real(2.0).display(10), "2.0");
        assert_eq!(SqlValue::real(f64::INFINITY).display(10), "Inf");
        assert_eq!(SqlValue::text("it's").display(10), "'it''s'");
        assert_eq!(SqlValue::blob(&[10, 255]).display(10), "X'0AFF'");
        assert_eq!(SqlValue::text("abcdefgh").display(4), "'abc…");
    }

    #[test]
    fn a_statement_without_parameters_reads_from_json_and_errors_carry_their_codes() {
        let statement: Statement = serde_json::from_str(r#"{ "text": "SELECT 1" }"#).unwrap();

        assert_eq!(statement.parameters, Vec::new());
        assert_eq!(
            serde_json::to_value(SqliteFilesError::Parameters {
                expected: 2,
                given: 1
            })
            .unwrap(),
            serde_json::json!({ "code": "parameters", "expected": 2, "given": 1 })
        );
        assert_eq!(SqliteFilesError::Busy.code(), "sqlite_files.busy");
    }
}
