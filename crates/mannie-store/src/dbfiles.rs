//! The owner's SQLite files (ADR-0056): the engine that opens a file that
//! the owner registered with hardened connections of its own, apart from
//! Mannie's database, its writer and its readers; the core's registrations
//! of this computer's files, scoped to their plugin; and the journal of the
//! owner's own changes, the owner operations. The module's tables are named
//! `dbfiles_*`, a prefix that no plugin may take. The runtime provides the
//! service `sqlite-files` on it, and `mannie-host-kit` offers it to the
//! programs of plugins, such as the worker.
//!
//! The engine's calls block on SQLite. [`run`] runs one on tokio's threads
//! for blocking work and interrupts its statement when its future is
//! dropped, as when a reply stops or a view closes.

mod authorizer;
mod browse;
mod edits;
mod engine;
mod files;
mod operations;
mod registry;
mod structure;
#[cfg(test)]
mod tests;
mod values;

pub use browse::{
    Cursor, Filter, FilterOperator, MAX_PAGE_ROWS, Order, Page, PageRequest, PageRow, page,
};
pub use edits::{Edit, Edited, RowMatch, edit, edit_text};
pub use engine::{
    Cancel, Column, Cut, EngineError, Limits, MAX_BATCH_STATEMENTS, Returned, Sql, Written,
    classify, classify_all_on_schema, classify_on_schema, count, read, write,
};
pub use files::{CheckedFile, FileRefusal, check_new, check_registered};
pub use operations::{MAX_STATEMENT_CHARS, NewOperation, OperationError, OperationState};
pub use registry::{MAX_REGISTRATIONS, NewRegistration, Registration, RegistryError};
pub use structure::{
    ColumnInfo, ForeignKeyInfo, IndexInfo, Structure, TableInfo, TableKind, TriggerInfo, structure,
};
pub use values::Value;

/// Every statement of the module's repositories, so a test can prepare each
/// one against the migrated schema.
#[cfg(test)]
pub(crate) fn statements() -> impl Iterator<Item = &'static str> {
    registry::STATEMENTS
        .iter()
        .chain(operations::STATEMENTS)
        .copied()
}

/// Stops a call of the engine when the future that waits for it is dropped.
struct StopOnDrop(Option<Cancel>);

impl StopOnDrop {
    /// Lets the call go on: it ended.
    fn disarm(mut self) {
        self.0 = None;
    }
}

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        if let Some(cancel) = self.0.take() {
            cancel.stop();
        }
    }
}

/// Creates the SQLite file `path` with the tables and rows of `sql`, for the
/// tests of the runtime and of plugins, which never reach SQLite themselves
/// (ADR-0056, decision 1). The engine never creates a file.
///
/// # Errors
///
/// When a file is at `path` already, or the file cannot be created, or `sql`
/// fails.
#[cfg(any(test, feature = "testing"))]
pub fn create_for_tests(path: &std::path::Path, sql: &str) -> Result<(), crate::StoreError> {
    use rusqlite::{Connection, OpenFlags};

    let failed = |source: rusqlite::Error| crate::StoreError::Open {
        path: path.to_path_buf(),
        source: crate::error::DatabaseError(source),
    };
    if path.exists() {
        return Err(failed(rusqlite::Error::InvalidPath(path.to_path_buf())));
    }
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .map_err(failed)?;
    connection.execute_batch(sql).map_err(failed)
}

/// Runs a blocking call of the engine, such as [`read`] or [`write`], on
/// tokio's threads for blocking work. Dropping the returned future stops the
/// call's statement within milliseconds (spike S3), and the call returns
/// [`EngineError::Stopped`] to no one.
pub async fn run<T, F>(work: F) -> Result<T, EngineError>
where
    T: Send + 'static,
    F: FnOnce(&Cancel) -> Result<T, EngineError> + Send + 'static,
{
    let cancel = Cancel::new();
    let stop = StopOnDrop(Some(cancel.clone()));
    let result = run_with(cancel, work).await;
    stop.disarm();
    result
}

/// Runs a blocking call of the engine as [`run`] does, with the caller's
/// cancel handle, which stops the call's statement when the caller stops
/// it. Dropping the returned future lets the call go on, so a caller that
/// must learn how a write ended, such as the journal of owner operations,
/// waits for it in a task of its own and stops it through the handle.
pub async fn run_with<T, F>(cancel: Cancel, work: F) -> Result<T, EngineError>
where
    T: Send + 'static,
    F: FnOnce(&Cancel) -> Result<T, EngineError> + Send + 'static,
{
    tokio::task::spawn_blocking(move || work(&cancel))
        .await
        .unwrap_or_else(|error| {
            tracing::error!(%error, "a call of the database engine failed to finish");
            Err(EngineError::Failed)
        })
}
