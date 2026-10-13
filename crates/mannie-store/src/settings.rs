//! The settings repository: each settings section is one versioned JSON
//! document in `settings_sections`.

use mannie_types::settings::SettingsSection;
use rusqlite::OptionalExtension;

use crate::error::StoreError;
use crate::{Durability, Store};

const SELECT_SECTION: &str = "SELECT version, document FROM settings_sections WHERE key = ?1";
const SELECT_SECTION_VERSION: &str = "SELECT version FROM settings_sections WHERE key = ?1";
const UPSERT_SECTION: &str = "INSERT INTO settings_sections (key, version, document, updated_at) \
     VALUES (?1, ?2, ?3, ?4) \
     ON CONFLICT (key) DO UPDATE SET version = excluded.version, document = excluded.document, \
     updated_at = excluded.updated_at";

/// Every statement of this module, for the test that prepares them against the
/// migrated schema.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[SELECT_SECTION, SELECT_SECTION_VERSION, UPSERT_SECTION];

/// Why a settings section could not be read or saved.
#[derive(Debug, thiserror::Error)]
pub enum SettingsStoreError {
    /// A newer version of Mannie wrote the section. This version neither reads
    /// nor overwrites it.
    #[error(
        "the {key} settings were written by a newer version of Mannie (document version {stored}, and this version knows version {known})"
    )]
    NewerVersion {
        /// The section key.
        key: &'static str,
        /// The version of the stored document.
        stored: u32,
        /// The version this binary knows.
        known: u32,
    },
    /// The stored document has an older version that this binary cannot
    /// upgrade.
    #[error(
        "the {key} settings have document version {stored}, which this version of Mannie cannot read"
    )]
    UnsupportedVersion {
        /// The section key.
        key: &'static str,
        /// The version of the stored document.
        stored: u32,
    },
    /// The stored document is not a valid section.
    #[error("the stored {key} settings are not valid")]
    Invalid {
        /// The section key.
        key: &'static str,
        /// What is wrong with the document.
        #[source]
        source: serde_json::Error,
    },
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl SettingsStoreError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NewerVersion { .. } => "settings.newer_version",
            Self::UnsupportedVersion { .. } => "settings.unsupported_version",
            Self::Invalid { .. } => "settings.invalid",
            Self::Store(error) => error.code(),
        }
    }
}

impl Store {
    /// Reads a settings section, or its defaults when it was never saved.
    pub async fn load_settings<S: SettingsSection>(&self) -> Result<S, SettingsStoreError> {
        let stored = self
            .read(|connection| {
                connection
                    .query_row(SELECT_SECTION, [S::KEY], |row| {
                        Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
                    })
                    .optional()
            })
            .await?;
        let Some((version, document)) = stored else {
            return Ok(S::default());
        };
        if version > S::VERSION {
            return Err(SettingsStoreError::NewerVersion {
                key: S::KEY,
                stored: version,
                known: S::VERSION,
            });
        }
        if version < S::VERSION {
            return Err(SettingsStoreError::UnsupportedVersion {
                key: S::KEY,
                stored: version,
            });
        }
        serde_json::from_str(&document).map_err(|source| SettingsStoreError::Invalid {
            key: S::KEY,
            source,
        })
    }

    /// Saves a settings section durably. A section that a newer version of
    /// Mannie wrote is not overwritten.
    pub async fn save_settings<S: SettingsSection>(
        &self,
        section: &S,
    ) -> Result<(), SettingsStoreError> {
        let document =
            serde_json::to_string(section).map_err(|source| SettingsStoreError::Invalid {
                key: S::KEY,
                source,
            })?;
        let updated_at = self.clock().now().as_millis();
        let stored_newer = self
            .write(Durability::Full, move |transaction| {
                let stored: Option<u32> = transaction
                    .query_row(SELECT_SECTION_VERSION, [S::KEY], |row| row.get(0))
                    .optional()?;
                if let Some(version) = stored.filter(|version| *version > S::VERSION) {
                    return Ok(Some(version));
                }
                transaction.execute(
                    UPSERT_SECTION,
                    rusqlite::params![S::KEY, S::VERSION, document, updated_at],
                )?;
                Ok(None)
            })
            .await?;
        match stored_newer {
            Some(version) => Err(SettingsStoreError::NewerVersion {
                key: S::KEY,
                stored: version,
                known: S::VERSION,
            }),
            None => Ok(()),
        }
    }
}
