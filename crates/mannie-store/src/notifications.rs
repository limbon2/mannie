//! Unread finished conversations persisted for desktop notifications and the
//! system tray badge (ADR-0074, decision 7).

use mannie_types::ids::ConversationId;
use rusqlite::{OptionalExtension, params};

use crate::{Durability, Store, StoreError};

const PRUNE_DELETED: &str = "DELETE FROM notifications_unread \
     WHERE conversation_id NOT IN (SELECT id FROM runtime_conversations)";
const LIST: &str = "SELECT conversation_id FROM notifications_unread ORDER BY conversation_id";
const UPSERT: &str = "INSERT INTO notifications_unread (conversation_id, marked_at) \
     VALUES (?1, ?2) ON CONFLICT(conversation_id) DO UPDATE SET marked_at = excluded.marked_at";
const DELETE: &str = "DELETE FROM notifications_unread WHERE conversation_id = ?1";
const CONVERSATION_EXISTS: &str = "SELECT 1 FROM runtime_conversations WHERE id = ?1";

#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[PRUNE_DELETED, LIST, UPSERT, DELETE, CONVERSATION_EXISTS];

impl Store {
    /// Removes unread markers whose conversation no longer exists and returns
    /// the remaining unread finished conversation IDs, sorted by ID.
    pub async fn load_unread_conversations(&self) -> Result<Vec<ConversationId>, StoreError> {
        let raw_ids = self
            .write(Durability::Normal, |transaction| {
                transaction.execute(PRUNE_DELETED, [])?;

                let mut statement = transaction.prepare(LIST)?;
                let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
                rows.collect::<Result<Vec<_>, _>>()
            })
            .await?;

        let mut ids = Vec::with_capacity(raw_ids.len());

        for raw in raw_ids {
            match ConversationId::parse(&raw) {
                Ok(id) => ids.push(id),
                Err(_invalid) => {
                    tracing::warn!(
                        conversation_id = %raw,
                        "skipped an unreadable conversation ID in notifications_unread"
                    );
                }
            }
        }
        Ok(ids)
    }

    /// Marks `conversation_id` as having an unread finished run.
    pub async fn mark_conversation_unread(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<(), StoreError> {
        let id = conversation_id.as_str().to_owned();
        let now = self.clock().now().as_millis();

        self.write(Durability::Normal, move |transaction| {
            transaction.execute(UPSERT, params![id, now])?;
            Ok(())
        })
        .await
    }

    /// Removes `conversation_id` from the unread set and returns whether a row
    /// was removed.
    pub async fn clear_conversation_unread(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<bool, StoreError> {
        let id = conversation_id.as_str().to_owned();
        let removed = self
            .write(Durability::Normal, move |transaction| {
                transaction.execute(DELETE, params![id])
            })
            .await?;
        Ok(removed > 0)
    }

    /// Whether `conversation_id` currently exists in `runtime_conversations`.
    pub async fn conversation_exists(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<bool, StoreError> {
        let id = conversation_id.as_str().to_owned();

        self.read(move |connection| {
            let found: Option<i64> = connection
                .prepare_cached(CONVERSATION_EXISTS)?
                .query_row(params![id], |row| row.get(0))
                .optional()?;
            Ok(found.is_some())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mannie_types::clock::{ManualClock, UtcMillis};
    use mannie_types::ids::ConversationId;
    use tempfile::TempDir;

    use crate::{Change, Durability, NewConversation, Store, StoreOptions};

    struct Fixture {
        store: Store,
        _directory: TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let clock = Arc::new(ManualClock::new(UtcMillis::from_millis(1_794_000_000_000)));
            let options = StoreOptions::new(directory.path().join("backups"), clock);
            let store = Store::open(&directory.path().join("test.sqlite"), &options).unwrap();
            Self {
                store,
                _directory: directory,
            }
        }

        async fn create_conversation(&self, id: &ConversationId, title: &str) {
            self.store
                .apply(
                    id,
                    vec![Change::CreateConversation {
                        conversation: NewConversation {
                            id: id.clone(),
                            title: title.to_owned(),
                            profile_id: None,
                            project_id: None,
                            task: None,
                            home: None,
                            agent_key: None,
                        },
                    }],
                    Durability::Normal,
                )
                .await
                .unwrap();
        }
    }

    fn id(last: u8) -> ConversationId {
        ConversationId::parse(&format!("0199e0f2-0000-7000-8000-0000000000{last:02x}")).unwrap()
    }

    #[tokio::test]
    async fn unread_markers_persist_clear_and_prune_deleted_conversations() {
        let fixture = Fixture::new();
        let first = id(1);
        let second = id(2);
        let ghost = id(3);

        fixture.create_conversation(&first, "First").await;
        fixture.create_conversation(&second, "Second").await;
        assert!(fixture.store.conversation_exists(&first).await.unwrap());
        assert!(!fixture.store.conversation_exists(&ghost).await.unwrap());

        fixture
            .store
            .mark_conversation_unread(&second)
            .await
            .unwrap();
        fixture
            .store
            .mark_conversation_unread(&first)
            .await
            .unwrap();
        fixture
            .store
            .mark_conversation_unread(&first)
            .await
            .unwrap();
        fixture
            .store
            .mark_conversation_unread(&ghost)
            .await
            .unwrap();

        let loaded = fixture.store.load_unread_conversations().await.unwrap();
        assert_eq!(loaded, vec![first.clone(), second.clone()]);

        assert!(
            fixture
                .store
                .clear_conversation_unread(&first)
                .await
                .unwrap()
        );
        assert!(
            !fixture
                .store
                .clear_conversation_unread(&first)
                .await
                .unwrap()
        );

        let remaining = fixture.store.load_unread_conversations().await.unwrap();
        assert_eq!(remaining, vec![second]);
    }
}
