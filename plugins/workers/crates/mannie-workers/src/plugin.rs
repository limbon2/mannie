//! The Workers plugin's definition (ADR-0031, decisions 1 and 10;
//! docs/features/workers/plugin.md): its ID, its tables, its default, its
//! start and stop, and the hub that the desktop app's commands reach while
//! it is on.
//!
//! Turning Workers off closes every link and lets each followed task go, as
//! quitting does: nothing is cancelled, and the tasks run on on their
//! workers. Turning it on connects to the paired workers and compares their
//! tasks with the master's rows, as at startup.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_plugins::{GRANT_MISSING, Plugin, PluginStartError, StartContext};
use mannie_runtime::PluginContributions;
use mannie_store::{ModuleStore, StoreModule};
use tokio::sync::watch;

use crate::commands::WorkersError;
use crate::executor::{WORKERS_KIND, WorkersExecutor};
use crate::hub::{Hub, HubConfig};
use crate::keys::SecretsKeyStore;
use crate::store::{WORKERS_MODULE, WorkersStore};
use crate::view::WorkerView;

/// The code of a start that could not read the paired workers.
pub const STORAGE_FAILED: &str = "storage_failed";

/// What runs while Workers is on.
#[derive(Debug)]
struct Running {
    hub: Hub,
    executor: WorkersExecutor,
}

#[derive(Debug)]
struct Inner {
    running: Mutex<Option<Running>>,
    /// Whether Workers is on, for the followers of the list of workers.
    on: watch::Sender<bool>,
    /// The longest wait between two attempts to reach a worker, when it is
    /// shorter than the hub's minute.
    max_retry_delay: Option<Duration>,
}

fn lock(running: &Mutex<Option<Running>>) -> MutexGuard<'_, Option<Running>> {
    // Every critical section only replaces the value.
    running.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The list of workers while Workers is on: the workers as they are and as
/// they change, and whether Workers is still on.
#[derive(Debug)]
pub struct WorkerSubscription {
    /// The paired workers.
    pub workers: watch::Receiver<Vec<WorkerView>>,
    /// Changes to false when Workers stops.
    pub on: watch::Receiver<bool>,
}

/// The Workers plugin. Cloning is cheap; clones share everything.
#[derive(Debug, Clone)]
pub struct WorkersPlugin {
    inner: Arc<Inner>,
}

impl Default for WorkersPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkersPlugin {
    /// The plugin, off.
    pub fn new() -> Self {
        Self::with_retry_delay(None)
    }

    /// The plugin, off, whose hub waits at most `max` between two attempts
    /// to reach a worker, as the end-to-end tests need to see a worker come
    /// back soon.
    pub fn with_max_retry_delay(max: Duration) -> Self {
        Self::with_retry_delay(Some(max))
    }

    fn with_retry_delay(max_retry_delay: Option<Duration>) -> Self {
        Self {
            inner: Arc::new(Inner {
                running: Mutex::new(None),
                on: watch::Sender::new(false),
                max_retry_delay,
            }),
        }
    }

    /// The hub while Workers is on, for the IPC commands, which refuse with
    /// `plugin_off` while it is off (ADR-0030, decision 5).
    pub fn hub(&self) -> Result<Hub, WorkersError> {
        lock(&self.inner.running)
            .as_ref()
            .map(|running| running.hub.clone())
            .ok_or(WorkersError::PluginOff)
    }

    /// Follows the list of workers while Workers is on.
    pub fn subscribe(&self) -> Result<WorkerSubscription, WorkersError> {
        let running = lock(&self.inner.running);
        let current = running.as_ref().ok_or(WorkersError::PluginOff)?;
        Ok(WorkerSubscription {
            workers: current.hub.worker_views(),
            on: self.inner.on.subscribe(),
        })
    }
}

/// The code of a hub that could not start.
fn start_code(error: &WorkersError) -> &'static str {
    match error {
        WorkersError::StorageFailed | WorkersError::StoreUnavailable => STORAGE_FAILED,
        _other => "start_failed",
    }
}

impl Plugin for WorkersPlugin {
    fn id(&self) -> &'static str {
        WORKERS_KIND
    }

    fn store_modules(&self) -> Vec<StoreModule> {
        vec![WORKERS_MODULE]
    }

    /// Workers starts when the store holds a paired worker, so an owner who
    /// used workers in 0.2.0 keeps them (ADR-0031, decision 10).
    fn default_on(&self, stores: Vec<ModuleStore>) -> BoxFuture<'static, bool> {
        async move {
            let Some(store) = stores.into_iter().find(|store| store.name() == WORKERS_MODULE.name)
            else {
                return false;
            };
            match WorkersStore::new(store).has_paired_workers().await {
                Ok(paired) => paired,
                Err(error) => {
                    tracing::error!(%error, error.code = error.code(), "could not read whether a worker is paired");
                    false
                }
            }
        }
        .boxed()
    }

    fn start(
        &self,
        context: StartContext,
    ) -> BoxFuture<'static, Result<PluginContributions, PluginStartError>> {
        let inner = Arc::clone(&self.inner);
        async move {
            let StartContext {
                stores,
                grants,
                clock,
                version,
            } = context;
            let store = stores
                .into_iter()
                .find(|store| store.name() == WORKERS_MODULE.name)
                .ok_or_else(|| PluginStartError::new(STORAGE_FAILED))?;
            let (Some(services), Some(lender), Some(secrets)) =
                (grants.executors, grants.credentials, grants.secrets)
            else {
                tracing::error!("the composition root did not grant Workers the services it needs");
                return Err(PluginStartError::new(GRANT_MISSING));
            };
            let keys = Arc::new(SecretsKeyStore::new(secrets));
            let config = HubConfig::new(version);
            let config = HubConfig {
                max_retry_delay: inner.max_retry_delay.unwrap_or(config.max_retry_delay),
                ..config
            };
            let hub = Hub::start(WorkersStore::new(store), keys, config, Arc::clone(&clock))
                .await
                .map_err(|error| {
                    tracing::error!(
                        error.code = error.code(),
                        "the Workers plugin could not start its hub"
                    );
                    PluginStartError::new(start_code(&error))
                })?;
            let executor = WorkersExecutor::new(services, lender, Arc::new(hub.clone()), clock);
            executor.start();
            *lock(&inner.running) = Some(Running {
                hub,
                executor: executor.clone(),
            });
            inner.on.send_replace(true);
            tracing::info!("the Workers plugin started");
            Ok(PluginContributions {
                executors: vec![Arc::new(executor)],
                ..PluginContributions::default()
            })
        }
        .boxed()
    }

    fn stop(&self) -> BoxFuture<'static, ()> {
        let inner = Arc::clone(&self.inner);
        async move {
            let running = lock(&inner.running).take();
            inner.on.send_replace(false);
            let Some(Running { hub, executor }) = running else {
                return;
            };
            // The host took the executor out of the runtime first, so each
            // run let its worker tasks go; the followers and the links end
            // now, and nothing is cancelled.
            executor.stop();
            hub.shutdown().await;
            tracing::info!("the Workers plugin stopped");
        }
        .boxed()
    }
}
