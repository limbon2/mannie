//! The service `sqlite-files` (ADR-0056): the owner's SQLite files on this
//! computer, which the owner chooses in the system's file dialog and
//! registers for a plugin. The runtime provides the service on the engine of
//! `mannie-store`, its registry and its journal of owner operations, and
//! keeps the tickets of the calls of plugins' tools that write to a
//! database, so the service writes only what the policy or the owner
//! allowed for the call (decision 6).
//!
//! The plugin and its view name a file by its key and never see its path.
//! A change from the view is an owner operation, journaled before it runs
//! and never run twice (decision 8); a write of a tool is part of its call's
//! operation, which the call's record journals (ADR-0013). Nothing logs a
//! statement's text, its values or SQLite's messages.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::FutureExt as _;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::host::{BindRefusal, PluginBinding, ServiceProvider};
use mannie_plugin_sdk::service::{BoundService, Service};
use mannie_plugin_sdk::services::sqlite_files::{
    Change, ChangeOutcome, ChooseRequest, ColumnValue, Cursor, DatabaseFile, DatabaseStructure,
    DatabaseTicket, FileRefusal, FileState, Filter, FilterOperator, ForeignKey, IndexInfo,
    MAX_FILTER_CHARS, MAX_NAME_CHARS, MAX_PAGE_ROWS, MAX_STATEMENTS, Order, OwnerChange, Page,
    PageRequest, PageRow, ReadLimits, ResultColumn, RowMatch, RowsCut, SqlValue, SqliteFiles,
    SqliteFilesApi, SqliteFilesError, Statement, StatementOutcome, StatementRows, TableColumn,
    TableInfo, TableKind, TriggerInfo,
};
use mannie_store::dbfiles::{
    self, EngineError, Limits, NewOperation, NewRegistration, OperationError, Registration,
    RegistryError,
};
use mannie_types::ids::{DatabaseKey, DatabaseOperationId};
use mannie_types::tools::{Access, StatementClass};
use sha2::{Digest, Sha256};

use crate::{Runtime, lock};

/// How long the structure of a database may take to read.
const STRUCTURE_TIME: Duration = Duration::from_secs(10);

/// The limits of a count of rows.
const COUNT_LIMITS: Limits = Limits {
    rows: 1,
    bytes: 64,
    time: Duration::from_secs(10),
};

/// The limits of a page of a table.
const PAGE_LIMITS: Limits = Limits {
    rows: MAX_PAGE_ROWS as usize + 1,
    bytes: 8 * 1024 * 1024,
    time: Duration::from_secs(10),
};

/// The limits of a write: the rows that its statements return, and how long
/// its transaction may take.
const WRITE_LIMITS: Limits = Limits {
    rows: 1_000,
    bytes: 4 * 1024 * 1024,
    time: Duration::from_secs(30),
};

/// The largest limits that a read may ask for.
const MAX_READ_ROWS: u32 = 10_000;
const MAX_READ_BYTES: u32 = 16 * 1024 * 1024;
const MAX_READ_MILLISECONDS: u32 = 60_000;

/// The extensions that SQLite files usually have, which the dialog's
/// first filter offers. Its second filter offers every file, since a SQLite
/// file may have any name; the checks read the file's header.
pub const DATABASE_EXTENSIONS: &[&str] = &["db", "sqlite", "sqlite3", "db3", "s3db", "sl3"];

/// The names of the file dialog's filters, from the plugin's catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseFilters {
    /// The name of the filter of [`DATABASE_EXTENSIONS`].
    pub databases: String,
    /// The name of the filter of every file.
    pub all_files: String,
}

/// Chooses a SQLite file in the system's file dialog. The composition root
/// of the desktop app provides it; a computer without one, such as a
/// worker, registers no file through this service.
pub trait DatabaseChooser: Send + Sync + fmt::Debug {
    /// Shows the dialog with the two filters and returns the file that the
    /// owner chose, or `None` when the owner closed it.
    fn choose(&self, filters: DatabaseFilters) -> BoxFuture<'static, Option<PathBuf>>;
}

/// A filter's name, trimmed, or the error of one that is empty, too long or
/// holds control characters. The dialog shows it, and it comes from the
/// webview.
fn checked_filter(label: &str) -> Result<String, SqliteFilesError> {
    let label = label.trim();
    let is_empty = label.is_empty();
    let is_long = label.chars().count() > MAX_FILTER_CHARS;
    let has_control = label.chars().any(char::is_control);
    if is_empty || is_long || has_control {
        return Err(SqliteFilesError::InvalidFilter);
    }
    Ok(label.to_owned())
}

/// A write that a call's authorized accesses hold.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AuthorizedWrite {
    key: DatabaseKey,
    identity: String,
    statement: String,
    parameters_digest: String,
    class: StatementClass,
}

/// The ticket of a call that writes to a database, while the call executes.
#[derive(Debug, Clone)]
struct TicketEntry {
    /// The plugin, or the plugin's feature, whose tool made the call, as its
    /// contributor key.
    contributor: String,
    /// The writes that the call's accesses authorized, in order.
    writes: Vec<AuthorizedWrite>,
}

/// The tickets of the calls that write to a database now, by their texts.
#[derive(Debug, Default)]
pub(crate) struct DatabaseTickets {
    tickets: Mutex<HashMap<String, TicketEntry>>,
}

/// A ticket while its call executes; dropping it ends the ticket.
#[derive(Debug)]
pub(crate) struct DatabaseTicketGuard {
    runtime: Runtime,
    text: String,
}

impl DatabaseTicketGuard {
    /// The ticket, as the call's execution hands it to the plugin.
    pub(crate) fn ticket(&self) -> DatabaseTicket {
        DatabaseTicket::new(self.text.clone())
    }
}

impl Drop for DatabaseTicketGuard {
    fn drop(&mut self) {
        lock(&self.runtime.inner.database_tickets.tickets).remove(&self.text);
    }
}

/// The digest of a statement's parameters, which an approval covers: the
/// SHA-256 of their values as JSON, in lower-case hexadecimal.
pub(crate) fn parameters_digest(parameters: &[SqlValue]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let json = serde_json::to_string(parameters).unwrap_or_default();
    let digest = Sha256::digest(json.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push(char::from(DIGITS[usize::from(byte >> 4)]));
        hex.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    hex
}

impl Runtime {
    /// The provider of the service `sqlite-files`, with the system's file
    /// dialog where the owner works.
    pub fn sqlite_files_provider(
        &self,
        chooser: Option<Arc<dyn DatabaseChooser>>,
    ) -> Arc<dyn ServiceProvider> {
        Arc::new(SqliteFilesProvider {
            runtime: self.clone(),
            chooser,
        })
    }

    /// The ticket of a call whose authorized accesses hold writes or
    /// changes of a schema of its plugin's databases, good while the call
    /// executes, or `None` for a call without them.
    pub(crate) fn issue_database_ticket(
        &self,
        plugin: &str,
        feature: Option<&str>,
        accesses: &[Access],
    ) -> Option<DatabaseTicketGuard> {
        let writes: Vec<AuthorizedWrite> = accesses
            .iter()
            .filter_map(|access| match access {
                Access::Database {
                    plugin: owner,
                    key,
                    identity,
                    statement,
                    parameters_digest,
                    class,
                    ..
                } if owner == plugin && *class != StatementClass::Read => Some(AuthorizedWrite {
                    key: key.clone(),
                    identity: identity.clone(),
                    statement: statement.clone(),
                    parameters_digest: parameters_digest.clone(),
                    class: *class,
                }),
                _other => None,
            })
            .collect();
        if writes.is_empty() {
            return None;
        }
        let contributor = match feature {
            None => plugin.to_owned(),
            Some(feature) => format!("{plugin}/{feature}"),
        };
        let text = uuid::Uuid::now_v7().simple().to_string();
        lock(&self.inner.database_tickets.tickets).insert(
            text.clone(),
            TicketEntry {
                contributor,
                writes,
            },
        );
        Some(DatabaseTicketGuard {
            runtime: self.clone(),
            text,
        })
    }
}

/// The runtime's provider of the service `sqlite-files`.
#[derive(Debug)]
struct SqliteFilesProvider {
    runtime: Runtime,
    chooser: Option<Arc<dyn DatabaseChooser>>,
}

impl ServiceProvider for SqliteFilesProvider {
    fn name(&self) -> &'static str {
        SqliteFiles::NAME
    }

    fn major(&self) -> u16 {
        SqliteFiles::MAJOR
    }

    /// Binds the service to the plugin's own registrations, for the plugin
    /// or the feature that declares it.
    fn bind(&self, binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        Ok(BoundService::new::<SqliteFiles>(Arc::new(BoundFiles {
            runtime: self.runtime.clone(),
            chooser: self.chooser.clone(),
            plugin: binding.id.as_str().to_owned(),
            contributor: binding.contributor().key(),
        })))
    }
}

/// The service bound to one plugin, or to one of its features.
#[derive(Debug, Clone)]
struct BoundFiles {
    runtime: Runtime,
    chooser: Option<Arc<dyn DatabaseChooser>>,
    plugin: String,
    contributor: String,
}

impl BoundFiles {
    /// The plugin's registration with this key.
    async fn registration(&self, key: &DatabaseKey) -> Result<Registration, SqliteFilesError> {
        self.runtime
            .inner
            .store
            .db_file(&self.plugin, key)
            .await
            .map_err(registry_error)
    }

    /// Mannie's own folders, where no registered file may be.
    fn protected(&self) -> Vec<PathBuf> {
        self.runtime.inner.tools.protected.clone()
    }
}

/// A name that the owner gave a database, trimmed, or the error of one
/// that is empty, too long or holds control characters.
fn checked_name(name: &str) -> Result<String, SqliteFilesError> {
    let name = name.trim();
    let is_empty = name.is_empty();
    let is_long = name.chars().count() > MAX_NAME_CHARS;
    let has_control = name.chars().any(char::is_control);
    if is_empty || is_long || has_control {
        return Err(SqliteFilesError::InvalidName);
    }
    Ok(name.to_owned())
}

/// The name of a registration that the owner left unnamed: the file's name
/// without its control characters, cut to the limit.
fn unnamed(path: &Path) -> Result<String, SqliteFilesError> {
    let shown: String = file_name(path)
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_NAME_CHARS)
        .collect();
    checked_name(&shown)
}

/// The name of a file, without its folder.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// What the core finds at a registered file's path now.
fn file_state(path: &Path, identity: &str) -> FileState {
    match dbfiles::check_registered(path, identity) {
        Ok(()) => FileState::Available,
        Err(dbfiles::FileRefusal::NotFound) => FileState::Missing,
        Err(dbfiles::FileRefusal::Unreadable) => FileState::Unreadable,
        Err(_changed) => FileState::Changed,
    }
}

/// The view of a registration with what the core finds at its path.
fn database_file(registration: &Registration, state: FileState) -> DatabaseFile {
    DatabaseFile {
        key: registration.key.clone(),
        name: registration.name.clone(),
        file_name: file_name(&registration.path),
        read_only: registration.read_only,
        state,
    }
}

/// The service's error for a refusal of a file's checks.
fn file_error(refusal: dbfiles::FileRefusal) -> SqliteFilesError {
    use dbfiles::FileRefusal as Checked;
    match refusal {
        Checked::NotFound => SqliteFilesError::FileMissing,
        Checked::Changed => SqliteFilesError::FileChanged,
        Checked::Link => SqliteFilesError::FileRefused {
            reason: FileRefusal::Link,
        },
        Checked::Network => SqliteFilesError::FileRefused {
            reason: FileRefusal::Network,
        },
        Checked::Companion => SqliteFilesError::FileRefused {
            reason: FileRefusal::Companion,
        },
        Checked::MannieFolder => SqliteFilesError::FileRefused {
            reason: FileRefusal::MannieFolder,
        },
        Checked::NotADatabase => SqliteFilesError::FileRefused {
            reason: FileRefusal::NotADatabase,
        },
        Checked::NotAbsolute | Checked::Uri | Checked::NotAFile => SqliteFilesError::FileRefused {
            reason: FileRefusal::NotAFile,
        },
        Checked::Unreadable => SqliteFilesError::Failed,
    }
}

/// The service's error for an error of the registry.
fn registry_error(error: RegistryError) -> SqliteFilesError {
    match error {
        RegistryError::AlreadyRegistered { key } => SqliteFilesError::AlreadyRegistered { key },
        RegistryError::TooMany => SqliteFilesError::TooManyFiles,
        RegistryError::NotFound => SqliteFilesError::NotRegistered,
        other => {
            tracing::error!(
                error.code = other.code(),
                "the registry of SQLite files failed"
            );
            SqliteFilesError::Failed
        }
    }
}

/// The service's error for an error of the engine. SQLite's messages stay
/// content, which the plugin's view shows and nothing logs.
pub(crate) fn engine_error(error: EngineError) -> SqliteFilesError {
    match error {
        EngineError::File(refusal) => file_error(refusal),
        EngineError::NotADatabase => SqliteFilesError::NotADatabase,
        EngineError::NotOneStatement => SqliteFilesError::NotOneStatement,
        EngineError::Parameters { expected, given } => SqliteFilesError::Parameters {
            expected: u32::try_from(expected).unwrap_or(u32::MAX),
            given: u32::try_from(given).unwrap_or(u32::MAX),
        },
        EngineError::Invalid { message } => SqliteFilesError::InvalidStatement { message },
        EngineError::Refused { message } => SqliteFilesError::Refused { message },
        EngineError::NotARead { class } => SqliteFilesError::NotARead { class },
        // A statement whose class is not the one that its approval named
        // goes back to the policy, which only a new call can ask.
        EngineError::ClassChanged { .. } => SqliteFilesError::NotAllowed,
        EngineError::Busy => SqliteFilesError::Busy,
        EngineError::TimedOut | EngineError::Stopped => SqliteFilesError::TimedOut,
        EngineError::Constraint { message } => SqliteFilesError::Constraint { message },
        EngineError::RowChanged => SqliteFilesError::RowChanged,
        EngineError::ReadOnlyFile => SqliteFilesError::FileReadOnly,
        EngineError::BatchSize => SqliteFilesError::TooManyStatements,
        EngineError::NoKey => SqliteFilesError::NoKey,
        EngineError::Failed => SqliteFilesError::Failed,
    }
}

/// The engine's value of a value of the service.
pub(crate) fn engine_value(value: &SqlValue) -> Result<dbfiles::Value, SqliteFilesError> {
    Ok(match value {
        SqlValue::Null => dbfiles::Value::Null,
        SqlValue::Integer { .. } => {
            dbfiles::Value::Integer(value.as_integer().ok_or(SqliteFilesError::InvalidValue)?)
        }
        SqlValue::Real { value } => dbfiles::Value::Real(*value),
        SqlValue::Infinity { negative: false } => dbfiles::Value::Real(f64::INFINITY),
        SqlValue::Infinity { negative: true } => dbfiles::Value::Real(f64::NEG_INFINITY),
        SqlValue::Text { value } => dbfiles::Value::Text(value.clone()),
        SqlValue::Blob { .. } => {
            dbfiles::Value::Blob(value.as_blob().ok_or(SqliteFilesError::InvalidValue)?)
        }
    })
}

/// The engine's values of several values of the service.
pub(crate) fn engine_values(values: &[SqlValue]) -> Result<Vec<dbfiles::Value>, SqliteFilesError> {
    values.iter().map(engine_value).collect()
}

/// The service's value of a value of the engine.
pub(crate) fn sql_value(value: &dbfiles::Value) -> SqlValue {
    match value {
        dbfiles::Value::Null => SqlValue::Null,
        dbfiles::Value::Integer(integer) => SqlValue::integer(*integer),
        dbfiles::Value::Real(real) => SqlValue::real(*real),
        dbfiles::Value::Text(text) => SqlValue::text(text.clone()),
        dbfiles::Value::Blob(bytes) => SqlValue::blob(bytes),
    }
}

/// The engine's statement of a statement of the service.
pub(crate) fn engine_sql(statement: &Statement) -> Result<dbfiles::Sql, SqliteFilesError> {
    Ok(dbfiles::Sql {
        text: statement.text.clone(),
        parameters: engine_values(&statement.parameters)?,
    })
}

/// The service's columns of the engine's columns.
fn result_columns(columns: &[dbfiles::Column]) -> Vec<ResultColumn> {
    columns
        .iter()
        .map(|column| ResultColumn {
            name: column.name.clone(),
            declared_type: column.declared_type.clone(),
        })
        .collect()
}

/// The service's rows of what a statement returned.
pub(crate) fn statement_rows(returned: &dbfiles::Returned) -> StatementRows {
    StatementRows {
        columns: result_columns(&returned.columns),
        rows: returned
            .rows
            .iter()
            .map(|row| row.iter().map(sql_value).collect())
            .collect(),
        cut: returned.cut.map(|cut| match cut {
            dbfiles::Cut::Rows => RowsCut::Rows,
            dbfiles::Cut::Bytes => RowsCut::Bytes,
        }),
    }
}

/// The service's structure of the engine's structure.
fn database_structure(structure: dbfiles::Structure) -> DatabaseStructure {
    DatabaseStructure {
        tables: structure
            .tables
            .into_iter()
            .map(|table| TableInfo {
                name: table.name,
                kind: match table.kind {
                    dbfiles::TableKind::Table => TableKind::Table,
                    dbfiles::TableKind::View => TableKind::View,
                    dbfiles::TableKind::Virtual => TableKind::Virtual,
                    dbfiles::TableKind::Shadow => TableKind::Shadow,
                },
                sql: table.sql,
                columns: table
                    .columns
                    .into_iter()
                    .map(|column| TableColumn {
                        name: column.name,
                        declared_type: column.declared_type,
                        not_null: column.not_null,
                        default: column.default,
                        primary_key: column.primary_key,
                        hidden: column.hidden,
                    })
                    .collect(),
                without_rowid: table.without_rowid,
                strict: table.strict,
                foreign_keys: table
                    .foreign_keys
                    .into_iter()
                    .map(|key| ForeignKey {
                        columns: key.columns,
                        parent: key.parent,
                        parent_columns: key.parent_columns,
                        on_update: key.on_update,
                        on_delete: key.on_delete,
                    })
                    .collect(),
            })
            .collect(),
        indexes: structure
            .indexes
            .into_iter()
            .map(|index| IndexInfo {
                name: index.name,
                table: index.table,
                unique: index.unique,
                automatic: index.automatic,
                partial: index.partial,
                columns: index.columns,
                sql: index.sql,
            })
            .collect(),
        triggers: structure
            .triggers
            .into_iter()
            .map(|trigger| TriggerInfo {
                name: trigger.name,
                table: trigger.table,
                sql: trigger.sql,
            })
            .collect(),
    }
}

/// The engine's cursor of a cursor of the service.
fn engine_cursor(cursor: &Cursor) -> Result<dbfiles::Cursor, SqliteFilesError> {
    Ok(match cursor {
        Cursor::After { order, key } => dbfiles::Cursor::After {
            order: order.as_ref().map(engine_value).transpose()?,
            key: engine_values(key)?,
        },
        Cursor::Offset { rows } => dbfiles::Cursor::Offset(u64::from(*rows)),
    })
}

/// The service's cursor of a cursor of the engine.
fn sdk_cursor(cursor: &dbfiles::Cursor) -> Cursor {
    match cursor {
        dbfiles::Cursor::After { order, key } => Cursor::After {
            order: order.as_ref().map(sql_value),
            key: key.iter().map(sql_value).collect(),
        },
        dbfiles::Cursor::Offset(rows) => Cursor::Offset {
            rows: u32::try_from(*rows).unwrap_or(u32::MAX),
        },
    }
}

/// The engine's request of a page.
fn engine_page_request(request: &PageRequest) -> Result<dbfiles::PageRequest, SqliteFilesError> {
    let filters = request
        .filters
        .iter()
        .map(|filter: &Filter| {
            Ok(dbfiles::Filter {
                column: filter.column.clone(),
                operator: match filter.operator {
                    FilterOperator::Equal => dbfiles::FilterOperator::Equal,
                    FilterOperator::NotEqual => dbfiles::FilterOperator::NotEqual,
                    FilterOperator::Less => dbfiles::FilterOperator::Less,
                    FilterOperator::LessOrEqual => dbfiles::FilterOperator::LessOrEqual,
                    FilterOperator::Greater => dbfiles::FilterOperator::Greater,
                    FilterOperator::GreaterOrEqual => dbfiles::FilterOperator::GreaterOrEqual,
                    FilterOperator::Like => dbfiles::FilterOperator::Like,
                    FilterOperator::IsNull => dbfiles::FilterOperator::IsNull,
                    FilterOperator::IsNotNull => dbfiles::FilterOperator::IsNotNull,
                },
                value: engine_value(&filter.value)?,
            })
        })
        .collect::<Result<Vec<_>, SqliteFilesError>>()?;
    Ok(dbfiles::PageRequest {
        table: request.table.clone(),
        order: request.order.as_ref().map(|order: &Order| dbfiles::Order {
            column: order.column.clone(),
            descending: order.descending,
        }),
        filters,
        after: request.after.as_ref().map(engine_cursor).transpose()?,
        rows: usize::try_from(request.rows.min(MAX_PAGE_ROWS)).unwrap_or(1),
    })
}

/// The service's page of a page of the engine.
fn sdk_page(page: &dbfiles::Page) -> Page {
    Page {
        key: page.key.clone(),
        columns: result_columns(&page.columns),
        rows: page
            .rows
            .iter()
            .map(|row| PageRow {
                key: row.key.iter().map(sql_value).collect(),
                values: row.values.iter().map(sql_value).collect(),
            })
            .collect(),
        next: page.next.as_ref().map(sdk_cursor),
    }
}

/// The engine's limits of a read's limits, within the largest that a read
/// may ask for.
fn engine_limits(limits: ReadLimits) -> Limits {
    Limits {
        rows: usize::try_from(limits.rows.clamp(1, MAX_READ_ROWS)).unwrap_or(1),
        bytes: usize::try_from(limits.bytes.clamp(1, MAX_READ_BYTES)).unwrap_or(1),
        time: Duration::from_millis(u64::from(
            limits.milliseconds.clamp(1, MAX_READ_MILLISECONDS),
        )),
    }
}

/// The engine's row of a row that the view showed.
fn engine_row(row: &RowMatch) -> Result<dbfiles::RowMatch, SqliteFilesError> {
    Ok(dbfiles::RowMatch {
        key: engine_values(&row.key)?,
        expected: engine_columns(&row.expected)?,
    })
}

/// The engine's columns with values.
fn engine_columns(
    columns: &[ColumnValue],
) -> Result<Vec<(String, dbfiles::Value)>, SqliteFilesError> {
    columns
        .iter()
        .map(|column| Ok((column.column.clone(), engine_value(&column.value)?)))
        .collect()
}

/// The table of a database's structure with this name.
fn table_of(
    structure: dbfiles::Structure,
    table: &str,
) -> Result<dbfiles::TableInfo, SqliteFilesError> {
    structure
        .tables
        .into_iter()
        .find(|info| info.name == table)
        .ok_or(SqliteFilesError::NoKey)
}

/// Stops a running owner operation when the caller that waits for it is
/// dropped.
struct StopWhenDropped(Option<dbfiles::Cancel>);

impl StopWhenDropped {
    /// Lets the operation go on: it ended.
    fn disarm(mut self) {
        self.0 = None;
    }
}

impl Drop for StopWhenDropped {
    fn drop(&mut self) {
        if let Some(cancel) = self.0.take() {
            cancel.stop();
        }
    }
}

/// Runs an owner's planned change on the file.
fn run_planned(
    path: &Path,
    identity: &str,
    planned: Planned,
    cancel: &dbfiles::Cancel,
) -> Result<ChangeOutcome, EngineError> {
    match planned {
        Planned::Statements { statements, class } => {
            let written = dbfiles::write(path, identity, &statements, class, WRITE_LIMITS, cancel)?;
            let changes = written.iter().map(|done| done.changes).sum();
            let rows = written
                .last()
                .and_then(|done| done.rows.as_ref().map(statement_rows));
            Ok(ChangeOutcome {
                changes,
                rowid: None,
                rows,
            })
        }
        Planned::Edit { table, edit } => {
            let edited = dbfiles::edit(path, identity, &table, &edit, WRITE_LIMITS, cancel)?;
            Ok(ChangeOutcome {
                changes: edited.changes,
                rowid: edited.rowid,
                rows: None,
            })
        }
    }
}

/// Commits the record of an owner operation as executing, before it runs.
async fn begin_operation(
    runtime: &Runtime,
    operation: &NewOperation,
) -> Result<(), SqliteFilesError> {
    runtime
        .inner
        .store
        .begin_db_operation(operation)
        .await
        .map_err(|error| match error {
            OperationError::AlreadyJournaled => SqliteFilesError::AlreadyRun,
            OperationError::NotAChange => SqliteFilesError::NotAllowed,
            OperationError::Store(error) => {
                tracing::error!(
                    error.code = error.code(),
                    "the journal of owner operations failed"
                );
                SqliteFilesError::Failed
            }
        })
}

/// Records how an owner operation ended, and returns its outcome. A record
/// that fails to change stays executing, which the next start marks
/// uncertain.
async fn record_outcome(
    runtime: &Runtime,
    id: &DatabaseOperationId,
    ran: Result<ChangeOutcome, EngineError>,
) -> Result<ChangeOutcome, SqliteFilesError> {
    let store = &runtime.inner.store;
    match ran {
        Ok(outcome) => {
            if let Err(error) = store.complete_db_operation(id, outcome.changes).await {
                tracing::error!(
                    error.code = error.code(),
                    "an owner operation completed and its record did not"
                );
            }
            Ok(outcome)
        }
        Err(error) => {
            let failure = engine_error(error);
            if let Err(error) = store.fail_db_operation(id, failure.code()).await {
                tracing::error!(
                    error.code = error.code(),
                    "an owner operation failed and its record did not"
                );
            }
            Err(failure)
        }
    }
}

/// What an owner's change runs: statements of a class, or an edit of a
/// table's rows, with the text and the parameters that the journal records.
enum Planned {
    Statements {
        statements: Vec<dbfiles::Sql>,
        class: StatementClass,
    },
    Edit {
        table: Box<dbfiles::TableInfo>,
        edit: dbfiles::Edit,
    },
}

impl SqliteFilesApi for BoundFiles {
    fn choose(
        &self,
        request: ChooseRequest,
    ) -> BoxFuture<'static, Result<Option<DatabaseFile>, SqliteFilesError>> {
        let bound = self.clone();
        async move {
            let chooser = bound.chooser.clone().ok_or(SqliteFilesError::NoDialog)?;
            let name = request.name.as_deref().map(checked_name).transpose()?;
            let read_only = request.read_only;
            let filters = DatabaseFilters {
                databases: checked_filter(&request.databases_filter)?,
                all_files: checked_filter(&request.all_files_filter)?,
            };
            let Some(path) = chooser.choose(filters).await else {
                return Ok(None);
            };
            let protected = bound.protected();
            let checked =
                tokio::task::spawn_blocking(move || dbfiles::check_new(&path, &protected))
                    .await
                    .map_err(|_join_error| SqliteFilesError::Failed)?
                    .map_err(file_error)?;
            let name = name.map_or_else(|| unnamed(&checked.path), Ok)?;
            let registration = bound
                .runtime
                .inner
                .store
                .add_db_file(
                    &bound.plugin,
                    NewRegistration {
                        key: bound.runtime.inner.ids.database_key(),
                        name,
                        path: checked.path,
                        identity: checked.identity,
                        read_only,
                    },
                )
                .await
                .map_err(registry_error)?;
            Ok(Some(database_file(&registration, FileState::Available)))
        }
        .boxed()
    }

    fn files(&self) -> BoxFuture<'static, Result<Vec<DatabaseFile>, SqliteFilesError>> {
        let bound = self.clone();
        async move {
            let registrations = bound
                .runtime
                .inner
                .store
                .db_files(&bound.plugin)
                .await
                .map_err(registry_error)?;
            tokio::task::spawn_blocking(move || {
                registrations
                    .iter()
                    .map(|registration| {
                        database_file(
                            registration,
                            file_state(&registration.path, &registration.identity),
                        )
                    })
                    .collect()
            })
            .await
            .map_err(|_join_error| SqliteFilesError::Failed)
        }
        .boxed()
    }

    fn rename(
        &self,
        key: DatabaseKey,
        name: String,
    ) -> BoxFuture<'static, Result<(), SqliteFilesError>> {
        let bound = self.clone();
        async move {
            let name = checked_name(&name)?;
            bound
                .runtime
                .inner
                .store
                .rename_db_file(&bound.plugin, &key, &name)
                .await
                .map_err(registry_error)
        }
        .boxed()
    }

    fn mark(
        &self,
        key: DatabaseKey,
        read_only: bool,
    ) -> BoxFuture<'static, Result<(), SqliteFilesError>> {
        let bound = self.clone();
        async move {
            bound
                .runtime
                .inner
                .store
                .mark_db_file(&bound.plugin, &key, read_only)
                .await
                .map_err(registry_error)
        }
        .boxed()
    }

    fn accept(
        &self,
        key: DatabaseKey,
    ) -> BoxFuture<'static, Result<DatabaseFile, SqliteFilesError>> {
        let bound = self.clone();
        async move {
            let registration = bound.registration(&key).await?;
            let protected = bound.protected();
            let path = registration.path.clone();
            let checked =
                tokio::task::spawn_blocking(move || dbfiles::check_new(&path, &protected))
                    .await
                    .map_err(|_join_error| SqliteFilesError::Failed)?
                    .map_err(file_error)?;
            // The registration keeps its path: a file that canonicalizes to
            // another place now came through a link, which the checks refuse.
            if checked.path != registration.path {
                return Err(SqliteFilesError::FileRefused {
                    reason: FileRefusal::Link,
                });
            }
            bound
                .runtime
                .inner
                .store
                .accept_db_file(&bound.plugin, &key, &checked.identity)
                .await
                .map_err(registry_error)?;
            let accepted = Registration {
                identity: checked.identity,
                ..registration
            };
            Ok(database_file(&accepted, FileState::Available))
        }
        .boxed()
    }

    fn forget(&self, key: DatabaseKey) -> BoxFuture<'static, Result<(), SqliteFilesError>> {
        let bound = self.clone();
        async move {
            bound
                .runtime
                .inner
                .store
                .forget_db_file(&bound.plugin, &key)
                .await
                .map_err(registry_error)
        }
        .boxed()
    }

    fn structure(
        &self,
        key: DatabaseKey,
    ) -> BoxFuture<'static, Result<DatabaseStructure, SqliteFilesError>> {
        let bound = self.clone();
        async move {
            let registration = bound.registration(&key).await?;
            let read = dbfiles::run(move |cancel| {
                dbfiles::structure(
                    &registration.path,
                    &registration.identity,
                    STRUCTURE_TIME,
                    cancel,
                )
            })
            .await
            .map_err(engine_error)?;
            Ok(database_structure(read))
        }
        .boxed()
    }

    fn count(
        &self,
        key: DatabaseKey,
        table: String,
    ) -> BoxFuture<'static, Result<u64, SqliteFilesError>> {
        let bound = self.clone();
        async move {
            let registration = bound.registration(&key).await?;
            dbfiles::run(move |cancel| {
                dbfiles::count(
                    &registration.path,
                    &registration.identity,
                    &table,
                    COUNT_LIMITS,
                    cancel,
                )
            })
            .await
            .map_err(engine_error)
        }
        .boxed()
    }

    fn page(
        &self,
        key: DatabaseKey,
        request: PageRequest,
    ) -> BoxFuture<'static, Result<Page, SqliteFilesError>> {
        let bound = self.clone();
        async move {
            let registration = bound.registration(&key).await?;
            let request = engine_page_request(&request)?;
            let page = dbfiles::run(move |cancel| {
                let structure = dbfiles::structure(
                    &registration.path,
                    &registration.identity,
                    STRUCTURE_TIME,
                    cancel,
                )?;
                let table = structure
                    .tables
                    .into_iter()
                    .find(|table| table.name == request.table)
                    .ok_or(EngineError::NoKey)?;
                dbfiles::page(
                    &registration.path,
                    &registration.identity,
                    &table,
                    &request,
                    PAGE_LIMITS,
                    cancel,
                )
            })
            .await
            .map_err(engine_error)?;
            Ok(sdk_page(&page))
        }
        .boxed()
    }

    fn classify(
        &self,
        key: DatabaseKey,
        statement: String,
    ) -> BoxFuture<'static, Result<StatementClass, SqliteFilesError>> {
        let bound = self.clone();
        async move {
            let registration = bound.registration(&key).await?;
            dbfiles::run(move |_cancel| {
                dbfiles::classify(&registration.path, &registration.identity, &statement)
            })
            .await
            .map_err(engine_error)
        }
        .boxed()
    }

    fn read(
        &self,
        key: DatabaseKey,
        statement: Statement,
        limits: ReadLimits,
    ) -> BoxFuture<'static, Result<StatementRows, SqliteFilesError>> {
        let bound = self.clone();
        async move {
            let registration = bound.registration(&key).await?;
            let sql = engine_sql(&statement)?;
            let limits = engine_limits(limits);
            let returned = dbfiles::run(move |cancel| {
                dbfiles::read(
                    &registration.path,
                    &registration.identity,
                    &sql,
                    limits,
                    cancel,
                )
            })
            .await
            .map_err(engine_error)?;
            Ok(statement_rows(&returned))
        }
        .boxed()
    }

    fn change(
        &self,
        key: DatabaseKey,
        change: OwnerChange,
    ) -> BoxFuture<'static, Result<ChangeOutcome, SqliteFilesError>> {
        let bound = self.clone();
        async move { bound.run_owner_change(key, change).await }.boxed()
    }

    fn write(
        &self,
        ticket: DatabaseTicket,
        key: DatabaseKey,
        statements: Vec<Statement>,
    ) -> BoxFuture<'static, Result<Vec<StatementOutcome>, SqliteFilesError>> {
        let bound = self.clone();
        async move { bound.run_tool_write(&ticket, key, statements).await }.boxed()
    }
}

impl BoundFiles {
    /// Plans an owner's change: its statements of their class, refusing a
    /// read and a change of the schema that the owner has not confirmed, or
    /// an edit of a table's rows, which writes.
    async fn plan(
        &self,
        registration: &Registration,
        change: &Change,
    ) -> Result<Planned, SqliteFilesError> {
        let path = registration.path.clone();
        let identity = registration.identity.clone();
        match change {
            Change::Statements {
                statements,
                schema_confirmed,
            } => {
                if statements.is_empty() || statements.len() > MAX_STATEMENTS {
                    return Err(SqliteFilesError::TooManyStatements);
                }
                let sqls = statements
                    .iter()
                    .map(engine_sql)
                    .collect::<Result<Vec<_>, _>>()?;
                let texts: Vec<String> = sqls.iter().map(|sql| sql.text.clone()).collect();
                let classes = dbfiles::run(move |_cancel| {
                    texts
                        .iter()
                        .map(|text| dbfiles::classify(&path, &identity, text))
                        .collect::<Result<Vec<_>, _>>()
                })
                .await
                .map_err(engine_error)?;
                if let Some(read) = classes.iter().find(|class| **class == StatementClass::Read) {
                    return Err(SqliteFilesError::NotARead { class: *read });
                }
                let class = classes.into_iter().max().unwrap_or(StatementClass::Write);
                if class == StatementClass::Schema && !schema_confirmed {
                    return Err(SqliteFilesError::SchemaUnconfirmed);
                }
                Ok(Planned::Statements {
                    statements: sqls,
                    class,
                })
            }
            Change::UpdateRow { table, row, set } => {
                let edit = dbfiles::Edit::Update {
                    row: engine_row(row)?,
                    set: engine_columns(set)?,
                };
                self.planned_edit(path, identity, table.clone(), edit).await
            }
            Change::InsertRow { table, values } => {
                let edit = dbfiles::Edit::Insert {
                    values: engine_columns(values)?,
                };
                self.planned_edit(path, identity, table.clone(), edit).await
            }
            Change::DeleteRows { table, rows } => {
                let edit = dbfiles::Edit::Delete {
                    rows: rows.iter().map(engine_row).collect::<Result<Vec<_>, _>>()?,
                };
                self.planned_edit(path, identity, table.clone(), edit).await
            }
        }
    }

    /// Plans an edit of a table's rows, with the table's structure.
    async fn planned_edit(
        &self,
        path: PathBuf,
        identity: String,
        table: String,
        edit: dbfiles::Edit,
    ) -> Result<Planned, SqliteFilesError> {
        let structure = dbfiles::run(move |cancel| {
            dbfiles::structure(&path, &identity, STRUCTURE_TIME, cancel)
        })
        .await
        .map_err(engine_error)?;
        let table = table_of(structure, &table)?;
        Ok(Planned::Edit {
            table: Box::new(table),
            edit,
        })
    }

    /// Runs the owner's change as an owner operation: the journal records it
    /// before it runs, with the statement's text and the digest of its
    /// values, and records its outcome after.
    async fn run_owner_change(
        &self,
        key: DatabaseKey,
        change: OwnerChange,
    ) -> Result<ChangeOutcome, SqliteFilesError> {
        let registration = self.registration(&key).await?;
        if registration.read_only {
            return Err(SqliteFilesError::MarkedReadOnly);
        }
        let planned = self.plan(&registration, &change.change).await?;
        let (statement, class) = match &planned {
            Planned::Statements { statements, class } => (
                statements
                    .iter()
                    .map(|sql| sql.text.as_str())
                    .collect::<Vec<_>>()
                    .join(";\n"),
                *class,
            ),
            Planned::Edit { table, edit } => (
                dbfiles::edit_text(table, edit).map_err(engine_error)?,
                StatementClass::Write,
            ),
        };
        let digest = parameters_digest(&change_parameters(&change.change));
        let operation = NewOperation {
            id: change.id.clone(),
            plugin: self.plugin.clone(),
            key: key.clone(),
            statement,
            parameters_digest: digest,
            class,
        };
        // The operation begins its record, runs and records how it ended in
        // a task of its own, which a caller that stops waiting, as when the
        // view closes, does not drop, so no record stays executing while
        // Mannie runs. The caller's drop stops the statement instead, and the
        // record says whether the change committed before it stopped.
        let cancel = dbfiles::Cancel::new();
        let running = cancel.clone();
        let runtime = self.runtime.clone();
        let finished = tokio::spawn(async move {
            begin_operation(&runtime, &operation).await?;
            let ran = dbfiles::run_with(running, move |cancel| {
                run_planned(&registration.path, &registration.identity, planned, cancel)
            })
            .await;
            record_outcome(&runtime, &operation.id, ran).await
        });
        let stop = StopWhenDropped(Some(cancel));
        let result = finished.await.unwrap_or_else(|_join_error| {
            tracing::error!("the task of an owner operation failed to finish");
            Err(SqliteFilesError::Failed)
        });
        stop.disarm();
        result
    }

    /// Takes the writes of one database out of a ticket of the bound
    /// contributor, so each runs once; the ticket ends with its last
    /// database's writes, or with its call.
    fn take_writes(
        &self,
        ticket: &DatabaseTicket,
        key: &DatabaseKey,
    ) -> Result<Vec<AuthorizedWrite>, SqliteFilesError> {
        let mut tickets = lock(&self.runtime.inner.database_tickets.tickets);
        let Some(entry) = tickets.get_mut(ticket.as_str()) else {
            return Err(SqliteFilesError::NotAllowed);
        };
        if entry.contributor != self.contributor {
            return Err(SqliteFilesError::NotAllowed);
        }
        let (allowed, rest): (Vec<AuthorizedWrite>, Vec<AuthorizedWrite>) =
            std::mem::take(&mut entry.writes)
                .into_iter()
                .partition(|write| write.key == *key);
        entry.writes = rest;
        if entry.writes.is_empty() {
            tickets.remove(ticket.as_str());
        }
        Ok(allowed)
    }

    /// Runs the statements on one database that a tool's call's ticket
    /// allowed, exactly as they were allowed and in their order.
    async fn run_tool_write(
        &self,
        ticket: &DatabaseTicket,
        key: DatabaseKey,
        statements: Vec<Statement>,
    ) -> Result<Vec<StatementOutcome>, SqliteFilesError> {
        let allowed = self.take_writes(ticket, &key)?;
        let matches = !allowed.is_empty()
            && allowed.len() == statements.len()
            && allowed.iter().zip(&statements).all(|(write, statement)| {
                write.statement == statement.text
                    && write.parameters_digest == parameters_digest(&statement.parameters)
            });
        if !matches {
            return Err(SqliteFilesError::NotAllowed);
        }
        let registration = self.registration(&key).await?;
        if registration.read_only {
            return Err(SqliteFilesError::MarkedReadOnly);
        }
        // The file that the owner allowed the writes on is still the
        // registered one; a replaced file goes back to the policy.
        if allowed
            .iter()
            .any(|write| write.identity != registration.identity)
        {
            return Err(SqliteFilesError::FileChanged);
        }
        let class = allowed
            .iter()
            .map(|write| write.class)
            .max()
            .unwrap_or(StatementClass::Write);
        let sqls = statements
            .iter()
            .map(engine_sql)
            .collect::<Result<Vec<_>, _>>()?;
        let written = dbfiles::run(move |cancel| {
            dbfiles::write(
                &registration.path,
                &registration.identity,
                &sqls,
                class,
                WRITE_LIMITS,
                cancel,
            )
        })
        .await
        .map_err(engine_error)?;
        Ok(written
            .iter()
            .map(|done| StatementOutcome {
                changes: done.changes,
                rows: done.rows.as_ref().map(statement_rows),
            })
            .collect())
    }
}

/// The values of a change in order, which the journal's digest covers.
fn change_parameters(change: &Change) -> Vec<SqlValue> {
    let columns = |columns: &[ColumnValue]| -> Vec<SqlValue> {
        columns.iter().map(|column| column.value.clone()).collect()
    };
    let row = |row: &RowMatch| -> Vec<SqlValue> {
        row.key
            .iter()
            .cloned()
            .chain(columns(&row.expected))
            .collect()
    };
    match change {
        Change::Statements { statements, .. } => statements
            .iter()
            .flat_map(|statement| statement.parameters.iter().cloned())
            .collect(),
        Change::UpdateRow {
            row: matched, set, ..
        } => columns(set).into_iter().chain(row(matched)).collect(),
        Change::InsertRow { values, .. } => columns(values),
        Change::DeleteRows { rows, .. } => rows.iter().flat_map(row).collect(),
    }
}
