//! The journal of owner operations on registered databases (ADR-0056,
//! decision 8): the owner's own changes from a plugin's view. The service
//! commits an operation's record as executing, with full durability, before
//! the write runs, and records its outcome after it. Recovery at start marks
//! an operation that a crash cut off uncertain, and an operation with a
//! record never runs again, as the export journal works (ADR-0040,
//! decision 6). The record keeps the statement's text and a digest of its
//! parameters, never their values.

use mannie_types::ids::{DatabaseKey, DatabaseOperationId};
use mannie_types::tools::StatementClass;
use rusqlite::{OptionalExtension, params};

use crate::error::StoreError;
use crate::writer::TransactError;
use crate::{Durability, Store};

/// The most characters of a statement's text that a record keeps; a longer
/// text is cut, and its record says so.
pub const MAX_STATEMENT_CHARS: usize = 16_384;

const INSERT: &str = "INSERT INTO dbfiles_operations \
     (id, plugin, key, statement, statement_cut, parameters_digest, class, state, created_at, updated_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'executing', ?8, ?8)";
const SELECT_EXISTS: &str = "SELECT 1 FROM dbfiles_operations WHERE id = ?1";
const COMPLETE: &str = "UPDATE dbfiles_operations SET state = 'completed', changes = ?2, updated_at = ?3 \
     WHERE id = ?1 AND state = 'executing'";
const FAIL: &str = "UPDATE dbfiles_operations SET state = 'failed', failure = ?2, updated_at = ?3 \
     WHERE id = ?1 AND state = 'executing'";
const RECOVER: &str = "UPDATE dbfiles_operations SET state = 'uncertain', updated_at = ?1 \
     WHERE state = 'executing'";
const UNCERTAIN: &str = "UPDATE dbfiles_operations SET state = 'uncertain', updated_at = ?2 \
     WHERE id = ?1 AND state = 'executing'";
const SELECT_STATE: &str = "SELECT state FROM dbfiles_operations WHERE id = ?1";

#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    INSERT,
    SELECT_EXISTS,
    COMPLETE,
    FAIL,
    RECOVER,
    UNCERTAIN,
    SELECT_STATE,
];

/// An owner operation before it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOperation {
    /// The operation's ID, which the view's request carries, so a request
    /// that arrives twice runs once.
    pub id: DatabaseOperationId,
    /// The plugin whose view asked for it.
    pub plugin: String,
    /// The database's key.
    pub key: DatabaseKey,
    /// The text of its statements.
    pub statement: String,
    /// The digest of its parameters' values.
    pub parameters_digest: String,
    /// What it does: a write or a schema change.
    pub class: StatementClass,
}

/// Where an owner operation is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationState {
    /// It began and has no outcome yet.
    Executing,
    /// It completed.
    Completed,
    /// It failed without a change.
    Failed,
    /// Mannie stopped while it ran, so it may have taken effect.
    Uncertain,
}

/// Why an operation could not begin.
#[derive(Debug, thiserror::Error)]
pub enum OperationError {
    /// The journal has a record of an operation with this ID: it ran, or
    /// may have run, and never runs again.
    #[error("the operation has run already")]
    AlreadyJournaled,
    /// A read is no operation.
    #[error("a read is no operation")]
    NotAChange,
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl From<TransactError<Self>> for OperationError {
    fn from(error: TransactError<Self>) -> Self {
        match error {
            TransactError::Rejected(error) => error,
            TransactError::Store(error) => Self::Store(error),
        }
    }
}

/// The class of an operation as its record names it.
fn class_text(class: StatementClass) -> Option<&'static str> {
    match class {
        StatementClass::Read => None,
        StatementClass::Write => Some("write"),
        StatementClass::Schema => Some("schema"),
    }
}

/// A statement's text cut to [`MAX_STATEMENT_CHARS`], and whether it was.
fn cut_statement(text: &str) -> (String, bool) {
    match text.char_indices().nth(MAX_STATEMENT_CHARS) {
        Some((end, _)) => (text[..end].to_owned(), true),
        None => (text.to_owned(), false),
    }
}

impl Store {
    /// Commits the record of an owner operation as executing, before its
    /// write runs. An ID that the journal has refuses the operation.
    pub async fn begin_db_operation(&self, operation: &NewOperation) -> Result<(), OperationError> {
        let class = class_text(operation.class).ok_or(OperationError::NotAChange)?;
        let (statement, cut) = cut_statement(&operation.statement);
        let id = operation.id.to_string();
        let plugin = operation.plugin.clone();
        let key = operation.key.to_string();
        let digest = operation.parameters_digest.clone();
        let now = self.clock().now().as_millis();
        self.transact(Durability::Full, move |transaction| {
            let exists = transaction
                .query_row(SELECT_EXISTS, [&id], |_row| Ok(()))
                .optional()?
                .is_some();
            if exists {
                return Err(TransactError::Rejected(OperationError::AlreadyJournaled));
            }
            transaction.execute(
                INSERT,
                params![
                    id,
                    plugin,
                    key,
                    statement,
                    i64::from(cut),
                    digest,
                    class,
                    now
                ],
            )?;
            Ok(())
        })
        .await
        .map_err(OperationError::from)
    }

    /// Records that an owner operation completed, with the rows it changed.
    pub async fn complete_db_operation(
        &self,
        id: &DatabaseOperationId,
        changes: u64,
    ) -> Result<(), StoreError> {
        let id = id.to_string();
        let changes = i64::try_from(changes).unwrap_or(i64::MAX);
        let now = self.clock().now().as_millis();
        self.write(Durability::Full, move |transaction| {
            transaction.execute(COMPLETE, params![id, changes, now])?;
            Ok(())
        })
        .await
    }

    /// Records that an owner operation failed without a change, with the
    /// stable code of its failure.
    pub async fn fail_db_operation(
        &self,
        id: &DatabaseOperationId,
        code: &str,
    ) -> Result<(), StoreError> {
        let (id, code) = (id.to_string(), code.to_owned());
        let now = self.clock().now().as_millis();
        self.write(Durability::Full, move |transaction| {
            transaction.execute(FAIL, params![id, code, now])?;
            Ok(())
        })
        .await
    }

    /// Records that an owner operation may have taken effect without its
    /// outcome reaching Mannie, such as a write on a worker's database whose
    /// answer the link lost (ADR-0057, decision 8). It never runs again.
    pub async fn mark_db_operation_uncertain(
        &self,
        id: &DatabaseOperationId,
    ) -> Result<(), StoreError> {
        let id = id.to_string();
        let now = self.clock().now().as_millis();
        self.write(Durability::Full, move |transaction| {
            transaction.execute(UNCERTAIN, params![id, now])?;
            Ok(())
        })
        .await
    }

    /// Marks the owner operations that a stop cut off uncertain, and
    /// returns how many there were. Nothing runs again.
    pub async fn recover_db_operations(&self) -> Result<usize, StoreError> {
        let now = self.clock().now().as_millis();
        self.write(Durability::Full, move |transaction| {
            transaction.execute(RECOVER, [now])
        })
        .await
    }

    /// Where an owner operation is in its life, or `None` for an ID that
    /// the journal does not have.
    pub async fn db_operation_state(
        &self,
        id: &DatabaseOperationId,
    ) -> Result<Option<OperationState>, StoreError> {
        let id = id.to_string();
        let state: Option<String> = self
            .read(move |connection| {
                connection
                    .query_row(SELECT_STATE, [id], |row| row.get(0))
                    .optional()
            })
            .await?;
        Ok(state.map(|state| match state.as_str() {
            "completed" => OperationState::Completed,
            "failed" => OperationState::Failed,
            "uncertain" => OperationState::Uncertain,
            _executing => OperationState::Executing,
        }))
    }
}
