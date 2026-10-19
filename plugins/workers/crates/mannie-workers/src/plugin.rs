//! The Workers plugin's definition (ADR-0031, decisions 1 and 10;
//! plugins/workers/docs/plugin.md): its interface, its tables, its default,
//! its start and stop, and the handlers of its commands and channel, which
//! reach the hub of their start. Its manifest, `plugins/workers/plugin.toml`,
//! gives its ID and the services that its start receives. The host answers
//! `plugin_off` while Workers is off and ends the list of workers when it
//! stops (ADR-0034, decision 8).
//!
//! Turning Workers off closes every link and lets each followed task go, as
//! quitting does: nothing is cancelled, and the tasks run on on their
//! workers. Turning it on connects to the paired workers and compares their
//! tasks with the master's rows, as at startup.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{StreamExt, stream};
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::{DefaultContext, HostInfo, HostProfile, StartContext};
use mannie_plugin_sdk::contracts::WorkItem;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::error::StartFailure;
use mannie_plugin_sdk::ipc::{Handlers, Interface, PluginError, PluginStream};
use mannie_plugin_sdk::plugin::{Instance, Plugin, Started};
use mannie_plugin_sdk::points::executors::Executors;
use mannie_plugin_sdk::services::credentials::Credentials;
use mannie_plugin_sdk::services::executor_host::ExecutorHost;
use mannie_plugin_sdk::services::secrets::Secrets;
use mannie_plugin_sdk::services::store::{Store, StoreModule};
use tokio::sync::watch;

use crate::commands::{
    PairWorker, PairWorkerCommand, RemoveWorkerCommand, RenameWorker, RenameWorkerCommand,
    SubscribeWorkersChannel, WorkerRef, WorkersError, interface,
};
use crate::executor::WorkersExecutor;
use crate::hub::{Hub, HubConfig};
use crate::keys::SecretsKeyStore;
use crate::store::{WORKERS_MODULE, WorkersStore, has_paired_workers};
use crate::view::{WorkerListUpdate, WorkerView};

/// The code of a start that could not read the paired workers.
pub const STORAGE_FAILED: &str = "storage_failed";

/// The code of Workers' work in progress, its tasks that have not ended,
/// which the plugin's UI words.
pub const ACTIVE_TASKS: &str = "active_tasks";

/// The longest wait of a validation build between two attempts to reach a
/// worker, so the end-to-end tests see a worker come back soon.
const VALIDATION_RETRY_DELAY: Duration = Duration::from_secs(2);

/// The Workers plugin.
#[derive(Debug, Clone, Copy, Default)]
pub struct WorkersPlugin;

impl WorkersPlugin {
    /// The plugin, off.
    pub fn new() -> Self {
        Self
    }
}

/// The code of a hub that could not start.
fn start_code(error: &WorkersError) -> &'static str {
    match error {
        WorkersError::StorageFailed => STORAGE_FAILED,
        _other => "start_failed",
    }
}

/// The hub's configuration on `host`: a validation build tries a worker
/// again after two seconds at most, and every other build after the hub's
/// minute at most.
fn hub_config(host: &HostInfo) -> HubConfig {
    let config = HubConfig::new(host.version.clone());
    match host.profile {
        HostProfile::Validation => HubConfig {
            max_retry_delay: VALIDATION_RETRY_DELAY,
            ..config
        },
        _other => config,
    }
}

/// The paired workers as they are, then after each change, until the hub of
/// the start stops.
fn worker_list(workers: watch::Receiver<Vec<WorkerView>>) -> PluginStream<WorkerListUpdate> {
    stream::unfold((workers, true), |(mut workers, first)| async move {
        if !first && workers.changed().await.is_err() {
            return None;
        }
        let current = workers.borrow_and_update().clone();
        Some((
            WorkerListUpdate::Workers { workers: current },
            (workers, false),
        ))
    })
    .boxed()
}

/// The handlers of a start, which reach its hub.
fn handlers(hub: &Hub) -> Handlers {
    let pairing = hub.clone();
    let renaming = hub.clone();
    let removing = hub.clone();
    let following = hub.clone();
    Handlers::new()
        .command::<PairWorkerCommand, _, _>(move |input: PairWorker| {
            let hub = pairing.clone();
            async move { hub.pair(input).await }
        })
        .command::<RenameWorkerCommand, _, _>(move |input: RenameWorker| {
            let hub = renaming.clone();
            async move { hub.rename(&input.worker_id, &input.name).await }
        })
        .command::<RemoveWorkerCommand, _, _>(move |input: WorkerRef| {
            let hub = removing.clone();
            async move { hub.remove(&input.worker_id).await }
        })
        .channel::<SubscribeWorkersChannel, _, _>(move |()| {
            let workers = worker_list(following.worker_views());
            std::future::ready(Ok::<_, WorkersError>(workers))
        })
}

impl Plugin for WorkersPlugin {
    fn interface(&self) -> Interface {
        interface()
    }

    fn store_module(&self) -> Option<StoreModule> {
        Some(WORKERS_MODULE)
    }

    /// Workers starts when the store holds a paired worker, so an owner who
    /// used workers in 0.2.0 keeps them (ADR-0031, decision 10).
    fn default_on(&self, context: DefaultContext) -> BoxFuture<'static, bool> {
        Box::pin(async move {
            let Some(store) = context.store() else {
                return false;
            };
            match has_paired_workers(store.as_ref()).await {
                Ok(paired) => paired,
                Err(error) => {
                    tracing::error!(%error, error.code = error.code(), "could not read whether a worker is paired");
                    false
                }
            }
        })
    }

    fn start(&self, context: StartContext) -> BoxFuture<'static, Result<Started, StartFailure>> {
        Box::pin(async move {
            let store = context.service::<Store>()?;
            let host = context.service::<ExecutorHost>()?;
            let credentials = context.service::<Credentials>()?;
            let secrets = context.service::<Secrets>()?;
            let clock = context.clock();
            let workers = WorkersStore::new(store, Arc::clone(&clock));
            let keys = Arc::new(SecretsKeyStore::new(secrets));
            let hub = Hub::start(
                workers,
                keys,
                hub_config(context.host()),
                Arc::clone(&clock),
            )
            .await
            .map_err(|error| {
                tracing::error!(
                    error.code = error.code(),
                    "the Workers plugin could not start its hub"
                );
                StartFailure::new(start_code(&error))
            })?;
            let executor = WorkersExecutor::new(host, credentials, Arc::new(hub.clone()), clock);
            executor.start();
            tracing::info!("the Workers plugin started");
            let contributions = Contributions::new().add::<Executors>(Arc::new(executor.clone()));
            let handlers = handlers(&hub);
            let instance = WorkersInstance { hub, executor };
            Ok(Started::new(instance)
                .with_contributions(contributions)
                .with_handlers(handlers))
        })
    }
}

/// Workers while it is on: its hub and its executor.
#[derive(Debug)]
struct WorkersInstance {
    hub: Hub,
    executor: WorkersExecutor,
}

impl Instance for WorkersInstance {
    /// The tasks of Workers' executor that have not ended.
    fn work(&self) -> BoxFuture<'static, Vec<WorkItem>> {
        self.executor.work()
    }

    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        let Self { hub, executor } = *self;
        Box::pin(async move {
            // The host took the executor out of the runtime and ended the
            // calls and the channel of this start first, so each run let its
            // worker tasks go; the followers and the links end now, and
            // nothing is cancelled.
            executor.stop();
            hub.shutdown().await;
            tracing::info!("the Workers plugin stopped");
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use mannie_plugin_sdk::context::{HostInfo, HostProfile};

    use super::hub_config;

    #[test]
    fn validation_builds_try_a_worker_again_after_two_seconds_and_others_after_a_minute() {
        let config = |profile| hub_config(&HostInfo::new("0.3.0", profile));

        let validation = config(HostProfile::Validation);
        let development = config(HostProfile::Development);
        let release = config(HostProfile::Release);

        assert_eq!(validation.max_retry_delay, Duration::from_secs(2));
        assert_eq!(development.max_retry_delay, Duration::from_secs(60));
        assert_eq!(release.max_retry_delay, Duration::from_secs(60));
        assert_eq!(release.version, "0.3.0");
    }
}
