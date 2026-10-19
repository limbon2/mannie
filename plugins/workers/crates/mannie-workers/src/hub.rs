//! The master's side of the link (ADR-0016, ADR-0027; plugins/workers/docs/
//! pairing-and-link.md): the hub that keeps a connection to each paired
//! worker and shows its status, pairing, naming and removing a worker, and
//! the executor's `Delegation` port over the connections.
//!
//! The master's key lives in the operating system's credential store, which
//! the plugin reaches through [`MasterKeyStore`], a service limited to the
//! plugin's own entry (ADR-0010; ADR-0030, decision 13).

mod connection;
mod link;
mod pairing;

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use mannie_link::keys::{KeyPin, LinkKey};
use mannie_link::tls;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{Clock, WorkerId};
use tokio::sync::watch;
use tokio::task::JoinHandle;

pub use self::pairing::{DEFAULT_PORT, normalize_address};

use self::connection::{
    ConnectError, Connection, Incompatible, LAST_RETRY, channel, hello, retry_delay, status_of,
};
use crate::commands::{WorkersError, check_worker_name};
use crate::store::{PairedWorker, WorkersStore, WorkersStoreError};
use crate::view::{WorkerStatus, WorkerView};

/// Why the master's key could not be read or kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyStoreError {
    /// The credential store cannot be used.
    #[error("the credential store cannot be used")]
    Unavailable,
    /// The stored key is not valid.
    #[error("the stored key is not valid")]
    Invalid,
}

/// Where the master keeps its key: the operating system's credential store,
/// under `link/master-key`. The store does its blocking calls on a thread of
/// its own, so the hub awaits them.
pub trait MasterKeyStore: Send + Sync + fmt::Debug + 'static {
    /// The master's key, or `None` before the first pairing.
    fn load(&self) -> BoxFuture<'static, Result<Option<LinkKey>, KeyStoreError>>;
    /// Keeps the master's key.
    fn save(&self, key: &LinkKey) -> BoxFuture<'static, Result<(), KeyStoreError>>;
}

/// The master's key from `keys`, or a new one that `keys` then keeps, before
/// the first pairing.
async fn load_or_make_key(keys: &dyn MasterKeyStore) -> Result<LinkKey, KeyStoreError> {
    if let Some(stored) = keys.load().await? {
        return Ok(stored);
    }
    let made = LinkKey::generate().map_err(|_random| KeyStoreError::Unavailable)?;
    keys.save(&made).await?;
    tracing::info!(
        pin = made.pin().short(),
        "made the master's key of the link"
    );
    Ok(made)
}

/// How the hub works.
#[derive(Debug, Clone)]
pub struct HubConfig {
    /// The master's version of Mannie, which `Hello` sends.
    pub version: String,
    /// The longest wait between two attempts to reach a worker: a minute,
    /// or less where a test waits for a worker to come back.
    pub max_retry_delay: Duration,
}

impl HubConfig {
    /// The configuration of a master of `version`.
    pub fn new(version: String) -> Self {
        Self {
            version,
            max_retry_delay: LAST_RETRY,
        }
    }
}

/// A channel to a worker over the link's TLS with the worker's pin, or none
/// when it could not open, for the plugin's test tools
/// (mannie-workers-testing), which turn on the feature `testing`.
#[cfg(feature = "testing")]
pub async fn test_channel(
    address: &str,
    config: Arc<rustls::ClientConfig>,
) -> Option<(tonic::transport::Channel, KeyPin)> {
    channel(address, config).await.ok()
}

/// A paired worker as the hub keeps it: its record and its view.
#[derive(Debug, Clone)]
struct Paired {
    record: PairedWorker,
    pin: KeyPin,
}

/// The name that Mannie shows for a worker: the owner's, or else the one in
/// the worker's configuration.
fn shown_name(configured: &str, chosen: Option<&str>) -> String {
    chosen.unwrap_or(configured).to_owned()
}

fn view_of(paired: &Paired, status: WorkerStatus) -> WorkerView {
    let record = &paired.record;
    WorkerView {
        id: record.id.clone(),
        name: shown_name(&record.name, record.chosen_name.as_deref()),
        configured_name: record.name.clone(),
        chosen_name: record.chosen_name.clone(),
        address: record.address.clone(),
        status,
        details: record.details.clone(),
        key_pin: paired.pin.short().to_owned(),
        paired_at: record.paired_at,
        connected_at: record.connected_at,
    }
}

#[derive(Debug)]
struct Inner {
    store: WorkersStore,
    keys: Arc<dyn MasterKeyStore>,
    /// The master's key, once it was read or made.
    key: tokio::sync::Mutex<Option<Arc<LinkKey>>>,
    version: String,
    max_retry_delay: Duration,
    /// The paired workers.
    paired: Mutex<HashMap<WorkerId, Paired>>,
    /// The paired workers as the UI and the runtime see them, in the order
    /// of pairing.
    views: watch::Sender<Vec<WorkerView>>,
    /// The working connections.
    connections: Mutex<HashMap<WorkerId, Arc<Connection>>>,
    /// The loops that keep the connections.
    loops: Mutex<HashMap<WorkerId, JoinHandle<()>>>,
    shutdown: watch::Sender<bool>,
    /// The jitter of retries.
    random: Mutex<fastrand::Rng>,
    clock: Arc<dyn Clock>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every critical section only reads or replaces entries.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The master's link to its workers. Cloning is cheap; clones share
/// everything.
#[derive(Debug, Clone)]
pub struct Hub {
    inner: Arc<Inner>,
}

impl Hub {
    /// Starts the hub: reads the paired workers and connects to each.
    pub async fn start(
        store: WorkersStore,
        keys: Arc<dyn MasterKeyStore>,
        config: HubConfig,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, WorkersError> {
        let records = store.paired_workers().await.map_err(|error| {
            tracing::error!(%error, error.code = error.code(), "could not read the paired workers");
            WorkersError::StorageFailed
        })?;
        let mut paired = HashMap::new();
        let mut views = Vec::new();
        for record in records {
            let Some(pin) = KeyPin::parse(&record.key_pin) else {
                tracing::warn!(worker = %record.id, "a paired worker's pin is not valid; it is left out");
                continue;
            };
            let entry = Paired { record, pin };
            views.push(view_of(&entry, WorkerStatus::Connecting));
            paired.insert(entry.record.id.clone(), entry);
        }
        let hub = Self {
            inner: Arc::new(Inner {
                store,
                keys,
                key: tokio::sync::Mutex::new(None),
                version: config.version,
                max_retry_delay: config.max_retry_delay,
                paired: Mutex::new(paired),
                views: watch::Sender::new(views),
                connections: Mutex::default(),
                loops: Mutex::default(),
                shutdown: watch::Sender::new(false),
                random: Mutex::new(fastrand::Rng::new()),
                clock,
            }),
        };
        let ids: Vec<WorkerId> = lock(&hub.inner.paired).keys().cloned().collect();
        for id in ids {
            hub.keep_connected(id);
        }
        Ok(hub)
    }

    /// The paired workers, as they are now and as they change.
    pub fn worker_views(&self) -> watch::Receiver<Vec<WorkerView>> {
        self.inner.views.subscribe()
    }

    /// Ends every connection, because Mannie quits.
    pub async fn shutdown(&self) {
        self.inner.shutdown.send_replace(true);
        let loops: Vec<JoinHandle<()>> = lock(&self.inner.loops)
            .drain()
            .map(|(_id, task)| task)
            .collect();
        for task in loops {
            task.abort();
            let _ended = task.await;
        }
        lock(&self.inner.connections).clear();
    }

    /// The master's key, made and kept on first use.
    async fn key(&self) -> Result<Arc<LinkKey>, WorkersError> {
        let mut key = self.inner.key.lock().await;
        if let Some(key) = key.as_ref() {
            return Ok(Arc::clone(key));
        }
        let loaded = load_or_make_key(self.inner.keys.as_ref())
            .await
            .map_err(|error| {
                tracing::warn!(%error, "could not read or keep the master's key");
                WorkersError::CredentialStoreUnavailable
            })?;
        let loaded = Arc::new(loaded);
        *key = Some(Arc::clone(&loaded));
        Ok(loaded)
    }

    /// Changes a worker's view.
    fn update_view(&self, worker: &WorkerId, change: impl FnOnce(&mut WorkerView)) {
        self.inner.views.send_modify(|views| {
            if let Some(view) = views.iter_mut().find(|view| view.id == *worker) {
                change(view);
            }
        });
    }

    fn set_status(&self, worker: &WorkerId, status: WorkerStatus) {
        self.update_view(worker, |view| view.status = status);
    }

    /// The working connection to a worker.
    pub(crate) fn connection(&self, worker: &WorkerId) -> Option<Arc<Connection>> {
        lock(&self.inner.connections).get(worker).map(Arc::clone)
    }

    /// Starts the loop that keeps a worker connected.
    fn keep_connected(&self, worker: WorkerId) {
        let hub = self.clone();
        let id = worker.clone();
        let task = tokio::spawn(async move { hub.connection_loop(id).await });
        let previous = lock(&self.inner.loops).insert(worker, task);
        if let Some(previous) = previous {
            previous.abort();
        }
    }

    /// Keeps one worker connected until it is removed or Mannie quits.
    async fn connection_loop(&self, worker: WorkerId) {
        let mut shutdown = self.inner.shutdown.subscribe();
        let mut failures: u32 = 0;
        loop {
            if *shutdown.borrow_and_update() {
                return;
            }
            let Some(paired) = lock(&self.inner.paired).get(&worker).cloned() else {
                return;
            };
            self.set_status(&worker, WorkerStatus::Connecting);
            match self.connect(&paired).await {
                Ok(connection) => {
                    failures = 0;
                    lock(&self.inner.connections).insert(worker.clone(), Arc::clone(&connection));
                    self.set_status(&worker, WorkerStatus::Connected);
                    tracing::info!(worker = %worker, "connected to a worker");
                    tokio::select! {
                        () = connection.follow() => {}
                        _shutdown = shutdown.changed() => {}
                    }
                    lock(&self.inner.connections).remove(&worker);
                    tracing::info!(worker = %worker, "the link to a worker dropped");
                    self.set_status(&worker, WorkerStatus::Offline);
                }
                Err(error) => {
                    let status = status_of(&error);
                    if let ConnectError::Incompatible(incompatible) = error {
                        let Incompatible {
                            version,
                            name,
                            details,
                        } = *incompatible;
                        tracing::warn!(worker = %worker, %version, "a worker runs a version that this one does not work with");
                        self.update_view(&worker, |view| {
                            view.name = shown_name(&name, view.chosen_name.as_deref());
                            view.configured_name = name;
                            view.details = details;
                        });
                    }
                    self.set_status(&worker, status);
                }
            }
            failures = failures.saturating_add(1);
            let delay = retry_delay(failures, lock(&self.inner.random).f64())
                .min(self.inner.max_retry_delay);
            tokio::select! {
                () = tokio::time::sleep(delay) => {}
                _shutdown = shutdown.changed() => return,
            }
        }
    }

    /// Connects to a paired worker and says `Hello`.
    async fn connect(&self, paired: &Paired) -> Result<Arc<Connection>, ConnectError> {
        let key = self
            .key()
            .await
            .map_err(|_unavailable| ConnectError::Unreachable)?;
        let config = tls::client_config(&key, Some(paired.pin.clone()))
            .map_err(|_error| ConnectError::Unreachable)?;
        let (channel, _pin) = channel(&paired.record.address, config).await?;
        let hello = hello(&channel, &self.inner.version).await?;
        let id = &paired.record.id;
        if let Err(error) = self
            .inner
            .store
            .update_worker_hello(id, &hello.name, &hello.details)
            .await
        {
            tracing::warn!(%error, error.code = error.code(), "could not store what a worker said about itself");
        }
        let now = self.inner.clock.now();
        self.update_view(id, |view| {
            view.name = shown_name(&hello.name, view.chosen_name.as_deref());
            view.configured_name.clone_from(&hello.name);
            view.details = hello.details.clone();
            view.connected_at = Some(now);
        });
        Ok(Arc::new(Connection::new(channel)))
    }

    /// Gives a worker the name that the owner chose, or with a blank name
    /// goes back to the name in its configuration. Mannie and the
    /// conversation agent use the name from then on, so the agent's list of
    /// workers and `delegate_task` follow the watch channel.
    pub async fn rename(&self, worker: &WorkerId, name: &str) -> Result<WorkerView, WorkersError> {
        let chosen = check_worker_name(name)?;
        if let Some(chosen) = &chosen {
            self.check_name_free(chosen, Some(worker))?;
        }
        self.inner
            .store
            .set_worker_chosen_name(worker, chosen.as_deref())
            .await
            .map_err(|error| match error {
                WorkersStoreError::WorkerNotFound => WorkersError::WorkerNotFound,
                other => {
                    tracing::error!(error = %other, error.code = other.code(), "could not rename a worker");
                    WorkersError::StorageFailed
                }
            })?;
        if let Some(paired) = lock(&self.inner.paired).get_mut(worker) {
            paired.record.chosen_name.clone_from(&chosen);
        }
        let mut renamed = None;
        self.inner.views.send_modify(|views| {
            if let Some(view) = views.iter_mut().find(|view| view.id == *worker) {
                view.name = shown_name(&view.configured_name, chosen.as_deref());
                view.chosen_name = chosen;
                renamed = Some(view.clone());
            }
        });
        tracing::info!(worker = %worker, "renamed a worker");
        renamed.ok_or(WorkersError::WorkerNotFound)
    }

    /// Refuses a name that another paired worker shows, ignoring case, so
    /// that the owner and the conversation agent can tell workers apart.
    fn check_name_free(&self, name: &str, except: Option<&WorkerId>) -> Result<(), WorkersError> {
        let wanted = name.to_lowercase();
        let taken = self
            .inner
            .views
            .borrow()
            .iter()
            .filter(|view| Some(&view.id) != except)
            .any(|view| view.name.to_lowercase() == wanted);
        if taken {
            Err(WorkersError::NameTaken)
        } else {
            Ok(())
        }
    }

    /// Removes a worker: it is unpaired when it can be reached, and the
    /// master forgets it and the deletions that waited for it. A worker that
    /// was offline keeps its pairing until `mannie-worker pair --reset`.
    pub async fn remove(&self, worker: &WorkerId) -> Result<(), WorkersError> {
        if let Some(connection) = self.connection(worker)
            && let Err(error) = link::unpair(&connection).await
        {
            tracing::warn!(worker = %worker, %error, "could not unpair a worker that is removed");
        }
        let removed = self
            .inner
            .store
            .remove_paired_worker(worker)
            .await
            .map_err(|error| {
                tracing::error!(%error, error.code = error.code(), "could not remove a worker");
                WorkersError::StorageFailed
            })?;
        if !removed {
            return Err(WorkersError::WorkerNotFound);
        }
        lock(&self.inner.paired).remove(worker);
        let task = lock(&self.inner.loops).remove(worker);
        if let Some(task) = task {
            task.abort();
        }
        lock(&self.inner.connections).remove(worker);
        self.inner
            .views
            .send_modify(|views| views.retain(|view| view.id != *worker));
        tracing::info!(worker = %worker, "removed a worker");
        Ok(())
    }

    /// Adds a worker that pairing found, and starts connecting to it.
    fn add(&self, record: PairedWorker, pin: KeyPin) -> WorkerView {
        let paired = Paired { record, pin };
        let view = view_of(&paired, WorkerStatus::Connecting);
        let id = paired.record.id.clone();
        lock(&self.inner.paired).insert(id.clone(), paired);
        self.inner
            .views
            .send_modify(|views| views.push(view.clone()));
        self.keep_connected(id);
        view
    }
}
