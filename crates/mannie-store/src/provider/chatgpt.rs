//! The ChatGPT account repository (ADR-0062): the one account through which
//! profiles use the owner's ChatGPT plan. Its tokens live in the operating
//! system's credential store, split into parts (ADR-0010); this row holds the
//! generation of the current parts and their count, so that new parts are
//! saved before the row names them and old ones are deleted after.
//!
//! Each change of the tokens is a compare-and-set on the generation, so a
//! change that lost a race to another changes nothing.

use mannie_types::clock::UtcMillis;
use mannie_types::provider::ChatGptAccountState;
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::writer::TransactError;
use crate::{Durability, ProfileStoreError, Store};

const SELECT: &str = "SELECT host_id, client_id, subject, email, state, token_generation, token_parts, \
     notice_seen, updated_at FROM provider_chatgpt_account WHERE id = 1";
const INSERT_IF_MISSING: &str = "INSERT INTO provider_chatgpt_account (id, host_id, state, updated_at) \
     VALUES (1, ?1, 'signed_out', ?2) ON CONFLICT (id) DO NOTHING";
const SIGN_IN: &str = "UPDATE provider_chatgpt_account SET client_id = ?2, subject = ?3, email = ?4, \
     state = 'signed_in', token_generation = ?5, token_parts = ?6, updated_at = ?7 \
     WHERE id = 1 AND token_generation = ?1";
const REPLACE_TOKENS: &str = "UPDATE provider_chatgpt_account SET token_generation = ?2, token_parts = ?3, \
     updated_at = ?4 WHERE id = 1 AND token_generation = ?1 AND state = 'signed_in'";
const END_SESSION: &str = "UPDATE provider_chatgpt_account SET state = 'sign_in_needed', token_parts = 0, \
     updated_at = ?2 WHERE id = 1 AND token_generation = ?1 AND state = 'signed_in'";
const SIGN_OUT: &str = "UPDATE provider_chatgpt_account SET state = 'signed_out', token_parts = 0, \
     updated_at = ?1 WHERE id = 1";
const NOTICE_SEEN: &str =
    "UPDATE provider_chatgpt_account SET notice_seen = 1, updated_at = ?1 WHERE id = 1";

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    SELECT,
    INSERT_IF_MISSING,
    SIGN_IN,
    REPLACE_TOKENS,
    END_SESSION,
    SIGN_OUT,
    NOTICE_SEEN,
];

/// The ChatGPT account as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredChatGptAccount {
    /// The installation's ID for OpenAI's `ext_agent_host_id`.
    pub host_id: String,
    /// The client that OpenAI issued, once a sign-in registered Mannie.
    pub client_id: Option<String>,
    /// The `sub` of the account's ID token.
    pub subject: Option<String>,
    /// The account's email address.
    pub email: Option<String>,
    /// Where the account stands.
    pub state: ChatGptAccountState,
    /// The generation of the token parts in the credential store.
    pub token_generation: u32,
    /// How many parts the current generation has; 0 unless signed in.
    pub token_parts: u32,
    /// Whether the owner saw the notice that Mannie uses their plan.
    pub notice_seen: bool,
    /// When the row last changed.
    pub updated_at: UtcMillis,
}

/// A completed sign-in, whose token parts the caller saved under
/// `token_generation`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatGptSignInRecord {
    /// The generation that the row names before the sign-in.
    pub replaces_generation: u32,
    /// The client that OpenAI issued.
    pub client_id: String,
    /// The `sub` of the account's ID token.
    pub subject: String,
    /// The account's email address, when the ID token named it.
    pub email: Option<String>,
    /// The generation of the saved parts.
    pub token_generation: u32,
    /// How many parts were saved.
    pub token_parts: u32,
}

fn account_from_row(row: &Row<'_>) -> rusqlite::Result<RawAccount> {
    Ok(RawAccount {
        host_id: row.get(0)?,
        client_id: row.get(1)?,
        subject: row.get(2)?,
        email: row.get(3)?,
        state: row.get(4)?,
        token_generation: row.get(5)?,
        token_parts: row.get(6)?,
        notice_seen: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

struct RawAccount {
    host_id: String,
    client_id: Option<String>,
    subject: Option<String>,
    email: Option<String>,
    state: String,
    token_generation: i64,
    token_parts: i64,
    notice_seen: bool,
    updated_at: i64,
}

impl RawAccount {
    fn into_account(self) -> Result<StoredChatGptAccount, ProfileStoreError> {
        let invalid = |what: &str| ProfileStoreError::InvalidData {
            what: format!("the ChatGPT account: {what}"),
        };
        Ok(StoredChatGptAccount {
            host_id: self.host_id,
            client_id: self.client_id,
            subject: self.subject,
            email: self.email,
            state: ChatGptAccountState::parse(&self.state)
                .ok_or_else(|| invalid("unknown state"))?,
            token_generation: u32::try_from(self.token_generation)
                .map_err(|_out_of_range| invalid("the token generation is out of range"))?,
            token_parts: u32::try_from(self.token_parts)
                .map_err(|_out_of_range| invalid("the token parts are out of range"))?,
            notice_seen: self.notice_seen,
            updated_at: UtcMillis::from_millis(self.updated_at),
        })
    }
}

fn read(connection: &Connection) -> Result<Option<StoredChatGptAccount>, ProfileStoreError> {
    let raw = connection
        .prepare_cached(SELECT)?
        .query_row([], account_from_row)
        .optional()?;
    raw.map(RawAccount::into_account).transpose()
}

impl Store {
    /// The ChatGPT account, if a sign-in ever started.
    pub async fn chatgpt_account(&self) -> Result<Option<StoredChatGptAccount>, ProfileStoreError> {
        let raw = self
            .read(|connection| {
                connection
                    .prepare_cached(SELECT)?
                    .query_row([], account_from_row)
                    .optional()
            })
            .await?;
        raw.map(RawAccount::into_account).transpose()
    }

    /// The ChatGPT account, created signed out with `host_id` when no
    /// sign-in started before. An existing row keeps its host ID.
    pub async fn ensure_chatgpt_account(
        &self,
        host_id: String,
    ) -> Result<StoredChatGptAccount, ProfileStoreError> {
        let now = self.clock().now().as_millis();
        self.transact(Durability::Full, move |transaction| {
            transaction
                .prepare_cached(INSERT_IF_MISSING)?
                .execute(params![host_id, now])?;
            read(transaction)
                .map_err(TransactError::Rejected)?
                .ok_or(TransactError::Rejected(ProfileStoreError::NotFound))
        })
        .await
        .map_err(ProfileStoreError::from)
    }

    /// Records a completed sign-in, whose token parts replace the generation
    /// that the row names. Returns the account after the change, or `None`
    /// when the row names another generation than the caller read, so that
    /// the caller deletes the parts it saved.
    pub async fn record_chatgpt_sign_in(
        &self,
        sign_in: ChatGptSignInRecord,
    ) -> Result<Option<StoredChatGptAccount>, ProfileStoreError> {
        let now = self.clock().now().as_millis();
        self.transact(Durability::Full, move |transaction| {
            let changed = transaction.prepare_cached(SIGN_IN)?.execute(params![
                sign_in.replaces_generation,
                sign_in.client_id,
                sign_in.subject,
                sign_in.email,
                sign_in.token_generation,
                sign_in.token_parts,
                now
            ])?;
            if changed == 0 {
                return Ok(None);
            }
            read(transaction).map_err(TransactError::Rejected)
        })
        .await
        .map_err(ProfileStoreError::from)
    }

    /// Records refreshed tokens: the parts of generation `to` replace those
    /// of `from`, while the account is signed in. Returns whether the row
    /// changed; when it did not, the caller deletes the parts it saved.
    pub async fn record_chatgpt_tokens(
        &self,
        from: u32,
        to: u32,
        parts: u32,
    ) -> Result<bool, ProfileStoreError> {
        let now = self.clock().now().as_millis();
        self.transact(Durability::Full, move |transaction| {
            let changed = transaction
                .prepare_cached(REPLACE_TOKENS)?
                .execute(params![from, to, parts, now])?;
            Ok(changed == 1)
        })
        .await
        .map_err(ProfileStoreError::from)
    }

    /// Records that the session of generation `generation` ended, so the
    /// owner signs in again. Returns whether the row changed.
    pub async fn record_chatgpt_sign_in_needed(
        &self,
        generation: u32,
    ) -> Result<bool, ProfileStoreError> {
        let now = self.clock().now().as_millis();
        self.transact(Durability::Full, move |transaction| {
            let changed = transaction
                .prepare_cached(END_SESSION)?
                .execute(params![generation, now])?;
            Ok(changed == 1)
        })
        .await
        .map_err(ProfileStoreError::from)
    }

    /// Records a sign-out. The client and the account stay, so that signing
    /// in again reuses the registration. Returns the account before the
    /// change, so that the caller deletes its parts.
    pub async fn record_chatgpt_sign_out(
        &self,
    ) -> Result<Option<StoredChatGptAccount>, ProfileStoreError> {
        let now = self.clock().now().as_millis();
        self.transact(Durability::Full, move |transaction| {
            let before = read(transaction).map_err(TransactError::Rejected)?;
            transaction.prepare_cached(SIGN_OUT)?.execute([now])?;
            Ok(before)
        })
        .await
        .map_err(ProfileStoreError::from)
    }

    /// Records that the owner saw the notice that Mannie uses their ChatGPT
    /// plan.
    pub async fn record_chatgpt_notice_seen(&self) -> Result<(), ProfileStoreError> {
        let now = self.clock().now().as_millis();
        self.transact(Durability::Full, move |transaction| {
            transaction.prepare_cached(NOTICE_SEEN)?.execute([now])?;
            Ok(())
        })
        .await
        .map_err(ProfileStoreError::from)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mannie_types::clock::{Clock, ManualClock, UtcMillis};
    use mannie_types::provider::ChatGptAccountState;

    use super::ChatGptSignInRecord;
    use crate::{Store, StoreOptions};

    struct Fixture {
        _directory: tempfile::TempDir,
        store: Store,
    }

    fn fixture() -> Fixture {
        let directory = tempfile::tempdir().unwrap();
        let clock: Arc<dyn Clock> =
            Arc::new(ManualClock::new(UtcMillis::from_millis(1_794_096_000_000)));
        let options = StoreOptions::new(directory.path().join("backups"), clock);
        let store = Store::open(&directory.path().join("mannie.sqlite"), &options).unwrap();
        Fixture {
            _directory: directory,
            store,
        }
    }

    const HOST: &str = "urn:uuid:9f0c1d6e-3a52-4b8e-9d4c-2f1e8a7b6c5d";

    fn sign_in(replaces: u32, generation: u32, subject: &str) -> ChatGptSignInRecord {
        ChatGptSignInRecord {
            replaces_generation: replaces,
            client_id: "oaiapp_1".to_owned(),
            subject: subject.to_owned(),
            email: Some("owner@example.com".to_owned()),
            token_generation: generation,
            token_parts: 3,
        }
    }

    #[tokio::test]
    async fn the_chatgpt_account_is_one_row_that_keeps_its_host_id() {
        let fixture = fixture();

        assert_eq!(fixture.store.chatgpt_account().await.unwrap(), None);
        let first = fixture
            .store
            .ensure_chatgpt_account(HOST.to_owned())
            .await
            .unwrap();
        let again = fixture
            .store
            .ensure_chatgpt_account("urn:uuid:other".to_owned())
            .await
            .unwrap();

        assert_eq!(first.host_id, HOST);
        assert_eq!(again.host_id, HOST);
        assert_eq!(first.state, ChatGptAccountState::SignedOut);
        assert_eq!((first.token_generation, first.token_parts), (0, 0));
        assert!(!first.notice_seen);
    }

    #[tokio::test]
    async fn tokens_are_recorded_only_over_the_generation_they_replace() {
        let fixture = fixture();
        fixture
            .store
            .ensure_chatgpt_account(HOST.to_owned())
            .await
            .unwrap();

        let signed_in = fixture
            .store
            .record_chatgpt_sign_in(sign_in(0, 1, "user-1"))
            .await
            .unwrap()
            .unwrap();
        let stale_sign_in = fixture
            .store
            .record_chatgpt_sign_in(sign_in(0, 2, "user-1"))
            .await
            .unwrap();
        let refreshed = fixture.store.record_chatgpt_tokens(1, 2, 2).await.unwrap();
        let stale_refresh = fixture.store.record_chatgpt_tokens(1, 3, 2).await.unwrap();

        assert_eq!(signed_in.state, ChatGptAccountState::SignedIn);
        assert_eq!(signed_in.subject.as_deref(), Some("user-1"));
        assert_eq!(stale_sign_in, None);
        assert!(refreshed);
        assert!(!stale_refresh);
        let account = fixture.store.chatgpt_account().await.unwrap().unwrap();
        assert_eq!((account.token_generation, account.token_parts), (2, 2));
    }

    #[tokio::test]
    async fn an_ended_session_and_a_sign_out_keep_the_client_and_the_generation() {
        let fixture = fixture();
        fixture
            .store
            .ensure_chatgpt_account(HOST.to_owned())
            .await
            .unwrap();
        fixture
            .store
            .record_chatgpt_sign_in(sign_in(0, 1, "user-1"))
            .await
            .unwrap();

        let stale_end = fixture
            .store
            .record_chatgpt_sign_in_needed(5)
            .await
            .unwrap();
        let ended = fixture
            .store
            .record_chatgpt_sign_in_needed(1)
            .await
            .unwrap();
        let after_end = fixture.store.chatgpt_account().await.unwrap().unwrap();
        let refresh_after_end = fixture.store.record_chatgpt_tokens(1, 2, 2).await.unwrap();
        let before_sign_out = fixture
            .store
            .record_chatgpt_sign_out()
            .await
            .unwrap()
            .unwrap();
        let signed_out = fixture.store.chatgpt_account().await.unwrap().unwrap();

        assert!(!stale_end);
        assert!(ended);
        assert_eq!(after_end.state, ChatGptAccountState::SignInNeeded);
        assert_eq!((after_end.token_generation, after_end.token_parts), (1, 0));
        assert!(!refresh_after_end);
        assert_eq!(before_sign_out.state, ChatGptAccountState::SignInNeeded);
        assert_eq!(signed_out.state, ChatGptAccountState::SignedOut);
        assert_eq!(signed_out.client_id.as_deref(), Some("oaiapp_1"));
        assert_eq!(signed_out.subject.as_deref(), Some("user-1"));
        assert_eq!(signed_out.token_generation, 1);
    }

    #[tokio::test]
    async fn the_notice_is_seen_once_and_stays_seen() {
        let fixture = fixture();
        fixture
            .store
            .ensure_chatgpt_account(HOST.to_owned())
            .await
            .unwrap();

        fixture.store.record_chatgpt_notice_seen().await.unwrap();
        fixture.store.record_chatgpt_sign_out().await.unwrap();

        assert!(
            fixture
                .store
                .chatgpt_account()
                .await
                .unwrap()
                .unwrap()
                .notice_seen
        );
    }
}
