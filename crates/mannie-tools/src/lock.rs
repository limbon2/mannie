//! The machine-wide lock of file changes (docs/features/system-tools/
//! tools-and-results.md, file safety). File changes run one at a time on the
//! machine, whatever conversation makes them. The desktop app creates the lock
//! once and hands it to every tool call, so there is no global state.

use std::sync::Arc;

use tokio::sync::{Mutex, OwnedMutexGuard};

/// The lock that every file change takes.
#[derive(Debug, Clone, Default)]
pub struct MutationLock(Arc<Mutex<()>>);

impl MutationLock {
    /// A new lock. The composition root creates one for the machine.
    pub fn new() -> Self {
        Self::default()
    }

    /// Waits for the lock. A change holds it from its last check until its
    /// result is known.
    pub(crate) async fn acquire(&self) -> OwnedMutexGuard<()> {
        Arc::clone(&self.0).lock_owned().await
    }
}
