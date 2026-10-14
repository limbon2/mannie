//! The master's credential vault (ADR-0010): the operating system's credential
//! store through `mannie-secrets`. Every credential that passes through it is
//! registered with the log redactor before anything else happens to it.

use std::sync::{Mutex, PoisonError};

use mannie_logging::{ErrorChain, Redactor};
use mannie_runtime::{CredentialVault, VaultError};
use mannie_secrets::{CredentialError, CredentialRef, CredentialStore, UnavailableReason};
use mannie_types::provider::CredentialStoreProblem;
use mannie_types::secret::Secret;

/// The credential store, opened on first use, so that starting Mannie never
/// waits for it (NFR-PERF-03). A failed open is tried again on the next use,
/// so a Secret Service that starts after Mannie is still found.
#[derive(Debug)]
pub(crate) struct KeyringVault {
    service: String,
    store: Mutex<Option<CredentialStore>>,
    redactor: Redactor,
}

impl KeyringVault {
    /// A vault for the entries of `service`, the app's bundle identifier.
    pub(crate) fn new(service: &str, redactor: Redactor) -> Self {
        Self {
            service: service.to_owned(),
            store: Mutex::new(None),
            redactor,
        }
    }

    fn store(&self) -> Result<CredentialStore, VaultError> {
        let mut store = self.store.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(store) = store.as_ref() {
            return Ok(store.clone());
        }
        let opened = CredentialStore::open(&self.service).map_err(|error| {
            tracing::warn!(error = %ErrorChain(&error), error.code = error.code(), "the credential store could not be opened");
            vault_error(&error)
        })?;
        *store = Some(opened.clone());
        Ok(opened)
    }
}

fn parse(reference: &str) -> Result<CredentialRef, VaultError> {
    CredentialRef::parse(reference).map_err(|error| {
        tracing::error!(
            error.code = error.code(),
            "the runtime named a credential with an invalid reference"
        );
        VaultError::Unavailable(CredentialStoreProblem::Other)
    })
}

/// What the runtime and the owner learn from a credential store error. A
/// credential that cannot be read is as good as missing: saving it again
/// replaces it.
fn vault_error(error: &CredentialError) -> VaultError {
    match error {
        CredentialError::Unavailable {
            reason: UnavailableReason::NoSecretService,
            ..
        } => VaultError::Unavailable(CredentialStoreProblem::NoSecretService),
        CredentialError::Unavailable {
            reason: UnavailableReason::UnsupportedPlatform,
            ..
        } => VaultError::Unavailable(CredentialStoreProblem::UnsupportedPlatform),
        CredentialError::AccessDenied { .. } => {
            VaultError::Unavailable(CredentialStoreProblem::AccessDenied)
        }
        CredentialError::NotFound { .. } | CredentialError::Unreadable { .. } => {
            VaultError::Missing
        }
        CredentialError::InvalidReference { .. }
        | CredentialError::Rejected { .. }
        | CredentialError::Failed { .. } => VaultError::Unavailable(CredentialStoreProblem::Other),
    }
}

fn logged(error: &CredentialError, action: &str) -> VaultError {
    tracing::warn!(error = %ErrorChain(error), error.code = error.code(), action, "the credential store failed");
    vault_error(error)
}

impl CredentialVault for KeyringVault {
    fn save(&self, reference: &str, credential: &Secret) -> Result<(), VaultError> {
        self.redactor.register(credential);
        let reference = parse(reference)?;
        self.store()?
            .save(&reference, credential)
            .map_err(|error| logged(&error, "save"))
    }

    fn read(&self, reference: &str) -> Result<Secret, VaultError> {
        let reference = parse(reference)?;
        let secret = self
            .store()?
            .read(&reference)
            .map_err(|error| logged(&error, "read"))?;
        self.redactor.register(&secret);
        Ok(secret)
    }

    fn delete(&self, reference: &str) -> Result<(), VaultError> {
        let reference = parse(reference)?;
        self.store()?
            .delete(&reference)
            .map_err(|error| logged(&error, "delete"))
    }
}

#[cfg(test)]
mod tests {
    use mannie_runtime::VaultError;
    use mannie_secrets::{CredentialError, CredentialRef, UnavailableReason};
    use mannie_types::provider::CredentialStoreProblem;

    use super::vault_error;

    #[test]
    fn store_errors_become_the_problems_the_owner_can_act_on() {
        let reference = CredentialRef::parse("provider/test/1").unwrap();
        let cases = [
            (
                CredentialError::Unavailable {
                    reason: UnavailableReason::NoSecretService,
                    source: None,
                },
                VaultError::Unavailable(CredentialStoreProblem::NoSecretService),
            ),
            (
                CredentialError::Unavailable {
                    reason: UnavailableReason::UnsupportedPlatform,
                    source: None,
                },
                VaultError::Unavailable(CredentialStoreProblem::UnsupportedPlatform),
            ),
            (
                CredentialError::NotFound {
                    reference: reference.clone(),
                },
                VaultError::Missing,
            ),
            (
                CredentialError::Unreadable {
                    reference: reference.clone(),
                },
                VaultError::Missing,
            ),
            (
                CredentialError::Rejected { reference },
                VaultError::Unavailable(CredentialStoreProblem::Other),
            ),
            (
                CredentialError::Failed { source: None },
                VaultError::Unavailable(CredentialStoreProblem::Other),
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(vault_error(&error), expected, "{error}");
        }
    }
}
