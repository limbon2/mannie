//! The open sessions with hosts (plugins/ssh/docs/README.md, connections):
//! one for each host, which later calls reuse, with a keepalive while it is
//! open. A session closes after five minutes without a call, when its host
//! changes and when the plugin stops.
//!
//! A call holds a lease on the session while it runs, so a session that a
//! long command uses is never closed for being idle.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::client::{Session, close};

/// How long a session stays open without a call.
pub(crate) const IDLE_TIMEOUT: Duration = Duration::from_mins(5);

/// What a session was opened for. A session serves a host only while the
/// host's endpoint and pinned key are still these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionKey {
    /// `user@address:port`.
    pub(crate) endpoint: String,
    /// The fingerprint of the pinned key.
    pub(crate) fingerprint: String,
}

/// A session in the pool.
struct Pooled {
    key: SessionKey,
    session: Arc<Session>,
    used_at: Instant,
}

/// The open sessions, by host ID. Cloning is cheap; clones share the pool.
#[derive(Clone, Default)]
pub(crate) struct Sessions {
    pool: Arc<Mutex<HashMap<String, Pooled>>>,
}

impl fmt::Debug for Sessions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let open = self.lock().len();
        formatter
            .debug_struct("Sessions")
            .field("open", &open)
            .finish()
    }
}

/// A call's hold on a session. Dropping it marks the session as used now.
pub(crate) struct Lease {
    session: Arc<Session>,
    sessions: Sessions,
    host_id: String,
    /// Whether the session was open before this call, so a failure to open a
    /// channel may mean that the host closed it meanwhile.
    reused: bool,
}

impl fmt::Debug for Lease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Lease")
            .field("host_id", &self.host_id)
            .field("reused", &self.reused)
            .finish_non_exhaustive()
    }
}

impl Lease {
    /// The session.
    pub(crate) fn session(&self) -> &Session {
        &self.session
    }

    /// Whether the session was open before this call.
    pub(crate) fn reused(&self) -> bool {
        self.reused
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut pool = self.sessions.lock();
        if let Some(pooled) = pool.get_mut(&self.host_id)
            && Arc::ptr_eq(&pooled.session, &self.session)
        {
            pooled.used_at = Instant::now();
        }
    }
}

impl Sessions {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Pooled>> {
        // Every critical section only reads or replaces entries.
        self.pool.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A lease on the open session of `host_id`, when it still serves `key`
    /// and the host has not closed it. A session that no longer serves the
    /// host is closed.
    pub(crate) fn lease(&self, host_id: &str, key: &SessionKey) -> Option<Lease> {
        let mut pool = self.lock();
        let is_usable = pool
            .get(host_id)
            .is_some_and(|pooled| pooled.key == *key && !pooled.session.is_closed());
        if !is_usable {
            if let Some(stale) = pool.remove(host_id) {
                close_later(stale.session);
            }
            return None;
        }
        let pooled = pool.get_mut(host_id)?;
        pooled.used_at = Instant::now();
        Some(Lease {
            session: Arc::clone(&pooled.session),
            sessions: self.clone(),
            host_id: host_id.to_owned(),
            reused: true,
        })
    }

    /// Keeps a new session for `host_id` and leases it, closing the one that
    /// it replaces.
    pub(crate) fn keep(&self, host_id: &str, key: SessionKey, session: Session) -> Lease {
        let session = Arc::new(session);
        let replaced = self.lock().insert(
            host_id.to_owned(),
            Pooled {
                key,
                session: Arc::clone(&session),
                used_at: Instant::now(),
            },
        );
        if let Some(replaced) = replaced {
            close_later(replaced.session);
        }
        Lease {
            session,
            sessions: self.clone(),
            host_id: host_id.to_owned(),
            reused: false,
        }
    }

    /// Closes the session of `host_id`, such as after its host changed. A
    /// call that holds a lease keeps its channel until it ends.
    pub(crate) fn forget(&self, host_id: &str) {
        let removed = self.lock().remove(host_id);
        if let Some(pooled) = removed {
            close_later(pooled.session);
        }
    }

    /// Closes the sessions that no call holds and that were last used before
    /// `now` minus [`IDLE_TIMEOUT`].
    pub(crate) fn close_idle(&self, now: Instant) {
        let mut pool = self.lock();
        let idle: Vec<String> = pool
            .iter()
            .filter(|(_host, pooled)| {
                let is_leased = Arc::strong_count(&pooled.session) > 1;
                let is_idle = now.saturating_duration_since(pooled.used_at) >= IDLE_TIMEOUT;
                is_idle && !is_leased
            })
            .map(|(host, _pooled)| host.clone())
            .collect();
        for host in idle {
            if let Some(pooled) = pool.remove(&host) {
                tracing::debug!(host = %host, "an idle SSH session closes");
                close_later(pooled.session);
            }
        }
    }

    /// Closes every session, as the plugin stops.
    pub(crate) fn close_all(&self) {
        let closing: Vec<Pooled> = self.lock().drain().map(|(_host, pooled)| pooled).collect();
        for pooled in closing {
            close_later(pooled.session);
        }
    }
}

/// Ends a session without waiting for the host.
fn close_later(session: Arc<Session>) {
    tokio::spawn(async move { close(&session).await });
}
