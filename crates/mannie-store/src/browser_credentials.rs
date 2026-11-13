//! The owner's Browser credential metadata. The values stay in the OS vault.

use mannie_types::ids::ConversationId;
use rusqlite::{Connection, OptionalExtension, params};

use crate::{Durability, Store, StoreError};

const LIST: &str = "SELECT reference, origin, label FROM browser_site_credentials
    WHERE state = 'active' AND (?1 IS NULL OR origin = ?1) ORDER BY origin, label, reference";
const FIND: &str = "SELECT reference, origin, label FROM browser_site_credentials
    WHERE state = 'active' AND origin = ?1 AND reference = ?2";
const PENDING: &str = "SELECT reference FROM browser_site_credentials
    WHERE state IN ('pending', 'revoked') ORDER BY reference";
const STAGE: &str = "INSERT INTO browser_site_credentials (reference, origin, label, state)
    VALUES (?1, ?2, ?3, 'pending') ON CONFLICT (origin, label) WHERE state <> 'revoked' DO NOTHING";
const ACTIVATE: &str = "UPDATE browser_site_credentials SET state = 'active'
    WHERE reference = ?1 AND state = 'pending'";
const REVOKE: &str = "UPDATE browser_site_credentials SET state = 'revoked'
    WHERE reference = ?1 AND state = 'active'";
const NEEDS_PURGE: &str = "SELECT EXISTS (SELECT 1 FROM browser_site_credentials
    WHERE reference = ?1 AND state IN ('pending', 'revoked'))";
const PURGE: &str = "DELETE FROM browser_site_credentials
    WHERE reference = ?1 AND state IN ('pending', 'revoked')";
const RECORD: &str = "INSERT INTO browser_site_credential_changes
    (reference, action, recorded_at) VALUES (?1, ?2, ?3)";
const REVOKE_GRANTS: &str = "DELETE FROM tools_grants
    WHERE json_extract(scope, '$.kind') = 'browser_credential'
      AND json_extract(scope, '$.plugin') = 'browser'
      AND json_extract(scope, '$.reference') = ?1";
const GRANTED: &str = "SELECT EXISTS (
    SELECT 1 FROM tools_grants AS grants
    JOIN browser_site_credentials AS credentials ON credentials.reference = ?3
    WHERE grants.conversation_id = ?1
      AND credentials.state = 'active'
      AND credentials.origin = ?2
      AND json_extract(grants.scope, '$.kind') = 'browser_credential'
      AND json_extract(grants.scope, '$.plugin') = 'browser'
      AND json_extract(grants.scope, '$.origin') = ?2
      AND json_extract(grants.scope, '$.reference') = ?3
)";

#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    LIST,
    FIND,
    PENDING,
    STAGE,
    ACTIVATE,
    REVOKE,
    NEEDS_PURGE,
    PURGE,
    RECORD,
    REVOKE_GRANTS,
    GRANTED,
];

/// Nonsecret metadata for one saved site credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredSiteCredential {
    /// Opaque identity of the saved value.
    pub reference: String,
    /// Exact canonical HTTP or HTTPS origin.
    pub origin: String,
    /// Owner-supplied name for the credential.
    pub label: String,
}

fn descriptor(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredSiteCredential> {
    Ok(StoredSiteCredential {
        reference: row.get(0)?,
        origin: row.get(1)?,
        label: row.get(2)?,
    })
}

pub(crate) fn site_credential_is_active(
    connection: &Connection,
    origin: &str,
    reference: &str,
) -> rusqlite::Result<bool> {
    Ok(connection
        .query_row(FIND, params![origin, reference], descriptor)
        .optional()?
        .is_some())
}

impl Store {
    /// Lists active credential metadata, optionally restricted to one origin.
    pub async fn browser_site_credentials(
        &self,
        origin: Option<String>,
    ) -> Result<Vec<StoredSiteCredential>, StoreError> {
        self.read(move |connection| {
            connection
                .prepare_cached(LIST)?
                .query_map([origin], descriptor)?
                .collect()
        })
        .await
    }

    /// Reads the metadata for the exact active credential at this origin.
    pub async fn browser_site_credential(
        &self,
        origin: String,
        reference: String,
    ) -> Result<Option<StoredSiteCredential>, StoreError> {
        self.read(move |connection| {
            connection
                .query_row(FIND, params![origin, reference], descriptor)
                .optional()
        })
        .await
    }

    /// Checks for both a live catalog entry and a grant of that exact entry.
    pub async fn has_browser_site_credential(
        &self,
        conversation: &ConversationId,
        origin: &str,
        reference: &str,
    ) -> Result<bool, StoreError> {
        let conversation = conversation.as_str().to_owned();
        let origin = origin.to_owned();
        let reference = reference.to_owned();
        self.read(move |connection| {
            connection.query_row(GRANTED, params![conversation, origin, reference], |row| {
                row.get(0)
            })
        })
        .await
    }

    /// Lists entries to remove from the OS vault after an interrupted change.
    pub async fn browser_site_credentials_to_purge(&self) -> Result<Vec<String>, StoreError> {
        self.read(|connection| {
            connection
                .prepare_cached(PENDING)?
                .query_map([], |row| row.get(0))?
                .collect()
        })
        .await
    }

    /// Commits the owner's add request before the credential is written to the vault.
    pub async fn stage_browser_site_credential(
        &self,
        credential: StoredSiteCredential,
    ) -> Result<bool, StoreError> {
        let recorded_at = self.clock().now().as_millis();
        self.write(Durability::Full, move |transaction| {
            let inserted = transaction.execute(
                STAGE,
                params![credential.reference, credential.origin, credential.label],
            )?;
            if inserted != 0 {
                transaction.execute(RECORD, params![credential.reference, "add", recorded_at])?;
            }
            Ok(inserted != 0)
        })
        .await
    }

    /// Makes the catalog entry available after the vault has accepted its value.
    pub async fn activate_browser_site_credential(
        &self,
        reference: &str,
    ) -> Result<bool, StoreError> {
        self.record_browser_site_credential_change(reference, ACTIVATE, "saved")
            .await
    }

    /// Disables an entry and removes its grants before deleting its vault value.
    /// Returns whether a known entry needs to be purged.
    pub async fn revoke_browser_site_credential(
        &self,
        reference: &str,
    ) -> Result<bool, StoreError> {
        let reference = reference.to_owned();
        let recorded_at = self.clock().now().as_millis();
        self.write(Durability::Full, move |transaction| {
            if transaction.execute(REVOKE, [&reference])? != 0 {
                transaction.execute(REVOKE_GRANTS, [&reference])?;
                transaction.execute(RECORD, params![&reference, "remove", recorded_at])?;
            }
            transaction.query_row(NEEDS_PURGE, [&reference], |row| row.get(0))
        })
        .await
    }

    /// Finishes a removal or interrupted add after the vault value is gone.
    pub async fn purge_browser_site_credential(&self, reference: &str) -> Result<(), StoreError> {
        self.record_browser_site_credential_change(reference, PURGE, "purged")
            .await
            .map(|_| ())
    }

    async fn record_browser_site_credential_change(
        &self,
        reference: &str,
        statement: &'static str,
        action: &'static str,
    ) -> Result<bool, StoreError> {
        let reference = reference.to_owned();
        let recorded_at = self.clock().now().as_millis();
        self.write(Durability::Full, move |transaction| {
            let changed = transaction.execute(statement, [&reference])? != 0;
            if changed {
                transaction.execute(RECORD, params![reference, action, recorded_at])?;
            }
            Ok(changed)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mannie_types::clock::SystemClock;
    use mannie_types::ids::{ConversationId, GrantId};
    use mannie_types::tools::GrantScope;

    use crate::{Change, NewConversation, StoreOptions};

    use super::*;

    const ORIGIN: &str = "https://example.test:444";
    const REFERENCE: &str = "0199e0f2-0000-7000-8000-000000000007";

    async fn create_conversation(store: &Store, id: &str) -> ConversationId {
        let conversation = ConversationId::parse(id).unwrap();
        store
            .apply(
                &conversation,
                vec![Change::CreateConversation {
                    conversation: NewConversation {
                        id: conversation.clone(),
                        title: "Fixture".to_owned(),
                        profile_id: None,
                        project_id: None,
                        agent_key: None,
                        task: None,
                        home: None,
                    },
                }],
                Durability::Full,
            )
            .await
            .unwrap();
        conversation
    }

    async fn assert_wrong_origins_are_refused(store: &Store, conversation: &ConversationId) {
        for origin in ["https://example.test", "http://example.test:444"] {
            assert!(
                !store
                    .has_browser_site_credential(conversation, origin, REFERENCE)
                    .await
                    .unwrap()
            );
        }
    }

    #[tokio::test]
    async fn active_credentials_and_exact_conversation_grants_survive_reopening() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.sqlite");
        let options = StoreOptions::new(directory.path().join("backups"), Arc::new(SystemClock));
        let store = Store::open(&path, &options).unwrap();
        let credential = StoredSiteCredential {
            origin: ORIGIN.to_owned(),
            label: "Fixture".to_owned(),
            reference: REFERENCE.to_owned(),
        };
        let first = create_conversation(&store, "0199e0f2-0000-7000-8000-000000000001").await;
        let second = create_conversation(&store, "0199e0f2-0000-7000-8000-000000000002").await;
        let grant = GrantScope::BrowserCredential {
            plugin: "browser".to_owned(),
            origin: ORIGIN.to_owned(),
            reference: REFERENCE.to_owned(),
            label: "Fixture".to_owned(),
        };
        let grant_id = GrantId::parse("0199e0f2-0000-7000-8000-000000000003").unwrap();

        assert!(
            store
                .stage_browser_site_credential(credential.clone())
                .await
                .unwrap()
        );
        assert_eq!(store.browser_site_credentials(None).await.unwrap(), []);
        store
            .add_grants(&first, vec![(grant_id.clone(), grant.clone())])
            .await
            .unwrap_err();

        assert!(
            store
                .activate_browser_site_credential(REFERENCE)
                .await
                .unwrap()
        );
        let duplicate = StoredSiteCredential {
            reference: "0199e0f2-0000-7000-8000-000000000008".to_owned(),
            ..credential.clone()
        };
        assert!(
            !store
                .stage_browser_site_credential(duplicate)
                .await
                .unwrap()
        );
        store
            .add_grants(&first, vec![(grant_id.clone(), grant.clone())])
            .await
            .unwrap();
        drop(store);

        let store = Store::open(&path, &options).unwrap();
        assert_eq!(
            store.browser_site_credentials(None).await.unwrap(),
            [credential]
        );
        assert!(
            store
                .has_browser_site_credential(&first, ORIGIN, REFERENCE)
                .await
                .unwrap()
        );
        assert!(
            !store
                .has_browser_site_credential(&second, ORIGIN, REFERENCE)
                .await
                .unwrap()
        );
        assert_wrong_origins_are_refused(&store, &first).await;

        assert!(
            store
                .revoke_browser_site_credential(REFERENCE)
                .await
                .unwrap()
        );
        assert_eq!(store.grants(&first).await.unwrap(), []);
        assert_eq!(store.browser_site_credentials(None).await.unwrap(), []);
        assert_eq!(
            store.browser_site_credentials_to_purge().await.unwrap(),
            [REFERENCE]
        );
        store
            .add_grants(&first, vec![(grant_id, grant)])
            .await
            .unwrap_err();
        store
            .purge_browser_site_credential(REFERENCE)
            .await
            .unwrap();
        assert!(
            !store
                .revoke_browser_site_credential(REFERENCE)
                .await
                .unwrap()
        );
    }
}
