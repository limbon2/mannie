//! The worker's lease vault (docs/features/workers/credential-leases.md): the
//! credentials that the master leases to tasks, in memory only. The log
//! redactor hides each credential while its lease lives and forgets it when
//! the lease is dropped, and a run that waits for its lease learns when it
//! arrives.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_logging::{RedactionGuard, Redactor};
use mannie_runtime::{CredentialVault, VaultError};
use mannie_types::secret::Secret;
use tokio::sync::watch;

/// A lease while it lives: the credential and its hold on the redactor.
struct Lease {
    credential: Secret,
    _redaction: RedactionGuard,
}

/// The leases of a worker's tasks, in memory.
pub struct LeaseVault {
    redactor: Arc<Redactor>,
    leases: Mutex<HashMap<String, Lease>>,
    /// Counts the leases that arrived, for the runs that wait for one.
    arrived: watch::Sender<u64>,
}

impl fmt::Debug for LeaseVault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LeaseVault")
            .field("leases", &self.leases().len())
            .finish_non_exhaustive()
    }
}

impl LeaseVault {
    /// An empty vault, whose leases `redactor` hides from the logs.
    pub fn new(redactor: Arc<Redactor>) -> Self {
        Self {
            redactor,
            leases: Mutex::default(),
            arrived: watch::Sender::new(0),
        }
    }

    fn leases(&self) -> MutexGuard<'_, HashMap<String, Lease>> {
        // Every critical section only inserts or removes entries.
        self.leases.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// How many leases the vault holds.
    pub fn len(&self) -> usize {
        self.leases().len()
    }

    /// Whether the vault holds no lease.
    pub fn is_empty(&self) -> bool {
        self.leases().is_empty()
    }

    /// Drops every lease, because the worker is unpaired.
    pub fn clear(&self) {
        let dropped = std::mem::take(&mut *self.leases());
        if !dropped.is_empty() {
            tracing::info!(leases = dropped.len(), "dropped every lease");
        }
    }
}

impl CredentialVault for LeaseVault {
    fn save(&self, reference: &str, credential: &Secret) -> Result<(), VaultError> {
        let lease = Lease {
            credential: credential.clone(),
            _redaction: self.redactor.register_scoped(credential),
        };
        self.leases().insert(reference.to_owned(), lease);
        self.arrived
            .send_modify(|count| *count = count.wrapping_add(1));
        Ok(())
    }

    fn read(&self, reference: &str) -> Result<Secret, VaultError> {
        self.leases()
            .get(reference)
            .map(|lease| lease.credential.clone())
            .ok_or(VaultError::Missing)
    }

    fn delete(&self, reference: &str) -> Result<(), VaultError> {
        self.leases()
            .remove(reference)
            .map(drop)
            .ok_or(VaultError::Missing)
    }

    fn arrival(&self, _reference: &str) -> Option<BoxFuture<'static, ()>> {
        // The receiver counts the leases so far, so one that arrives after
        // this call ends the wait.
        let mut arrived = self.arrived.subscribe();
        Some(
            async move {
                let _changed = arrived.changed().await;
            }
            .boxed(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mannie_logging::Redactor;
    use mannie_runtime::{CredentialVault, VaultError};
    use mannie_types::secret::Secret;

    use super::LeaseVault;

    #[tokio::test]
    async fn a_lease_is_hidden_from_the_logs_while_it_lives() {
        let redactor = Arc::new(Redactor::new());
        let vault = LeaseVault::new(Arc::clone(&redactor));
        let waiting = vault.arrival("lease/a").unwrap();

        vault
            .save("lease/a", &Secret::new("sk-planted-lease".to_owned()))
            .unwrap();
        waiting.await;

        assert_eq!(
            redactor.redact("key sk-planted-lease"),
            format!("key {}", mannie_logging::REDACTED)
        );
        assert_eq!(vault.read("lease/a").unwrap().expose(), "sk-planted-lease");
        vault.delete("lease/a").unwrap();
        assert_eq!(redactor.registered_count(), 0);
        assert!(matches!(vault.read("lease/a"), Err(VaultError::Missing)));
        assert_eq!(vault.delete("lease/a"), Err(VaultError::Missing));
    }

    #[tokio::test]
    async fn unpairing_drops_every_lease() {
        let redactor = Arc::new(Redactor::new());
        let vault = LeaseVault::new(Arc::clone(&redactor));
        vault
            .save("lease/a", &Secret::new("one".to_owned()))
            .unwrap();
        vault
            .save("lease/b", &Secret::new("two".to_owned()))
            .unwrap();

        vault.clear();

        assert!(vault.is_empty());
        assert_eq!(redactor.registered_count(), 0);
    }
}
