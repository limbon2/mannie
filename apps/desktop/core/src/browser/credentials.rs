//! The Browser's exact-origin credential catalog. Saved values belong to the
//! desktop's credential vault. Only authorized protected fills read them.

use std::sync::Arc;

use mannie_plugin_sdk::contracts::{ConversationId, Secret};
use mannie_plugin_sdk::services::browser::{BrowserCredentialRef, BrowserError};
use mannie_runtime::{CredentialVault, Protection, VaultError};
use mannie_store::{Store, StoreError, StoredSiteCredential};
use tokio::sync::{RwLock, RwLockReadGuard};
use uuid::Uuid;

use super::actions::credential_origin;

#[derive(Debug)]
pub(super) struct SiteCredentials {
    store: Store,
    vault: Arc<dyn CredentialVault>,
    // A removal waits for fills that already read the vault. An add and
    // recovery cannot claim or purge the same pending record.
    changes: RwLock<()>,
}

pub(super) struct ProtectedFill<'a> {
    pub(super) value: Secret,
    _protection: Option<Protection>,
    _catalog: RwLockReadGuard<'a, ()>,
}

impl SiteCredentials {
    pub(super) fn new(store: Store, vault: Arc<dyn CredentialVault>) -> Self {
        Self {
            store,
            vault,
            changes: RwLock::new(()),
        }
    }

    pub(super) async fn list(&self) -> Result<Vec<BrowserCredentialRef>, BrowserError> {
        let _change = self.changes.write().await;
        self.recover().await?;
        self.for_origin(None).await
    }

    pub(super) async fn for_origin(
        &self,
        origin: Option<String>,
    ) -> Result<Vec<BrowserCredentialRef>, BrowserError> {
        self.store
            .browser_site_credentials(origin)
            .await
            .map(|rows| rows.into_iter().map(credential_ref).collect())
            .map_err(|error| catalog_failed(&error))
    }

    pub(super) async fn find(
        &self,
        origin: &str,
        reference: &str,
    ) -> Result<BrowserCredentialRef, BrowserError> {
        if !valid_reference(reference) {
            return Err(BrowserError::CredentialMissing);
        }
        self.store
            .browser_site_credential(origin.to_owned(), reference.to_owned())
            .await
            .map_err(|error| catalog_failed(&error))?
            .map(credential_ref)
            .ok_or(BrowserError::CredentialMissing)
    }

    pub(super) async fn add(
        &self,
        origin: String,
        label: String,
        value: Secret,
    ) -> Result<BrowserCredentialRef, BrowserError> {
        let origin = credential_origin(origin.trim())?;
        let label = label.trim();
        let valid = origin.len() <= 512
            && (1..=80).contains(&label.len())
            && !label.chars().any(char::is_control)
            && (1..=8192).contains(&value.expose().len())
            && !label.contains(value.expose());
        if !valid {
            return Err(BrowserError::InvalidBrowserInput);
        }
        let _change = self.changes.write().await;
        self.recover().await?;
        let credential = StoredSiteCredential {
            reference: Uuid::now_v7().to_string(),
            origin,
            label: label.to_owned(),
        };
        if !self
            .store
            .stage_browser_site_credential(credential.clone())
            .await
            .map_err(|error| catalog_failed(&error))?
        {
            return Err(BrowserError::CredentialAlreadyExists);
        }
        let entry = vault_entry(&credential.reference);
        let saved = self
            .in_vault(move |vault| {
                let _protection = vault.protect(&value);
                vault.save_part(&entry, &value)
            })
            .await;
        if saved.is_err() {
            // Leave a durable record for the next attempt if vault cleanup fails.
            self.purge(&credential.reference).await?;
            return Err(BrowserError::CredentialStoreUnavailable);
        }
        if !self
            .store
            .activate_browser_site_credential(&credential.reference)
            .await
            .map_err(|error| catalog_failed(&error))?
        {
            return Err(BrowserError::CredentialCatalogFailed);
        }
        Ok(credential_ref(credential))
    }

    pub(super) async fn remove(&self, reference: String) -> Result<(), BrowserError> {
        if !valid_reference(&reference) {
            return Err(BrowserError::InvalidBrowserInput);
        }
        let _change = self.changes.write().await;
        if self
            .store
            .revoke_browser_site_credential(&reference)
            .await
            .map_err(|error| catalog_failed(&error))?
        {
            self.purge(&reference).await?;
        }
        Ok(())
    }

    pub(super) async fn fill(
        &self,
        conversation: &ConversationId,
        origin: &str,
        reference: &str,
    ) -> Result<ProtectedFill<'_>, BrowserError> {
        let catalog = self.changes.read().await;
        if !self
            .store
            .has_browser_site_credential(conversation, origin, reference)
            .await
            .map_err(|error| catalog_failed(&error))?
        {
            return Err(BrowserError::NavigationDenied);
        }
        let entry = vault_entry(reference);
        let (value, protection) = self
            .in_vault(move |vault| {
                let value = vault.read_part(&entry)?;
                let protection = vault.protect(&value);
                Ok((value, protection))
            })
            .await
            .map_err(|error| match error {
                VaultError::Missing => BrowserError::CredentialMissing,
                VaultError::Unavailable(_) => BrowserError::CredentialStoreUnavailable,
            })?;
        Ok(ProtectedFill {
            value,
            _protection: protection,
            _catalog: catalog,
        })
    }

    async fn recover(&self) -> Result<(), BrowserError> {
        let pending = self
            .store
            .browser_site_credentials_to_purge()
            .await
            .map_err(|error| catalog_failed(&error))?;
        for reference in pending {
            self.purge(&reference).await?;
        }
        Ok(())
    }

    async fn purge(&self, reference: &str) -> Result<(), BrowserError> {
        let entry = vault_entry(reference);
        self.in_vault(move |vault| match vault.delete(&entry) {
            Ok(()) | Err(VaultError::Missing) => Ok(()),
            Err(error) => Err(error),
        })
        .await
        .map_err(|_vault| BrowserError::CredentialStoreUnavailable)?;
        self.store
            .purge_browser_site_credential(reference)
            .await
            .map_err(|error| catalog_failed(&error))
    }

    async fn in_vault<T: Send + 'static>(
        &self,
        action: impl FnOnce(&dyn CredentialVault) -> Result<T, VaultError> + Send + 'static,
    ) -> Result<T, VaultError> {
        let vault = Arc::clone(&self.vault);
        tokio::task::spawn_blocking(move || action(vault.as_ref()))
            .await
            .unwrap_or(Err(VaultError::Unavailable(
                mannie_types::provider::CredentialStoreProblem::Other,
            )))
    }
}

fn credential_ref(credential: StoredSiteCredential) -> BrowserCredentialRef {
    BrowserCredentialRef {
        reference: credential.reference,
        origin: credential.origin,
        label: credential.label,
    }
}

fn valid_reference(reference: &str) -> bool {
    Uuid::parse_str(reference).is_ok_and(|id| id.to_string() == reference)
}

fn vault_entry(reference: &str) -> String {
    format!("browser/credential/{reference}")
}

fn catalog_failed(error: &StoreError) -> BrowserError {
    tracing::warn!(%error, "could not use the Browser credential catalog");
    BrowserError::CredentialCatalogFailed
}

#[cfg(test)]
mod tests {
    use mannie_runtime::testing::MemoryVault;
    use mannie_store::StoreOptions;
    use mannie_types::{clock::SystemClock, provider::CredentialStoreProblem};

    use super::*;

    #[tokio::test]
    async fn a_failed_vault_delete_revokes_the_site_credential_until_cleanup_succeeds() {
        let directory = tempfile::tempdir().unwrap();
        let options = StoreOptions::new(directory.path().join("backups"), Arc::new(SystemClock));
        let store = Store::open(&directory.path().join("test.sqlite"), &options).unwrap();
        let vault = Arc::new(MemoryVault::new());
        let catalog = SiteCredentials::new(store.clone(), Arc::<MemoryVault>::clone(&vault));
        let secret = "owner-credential-for-this-fixture";
        let credential = catalog
            .add(
                " https://EXAMPLE.test:443/ ".to_owned(),
                "Fixture login".to_owned(),
                Secret::new(secret.to_owned()),
            )
            .await
            .unwrap();
        assert_eq!(credential.origin, "https://example.test");
        let entry = vault_entry(&credential.reference);
        assert_eq!(vault.credential(&entry).as_deref(), Some(secret));

        vault.fail_deletes(Some(CredentialStoreProblem::Other));
        assert_eq!(
            catalog.remove(credential.reference.clone()).await,
            Err(BrowserError::CredentialStoreUnavailable)
        );
        assert_eq!(store.browser_site_credentials(None).await.unwrap(), []);
        assert_eq!(vault.credential(&entry).as_deref(), Some(secret));

        vault.fail_deletes(None);
        assert_eq!(catalog.list().await.unwrap(), []);
        assert_eq!(vault.credential(&entry), None);
    }

    #[tokio::test]
    async fn recovery_purges_a_value_from_an_interrupted_add() {
        let directory = tempfile::tempdir().unwrap();
        let options = StoreOptions::new(directory.path().join("backups"), Arc::new(SystemClock));
        let store = Store::open(&directory.path().join("test.sqlite"), &options).unwrap();
        let vault = Arc::new(MemoryVault::new());
        let pending = StoredSiteCredential {
            reference: Uuid::now_v7().to_string(),
            origin: "https://example.test".to_owned(),
            label: "Interrupted add".to_owned(),
        };
        let entry = vault_entry(&pending.reference);
        assert!(store.stage_browser_site_credential(pending).await.unwrap());
        vault.insert(&entry, "interrupted-credential-for-this-fixture");

        let catalog = SiteCredentials::new(store, Arc::<MemoryVault>::clone(&vault));
        assert_eq!(catalog.list().await.unwrap(), []);
        assert_eq!(vault.credential(&entry), None);
    }
}
