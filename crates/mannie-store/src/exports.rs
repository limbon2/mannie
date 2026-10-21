//! The durable lifecycle of owner-initiated exports (ADR-0040). Recovery
//! records uncertainty and never repeats filesystem operations.

use mannie_types::ids::{ConversationId, ExportId};
use rusqlite::params;

use crate::{Durability, Store, StoreError};

const INSERT: &str = "INSERT INTO exports_operations (id, conversation_id, destination, state, created_at, updated_at) VALUES (?1, ?2, ?3, 'executing', ?4, ?4)";
const FINISH: &str = "UPDATE exports_operations SET state = ?2, updated_at = ?3 WHERE id = ?1 AND state = 'executing'";
const RECOVER: &str =
    "UPDATE exports_operations SET state = 'uncertain', updated_at = ?1 WHERE state = 'executing'";

#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[INSERT, FINISH, RECOVER];

impl Store {
    /// Commits the execution record before any export file is created.
    pub async fn begin_export(
        &self,
        id: &ExportId,
        conversation: &ConversationId,
        destination: &str,
    ) -> Result<(), StoreError> {
        let (id, conversation, destination) = (
            id.to_string(),
            conversation.to_string(),
            destination.to_owned(),
        );
        let now = self.clock().now().as_millis();
        self.write(Durability::Full, move |transaction| {
            transaction.execute(INSERT, params![id, conversation, destination, now])?;
            Ok(())
        })
        .await
    }

    /// Records the final outcome of an export without storing its contents.
    pub async fn finish_export(&self, id: &ExportId, completed: bool) -> Result<(), StoreError> {
        let id = id.to_string();
        let now = self.clock().now().as_millis();
        self.write(Durability::Full, move |transaction| {
            transaction.execute(
                FINISH,
                params![id, if completed { "completed" } else { "failed" }, now],
            )?;
            Ok(())
        })
        .await
    }

    /// Marks interrupted exports uncertain. No path is opened or retried.
    pub async fn recover_exports(&self) -> Result<(), StoreError> {
        let now = self.clock().now().as_millis();
        self.write(Durability::Full, move |transaction| {
            transaction.execute(RECOVER, [now])?;
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StoreOptions;
    use mannie_types::clock::SystemClock;
    use std::sync::Arc;

    #[tokio::test]
    async fn recovery_marks_only_executing_records_uncertain_without_touching_destinations() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(
            &directory.path().join("test.sqlite"),
            &StoreOptions::new(directory.path().join("backups"), Arc::new(SystemClock)),
        )
        .unwrap();
        let id = ConversationId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap();
        let completed = ExportId::parse("0199e0f2-0000-7000-8000-000000000002").unwrap();
        let failed = ExportId::parse("0199e0f2-0000-7000-8000-000000000003").unwrap();
        let interrupted = ExportId::parse("0199e0f2-0000-7000-8000-000000000004").unwrap();
        let missing_destination = directory.path().join("must-not-be-created");
        store
            .begin_export(&completed, &id, "completed-folder")
            .await
            .unwrap();
        store.finish_export(&completed, true).await.unwrap();
        store
            .begin_export(&failed, &id, "failed-folder")
            .await
            .unwrap();
        store.finish_export(&failed, false).await.unwrap();
        store
            .begin_export(&interrupted, &id, &missing_destination.to_string_lossy())
            .await
            .unwrap();
        store.recover_exports().await.unwrap();
        store.recover_exports().await.unwrap();
        let states: Vec<(String, String)> = store
            .read(|connection| {
                connection
                    .prepare("SELECT id, state FROM exports_operations ORDER BY id")?
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect()
            })
            .await
            .unwrap();
        assert_eq!(
            states,
            [
                (completed.to_string(), "completed".to_owned()),
                (failed.to_string(), "failed".to_owned()),
                (interrupted.to_string(), "uncertain".to_owned())
            ]
        );
        assert!(!missing_destination.exists());
    }
}
