//! The provider profiles repository (docs/features/chat/providers.md). The
//! credential of a profile lives in the operating system's credential store;
//! this table holds only the name of its entry (ADR-0010).

use mannie_types::clock::UtcMillis;
use mannie_types::ids::ProfileId;
use mannie_types::provider::{ApiInterface, ProviderProfile, ReasoningEffort, ServerKind};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;
use crate::writer::TransactError;
use crate::{Durability, Store};

pub(crate) mod search;

/// The version of the profile document this module writes.
const DOCUMENT_VERSION: u32 = 1;

const SELECT_ALL: &str = "SELECT id, name, interface, base_url, model, credential_ref, is_default, config_revision, \
     version, document, created_at, updated_at FROM provider_profiles ORDER BY created_at, id";
const SELECT_ONE: &str = "SELECT id, name, interface, base_url, model, credential_ref, is_default, config_revision, \
     version, document, created_at, updated_at FROM provider_profiles WHERE id = ?1";
const SELECT_DEFAULT: &str = "SELECT id, name, interface, base_url, model, credential_ref, is_default, config_revision, \
     version, document, created_at, updated_at FROM provider_profiles WHERE is_default = 1";
const SELECT_NAMES: &str = "SELECT id, name FROM provider_profiles";
const SELECT_HAS_DEFAULT: &str = "SELECT 1 FROM provider_profiles WHERE is_default = 1";
const INSERT: &str = "INSERT INTO provider_profiles \
     (id, name, interface, base_url, model, credential_ref, is_default, config_revision, version, document, \
     created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9, ?10, ?10)";
const UPDATE: &str = "UPDATE provider_profiles SET name = ?2, interface = ?3, base_url = ?4, model = ?5, \
     credential_ref = ?6, config_revision = ?7, version = ?8, document = ?9, updated_at = ?10 WHERE id = ?1";
const DELETE: &str = "DELETE FROM provider_profiles WHERE id = ?1";
const CLEAR_DEFAULT: &str = "UPDATE provider_profiles SET is_default = 0 WHERE is_default = 1";
const SET_DEFAULT: &str = "UPDATE provider_profiles SET is_default = 1 WHERE id = ?1";
const SELECT_OLDEST: &str = "SELECT id FROM provider_profiles ORDER BY created_at, id LIMIT 1";
const UPDATE_DOCUMENT: &str =
    "UPDATE provider_profiles SET version = ?2, document = ?3 WHERE id = ?1";

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    SELECT_ALL,
    SELECT_ONE,
    SELECT_DEFAULT,
    SELECT_NAMES,
    SELECT_HAS_DEFAULT,
    INSERT,
    UPDATE,
    DELETE,
    CLEAR_DEFAULT,
    SET_DEFAULT,
    SELECT_OLDEST,
    UPDATE_DOCUMENT,
];

/// The document of a profile: what is not a column of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileDocument {
    context_window: u32,
    output_limit: u32,
    reasoning_effort: ReasoningEffort,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    server: Option<ServerKind>,
    /// Whether the model gets tools. Profiles stored before system tools
    /// read it as on, the default.
    #[serde(default = "uses_tools_by_default")]
    use_tools: bool,
}

fn uses_tools_by_default() -> bool {
    true
}

/// A provider profile as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredProfile {
    /// The profile.
    pub id: ProfileId,
    /// Its name.
    pub name: String,
    /// The API it speaks.
    pub interface: ApiInterface,
    /// The server's base URL.
    pub base_url: String,
    /// The model.
    pub model: String,
    /// The name of its entry in the credential store, if it has a credential.
    pub credential_ref: Option<String>,
    /// Whether it is the default profile.
    pub is_default: bool,
    /// Increases when the interface, the base URL, the model or the credential
    /// changes.
    pub config_revision: u32,
    /// The context window, in tokens.
    pub context_window: u32,
    /// The output limit, in tokens.
    pub output_limit: u32,
    /// The reasoning effort.
    pub reasoning_effort: ReasoningEffort,
    /// Whether the model gets tools.
    pub use_tools: bool,
    /// The server the provider test recognized.
    pub server: Option<ServerKind>,
    /// When it was created.
    pub created_at: UtcMillis,
    /// When it last changed.
    pub updated_at: UtcMillis,
}

impl StoredProfile {
    /// The profile as the UI shows it.
    pub fn view(&self) -> ProviderProfile {
        ProviderProfile {
            id: self.id.clone(),
            name: self.name.clone(),
            interface: self.interface,
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            has_credential: self.credential_ref.is_some(),
            context_window: self.context_window,
            output_limit: self.output_limit,
            reasoning_effort: self.reasoning_effort,
            use_tools: self.use_tools,
            is_default: self.is_default,
            server: self.server,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }

    /// The binding that provider-private data of this profile carries: the
    /// profile and its configuration revision (ADR-0011, decision 6).
    pub fn binding(&self) -> String {
        format!("{}:{}", self.id, self.config_revision)
    }
}

/// A profile's fields, checked by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileFields {
    /// The name.
    pub name: String,
    /// The API interface.
    pub interface: ApiInterface,
    /// The base URL.
    pub base_url: String,
    /// The model.
    pub model: String,
    /// The name of the credential's entry, if the profile has a credential.
    pub credential_ref: Option<String>,
    /// The context window, in tokens.
    pub context_window: u32,
    /// The output limit, in tokens.
    pub output_limit: u32,
    /// The reasoning effort.
    pub reasoning_effort: ReasoningEffort,
    /// Whether the model gets tools.
    pub use_tools: bool,
}

/// Why a profile could not be saved or deleted.
#[derive(Debug, thiserror::Error)]
pub enum ProfileStoreError {
    /// Another profile has the same name, regardless of case.
    #[error("another profile has the same name")]
    NameTaken,
    /// The profile does not exist.
    #[error("the profile does not exist")]
    NotFound,
    /// A stored profile does not have the shape this version expects.
    #[error("a stored profile is not valid: {what}")]
    InvalidData {
        /// What is wrong.
        what: String,
    },
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl ProfileStoreError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NameTaken => "profile_store.name_taken",
            Self::NotFound => "profile_store.not_found",
            Self::InvalidData { .. } => "profile_store.invalid_data",
            Self::Store(error) => error.code(),
        }
    }
}

impl From<rusqlite::Error> for ProfileStoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Store(StoreError::query(error))
    }
}

impl From<TransactError<Self>> for ProfileStoreError {
    fn from(error: TransactError<Self>) -> Self {
        match error {
            TransactError::Rejected(error) => error,
            TransactError::Store(error) => Self::Store(error),
        }
    }
}

struct RawProfile {
    id: String,
    name: String,
    interface: String,
    base_url: String,
    model: String,
    credential_ref: Option<String>,
    is_default: bool,
    config_revision: i64,
    version: i64,
    document: String,
    created_at: i64,
    updated_at: i64,
}

fn raw_from_row(row: &Row<'_>) -> rusqlite::Result<RawProfile> {
    Ok(RawProfile {
        id: row.get(0)?,
        name: row.get(1)?,
        interface: row.get(2)?,
        base_url: row.get(3)?,
        model: row.get(4)?,
        credential_ref: row.get(5)?,
        is_default: row.get(6)?,
        config_revision: row.get(7)?,
        version: row.get(8)?,
        document: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

impl RawProfile {
    fn into_profile(self) -> Result<StoredProfile, ProfileStoreError> {
        let invalid = |what: String| ProfileStoreError::InvalidData { what };
        if self.version != i64::from(DOCUMENT_VERSION) {
            return Err(invalid(format!(
                "profile {} has document version {}",
                self.id, self.version
            )));
        }
        let document: ProfileDocument = serde_json::from_str(&self.document)
            .map_err(|error| invalid(format!("profile {}: {error}", self.id)))?;
        let interface = ApiInterface::parse(&self.interface)
            .ok_or_else(|| invalid(format!("unknown interface {:?}", self.interface)))?;
        Ok(StoredProfile {
            id: ProfileId::parse(&self.id).map_err(|error| invalid(error.to_string()))?,
            name: self.name,
            interface,
            base_url: self.base_url,
            model: self.model,
            credential_ref: self.credential_ref,
            is_default: self.is_default,
            config_revision: u32::try_from(self.config_revision).map_err(|_out_of_range| {
                invalid("the configuration revision is out of range".to_owned())
            })?,
            context_window: document.context_window,
            output_limit: document.output_limit,
            reasoning_effort: document.reasoning_effort,
            use_tools: document.use_tools,
            server: document.server,
            created_at: UtcMillis::from_millis(self.created_at),
            updated_at: UtcMillis::from_millis(self.updated_at),
        })
    }
}

fn read_one(connection: &Connection, id: &str) -> Result<Option<StoredProfile>, ProfileStoreError> {
    let raw = connection
        .prepare_cached(SELECT_ONE)?
        .query_row([id], raw_from_row)
        .optional()?;
    raw.map(RawProfile::into_profile).transpose()
}

/// Refuses a name that another profile has, comparing case the way people
/// read it: `to_lowercase` covers more than SQLite's ASCII-only NOCASE.
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
    fields: &ProfileFields,
    server: Option<ServerKind>,
) -> Result<String, ProfileStoreError> {
    let document = ProfileDocument {
        context_window: fields.context_window,
        output_limit: fields.output_limit,
        reasoning_effort: fields.reasoning_effort,
        server,
        use_tools: fields.use_tools,
    };
    serde_json::to_string(&document).map_err(|error| ProfileStoreError::InvalidData {
        what: error.to_string(),
    })
}

impl Store {
    /// Every profile, the oldest first.
    pub async fn list_profiles(&self) -> Result<Vec<StoredProfile>, ProfileStoreError> {
        let raw: Vec<RawProfile> = self
            .read(|connection| {
                let mut select = connection.prepare_cached(SELECT_ALL)?;
                let rows = select.query_map([], raw_from_row)?;
                rows.collect()
            })
            .await?;
        raw.into_iter().map(RawProfile::into_profile).collect()
    }

    /// One profile.
    pub async fn profile(
        &self,
        id: &ProfileId,
    ) -> Result<Option<StoredProfile>, ProfileStoreError> {
        let id = id.as_str().to_owned();
        let raw = self
            .read(move |connection| {
                connection
                    .prepare_cached(SELECT_ONE)?
                    .query_row([id], raw_from_row)
                    .optional()
            })
            .await?;
        raw.map(RawProfile::into_profile).transpose()
    }

    /// The default profile, if any profile exists.
    pub async fn default_profile(&self) -> Result<Option<StoredProfile>, ProfileStoreError> {
        let raw = self
            .read(|connection| {
                connection
                    .prepare_cached(SELECT_DEFAULT)?
                    .query_row([], raw_from_row)
                    .optional()
            })
            .await?;
        raw.map(RawProfile::into_profile).transpose()
    }

    /// Adds a profile. The first profile becomes the default.
    pub async fn insert_profile(
        &self,
        id: &ProfileId,
        fields: ProfileFields,
    ) -> Result<StoredProfile, ProfileStoreError> {
        let id = id.as_str().to_owned();
        let now = self.clock().now().as_millis();
        let document = document_json(&fields, None)?;
        self.transact(Durability::Full, move |transaction| {
            refuse_taken_name(transaction, &fields.name, None)?;
            let has_default = transaction
                .prepare_cached(SELECT_HAS_DEFAULT)?
                .query_row([], |_row| Ok(()))
                .optional()?
                .is_some();
            transaction.prepare_cached(INSERT)?.execute(params![
                id,
                fields.name,
                fields.interface.as_str(),
                fields.base_url,
                fields.model,
                fields.credential_ref,
                !has_default,
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

    /// Replaces a profile's fields. The configuration revision increases when
    /// the interface, the base URL, the model or the credential changes.
    /// Returns the profile before and after the change.
    pub async fn update_profile(
        &self,
        id: &ProfileId,
        fields: ProfileFields,
    ) -> Result<(StoredProfile, StoredProfile), ProfileStoreError> {
        let id = id.as_str().to_owned();
        let now = self.clock().now().as_millis();
        self.transact(Durability::Full, move |transaction| {
            let before = read_one(transaction, &id)
                .map_err(TransactError::Rejected)?
                .ok_or(TransactError::Rejected(ProfileStoreError::NotFound))?;
            refuse_taken_name(transaction, &fields.name, Some(&id))?;
            let connection_changed = before.interface != fields.interface
                || before.base_url != fields.base_url
                || before.model != fields.model
                || before.credential_ref != fields.credential_ref;
            let revision = if connection_changed {
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
                fields.interface.as_str(),
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

    /// Deletes a profile and returns it, so that the caller can delete its
    /// credential. When it was the default, the oldest remaining profile
    /// becomes the default.
    pub async fn delete_profile(&self, id: &ProfileId) -> Result<StoredProfile, ProfileStoreError> {
        let id = id.as_str().to_owned();
        self.transact(Durability::Full, move |transaction| {
            let deleted = read_one(transaction, &id)
                .map_err(TransactError::Rejected)?
                .ok_or(TransactError::Rejected(ProfileStoreError::NotFound))?;
            transaction.prepare_cached(DELETE)?.execute([&id])?;
            if deleted.is_default {
                let oldest: Option<String> = transaction
                    .prepare_cached(SELECT_OLDEST)?
                    .query_row([], |row| row.get(0))
                    .optional()?;
                if let Some(oldest) = oldest {
                    transaction.prepare_cached(SET_DEFAULT)?.execute([oldest])?;
                }
            }
            Ok(deleted)
        })
        .await
        .map_err(ProfileStoreError::from)
    }

    /// Makes a profile the default.
    pub async fn set_default_profile(&self, id: &ProfileId) -> Result<(), ProfileStoreError> {
        let id = id.as_str().to_owned();
        self.transact(Durability::Full, move |transaction| {
            let exists = read_one(transaction, &id)
                .map_err(TransactError::Rejected)?
                .is_some();
            if !exists {
                return Err(TransactError::Rejected(ProfileStoreError::NotFound));
            }
            transaction.prepare_cached(CLEAR_DEFAULT)?.execute([])?;
            transaction.prepare_cached(SET_DEFAULT)?.execute([&id])?;
            Ok(())
        })
        .await
        .map_err(ProfileStoreError::from)
    }

    /// Records the server that the provider test recognized.
    pub async fn record_profile_server(
        &self,
        id: &ProfileId,
        server: ServerKind,
    ) -> Result<(), ProfileStoreError> {
        let id = id.as_str().to_owned();
        self.transact(Durability::Full, move |transaction| {
            let profile = read_one(transaction, &id)
                .map_err(TransactError::Rejected)?
                .ok_or(TransactError::Rejected(ProfileStoreError::NotFound))?;
            let fields = ProfileFields {
                name: profile.name,
                interface: profile.interface,
                base_url: profile.base_url,
                model: profile.model,
                credential_ref: profile.credential_ref,
                context_window: profile.context_window,
                output_limit: profile.output_limit,
                reasoning_effort: profile.reasoning_effort,
                use_tools: profile.use_tools,
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
mod tests;
