//! The master's key of the link in the operating system's credential store,
//! through the plugin's own entries (ADR-0016; ADR-0030, decision 13). The
//! key is kept as the base64 of its PKCS#8 document, under the entry that
//! 0.2.0 used, so paired workers keep trusting it.

use std::sync::Arc;

use base64::Engine as _;
use mannie_link::keys::LinkKey;
use mannie_plugins::{PluginSecrets, SecretsError};
use mannie_types::secret::Secret;

use crate::hub::{KeyStoreError, MasterKeyStore};

/// The plugin's entry of the master's key.
pub const MASTER_KEY_ENTRY: &str = "link/master-key";

/// The master's key in the plugin's entries of the credential store. The hub
/// reads it on first use and makes it before the first pairing.
#[derive(Debug)]
pub struct SecretsKeyStore {
    secrets: Arc<dyn PluginSecrets>,
}

impl SecretsKeyStore {
    /// A key store in `secrets`.
    pub fn new(secrets: Arc<dyn PluginSecrets>) -> Self {
        Self { secrets }
    }
}

fn unavailable(error: SecretsError) -> KeyStoreError {
    tracing::warn!(%error, "the credential store did not keep the master's key");
    KeyStoreError::Unavailable
}

impl MasterKeyStore for SecretsKeyStore {
    fn load(&self) -> Result<Option<LinkKey>, KeyStoreError> {
        let Some(text) = self.secrets.load(MASTER_KEY_ENTRY).map_err(unavailable)? else {
            return Ok(None);
        };
        let pkcs8 = base64::engine::general_purpose::STANDARD
            .decode(text.expose())
            .map_err(|_invalid| {
                tracing::error!("the master's key in the credential store is not base64");
                KeyStoreError::Invalid
            })?;
        LinkKey::from_pkcs8(pkcs8).map(Some).map_err(|_invalid| {
            tracing::error!("the master's key in the credential store is not a valid key");
            KeyStoreError::Invalid
        })
    }

    fn save(&self, key: &LinkKey) -> Result<(), KeyStoreError> {
        // The text moves into the secret, which clears it when it drops.
        let text = Secret::new(base64::engine::general_purpose::STANDARD.encode(key.pkcs8()));
        self.secrets
            .save(MASTER_KEY_ENTRY, &text)
            .map_err(unavailable)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex, PoisonError};

    use mannie_link::keys::LinkKey;
    use mannie_plugins::{PluginSecrets, SecretsError};
    use mannie_types::secret::Secret;

    use super::{MASTER_KEY_ENTRY, SecretsKeyStore};
    use crate::hub::{KeyStoreError, MasterKeyStore};

    /// Secrets in memory, limited to the master's key as the desktop app
    /// limits the plugin's entries.
    #[derive(Debug, Default)]
    struct Entries(Mutex<BTreeMap<String, String>>);

    impl PluginSecrets for Entries {
        fn load(&self, entry: &str) -> Result<Option<Secret>, SecretsError> {
            if entry != MASTER_KEY_ENTRY {
                return Err(SecretsError::NotPermitted);
            }
            let entries = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            Ok(entries.get(entry).cloned().map(Secret::new))
        }

        fn save(&self, entry: &str, secret: &Secret) -> Result<(), SecretsError> {
            if entry != MASTER_KEY_ENTRY {
                return Err(SecretsError::NotPermitted);
            }
            let mut entries = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            entries.insert(entry.to_owned(), secret.expose().to_owned());
            Ok(())
        }
    }

    #[test]
    fn the_master_key_is_kept_and_read_back_under_its_entry() {
        let entries = Arc::new(Entries::default());
        let store = SecretsKeyStore::new(Arc::<Entries>::clone(&entries));
        let key = LinkKey::generate().unwrap();

        let before = store.load().unwrap();
        store.save(&key).unwrap();
        let after = store.load().unwrap().unwrap();

        assert!(before.is_none());
        assert!(after.pin() == key.pin(), "the same key comes back");
    }

    #[test]
    fn a_stored_key_that_is_not_a_key_is_refused() {
        let entries = Arc::new(Entries::default());
        entries
            .save(MASTER_KEY_ENTRY, &Secret::new("not base64!".to_owned()))
            .unwrap();
        let store = SecretsKeyStore::new(entries);

        assert_eq!(store.load().unwrap_err(), KeyStoreError::Invalid);
    }
}
