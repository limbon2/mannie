//! The operating system's credential store (ADR-0010).
//!
//! Provider credentials live only here, on the master: in Windows Credential
//! Manager on Windows, and in the Secret Service (GNOME Keyring, KWallet) on
//! Linux. The database and the settings hold only a [`CredentialRef`]. When no
//! credential store is available, [`CredentialStore::open`] fails with
//! [`CredentialError::Unavailable`], and Mannie refuses to save a credential;
//! there is no fallback to a file or to the database.
//!
//! Every call blocks on the operating system. Callers run them on a blocking
//! thread (`tokio::task::spawn_blocking`), never on an async worker thread.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use keyring_core::{CredentialStore as PlatformStore, Entry};
use mannie_types::secret::Secret;

/// The longest reference Mannie accepts. Platform stores limit the length of
/// their entry names.
const MAX_REFERENCE_LENGTH: usize = 100;

/// The name under which Mannie stores a credential: what the database and the
/// settings keep instead of the credential itself.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CredentialRef(String);

impl CredentialRef {
    /// Checks a reference: 1 to 100 ASCII letters, digits and `.`, `_`, `-`
    /// or `/`, such as `provider/0192f3a4-5b6c-7d8e-9f00-112233445566`.
    pub fn parse(reference: &str) -> Result<Self, CredentialError> {
        let has_valid_length = (1..=MAX_REFERENCE_LENGTH).contains(&reference.len());
        let has_valid_characters = reference
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'));
        if has_valid_length && has_valid_characters {
            Ok(Self(reference.to_owned()))
        } else {
            Err(CredentialError::InvalidReference {
                reference: reference.to_owned(),
            })
        }
    }

    /// The reference as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CredentialRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Why no credential store is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnavailableReason {
    /// Linux without a running Secret Service, such as GNOME Keyring or
    /// KWallet, on the session bus.
    NoSecretService,
    /// An operating system on which Mannie has no credential store. The
    /// desktop app does not run there.
    UnsupportedPlatform,
}

/// The error a platform store reports, without any data that could hold a
/// secret.
pub type PlatformError = Box<dyn std::error::Error + Send + Sync>;

/// Why a credential could not be saved, read or deleted.
#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    /// No credential store is available, so Mannie stores no credentials.
    #[error("no credential store is available ({reason:?}), so Mannie does not store credentials")]
    Unavailable {
        /// Why no store is available.
        reason: UnavailableReason,
        /// The platform's error, when there is one.
        #[source]
        source: Option<PlatformError>,
    },
    /// The store is locked, or the owner declined access.
    #[error("the credential store refused access")]
    AccessDenied {
        /// The platform's error.
        #[source]
        source: PlatformError,
    },
    /// No credential is stored under the reference.
    #[error("no credential is stored under {reference}")]
    NotFound {
        /// The reference.
        reference: CredentialRef,
    },
    /// The stored credential is not UTF-8 text, or the store's data for it is
    /// malformed. Its bytes are discarded and never reported.
    #[error("the credential stored under {reference} cannot be read")]
    Unreadable {
        /// The reference.
        reference: CredentialRef,
    },
    /// The reference is not a valid credential reference.
    #[error("{reference:?} is not a valid credential reference")]
    InvalidReference {
        /// The rejected reference.
        reference: String,
    },
    /// The store rejected the credential or its name, for example because it
    /// is too long for the platform.
    #[error("the credential store rejected the credential stored under {reference}")]
    Rejected {
        /// The reference.
        reference: CredentialRef,
    },
    /// The store failed.
    #[error("the credential store failed")]
    Failed {
        /// The platform's error, when there is one.
        #[source]
        source: Option<PlatformError>,
    },
}

impl CredentialError {
    /// The stable code of the error.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "credentials.unavailable",
            Self::AccessDenied { .. } => "credentials.access_denied",
            Self::NotFound { .. } => "credentials.not_found",
            Self::Unreadable { .. } => "credentials.unreadable",
            Self::InvalidReference { .. } => "credentials.invalid_reference",
            Self::Rejected { .. } => "credentials.rejected",
            Self::Failed { .. } => "credentials.failed",
        }
    }

    /// Maps a keyring error. Variants that carry the secret's bytes lose them
    /// here, so no error of this crate can show a secret.
    fn from_keyring(error: keyring_core::Error, reference: &CredentialRef) -> Self {
        match error {
            keyring_core::Error::NoStorageAccess(source) => Self::AccessDenied { source },
            keyring_core::Error::NoEntry => Self::NotFound {
                reference: reference.clone(),
            },
            keyring_core::Error::BadEncoding(_) | keyring_core::Error::BadDataFormat(..) => {
                Self::Unreadable {
                    reference: reference.clone(),
                }
            }
            keyring_core::Error::Invalid(..) | keyring_core::Error::TooLong(..) => Self::Rejected {
                reference: reference.clone(),
            },
            keyring_core::Error::PlatformFailure(source) => Self::Failed {
                source: Some(source),
            },
            other => {
                tracing::warn!(error = %describe_without_data(&other), "the credential store reported an unexpected error");
                Self::Failed { source: None }
            }
        }
    }
}

/// A description of a keyring error that leaves out any attached data.
fn describe_without_data(error: &keyring_core::Error) -> &'static str {
    match error {
        keyring_core::Error::BadStoreFormat(_) => "the store's data is malformed",
        keyring_core::Error::Ambiguous(_) => "more than one credential matches",
        keyring_core::Error::NoDefaultStore => "no default store is set",
        keyring_core::Error::NotSupportedByStore(_) => "the store does not support the operation",
        _ => "an error that keyring-core added in a later version",
    }
}

/// The operating system's credential store, with Mannie's entries named under
/// one service name, the bundle identifier (D9).
#[derive(Clone)]
pub struct CredentialStore {
    platform: Arc<PlatformStore>,
    service: String,
}

impl fmt::Debug for CredentialStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialStore")
            .field("service", &self.service)
            .finish_non_exhaustive()
    }
}

impl CredentialStore {
    /// Opens the credential store of the operating system. `service` names
    /// Mannie's entries; the desktop app passes its bundle identifier.
    pub fn open(service: &str) -> Result<Self, CredentialError> {
        let platform = platform::open()?;
        tracing::info!(store = %platform.vendor(), "opened the credential store");
        Ok(Self {
            platform,
            service: service.to_owned(),
        })
    }

    /// Saves a credential under `reference`, replacing a credential that is
    /// already stored there.
    pub fn save(&self, reference: &CredentialRef, secret: &Secret) -> Result<(), CredentialError> {
        let entry = self.entry(reference)?;
        entry
            .set_password(secret.expose())
            .map_err(|error| CredentialError::from_keyring(error, reference))?;
        tracing::info!(%reference, "saved a credential");
        Ok(())
    }

    /// Reads the credential stored under `reference`.
    pub fn read(&self, reference: &CredentialRef) -> Result<Secret, CredentialError> {
        let entry = self.entry(reference)?;
        let value = entry
            .get_password()
            .map_err(|error| CredentialError::from_keyring(error, reference))?;
        Ok(Secret::new(value))
    }

    /// Deletes the credential stored under `reference`.
    pub fn delete(&self, reference: &CredentialRef) -> Result<(), CredentialError> {
        let entry = self.entry(reference)?;
        entry
            .delete_credential()
            .map_err(|error| CredentialError::from_keyring(error, reference))?;
        tracing::info!(%reference, "deleted a credential");
        Ok(())
    }

    fn entry(&self, reference: &CredentialRef) -> Result<Entry, CredentialError> {
        let modifiers = platform::modifiers(&self.service, reference);
        let modifiers: HashMap<&str, &str> = modifiers
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect();
        self.platform
            .build(&self.service, reference.as_str(), Some(&modifiers))
            .map_err(|error| CredentialError::from_keyring(error, reference))
    }
}

#[cfg(windows)]
mod platform {
    use std::sync::Arc;

    use keyring_core::CredentialStore as PlatformStore;

    use crate::{CredentialError, CredentialRef};

    pub(crate) fn open() -> Result<Arc<PlatformStore>, CredentialError> {
        let store = windows_native_keyring_store::Store::new().map_err(|error| {
            CredentialError::Failed {
                source: Some(Box::new(error)),
            }
        })?;
        Ok(store)
    }

    /// The entry's name in Credential Manager is `<service>/<reference>`, and
    /// its persistence is local: the credential never roams to other machines
    /// of a domain, so it stays on the master (ADR-0010).
    pub(crate) fn modifiers(
        service: &str,
        reference: &CredentialRef,
    ) -> Vec<(&'static str, String)> {
        vec![
            ("target", format!("{service}/{reference}")),
            ("persistence", "Local".to_owned()),
        ]
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::sync::Arc;

    use keyring_core::CredentialStore as PlatformStore;

    use crate::{CredentialError, CredentialRef, UnavailableReason};

    /// Connects to the Secret Service on the session bus. Without one, no
    /// store is available.
    pub(crate) fn open() -> Result<Arc<PlatformStore>, CredentialError> {
        match zbus_secret_service_keyring_store::Store::new() {
            Ok(store) => Ok(store),
            Err(
                keyring_core::Error::PlatformFailure(source)
                | keyring_core::Error::NoStorageAccess(source),
            ) => Err(CredentialError::Unavailable {
                reason: UnavailableReason::NoSecretService,
                source: Some(source),
            }),
            Err(_) => Err(CredentialError::Unavailable {
                reason: UnavailableReason::NoSecretService,
                source: None,
            }),
        }
    }

    /// Items go to the default collection, the login keyring, with a label
    /// that the keyring's own UI shows.
    pub(crate) fn modifiers(
        _service: &str,
        reference: &CredentialRef,
    ) -> Vec<(&'static str, String)> {
        vec![("label", format!("Mannie: {reference}"))]
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use std::sync::Arc;

    use keyring_core::CredentialStore as PlatformStore;

    use crate::{CredentialError, CredentialRef, UnavailableReason};

    pub(crate) fn open() -> Result<Arc<PlatformStore>, CredentialError> {
        Err(CredentialError::Unavailable {
            reason: UnavailableReason::UnsupportedPlatform,
            source: None,
        })
    }

    pub(crate) fn modifiers(
        _service: &str,
        _reference: &CredentialRef,
    ) -> Vec<(&'static str, String)> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{CredentialError, CredentialRef};

    #[test]
    fn references_accept_names_and_paths_and_reject_everything_else() {
        let long = "a".repeat(101);
        for valid in [
            "provider/0192f3a4-5b6c-7d8e-9f00-112233445566",
            "a",
            "worker.key_v1",
        ] {
            assert_eq!(CredentialRef::parse(valid).unwrap().as_str(), valid);
        }
        for invalid in [
            "",
            "with space",
            "semi;colon",
            "ümlaut",
            "back\\slash",
            long.as_str(),
        ] {
            let error = CredentialRef::parse(invalid).unwrap_err();
            assert_eq!(error.code(), "credentials.invalid_reference", "{invalid:?}");
        }
    }

    #[test]
    fn errors_that_carry_secret_bytes_drop_them() {
        let reference = CredentialRef::parse("provider/test").unwrap();
        let secret_bytes = b"planted-secret-bytes".to_vec();

        let encoding = CredentialError::from_keyring(
            keyring_core::Error::BadEncoding(secret_bytes.clone()),
            &reference,
        );
        let format = CredentialError::from_keyring(
            keyring_core::Error::BadDataFormat(secret_bytes, "malformed".into()),
            &reference,
        );

        for error in [encoding, format] {
            let shown = format!("{error} {error:?}");
            assert!(!shown.contains("planted"), "{shown}");
            assert!(!shown.contains("112, 108, 97, 110"), "{shown}");
            assert_eq!(error.code(), "credentials.unreadable");
        }
    }

    #[test]
    fn a_missing_entry_is_not_found() {
        let reference = CredentialRef::parse("provider/test").unwrap();

        let error = CredentialError::from_keyring(keyring_core::Error::NoEntry, &reference);

        assert!(matches!(error, CredentialError::NotFound { .. }));
        assert_eq!(
            error.to_string(),
            "no credential is stored under provider/test"
        );
    }
}
