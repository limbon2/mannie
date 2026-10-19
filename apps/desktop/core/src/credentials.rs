//! The master's credential vault (ADR-0010): the operating system's credential
//! store through `mannie-secrets`. Every credential that passes through it is
//! registered with the log redactor before anything else happens to it.
//! Plugins reach it only through the entries that their manifests list
//! (ADR-0030, decision 13; ADR-0034, decision 10), such as the Workers
//! plugin's key of the link (ADR-0016).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use mannie_logging::{ErrorChain, Redactor};
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::host::{BindRefusal, PluginBinding, ServiceProvider};
use mannie_plugin_sdk::service::{BoundService, Service};
use mannie_plugin_sdk::services::secrets::{Secrets, SecretsApi, SecretsError};
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

/// The names that plugins had in the credential store before each plugin
/// had entries of its own, by plugin (ADR-0034, decision 10). The list is
/// frozen: a manifest may list a legacy name only as this list gives it to
/// the plugin.
const LEGACY_SECRET_ENTRIES: &[(&str, &str)] = &[("workers", "link/master-key")];

/// The desktop app's provider of the SDK's service `secrets` (ADR-0034,
/// decision 5), which binds each plugin to the entries that its manifest
/// lists.
#[derive(Debug)]
pub(crate) struct SecretsProvider {
    vault: Arc<KeyringVault>,
}

impl SecretsProvider {
    /// The provider over `vault`.
    pub(crate) fn new(vault: Arc<KeyringVault>) -> Self {
        Self { vault }
    }
}

impl ServiceProvider for SecretsProvider {
    fn name(&self) -> &'static str {
        Secrets::NAME
    }

    fn major(&self) -> u16 {
        Secrets::MAJOR
    }

    fn bind(&self, binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        let secrets: Arc<dyn SecretsApi> =
            Arc::new(ScopedSecrets::for_plugin(Arc::clone(&self.vault), binding)?);
        Ok(BoundService::new::<Secrets>(secrets))
    }
}

/// The credential store limited to the entries of one plugin (ADR-0030,
/// decision 13), as the SDK's service `secrets`. Every other entry is
/// refused before the store is touched. The store's calls may block, so they
/// run on a blocking thread and the plugin awaits them.
#[derive(Debug)]
pub(crate) struct ScopedSecrets {
    vault: Arc<KeyringVault>,
    /// The name under which the store keeps each entry that the plugin's
    /// manifest lists, by the name that the plugin passes.
    names: BTreeMap<String, String>,
}

impl ScopedSecrets {
    /// The entries of the plugin that `binding` names: an entry of
    /// `entries` is kept as `<id>/<entry>`, and a legacy name as itself when
    /// the frozen list gives it to the plugin. A legacy name that the list
    /// does not give the plugin, or a name that the credential store cannot
    /// keep, refuses the binding.
    pub(crate) fn for_plugin(
        vault: Arc<KeyringVault>,
        binding: &PluginBinding<'_>,
    ) -> Result<Self, BindRefusal> {
        let plugin = binding.id.as_str();
        let listed = &binding.manifest.secrets;
        let mut names = BTreeMap::new();
        for entry in &listed.entries {
            names.insert(entry.clone(), format!("{plugin}/{entry}"));
        }
        for legacy in &listed.legacy {
            let frozen = LEGACY_SECRET_ENTRIES
                .iter()
                .any(|(owner, name)| *owner == plugin && name == legacy);
            if !frozen {
                return Err(BindRefusal::new(
                    Secrets::NAME,
                    format!("the legacy name {legacy} is not one of the plugin {plugin}"),
                ));
            }
            names.insert(legacy.clone(), legacy.clone());
        }
        if let Some(stored) = names
            .values()
            .find(|stored| CredentialRef::parse(stored).is_err())
        {
            return Err(BindRefusal::new(
                Secrets::NAME,
                format!("the credential store cannot keep the name {stored}"),
            ));
        }
        Ok(Self { vault, names })
    }

    /// The name under which the store keeps `entry`, or a refusal for a name
    /// that the plugin's manifest does not list.
    fn stored_name(&self, entry: &str) -> Result<String, SecretsError> {
        self.names.get(entry).cloned().ok_or_else(|| {
            tracing::error!(
                entry,
                "a plugin asked for an entry of the credential store that its manifest does not list"
            );
            SecretsError::NotPermitted
        })
    }
}

impl SecretsApi for ScopedSecrets {
    fn load(&self, entry: &str) -> BoxFuture<'static, Result<Option<Secret>, SecretsError>> {
        let stored = self.stored_name(entry);
        let vault = Arc::clone(&self.vault);
        Box::pin(async move {
            let entry = stored?;
            on_blocking_thread(move || match vault.read(&entry) {
                Ok(secret) => Ok(Some(secret)),
                Err(VaultError::Missing) => Ok(None),
                Err(VaultError::Unavailable(_problem)) => Err(SecretsError::Unavailable),
            })
            .await
        })
    }

    fn save(&self, entry: &str, secret: Secret) -> BoxFuture<'static, Result<(), SecretsError>> {
        let stored = self.stored_name(entry);
        let vault = Arc::clone(&self.vault);
        Box::pin(async move {
            let entry = stored?;
            on_blocking_thread(move || {
                vault
                    .save(&entry, &secret)
                    .map_err(|_unavailable| SecretsError::Unavailable)
            })
            .await
        })
    }
}

/// Runs a call of the credential store on a blocking thread. A call that
/// did not finish, such as one that the runtime's shutdown canceled, leaves
/// the store unavailable to the plugin.
async fn on_blocking_thread<T: Send + 'static>(
    call: impl FnOnce() -> Result<T, SecretsError> + Send + 'static,
) -> Result<T, SecretsError> {
    tokio::task::spawn_blocking(call)
        .await
        .unwrap_or_else(|_unfinished| {
            tracing::error!("a call of the credential store did not finish");
            Err(SecretsError::Unavailable)
        })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mannie_logging::Redactor;
    use mannie_plugin_sdk::context::HostProfile;
    use mannie_plugin_sdk::host::PluginBinding;
    use mannie_plugin_sdk::manifest::Manifest;
    use mannie_plugin_sdk::services::secrets::{SecretsApi, SecretsError};
    use mannie_runtime::VaultError;
    use mannie_secrets::{CredentialError, CredentialRef, UnavailableReason};
    use mannie_types::provider::CredentialStoreProblem;
    use mannie_types::secret::Secret;

    use super::{KeyringVault, ScopedSecrets, vault_error};

    /// A vault whose store has not opened, which none of these tests opens.
    fn vault() -> Arc<KeyringVault> {
        Arc::new(KeyringVault::new(
            "io.github.limbon2.mannie.tests",
            Redactor::new(),
        ))
    }

    /// The manifest of `id` with `secrets`, the body of its `[secrets]`.
    fn manifest(id: &str, secrets: &str) -> Manifest {
        Manifest::parse(&format!(
            "id = \"{id}\"\nsdk = 1\norder = 10\n\n[rust]\nentry = \"mannie-{id}\"\n\n[services]\nsecrets = 1\n\n[secrets]\n{secrets}\n"
        ))
        .unwrap()
    }

    fn scoped(vault: &Arc<KeyringVault>, manifest: &Manifest) -> Result<ScopedSecrets, String> {
        let binding = PluginBinding::new(&manifest.id, manifest, HostProfile::Release);
        ScopedSecrets::for_plugin(Arc::clone(vault), &binding).map_err(|refusal| refusal.detail)
    }

    #[tokio::test]
    async fn workers_reaches_its_key_by_its_legacy_name_and_nothing_else_reaches_the_store() {
        // The store opens on first use, so a refused entry that left it
        // unopened never reached the operating system's credential store.
        let vault = vault();
        let workers = manifest("workers", "legacy = [\"link/master-key\"]");
        let secrets = scoped(&vault, &workers).unwrap();
        let planted = Secret::new("planted-secret".to_owned());

        let key = secrets.stored_name("link/master-key");
        let read = secrets.load("provider/profile/1").await;
        let written = secrets.save("link/other-key", planted).await;
        let undeclared = secrets.load("token").await;

        assert_eq!(key.as_deref(), Ok("link/master-key"));
        assert_eq!(read.unwrap_err(), SecretsError::NotPermitted);
        assert_eq!(written.unwrap_err(), SecretsError::NotPermitted);
        assert_eq!(undeclared.unwrap_err(), SecretsError::NotPermitted);
        assert!(vault.store.lock().unwrap().is_none());
    }

    #[test]
    fn an_entry_is_kept_under_the_plugins_id() {
        let vault = vault();
        let notes = manifest("notes", "entries = [\"token\", \"sync/key\"]");
        let secrets = scoped(&vault, &notes).unwrap();

        assert_eq!(secrets.stored_name("token").as_deref(), Ok("notes/token"));
        assert_eq!(
            secrets.stored_name("sync/key").as_deref(),
            Ok("notes/sync/key")
        );
        assert_eq!(
            secrets.stored_name("notes/token"),
            Err(SecretsError::NotPermitted)
        );
        assert!(vault.store.lock().unwrap().is_none());
    }

    #[test]
    fn a_legacy_name_that_the_frozen_list_does_not_give_the_plugin_or_a_name_too_long_is_refused() {
        let vault = vault();
        let images = manifest("images", "legacy = [\"link/master-key\"]");
        let other_key = manifest("workers", "legacy = [\"link/other-key\"]");
        let long = manifest("notes", &format!("entries = [\"{}\"]", "a".repeat(95)));

        let refusals = [
            scoped(&vault, &images).unwrap_err(),
            scoped(&vault, &other_key).unwrap_err(),
            scoped(&vault, &long).unwrap_err(),
        ];

        assert!(refusals[0].contains("link/master-key"), "{}", refusals[0]);
        assert!(refusals[1].contains("link/other-key"), "{}", refusals[1]);
        assert!(refusals[2].contains("cannot keep"), "{}", refusals[2]);
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
