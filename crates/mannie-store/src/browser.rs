//! Owner-confirmed Browser certificate exceptions and their journal.

use mannie_types::ids::ConversationId;
use rusqlite::params;

use crate::{Durability, Store, StoreError};

const CONFIRM: &str = "INSERT INTO browser_certificate_confirmations
    (conversation_id, session_id, hostname, fingerprint, confirmed_at)
    VALUES (?1, ?2, ?3, ?4, ?5)
    ON CONFLICT (session_id, hostname, fingerprint) DO NOTHING";
const LIST_SAVED: &str =
    "SELECT hostname, fingerprint FROM browser_saved_certificates ORDER BY hostname";
const SAVE: &str = "INSERT INTO browser_saved_certificates (hostname, fingerprint)
    VALUES (?1, ?2)
    ON CONFLICT (hostname) DO NOTHING";
const SAVED_FINGERPRINT: &str =
    "SELECT fingerprint FROM browser_saved_certificates WHERE hostname = ?1";
const FORGET: &str =
    "DELETE FROM browser_saved_certificates WHERE hostname = ?1 AND fingerprint = ?2";
const RECORD_CHANGE: &str = "INSERT INTO browser_certificate_trust_changes
    (hostname, fingerprint, action, recorded_at) VALUES (?1, ?2, ?3, ?4)";

#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    CONFIRM,
    LIST_SAVED,
    SAVE,
    SAVED_FINGERPRINT,
    FORGET,
    RECORD_CHANGE,
];

impl Store {
    /// Commits the owner's confirmation before the live engine can use it.
    pub async fn record_browser_certificate_confirmation(
        &self,
        conversation: &ConversationId,
        session_id: &str,
        hostname: &str,
        fingerprint: &[u8; 32],
    ) -> Result<(), StoreError> {
        let conversation = conversation.to_string();
        let session_id = session_id.to_owned();
        let hostname = hostname.to_owned();
        let fingerprint = *fingerprint;
        let confirmed_at = self.clock().now().as_millis();
        self.write(Durability::Full, move |transaction| {
            transaction.execute(
                CONFIRM,
                params![
                    conversation,
                    session_id,
                    hostname,
                    &fingerprint[..],
                    confirmed_at
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// Lists the owner's saved trust by hostname. Session-only confirmations
    /// are kept in their own journal.
    pub async fn list_browser_saved_certificates(
        &self,
    ) -> Result<Vec<(String, [u8; 32])>, StoreError> {
        self.read(|connection| {
            connection
                .prepare(LIST_SAVED)?
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .await
    }

    /// Saves and journals the owner's trust. An identical pin is idempotent.
    /// Returns false if the hostname is already saved with another fingerprint.
    pub async fn save_browser_certificate(
        &self,
        hostname: &str,
        fingerprint: &[u8; 32],
    ) -> Result<bool, StoreError> {
        let hostname = hostname.to_owned();
        let fingerprint = *fingerprint;
        let recorded_at = self.clock().now().as_millis();
        self.write(Durability::Full, move |transaction| {
            let inserted = transaction.execute(SAVE, params![hostname, &fingerprint[..]])?;
            if inserted == 0 {
                let saved: [u8; 32] =
                    transaction.query_row(SAVED_FINGERPRINT, [&hostname], |row| row.get(0))?;
                return Ok(saved == fingerprint);
            }
            transaction.execute(
                RECORD_CHANGE,
                params![hostname, &fingerprint[..], "save", recorded_at],
            )?;
            Ok(true)
        })
        .await
    }

    /// Removes and journals the exact pin the owner selected. A stale removal
    /// leaves any replacement intact and returns false.
    pub async fn forget_browser_certificate(
        &self,
        hostname: &str,
        fingerprint: &[u8; 32],
    ) -> Result<bool, StoreError> {
        let hostname = hostname.to_owned();
        let fingerprint = *fingerprint;
        let recorded_at = self.clock().now().as_millis();
        self.write(Durability::Full, move |transaction| {
            let deleted = transaction.execute(FORGET, params![hostname, &fingerprint[..]])?;
            if deleted == 0 {
                return Ok(false);
            }
            transaction.execute(
                RECORD_CHANGE,
                params![hostname, &fingerprint[..], "forget", recorded_at],
            )?;
            Ok(true)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::sync::Arc;

    use mannie_types::clock::SystemClock;
    use mannie_types::ids::ConversationId;

    use crate::runtime::{Change, NewConversation};
    use crate::schema::CORE_MODULES;
    use crate::{Durability, Store, StoreOptions};

    #[tokio::test]
    async fn saved_trust_survives_reopen_and_forget_checks_the_fingerprint() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.sqlite");
        let options = StoreOptions::new(directory.path().join("backups"), Arc::new(SystemClock));
        let store = Store::open(&path, &options).unwrap();
        let original = [1; 32];
        let replacement = [2; 32];

        assert!(
            store
                .save_browser_certificate("localhost", &original)
                .await
                .unwrap()
        );
        assert!(
            store
                .save_browser_certificate("localhost", &original)
                .await
                .unwrap()
        );
        assert!(
            !store
                .save_browser_certificate("localhost", &replacement)
                .await
                .unwrap()
        );
        drop(store);

        let store = Store::open(&path, &options).unwrap();
        assert_eq!(
            store.list_browser_saved_certificates().await.unwrap(),
            vec![("localhost".to_owned(), original)],
        );
        assert!(
            !store
                .forget_browser_certificate("localhost", &replacement)
                .await
                .unwrap()
        );
        assert!(
            store
                .forget_browser_certificate("localhost", &original)
                .await
                .unwrap()
        );
        assert!(
            !store
                .forget_browser_certificate("localhost", &original)
                .await
                .unwrap()
        );
        assert_eq!(store.list_browser_saved_certificates().await.unwrap(), []);
        assert!(
            store
                .save_browser_certificate("localhost", &replacement)
                .await
                .unwrap()
        );
        assert!(
            !store
                .forget_browser_certificate("localhost", &original)
                .await
                .unwrap()
        );
        assert_eq!(
            store.list_browser_saved_certificates().await.unwrap(),
            vec![("localhost".to_owned(), replacement)],
        );

        let operations: Vec<(String, [u8; 32])> = store
            .read(|connection| {
                connection
                    .prepare("SELECT action, fingerprint FROM browser_certificate_trust_changes ORDER BY id")?
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect()
            })
            .await
            .unwrap();
        assert_eq!(
            operations,
            vec![
                ("save".to_owned(), original),
                ("forget".to_owned(), original),
                ("save".to_owned(), replacement),
            ],
        );
    }

    #[tokio::test]
    async fn a_journal_failure_rolls_back_saved_trust_changes() {
        let directory = tempfile::tempdir().unwrap();
        let options = StoreOptions::new(directory.path().join("backups"), Arc::new(SystemClock));
        let store = Store::open(&directory.path().join("test.sqlite"), &options).unwrap();
        let original = [1; 32];
        assert!(
            store
                .save_browser_certificate("localhost", &original)
                .await
                .unwrap()
        );

        store
            .maintain(|connection| {
                connection.execute_batch(
                    "CREATE TEMP TRIGGER refuse_trust_changes BEFORE INSERT ON browser_certificate_trust_changes
                    BEGIN
                        SELECT RAISE(ABORT, 'journal failure');
                    END;",
                )
            })
            .await
            .unwrap();

        store
            .save_browser_certificate("other.localhost", &original)
            .await
            .unwrap_err();
        store
            .forget_browser_certificate("localhost", &original)
            .await
            .unwrap_err();
        assert_eq!(
            store.list_browser_saved_certificates().await.unwrap(),
            vec![("localhost".to_owned(), original)],
        );
    }

    #[tokio::test]
    async fn upgrading_does_not_promote_session_confirmations_to_saved_trust() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.sqlite");
        let options = StoreOptions::new(directory.path().join("backups"), Arc::new(SystemClock));
        let mut modules = CORE_MODULES;
        let browser = modules
            .iter_mut()
            .find(|module| module.name == "browser")
            .unwrap();
        browser.migrations = Cow::Owned(browser.migrations[..1].to_vec());
        let store = Store::open_with_modules(&path, &options, &modules).unwrap();
        let conversation = ConversationId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap();
        store
            .apply(
                &conversation,
                vec![Change::CreateConversation {
                    conversation: NewConversation {
                        id: conversation.clone(),
                        title: "Fixture".to_owned(),
                        profile_id: None,
                        project_id: None,
                        task: None,
                        home: None,
                        agent_key: None,
                    },
                }],
                Durability::Full,
            )
            .await
            .unwrap();
        store
            .record_browser_certificate_confirmation(
                &conversation,
                "fixture",
                "localhost",
                &[1; 32],
            )
            .await
            .unwrap();
        drop(store);

        let store = Store::open(&path, &options).unwrap();
        assert_eq!(store.list_browser_saved_certificates().await.unwrap(), []);
        let confirmations: u32 = store
            .read(|connection| {
                connection.query_row(
                    "SELECT COUNT(*) FROM browser_certificate_confirmations",
                    [],
                    |row| row.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(confirmations, 1);
    }
}
