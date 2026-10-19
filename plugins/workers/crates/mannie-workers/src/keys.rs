//! The master's key of the link in the operating system's credential store,
//! through the plugin's own entries (ADR-0016; ADR-0030, decision 13). The
//! key is kept as the base64 of its PKCS#8 document, under the entry that
//! 0.2.0 used, so paired workers keep trusting it.

use std::sync::Arc;

use base64::Engine as _;
use mannie_link::keys::LinkKey;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::Secret;
use mannie_plugin_sdk::services::secrets::{SecretsApi, SecretsError};

use crate::hub::{KeyStoreError, MasterKeyStore};

/// The plugin's entry of the master's key.
pub const MASTER_KEY_ENTRY: &str = "link/master-key";

/// The master's key in the plugin's entries of the credential store. The hub
/// reads it on first use and makes it before the first pairing.
#[derive(Debug)]
pub struct SecretsKeyStore {
    secrets: Arc<dyn SecretsApi>,
}

impl SecretsKeyStore {
    /// A key store in `secrets`, the plugin's service `secrets`.
    pub fn new(secrets: Arc<dyn SecretsApi>) -> Self {
        Self { secrets }
    }
}

fn unavailable(error: SecretsError) -> KeyStoreError {
    tracing::warn!(%error, "the credential store did not keep the master's key");
    KeyStoreError::Unavailable
}

/// The master's key from the base64 of its PKCS#8 document.
fn decode(text: &Secret) -> Result<LinkKey, KeyStoreError> {
    let pkcs8 = base64::engine::general_purpose::STANDARD
        .decode(text.expose())
        .map_err(|_invalid| {
            tracing::error!("the master's key in the credential store is not base64");
            KeyStoreError::Invalid
        })?;
    LinkKey::from_pkcs8(pkcs8).map_err(|_invalid| {
        tracing::error!("the master's key in the credential store is not a valid key");
        KeyStoreError::Invalid
    })
}

impl MasterKeyStore for SecretsKeyStore {
    fn load(&self) -> BoxFuture<'static, Result<Option<LinkKey>, KeyStoreError>> {
        let loading = self.secrets.load(MASTER_KEY_ENTRY);
        Box::pin(async move {
            match loading.await.map_err(unavailable)? {
                Some(text) => decode(&text).map(Some),
                None => Ok(None),
            }
        })
    }

    fn save(&self, key: &LinkKey) -> BoxFuture<'static, Result<(), KeyStoreError>> {
        // The text moves into the secret, which clears it when it drops.
        let text = Secret::new(base64::engine::general_purpose::STANDARD.encode(key.pkcs8()));
        let saving = self.secrets.save(MASTER_KEY_ENTRY, text);
        Box::pin(async move { saving.await.map_err(unavailable) })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex, PoisonError};

    use mannie_link::keys::LinkKey;
    use mannie_plugin_sdk::BoxFuture;
    use mannie_plugin_sdk::contracts::Secret;
    use mannie_plugin_sdk::services::secrets::{SecretsApi, SecretsError};

    use super::{MASTER_KEY_ENTRY, SecretsKeyStore};
    use crate::hub::{KeyStoreError, MasterKeyStore};

    /// Secrets in memory, limited to the master's key as the desktop app
    /// limits the plugin's entries, whose store can be made unavailable.
    #[derive(Debug, Default)]
    struct Entries {
        entries: Mutex<BTreeMap<String, String>>,
        unavailable: Mutex<bool>,
    }

    impl Entries {
        fn set_unavailable(&self) {
            *self
                .unavailable
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = true;
        }

        fn check(&self, entry: &str) -> Result<(), SecretsError> {
            if *self
                .unavailable
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
            {
                return Err(SecretsError::Unavailable);
            }
            if entry == MASTER_KEY_ENTRY {
                Ok(())
            } else {
                Err(SecretsError::NotPermitted)
            }
        }
    }

    impl SecretsApi for Entries {
        fn load(&self, entry: &str) -> BoxFuture<'static, Result<Option<Secret>, SecretsError>> {
            let loaded = self.check(entry).map(|()| {
                let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
                entries.get(entry).cloned().map(Secret::new)
            });
            Box::pin(std::future::ready(loaded))
        }

        fn save(
            &self,
            entry: &str,
            secret: Secret,
        ) -> BoxFuture<'static, Result<(), SecretsError>> {
            let saved = self.check(entry).map(|()| {
                let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
                entries.insert(entry.to_owned(), secret.expose().to_owned());
            });
            Box::pin(std::future::ready(saved))
        }
    }

    #[tokio::test]
    async fn the_master_key_is_kept_and_read_back_under_its_entry() {
        let entries = Arc::new(Entries::default());
        let store = SecretsKeyStore::new(Arc::<Entries>::clone(&entries));
        let key = LinkKey::generate().unwrap();

        let before = store.load().await.unwrap();
        store.save(&key).await.unwrap();
        let after = store.load().await.unwrap().unwrap();

        assert!(before.is_none());
        assert!(after.pin() == key.pin(), "the same key comes back");
    }

    #[tokio::test]
    async fn a_stored_key_that_is_not_a_key_is_refused() {
        let entries = Arc::new(Entries::default());
        entries
            .save(MASTER_KEY_ENTRY, Secret::new("not base64!".to_owned()))
            .await
            .unwrap();
        let store = SecretsKeyStore::new(entries);

        assert_eq!(store.load().await.unwrap_err(), KeyStoreError::Invalid);
    }

    #[tokio::test]
    async fn an_unavailable_credential_store_reports_the_key_store_unavailable() {
        let entries = Arc::new(Entries::default());
        entries.set_unavailable();
        let store = SecretsKeyStore::new(entries);
        let key = LinkKey::generate().unwrap();

        assert_eq!(store.load().await.unwrap_err(), KeyStoreError::Unavailable);
        assert_eq!(
            store.save(&key).await.unwrap_err(),
            KeyStoreError::Unavailable
        );
    }
}
