//! The search profiles repository (ADR-0047): embedding and rerank models,
//! each on one server. The credential of a profile lives in the operating
//! system's credential store; this table holds only the name of its entry
//! (ADR-0010).

use mannie_types::clock::UtcMillis;
use mannie_types::ids::ProfileId;
use mannie_types::provider::{SearchKind, SearchProfile, ServerKind};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::Store;
use crate::writer::TransactError;
use crate::{Durability, ProfileStoreError};

/// The version of the document this module writes.
const DOCUMENT_VERSION: u32 = 1;

const SELECT_ALL: &str = "SELECT id, name, kind, base_url, model, credential_ref, config_revision, version, \
     document, created_at, updated_at FROM provider_search_profiles ORDER BY created_at, id";
const SELECT_ONE: &str = "SELECT id, name, kind, base_url, model, credential_ref, config_revision, version, \
     document, created_at, updated_at FROM provider_search_profiles WHERE id = ?1";
const SELECT_NAMES: &str = "SELECT id, name FROM provider_search_profiles";
const INSERT: &str = "INSERT INTO provider_search_profiles \
     (id, name, kind, base_url, model, credential_ref, config_revision, version, document, created_at, \
     updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8, ?9, ?9)";
const UPDATE: &str = "UPDATE provider_search_profiles SET name = ?2, kind = ?3, base_url = ?4, model = ?5, \
     credential_ref = ?6, config_revision = ?7, version = ?8, document = ?9, updated_at = ?10 WHERE id = ?1";
const DELETE: &str = "DELETE FROM provider_search_profiles WHERE id = ?1";
const UPDATE_DOCUMENT: &str =
    "UPDATE provider_search_profiles SET version = ?2, document = ?3 WHERE id = ?1";

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    SELECT_ALL,
    SELECT_ONE,
    SELECT_NAMES,
    INSERT,
    UPDATE,
    DELETE,
    UPDATE_DOCUMENT,
];

/// The document of a search profile: what is not a column of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchDocument {
    #[serde(default)]
    query_prefix: String,
    #[serde(default)]
    document_prefix: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    server: Option<ServerKind>,
}

/// A search profile as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSearchProfile {
    /// The profile.
    pub id: ProfileId,
    /// Its name.
    pub name: String,
    /// What its model does.
    pub kind: SearchKind,
    /// The server's base URL.
    pub base_url: String,
    /// The model.
    pub model: String,
    /// The name of its entry in the credential store, if it has a credential.
    pub credential_ref: Option<String>,
    /// Increases when the kind, the base URL, the model, the credential or a
    /// prefix changes.
    pub config_revision: u32,
    /// The text before each query that an embedding profile embeds.
    pub query_prefix: String,
    /// The text before each document that an embedding profile embeds.
    pub document_prefix: String,
    /// The server the test recognized.
    pub server: Option<ServerKind>,
    /// When it was created.
    pub created_at: UtcMillis,
    /// When it last changed.
    pub updated_at: UtcMillis,
}

impl StoredSearchProfile {
    /// The profile as the UI shows it.
    pub fn view(&self) -> SearchProfile {
        SearchProfile {
            id: self.id.clone(),
            name: self.name.clone(),
            kind: self.kind,
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            has_credential: self.credential_ref.is_some(),
            query_prefix: self.query_prefix.clone(),
            document_prefix: self.document_prefix.clone(),
            server: self.server,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// A search profile's fields, checked by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchProfileFields {
    /// The name.
    pub name: String,
    /// What the model does.
    pub kind: SearchKind,
    /// The base URL.
    pub base_url: String,
    /// The model.
    pub model: String,
    /// The name of the credential's entry, if the profile has a credential.
    pub credential_ref: Option<String>,
    /// The query prefix.
    pub query_prefix: String,
    /// The document prefix.
    pub document_prefix: String,
}

struct RawSearchProfile {
    id: String,
    name: String,
    kind: String,
    base_url: String,
    model: String,
    credential_ref: Option<String>,
    config_revision: i64,
    version: i64,
    document: String,
    created_at: i64,
    updated_at: i64,
}

fn raw_from_row(row: &Row<'_>) -> rusqlite::Result<RawSearchProfile> {
    Ok(RawSearchProfile {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        base_url: row.get(3)?,
        model: row.get(4)?,
        credential_ref: row.get(5)?,
        config_revision: row.get(6)?,
        version: row.get(7)?,
        document: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

impl RawSearchProfile {
    fn into_profile(self) -> Result<StoredSearchProfile, ProfileStoreError> {
        let invalid = |what: String| ProfileStoreError::InvalidData { what };
        if self.version != i64::from(DOCUMENT_VERSION) {
            return Err(invalid(format!(
                "search profile {} has document version {}",
                self.id, self.version
            )));
        }
        let document: SearchDocument = serde_json::from_str(&self.document)
            .map_err(|error| invalid(format!("search profile {}: {error}", self.id)))?;
        let kind = SearchKind::parse(&self.kind)
            .ok_or_else(|| invalid(format!("unknown kind {:?}", self.kind)))?;
        Ok(StoredSearchProfile {
            id: ProfileId::parse(&self.id).map_err(|error| invalid(error.to_string()))?,
            name: self.name,
            kind,
            base_url: self.base_url,
            model: self.model,
            credential_ref: self.credential_ref,
            config_revision: u32::try_from(self.config_revision).map_err(|_out_of_range| {
                invalid("the configuration revision is out of range".to_owned())
            })?,
            query_prefix: document.query_prefix,
            document_prefix: document.document_prefix,
            server: document.server,
            created_at: UtcMillis::from_millis(self.created_at),
            updated_at: UtcMillis::from_millis(self.updated_at),
        })
    }
}

fn read_one(
    connection: &Connection,
    id: &str,
) -> Result<Option<StoredSearchProfile>, ProfileStoreError> {
    let raw = connection
        .prepare_cached(SELECT_ONE)?
        .query_row([id], raw_from_row)
        .optional()?;
    raw.map(RawSearchProfile::into_profile).transpose()
}

/// Refuses a name that another search profile has, comparing case the way
/// people read it, as the chat profiles do.
fn refuse_taken_name(
    transaction: &Transaction<'_>,
    name: &str,
    except: Option<&str>,
) -> Result<(), TransactError<ProfileStoreError>> {
    let wanted = name.to_lowercase();
    let mut select = transaction.prepare_cached(SELECT_NAMES)?;
    let rows = select.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (id, existing) = row?;
        let is_other = except != Some(id.as_str());
        if is_other && existing.to_lowercase() == wanted {
            return Err(TransactError::Rejected(ProfileStoreError::NameTaken));
        }
    }
    Ok(())
}

fn document_json(
    fields: &SearchProfileFields,
    server: Option<ServerKind>,
) -> Result<String, ProfileStoreError> {
    let document = SearchDocument {
        query_prefix: fields.query_prefix.clone(),
        document_prefix: fields.document_prefix.clone(),
        server,
    };
    serde_json::to_string(&document).map_err(|error| ProfileStoreError::InvalidData {
        what: error.to_string(),
    })
}

impl Store {
    /// Every search profile, the oldest first.
    pub async fn list_search_profiles(
        &self,
    ) -> Result<Vec<StoredSearchProfile>, ProfileStoreError> {
        let raw: Vec<RawSearchProfile> = self
            .read(|connection| {
                let mut select = connection.prepare_cached(SELECT_ALL)?;
                let rows = select.query_map([], raw_from_row)?;
                rows.collect()
            })
            .await?;
        raw.into_iter()
            .map(RawSearchProfile::into_profile)
            .collect()
    }

    /// One search profile.
    pub async fn search_profile(
        &self,
        id: &ProfileId,
    ) -> Result<Option<StoredSearchProfile>, ProfileStoreError> {
        let id = id.as_str().to_owned();
        let raw = self
            .read(move |connection| {
                connection
                    .prepare_cached(SELECT_ONE)?
                    .query_row([id], raw_from_row)
                    .optional()
            })
            .await?;
        raw.map(RawSearchProfile::into_profile).transpose()
    }

    /// Adds a search profile.
    pub async fn insert_search_profile(
        &self,
        id: &ProfileId,
        fields: SearchProfileFields,
    ) -> Result<StoredSearchProfile, ProfileStoreError> {
        let id = id.as_str().to_owned();
        let now = self.clock().now().as_millis();
        let document = document_json(&fields, None)?;
        self.transact(Durability::Full, move |transaction| {
            refuse_taken_name(transaction, &fields.name, None)?;
            transaction.prepare_cached(INSERT)?.execute(params![
                id,
                fields.name,
                fields.kind.as_str(),
                fields.base_url,
                fields.model,
                fields.credential_ref,
                DOCUMENT_VERSION,
                document,
                now
            ])?;
            read_one(transaction, &id)
                .map_err(TransactError::Rejected)?
                .ok_or(TransactError::Rejected(ProfileStoreError::NotFound))
        })
        .await
        .map_err(ProfileStoreError::from)
    }

    /// Replaces a search profile's fields. The configuration revision
    /// increases when the kind, the base URL, the model, the credential or a
    /// prefix changes, so that what the profile's vectors mean is known.
    /// Returns the profile before and after the change.
    pub async fn update_search_profile(
        &self,
        id: &ProfileId,
        fields: SearchProfileFields,
    ) -> Result<(StoredSearchProfile, StoredSearchProfile), ProfileStoreError> {
        let id = id.as_str().to_owned();
        let now = self.clock().now().as_millis();
        self.transact(Durability::Full, move |transaction| {
            let before = read_one(transaction, &id)
                .map_err(TransactError::Rejected)?
                .ok_or(TransactError::Rejected(ProfileStoreError::NotFound))?;
            refuse_taken_name(transaction, &fields.name, Some(&id))?;
            let connection_changed = before.kind != fields.kind
                || before.base_url != fields.base_url
                || before.model != fields.model
                || before.credential_ref != fields.credential_ref;
            let prefixes_changed = before.query_prefix != fields.query_prefix
                || before.document_prefix != fields.document_prefix;
            let revision = if connection_changed || prefixes_changed {
                before.config_revision.saturating_add(1)
            } else {
                before.config_revision
            };
            // The server the test found out stays only while the connection
            // stays the same.
            let server = if connection_changed {
                None
            } else {
                before.server
            };
            let document = document_json(&fields, server).map_err(TransactError::Rejected)?;
            transaction.prepare_cached(UPDATE)?.execute(params![
                id,
                fields.name,
                fields.kind.as_str(),
                fields.base_url,
                fields.model,
                fields.credential_ref,
                revision,
                DOCUMENT_VERSION,
                document,
                now
            ])?;
            let after = read_one(transaction, &id)
                .map_err(TransactError::Rejected)?
                .ok_or(TransactError::Rejected(ProfileStoreError::NotFound))?;
            Ok((before, after))
        })
        .await
        .map_err(ProfileStoreError::from)
    }

    /// Deletes a search profile and returns it, so that the caller can delete
    /// its credential.
    pub async fn delete_search_profile(
        &self,
        id: &ProfileId,
    ) -> Result<StoredSearchProfile, ProfileStoreError> {
        let id = id.as_str().to_owned();
        self.transact(Durability::Full, move |transaction| {
            let deleted = read_one(transaction, &id)
                .map_err(TransactError::Rejected)?
                .ok_or(TransactError::Rejected(ProfileStoreError::NotFound))?;
            transaction.prepare_cached(DELETE)?.execute([&id])?;
            Ok(deleted)
        })
        .await
        .map_err(ProfileStoreError::from)
    }

    /// Records the server that the test of a search profile recognized.
    pub async fn record_search_profile_server(
        &self,
        id: &ProfileId,
        server: ServerKind,
    ) -> Result<(), ProfileStoreError> {
        let id = id.as_str().to_owned();
        self.transact(Durability::Full, move |transaction| {
            let profile = read_one(transaction, &id)
                .map_err(TransactError::Rejected)?
                .ok_or(TransactError::Rejected(ProfileStoreError::NotFound))?;
            let fields = SearchProfileFields {
                name: profile.name,
                kind: profile.kind,
                base_url: profile.base_url,
                model: profile.model,
                credential_ref: profile.credential_ref,
                query_prefix: profile.query_prefix,
                document_prefix: profile.document_prefix,
            };
            let document = document_json(&fields, Some(server)).map_err(TransactError::Rejected)?;
            transaction
                .prepare_cached(UPDATE_DOCUMENT)?
                .execute(params![id, DOCUMENT_VERSION, document])?;
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
    use mannie_types::ids::ProfileId;
    use mannie_types::provider::{SearchKind, ServerKind};

    use super::SearchProfileFields;
    use crate::{ProfileStoreError, Store, StoreOptions};

    fn fields(name: &str) -> SearchProfileFields {
        SearchProfileFields {
            name: name.to_owned(),
            kind: SearchKind::Embeddings,
            base_url: "http://gpu-box.local:8080/v1".to_owned(),
            model: "an-embedding-model".to_owned(),
            credential_ref: None,
            query_prefix: "Instruct: find passages\nQuery: ".to_owned(),
            document_prefix: String::new(),
        }
    }

    fn id(number: u32) -> ProfileId {
        ProfileId::parse(&format!("0199e0f2-0000-7000-8000-{number:012}")).unwrap()
    }

    fn open_store() -> (tempfile::TempDir, Store) {
        let folder = tempfile::tempdir().unwrap();
        let clock: Arc<dyn Clock> =
            Arc::new(ManualClock::new(UtcMillis::from_millis(1_791_892_800_000)));
        let options = StoreOptions::new(folder.path().join("backups"), clock);
        let store = Store::open(&folder.path().join("mannie.sqlite"), &options).unwrap();
        (folder, store)
    }

    #[tokio::test]
    async fn a_profile_keeps_its_fields_and_its_prefixes_exactly() {
        let (_folder, store) = open_store();

        let stored = store
            .insert_search_profile(&id(1), fields("Embeddings"))
            .await
            .unwrap();

        assert_eq!(stored.kind, SearchKind::Embeddings);
        assert_eq!(stored.query_prefix, "Instruct: find passages\nQuery: ");
        assert_eq!(stored.config_revision, 1);
        assert_eq!(store.list_search_profiles().await.unwrap(), vec![stored]);
    }

    #[tokio::test]
    async fn names_are_unique_regardless_of_case() {
        let (_folder, store) = open_store();
        store
            .insert_search_profile(&id(1), fields("Ärzte"))
            .await
            .unwrap();

        let taken = store.insert_search_profile(&id(2), fields("ärzte")).await;

        assert!(
            matches!(taken, Err(ProfileStoreError::NameTaken)),
            "{taken:?}"
        );
    }

    #[tokio::test]
    async fn the_revision_follows_the_connection_and_the_prefixes_and_the_server_the_connection() {
        let (_folder, store) = open_store();
        store
            .insert_search_profile(&id(1), fields("Embeddings"))
            .await
            .unwrap();
        store
            .record_search_profile_server(&id(1), ServerKind::Vllm)
            .await
            .unwrap();

        let (_, renamed) = store
            .update_search_profile(&id(1), fields("Renamed"))
            .await
            .unwrap();
        let new_prefix = SearchProfileFields {
            document_prefix: "passage: ".to_owned(),
            ..fields("Renamed")
        };
        let (_, prefixed) = store
            .update_search_profile(&id(1), new_prefix.clone())
            .await
            .unwrap();
        let moved = SearchProfileFields {
            base_url: "https://search.example.com/v1".to_owned(),
            ..new_prefix
        };
        let (_, after_move) = store.update_search_profile(&id(1), moved).await.unwrap();

        assert_eq!(
            (renamed.config_revision, renamed.server),
            (1, Some(ServerKind::Vllm))
        );
        assert_eq!(
            (prefixed.config_revision, prefixed.server),
            (2, Some(ServerKind::Vllm))
        );
        assert_eq!((after_move.config_revision, after_move.server), (3, None));
    }

    #[tokio::test]
    async fn deleting_returns_the_profile_and_a_missing_one_is_not_found() {
        let (_folder, store) = open_store();
        store
            .insert_search_profile(&id(1), fields("Embeddings"))
            .await
            .unwrap();

        let deleted = store.delete_search_profile(&id(1)).await.unwrap();
        let again = store.delete_search_profile(&id(1)).await;

        assert_eq!(deleted.name, "Embeddings");
        assert!(matches!(again, Err(ProfileStoreError::NotFound)));
        assert!(store.search_profile(&id(1)).await.unwrap().is_none());
    }
}
