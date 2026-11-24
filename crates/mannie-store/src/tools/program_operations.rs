//! The journal of owner-initiated runs of installed programs from plugin views
//! (ADR-0078, decision 1). Each record commits as executing with full
//! durability before any process starts, keeps a digest of the operation's
//! arguments rather than their text, and never runs twice under the same ID.
//! Startup recovery marks any executing record uncertain.

use mannie_types::ids::{ProgramOperationId, ProjectId};
use rusqlite::{OptionalExtension, params};

use crate::error::StoreError;
use crate::writer::TransactError;
use crate::{Durability, Store};

const INSERT: &str = "INSERT INTO tools_program_operations \
     (id, plugin, project_id, program, action, arguments_digest, effects, state, created_at, updated_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'executing', ?8, ?8)";
const SELECT_EXISTS: &str = "SELECT 1 FROM tools_program_operations WHERE id = ?1";
const COMPLETE: &str = "UPDATE tools_program_operations SET state = 'completed', exit_code = ?2, updated_at = ?3 \
     WHERE id = ?1 AND state = 'executing'";
const FAIL: &str = "UPDATE tools_program_operations SET state = 'failed', failure = ?2, updated_at = ?3 \
     WHERE id = ?1 AND state = 'executing'";
const UNCERTAIN: &str = "UPDATE tools_program_operations SET state = 'uncertain', updated_at = ?2 \
     WHERE id = ?1 AND state = 'executing'";
const RECOVER: &str = "UPDATE tools_program_operations SET state = 'uncertain', updated_at = ?1 \
     WHERE state = 'executing'";
const SELECT_STATE: &str = "SELECT state FROM tools_program_operations WHERE id = ?1";

#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    INSERT,
    SELECT_EXISTS,
    COMPLETE,
    FAIL,
    UNCERTAIN,
    RECOVER,
    SELECT_STATE,
];

/// An owner-initiated run of an installed program before it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewProgramOperation {
    /// The operation's ID, chosen per owner action so a repeated submission
    /// runs once.
    pub id: ProgramOperationId,
    /// The plugin whose view requested the action.
    pub plugin: String,
    /// The project the action targets.
    pub project_id: ProjectId,
    /// The declared program name from the plugin's manifest, such as `git`.
    pub program: String,
    /// A short stable action code, such as `commit` or `stage`.
    pub action: String,
    /// SHA-256 hex digest of the canonical arguments, never the raw text.
    pub arguments_digest: String,
    /// JSON encoding of redacted `ProgramEffect` values.
    pub effects: String,
}

/// Where an owner program operation stands in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramOperationState {
    /// It began and has no recorded outcome yet.
    Executing,
    /// It completed.
    Completed,
    /// It failed before completing.
    Failed,
    /// Mannie stopped or the operation was canceled while running.
    Uncertain,
}

/// Why an owner program operation could not begin.
#[derive(Debug, thiserror::Error)]
pub enum ProgramOperationError {
    /// The journal already holds an operation with this ID.
    #[error("the program operation has run already")]
    AlreadyJournaled,
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl From<TransactError<Self>> for ProgramOperationError {
    fn from(error: TransactError<Self>) -> Self {
        match error {
            TransactError::Rejected(error) => error,
            TransactError::Store(error) => Self::Store(error),
        }
    }
}

impl Store {
    /// Commits an owner program operation record as executing with full
    /// durability before the program starts. A duplicate ID is rejected.
    pub async fn begin_program_operation(
        &self,
        operation: &NewProgramOperation,
    ) -> Result<(), ProgramOperationError> {
        let id = operation.id.to_string();
        let plugin = operation.plugin.clone();
        let project_id = operation.project_id.to_string();
        let program = operation.program.clone();
        let action = operation.action.clone();
        let arguments_digest = operation.arguments_digest.clone();
        let effects = operation.effects.clone();
        let now = self.clock().now().as_millis();

        self.transact(Durability::Full, move |transaction| {
            let exists = transaction
                .query_row(SELECT_EXISTS, [&id], |_row| Ok(()))
                .optional()?
                .is_some();

            if exists {
                return Err(TransactError::Rejected(
                    ProgramOperationError::AlreadyJournaled,
                ));
            }

            transaction.execute(
                INSERT,
                params![
                    id,
                    plugin,
                    project_id,
                    program,
                    action,
                    arguments_digest,
                    effects,
                    now
                ],
            )?;
            Ok(())
        })
        .await
        .map_err(ProgramOperationError::from)
    }

    /// Records that an owner program operation completed.
    pub async fn complete_program_operation(
        &self,
        id: &ProgramOperationId,
        exit_code: Option<i32>,
    ) -> Result<(), StoreError> {
        let id = id.to_string();
        let exit_code = exit_code.map(i64::from);
        let now = self.clock().now().as_millis();

        self.write(Durability::Full, move |transaction| {
            transaction.execute(COMPLETE, params![id, exit_code, now])?;
            Ok(())
        })
        .await
    }

    /// Records that an owner program operation failed with a stable code.
    pub async fn fail_program_operation(
        &self,
        id: &ProgramOperationId,
        code: &str,
    ) -> Result<(), StoreError> {
        let id = id.to_string();
        let code = code.to_owned();
        let now = self.clock().now().as_millis();

        self.write(Durability::Full, move |transaction| {
            transaction.execute(FAIL, params![id, code, now])?;
            Ok(())
        })
        .await
    }

    /// Records that an owner program operation ended without a confirmed
    /// outcome. It never runs again.
    pub async fn mark_program_operation_uncertain(
        &self,
        id: &ProgramOperationId,
    ) -> Result<(), StoreError> {
        let id = id.to_string();
        let now = self.clock().now().as_millis();

        self.write(Durability::Full, move |transaction| {
            transaction.execute(UNCERTAIN, params![id, now])?;
            Ok(())
        })
        .await
    }

    /// Marks owner program operations that were executing when Mannie ended as
    /// uncertain and returns how many were recovered.
    pub async fn recover_program_operations(&self) -> Result<usize, StoreError> {
        let now = self.clock().now().as_millis();

        self.write(Durability::Full, move |transaction| {
            transaction.execute(RECOVER, [now])
        })
        .await
    }

    /// The current state of an owner program operation, or `None` if unknown.
    pub async fn program_operation_state(
        &self,
        id: &ProgramOperationId,
    ) -> Result<Option<ProgramOperationState>, StoreError> {
        let id = id.to_string();
        let state: Option<String> = self
            .read(move |connection| {
                connection
                    .query_row(SELECT_STATE, [id], |row| row.get(0))
                    .optional()
            })
            .await?;

        Ok(state.map(|state| match state.as_str() {
            "completed" => ProgramOperationState::Completed,
            "failed" => ProgramOperationState::Failed,
            "uncertain" => ProgramOperationState::Uncertain,
            _executing => ProgramOperationState::Executing,
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mannie_types::clock::SystemClock;
    use mannie_types::ids::{ProgramOperationId, ProjectId};

    use super::{NewProgramOperation, ProgramOperationError, ProgramOperationState};
    use crate::{Store, StoreOptions};

    fn operation(id: &str) -> NewProgramOperation {
        NewProgramOperation {
            id: ProgramOperationId::parse(id).unwrap(),
            plugin: "git".to_owned(),
            project_id: ProjectId::parse("0199e0f2-0000-7000-8000-000000000010").unwrap(),
            program: "git".to_owned(),
            action: "commit".to_owned(),
            arguments_digest: "abc123".to_owned(),
            effects: "[]".to_owned(),
        }
    }

    #[tokio::test]
    async fn owner_program_operations_record_outcomes_reject_duplicates_and_recover() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(
            &directory.path().join("test.sqlite"),
            &StoreOptions::new(directory.path().join("backups"), Arc::new(SystemClock)),
        )
        .unwrap();

        let completed = operation("0199e0f2-0000-7000-8000-000000000301");
        let failed = operation("0199e0f2-0000-7000-8000-000000000302");
        let interrupted = operation("0199e0f2-0000-7000-8000-000000000303");

        store.begin_program_operation(&completed).await.unwrap();
        assert!(matches!(
            store.begin_program_operation(&completed).await,
            Err(ProgramOperationError::AlreadyJournaled)
        ));
        store
            .complete_program_operation(&completed.id, Some(0))
            .await
            .unwrap();

        store.begin_program_operation(&failed).await.unwrap();
        store
            .fail_program_operation(&failed.id, "git_failed")
            .await
            .unwrap();

        store.begin_program_operation(&interrupted).await.unwrap();
        assert_eq!(store.recover_program_operations().await.unwrap(), 1);
        assert_eq!(store.recover_program_operations().await.unwrap(), 0);

        assert_eq!(
            store.program_operation_state(&completed.id).await.unwrap(),
            Some(ProgramOperationState::Completed)
        );
        assert_eq!(
            store.program_operation_state(&failed.id).await.unwrap(),
            Some(ProgramOperationState::Failed)
        );
        assert_eq!(
            store
                .program_operation_state(&interrupted.id)
                .await
                .unwrap(),
            Some(ProgramOperationState::Uncertain)
        );
    }
}
