//! The master's credential vault (ADR-0010): the operating system's credential
//! store through `mannie-secrets`. Every credential that passes through it is
//! registered with the log redactor before anything else happens to it.
//! Plugins reach it only through their own entries (ADR-0030, decision 13),
//! such as the Workers plugin's key of the link (ADR-0016).

use std::sync::{Arc, Mutex, PoisonError};

use mannie_logging::{ErrorChain, Redactor};
use mannie_plugins::{PluginSecrets, SecretsError};
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

/// The credential store limited to the entries of one plugin (ADR-0030,
/// decision 13). Every other entry is refused before the store is touched.
#[derive(Debug)]
pub(crate) struct ScopedSecrets {
    vault: Arc<KeyringVault>,
    entries: &'static [&'static str],
}

impl ScopedSecrets {
    /// The plugin's `entries` in `vault`.
    pub(crate) fn new(vault: Arc<KeyringVault>, entries: &'static [&'static str]) -> Self {
        Self { vault, entries }
    }

    fn check(&self, entry: &str) -> Result<(), SecretsError> {
        if self.entries.contains(&entry) {
            Ok(())
        } else {
            tracing::error!(
                entry,
                "a plugin asked for an entry of the credential store that is not its own"
            );
            Err(SecretsError::NotPermitted)
        }
    }
}

impl PluginSecrets for ScopedSecrets {
    fn load(&self, entry: &str) -> Result<Option<Secret>, SecretsError> {
        self.check(entry)?;
        match self.vault.read(entry) {
            Ok(secret) => Ok(Some(secret)),
            Err(VaultError::Missing) => Ok(None),
            Err(VaultError::Unavailable(_problem)) => Err(SecretsError::Unavailable),
        }
    }

    fn save(&self, entry: &str, secret: &Secret) -> Result<(), SecretsError> {
        self.check(entry)?;
        self.vault
            .save(entry, secret)
            .map_err(|_unavailable| SecretsError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mannie_logging::Redactor;
    use mannie_plugins::{PluginSecrets, SecretsError};
    use mannie_runtime::VaultError;
    use mannie_secrets::{CredentialError, CredentialRef, UnavailableReason};
    use mannie_types::provider::CredentialStoreProblem;
    use mannie_types::secret::Secret;

    use super::{KeyringVault, ScopedSecrets, vault_error};

    #[test]
    fn a_plugin_reaches_only_its_own_entries_and_another_never_reaches_the_store() {
        // The store opens on first use, so a refused entry that left it
        // unopened never reached the operating system's credential store.
        let vault = Arc::new(KeyringVault::new(
            "io.github.limbon2.mannie.tests",
            Redactor::new(),
        ));
        let secrets = ScopedSecrets::new(Arc::clone(&vault), &["link/master-key"]);
        let planted = Secret::new("planted-secret".to_owned());

        let read = secrets.load("provider/profile/1");
        let written = secrets.save("link/other-key", &planted);

        assert_eq!(read.unwrap_err(), SecretsError::NotPermitted);
        assert_eq!(written.unwrap_err(), SecretsError::NotPermitted);
        assert!(vault.store.lock().unwrap().is_none());
    }

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
