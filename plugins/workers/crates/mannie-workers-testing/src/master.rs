//! The master's side of the test tools: an in-process link from the
//! master's executor to a worker's runtime, a master and a worker joined by
//! it, the master's key in memory, and a master with a hub on loopback for
//! the link's tests (plugins/workers/docs/delegation.md and
//! pairing-and-link.md, acceptance). The worker's side of the link's tests is
//! [`crate::worker`].

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::worker::{LinkedWorker, LinkedWorkerOptions};
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::{FutureExt, StreamExt};
use mannie_link::host::WorkerDetails;
use mannie_link::keys::{KeyPin, LinkKey};
use mannie_link::proto::tasks::GetRequest;
use mannie_link::proto::tasks::tasks_service_client::TasksServiceClient;
use mannie_link::tls;
use mannie_plugin_harness::runtime::{AgentRegistry, Runtime, RuntimeConfig};
use mannie_plugin_harness::runtime::{
    FakeModel, Harness, HarnessOptions, MemoryVault, WorkerHarness, WorkerOptions, add_plugin,
    credentials, executor_host, fast_config, open_store_with, remove_plugin, start_master,
    test_tools,
};
use mannie_plugin_harness::store::{PluginModule, Store};
use mannie_plugin_harness::types::PartContent;
use mannie_plugin_harness::types::SystemClock;
use mannie_plugin_harness::types::TaskView;
use mannie_plugin_harness::types::{
    ApiInterface, CreateProviderProfile, ProfileDraft, ProviderProfile, ReasoningEffort,
};
use mannie_plugin_harness::types::{StartConversation, SubscribeConversation};
use mannie_plugin_sdk::contracts::Secret;
use mannie_plugin_sdk::contracts::{
    ApprovalChoice, DecideApproval, ReadToolOutput, ToolOutputPage, ToolsError,
};
use mannie_plugin_sdk::contracts::{AssignedTaskState, TaskAssignment};
use mannie_plugin_sdk::contracts::{ChatError, CheckpointRef, EarlierTurns, LoadEarlierTurns};
use mannie_plugin_sdk::contracts::{Checkpoint, ConversationUpdate};
use mannie_plugin_sdk::contracts::{Clock, UtcMillis};
use mannie_plugin_sdk::contracts::{ConversationId, TaskId, WorkerId};
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::points::executors::Executors;
use tokio::sync::watch;

use mannie_workers::commands::{PairWorker, WorkersError};
use mannie_workers::delegation::{Delegation, DelegationError};
use mannie_workers::executor::{WORKERS_KIND, WorkersExecutor};
use mannie_workers::hub::{Hub, HubConfig, KeyStoreError, MasterKeyStore};
use mannie_workers::store::{WORKERS_MODULE, WorkersStore};
use mannie_workers::view::{WorkerStatus, WorkerView};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The plugin's ID, `workers`.
///
/// # Panics
///
/// Never: the kind is a valid plugin ID, which the plugin's tests check.
pub fn workers_id() -> PluginId {
    PluginId::parse(WORKERS_KIND).expect("the kind `workers` is a valid plugin ID")
}

/// The plugin's module with its prefix, as the desktop app's composition
/// root hands it to the store.
pub fn workers_module() -> PluginModule {
    PluginModule::new(&workers_id(), WORKERS_MODULE)
}

/// The plugin's tables in `store`, which was opened with
/// [`workers_module`], stamped with `clock`.
///
/// # Panics
///
/// When the store was opened without the module, which fails the test.
pub fn workers_store(store: &Store, clock: Arc<dyn Clock>) -> WorkersStore {
    let module = store
        .plugin_store(&workers_id())
        .expect("the store opened with the module of Workers");
    WorkersStore::new(module, clock)
}

/// The faults that a [`LocalDelegation`] injects.
#[derive(Debug, Default)]
struct Faults {
    /// The worker cannot be reached: every call fails, and the watches and
    /// subscriptions end.
    offline: bool,
    /// How many of the next submissions reach the worker and lose their
    /// answers.
    lost_answers: u32,
    /// Submissions answer only once the worker's run of the task waits for
    /// its lease, so the master's follower of the worker sees that state
    /// before the run that submitted the task gets the answer.
    answer_when_waiting: bool,
    /// The next submission finds the worker gone offline, before the worker
    /// stored its task.
    offline_at_next_submit: bool,
}

/// A link to one worker in the same process: the master's executor reaches
/// the worker's runtime directly, and a test can take the worker offline,
/// lose or delay the answers of submissions, and see the leases delivered
/// and revoked (plugins/workers/docs/delegation.md, acceptance).
#[derive(Debug)]
pub struct LocalDelegation {
    worker: Mutex<Runtime>,
    view: watch::Sender<Vec<WorkerView>>,
    /// Ends the watches and subscriptions when the worker goes offline.
    online: watch::Sender<bool>,
    faults: Mutex<Faults>,
    /// The tasks of the leases that the master delivered, once for each
    /// delivery.
    delivered: Mutex<Vec<TaskId>>,
    /// The tasks of the leases that the master revoked.
    revoked: Mutex<Vec<TaskId>>,
}

/// The ID of the worker of a [`LocalDelegation`].
pub const LOCAL_WORKER_ID: &str = "0199e0f2-0000-7000-b000-000000000001";

/// The name of the worker of a [`LocalDelegation`].
pub const LOCAL_WORKER_NAME: &str = "build-01";

impl LocalDelegation {
    /// A port to `worker`, connected, at `address`.
    ///
    /// # Panics
    ///
    /// Never: the worker's ID is a valid UUID.
    pub fn new(worker: Runtime, address: &str) -> Self {
        let view = WorkerView {
            id: WorkerId::parse(LOCAL_WORKER_ID).expect("a valid ID"),
            name: LOCAL_WORKER_NAME.to_owned(),
            configured_name: LOCAL_WORKER_NAME.to_owned(),
            chosen_name: None,
            address: address.to_owned(),
            status: WorkerStatus::Connected,
            details: WorkerDetails {
                purpose: "Builds and tests the backend".to_owned(),
                facts: None,
                ceiling: None,
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
                max_parallel_tasks: 4,
            },
            key_pin: "0123456789ab".to_owned(),
            paired_at: UtcMillis::EPOCH,
            connected_at: None,
        };
        Self {
            worker: Mutex::new(worker),
            view: watch::Sender::new(vec![view]),
            online: watch::Sender::new(true),
            faults: Mutex::default(),
            delivered: Mutex::default(),
            revoked: Mutex::default(),
        }
    }

    /// The worker's ID.
    ///
    /// # Panics
    ///
    /// Never: the ID is a valid UUID.
    pub fn worker_id() -> WorkerId {
        WorkerId::parse(LOCAL_WORKER_ID).expect("a valid ID")
    }

    /// Replaces the worker's runtime, as after a restart of the worker.
    pub fn set_worker(&self, worker: Runtime) {
        *lock(&self.worker) = worker;
    }

    /// Takes the worker offline, or brings it back.
    pub fn set_online(&self, online: bool) {
        lock(&self.faults).offline = !online;
        self.online.send_replace(online);
        let status = if online {
            WorkerStatus::Connected
        } else {
            WorkerStatus::Offline
        };
        self.view.send_modify(|views| {
            for view in views {
                view.status = status;
            }
        });
    }

    /// Removes the worker from the paired workers.
    pub fn remove(&self) {
        self.view.send_replace(Vec::new());
    }

    /// Loses the answers of the next `count` submissions, after the worker
    /// stored their tasks.
    pub fn lose_submit_answers(&self, count: u32) {
        lock(&self.faults).lost_answers = count;
    }

    /// Answers each submission only once the worker's run of the task waits
    /// for its lease.
    pub fn answer_when_waiting(&self) {
        lock(&self.faults).answer_when_waiting = true;
    }

    /// Takes the worker offline as the next submission reaches the link,
    /// before the worker stores its task, so the submission waits for the
    /// worker to come back.
    pub fn go_offline_at_next_submit(&self) {
        lock(&self.faults).offline_at_next_submit = true;
    }

    /// The tasks of the leases that the master delivered, once for each
    /// delivery, in order.
    pub fn delivered_leases(&self) -> Vec<TaskId> {
        lock(&self.delivered).clone()
    }

    /// The tasks of the leases that the master revoked, in order.
    pub fn revoked_leases(&self) -> Vec<TaskId> {
        lock(&self.revoked).clone()
    }

    fn worker(&self) -> Result<Runtime, DelegationError> {
        if lock(&self.faults).offline {
            return Err(DelegationError::Unreachable);
        }
        Ok(lock(&self.worker).clone())
    }

    /// A stream that ends when the worker goes offline.
    fn while_online<T: Send + 'static>(
        &self,
        stream: BoxStream<'static, T>,
    ) -> BoxStream<'static, T> {
        let mut online = self.online.subscribe();
        let offline = async move {
            let _offline = online.wait_for(|online| !*online).await.is_ok();
        };
        stream.take_until(offline).boxed()
    }
}

fn refused(code: &str) -> DelegationError {
    DelegationError::Refused {
        code: code.to_owned(),
    }
}

impl Delegation for LocalDelegation {
    fn workers(&self) -> watch::Receiver<Vec<WorkerView>> {
        self.view.subscribe()
    }

    fn submit(
        &self,
        _worker: &WorkerId,
        assignment: TaskAssignment,
    ) -> BoxFuture<'static, Result<AssignedTaskState, DelegationError>> {
        let going_offline = std::mem::take(&mut lock(&self.faults).offline_at_next_submit);
        if going_offline {
            self.set_online(false);
        }
        let worker = self.worker();
        let (lose, answer_when_waiting) = {
            let mut faults = lock(&self.faults);
            let lose = faults.lost_answers > 0;
            faults.lost_answers = faults.lost_answers.saturating_sub(1);
            (lose, faults.answer_when_waiting)
        };
        async move {
            let worker = worker?;
            // The watch starts before the assignment, so it misses no state
            // of the task.
            let watch = answer_when_waiting.then(|| worker.watch_assigned_tasks());
            let mut state = worker
                .assign_task(assignment)
                .await
                .map_err(|error| refused(error.code()))?;
            if let Some(mut watch) = watch {
                while !state.waits_for_lease && state.state.is_active() {
                    let Some(next) = watch.next().await else {
                        break;
                    };
                    if next.task_id == state.task_id {
                        state = next;
                    }
                }
            }
            if lose {
                return Err(DelegationError::Unreachable);
            }
            Ok(state)
        }
        .boxed()
    }

    fn task(
        &self,
        _worker: &WorkerId,
        task: &TaskId,
    ) -> BoxFuture<'static, Result<Option<AssignedTaskState>, DelegationError>> {
        let worker = self.worker();
        let task = task.clone();
        async move {
            worker?
                .assigned_task(&task)
                .await
                .map_err(|error| refused(error.code()))
        }
        .boxed()
    }

    fn cancel(
        &self,
        _worker: &WorkerId,
        task: &TaskId,
    ) -> BoxFuture<'static, Result<Option<AssignedTaskState>, DelegationError>> {
        let worker = self.worker();
        let task = task.clone();
        async move {
            worker?
                .cancel_assigned_task(&task)
                .await
                .map_err(|error| refused(error.code()))
        }
        .boxed()
    }

    fn watch(
        &self,
        _worker: &WorkerId,
    ) -> BoxFuture<'static, Result<BoxStream<'static, AssignedTaskState>, DelegationError>> {
        let stream = self.worker().map(|worker| {
            let watch = worker.watch_assigned_tasks();
            let states = futures_util::stream::unfold(watch, |mut watch| async move {
                watch.next().await.map(|state| (state, watch))
            });
            self.while_online(states.boxed())
        });
        async move { stream }.boxed()
    }

    fn deliver_lease(
        &self,
        _worker: &WorkerId,
        task: &TaskId,
        binding: String,
        credential: Secret,
    ) -> BoxFuture<'static, Result<(), DelegationError>> {
        lock(&self.delivered).push(task.clone());
        let worker = self.worker();
        let task = task.clone();
        async move {
            worker?
                .deliver_lease(&task, &binding, credential)
                .await
                .map_err(|error| refused(error.code()))
        }
        .boxed()
    }

    fn revoke_lease(
        &self,
        _worker: &WorkerId,
        task: &TaskId,
    ) -> BoxFuture<'static, Result<(), DelegationError>> {
        lock(&self.revoked).push(task.clone());
        let worker = self.worker();
        let task = task.clone();
        async move {
            worker?
                .revoke_lease(&task)
                .await
                .map_err(|error| refused(error.code()))
        }
        .boxed()
    }

    fn delete_conversations(
        &self,
        _worker: &WorkerId,
        conversations: Vec<ConversationId>,
    ) -> BoxFuture<'static, Result<(), DelegationError>> {
        let worker = self.worker();
        async move {
            worker?
                .delete_assigned_conversations(&conversations)
                .await
                .map_err(|error| refused(error.code()))
        }
        .boxed()
    }

    fn subscribe(
        &self,
        _worker: &WorkerId,
        conversation: &ConversationId,
        after_seq: Option<u32>,
    ) -> BoxFuture<'static, Result<BoxStream<'static, ConversationUpdate>, ChatError>> {
        let worker = self.worker();
        let conversation = conversation.clone();
        let online = self.online.subscribe();
        async move {
            let worker = worker.map_err(|_unreachable| ChatError::WorkerUnreachable)?;
            let subscription = worker
                .subscribe(SubscribeConversation {
                    conversation_id: conversation,
                    after_seq,
                })
                .await?;
            let updates =
                futures_util::stream::unfold(subscription, |mut subscription| async move {
                    subscription
                        .next()
                        .await
                        .map(|update| (update, subscription))
                });
            let mut online = online;
            let offline = async move {
                let _offline = online.wait_for(|online| !*online).await.is_ok();
            };
            Ok(updates.take_until(offline).boxed())
        }
        .boxed()
    }

    fn earlier_turns(
        &self,
        _worker: &WorkerId,
        input: LoadEarlierTurns,
    ) -> BoxFuture<'static, Result<EarlierTurns, ChatError>> {
        let worker = self.worker();
        async move {
            worker
                .map_err(|_unreachable| ChatError::WorkerUnreachable)?
                .earlier_turns(input)
                .await
        }
        .boxed()
    }

    fn checkpoint(
        &self,
        _worker: &WorkerId,
        input: CheckpointRef,
    ) -> BoxFuture<'static, Result<Checkpoint, ChatError>> {
        let worker = self.worker();
        async move {
            worker
                .map_err(|_unreachable| ChatError::WorkerUnreachable)?
                .checkpoint(input)
                .await
        }
        .boxed()
    }

    fn tool_output(
        &self,
        _worker: &WorkerId,
        input: ReadToolOutput,
    ) -> BoxFuture<'static, Result<ToolOutputPage, ToolsError>> {
        let worker = self.worker();
        async move {
            worker
                .map_err(|_unreachable| ToolsError::WorkerUnreachable)?
                .tool_output(input)
                .await
        }
        .boxed()
    }

    fn decide_approval(
        &self,
        _worker: &WorkerId,
        input: DecideApproval,
    ) -> BoxFuture<'static, Result<(), ToolsError>> {
        let worker = self.worker();
        async move {
            worker
                .map_err(|_unreachable| ToolsError::WorkerUnreachable)?
                .decide_approval(input)
                .await
        }
        .boxed()
    }

    fn allow_by_full_access(
        &self,
        _worker: &WorkerId,
        input: DecideApproval,
    ) -> BoxFuture<'static, Result<(), ToolsError>> {
        let worker = self.worker();
        async move {
            worker
                .map_err(|_unreachable| ToolsError::WorkerUnreachable)?
                .allow_by_full_access(&input)
        }
        .boxed()
    }
}

/// A master's runtime and a worker's runtime in one process, joined by a
/// [`LocalDelegation`] through the plugin's executor.
#[derive(Debug)]
pub struct DelegationHarness {
    /// The master, whose profile uses tools.
    pub master: Harness,
    /// The worker.
    pub worker: WorkerHarness,
    /// The link between them.
    pub delegation: Arc<LocalDelegation>,
    /// The plugin's executor in the master's runtime, while Workers is on.
    pub executor: Option<WorkersExecutor>,
}

impl DelegationHarness {
    /// A master and a worker with the default options, the worker at a
    /// loopback address.
    pub async fn start() -> Self {
        Self::with_options(
            HarnessOptions {
                use_tools: true,
                ..HarnessOptions::default()
            },
            WorkerOptions::default(),
            "127.0.0.1:7445",
        )
        .await
    }

    /// A master and a worker with these options, the worker at `address`.
    /// Workers is on.
    pub async fn with_options(
        master: HarnessOptions,
        worker: WorkerOptions,
        address: &str,
    ) -> Self {
        let worker = WorkerHarness::with_options(worker).await;
        let delegation = Arc::new(LocalDelegation::new(worker.runtime.clone(), address));
        let master = Harness::with_options(HarnessOptions {
            store_modules: vec![workers_module()],
            ..master
        })
        .await;
        let mut harness = Self {
            master,
            worker,
            delegation,
            executor: None,
        };
        harness.turn_on();
        harness
    }

    /// Turns Workers on, as the plugin's start does: a new executor joins
    /// the master's runtime and follows the worker.
    ///
    /// # Panics
    ///
    /// When Workers is on already, which fails the test.
    pub fn turn_on(&mut self) {
        assert!(self.executor.is_none(), "Workers is on already");
        let link: Arc<dyn Delegation> = Arc::<LocalDelegation>::clone(&self.delegation);
        let runtime = &self.master.runtime;
        let executor = WorkersExecutor::new(
            executor_host(runtime, WORKERS_KIND),
            credentials(runtime, WORKERS_KIND),
            link,
            Arc::new(SystemClock),
        );
        executor.start();
        add_plugin(
            &self.master.runtime,
            WORKERS_KIND,
            Contributions::new().add::<Executors>(Arc::new(executor.clone())),
        )
        .expect("the executor joins the runtime");
        self.executor = Some(executor);
    }

    /// Turns Workers off, as the plugin's stop does: the executor leaves the
    /// runtime, whose runs let its tasks go, and stops following the worker.
    pub fn turn_off(&mut self) {
        remove_plugin(&self.master.runtime, WORKERS_KIND);
        if let Some(executor) = self.executor.take() {
            executor.stop();
        }
    }

    /// Restarts the worker; its runs end as interrupted.
    pub async fn restart_worker(mut self) -> Self {
        self.worker = self.worker.restart().await;
        self.delegation.set_worker(self.worker.runtime.clone());
        self
    }

    /// Restarts the master, as quitting and starting Mannie does, with
    /// Workers on as it was.
    pub async fn restart_master(mut self) -> Self {
        let was_on = self.executor.is_some();
        self.turn_off();
        self.master = self.master.restart().await;
        if was_on {
            self.turn_on();
        }
        self
    }
}

/// The master's key in memory, whose store can be made unavailable.
#[derive(Debug, Default)]
pub struct MemoryKeyStore {
    key: Mutex<Option<Vec<u8>>>,
    unavailable: Mutex<bool>,
}

impl MemoryKeyStore {
    /// Makes the store unavailable, or available again.
    pub fn set_unavailable(&self, unavailable: bool) {
        *lock(&self.unavailable) = unavailable;
    }
}

impl MemoryKeyStore {
    fn loaded(&self) -> Result<Option<LinkKey>, KeyStoreError> {
        if *lock(&self.unavailable) {
            return Err(KeyStoreError::Unavailable);
        }
        lock(&self.key)
            .clone()
            .map(|pkcs8| LinkKey::from_pkcs8(pkcs8).map_err(|_invalid| KeyStoreError::Invalid))
            .transpose()
    }

    fn saved(&self, key: &LinkKey) -> Result<(), KeyStoreError> {
        if *lock(&self.unavailable) {
            return Err(KeyStoreError::Unavailable);
        }
        *lock(&self.key) = Some(key.pkcs8().to_vec());
        Ok(())
    }
}

impl MasterKeyStore for MemoryKeyStore {
    fn load(&self) -> BoxFuture<'static, Result<Option<LinkKey>, KeyStoreError>> {
        Box::pin(std::future::ready(self.loaded()))
    }

    fn save(&self, key: &LinkKey) -> BoxFuture<'static, Result<(), KeyStoreError>> {
        Box::pin(std::future::ready(self.saved(key)))
    }
}

/// A master's runtime with its hub and the plugin's executor on a temporary
/// store.
#[derive(Debug)]
pub struct LinkedMaster {
    /// The master's runtime, which delegates through the executor.
    pub runtime: Runtime,
    /// The hub.
    pub hub: Hub,
    /// The plugin's executor over the hub.
    pub executor: WorkersExecutor,
    /// The master's store.
    pub store: Store,
    /// The plugin's tables.
    pub workers: WorkersStore,
    /// The master's model.
    pub model: FakeModel,
    /// The master's credential vault.
    pub vault: Arc<MemoryVault>,
    /// The master's key store.
    pub keys: Arc<MemoryKeyStore>,
    /// The master's profile, which uses tools and has a credential.
    pub profile: ProviderProfile,
    _directory: tempfile::TempDir,
}

/// The longest wait of a test master between two attempts to reach a
/// worker, so that a test sees a worker come back soon.
const TEST_MAX_RETRY_DELAY: Duration = Duration::from_secs(2);

/// How a linked master is set up.
#[derive(Debug, Clone)]
pub struct LinkedMasterOptions {
    /// The version that the master says it runs.
    pub version: String,
    /// The runtime's configuration.
    pub config: RuntimeConfig,
    /// The base URL of the master's profile, which the worker's runs call:
    /// a fixture server on the worker's host, for a worker that calls one.
    pub profile_url: String,
}

impl Default for LinkedMasterOptions {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            config: fast_config(),
            profile_url: "http://127.0.0.1:9/v1".to_owned(),
        }
    }
}

impl LinkedMaster {
    /// A master of this version.
    pub async fn start() -> Self {
        Self::with_options(LinkedMasterOptions::default()).await
    }

    /// A master of `version`, with `config`.
    pub async fn with_version(version: &str, config: RuntimeConfig) -> Self {
        Self::with_options(LinkedMasterOptions {
            version: version.to_owned(),
            config,
            ..LinkedMasterOptions::default()
        })
        .await
    }

    /// A master with `options`.
    ///
    /// # Panics
    ///
    /// When the master cannot be set up, which fails the test.
    pub async fn with_options(options: LinkedMasterOptions) -> Self {
        let LinkedMasterOptions {
            version,
            config,
            profile_url,
        } = options;
        let directory = tempfile::tempdir().expect("a temporary directory");
        let store = open_store_with(
            &directory.path().join("mannie.sqlite"),
            vec![workers_module()],
        );
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let workers = workers_store(&store, Arc::clone(&clock));
        let keys = Arc::new(MemoryKeyStore::default());
        let hub = Hub::start(
            workers.clone(),
            Arc::<MemoryKeyStore>::clone(&keys),
            HubConfig {
                version,
                max_retry_delay: TEST_MAX_RETRY_DELAY,
            },
            Arc::clone(&clock),
        )
        .await
        .expect("the hub starts");
        let model = FakeModel::new(ApiInterface::ChatCompletions);
        let vault = Arc::new(MemoryVault::new());
        let runtime = start_master(
            store.clone(),
            Arc::new(model.clone()),
            Arc::clone(&vault),
            config,
            test_tools(directory.path()),
            AgentRegistry::with_builtin(),
        )
        .await;
        let executor = WorkersExecutor::new(
            executor_host(&runtime, WORKERS_KIND),
            credentials(&runtime, WORKERS_KIND),
            Arc::new(hub.clone()),
            clock,
        );
        executor.start();
        add_plugin(
            &runtime,
            WORKERS_KIND,
            Contributions::new().add::<Executors>(Arc::new(executor.clone())),
        )
        .expect("the executor joins the runtime");
        let profile = runtime
            .create_profile(CreateProviderProfile {
                draft: ProfileDraft {
                    name: "Test".to_owned(),
                    interface: ApiInterface::ChatCompletions,
                    base_url: profile_url,
                    model: "test-model".to_owned(),
                    context_window: 128_000,
                    output_limit: 16_384,
                    reasoning_effort: ReasoningEffort::ServerDefault,
                    use_tools: true,
                },
                credential: Some(Secret::new(PLANTED_CREDENTIAL.to_owned())),
            })
            .await
            .expect("the test profile is valid");
        Self {
            runtime,
            hub,
            executor,
            store,
            workers,
            model,
            vault,
            keys,
            profile,
            _directory: directory,
        }
    }

    /// Pairs with the worker at `address` with `code`.
    pub async fn pair(&self, address: &str, code: &str) -> Result<WorkerView, WorkersError> {
        self.hub
            .pair(PairWorker {
                address: address.to_owned(),
                code: code.to_owned(),
                name: None,
            })
            .await
    }

    /// Pairs with the worker at `address` with `code`, and gives it `name`.
    pub async fn pair_named(
        &self,
        address: &str,
        code: &str,
        name: &str,
    ) -> Result<WorkerView, WorkersError> {
        self.hub
            .pair(PairWorker {
                address: address.to_owned(),
                code: code.to_owned(),
                name: Some(name.to_owned()),
            })
            .await
    }

    /// The pin of the master's key, once pairing made it.
    ///
    /// # Panics
    ///
    /// When the master has no key, which fails the test.
    pub fn key_pin(&self) -> KeyPin {
        self.keys
            .loaded()
            .expect("the key store is available")
            .expect("the master has a key")
            .pin()
    }

    /// Waits until the first paired worker shows `status`.
    ///
    /// # Panics
    ///
    /// When it does not within 20 seconds, which fails the test.
    pub async fn wait_for_status(&self, status: WorkerStatus) -> WorkerView {
        let mut workers = self.hub.worker_views();
        let wait = async {
            let found = workers
                .wait_for(|views| views.first().is_some_and(|view| view.status == status))
                .await
                .expect("the hub runs");
            found.first().cloned().expect("a worker")
        };
        tokio::time::timeout(Duration::from_secs(20), wait)
            .await
            .expect("the worker shows the status within 20 seconds")
    }

    /// Starts a conversation whose first message is `text`.
    ///
    /// # Panics
    ///
    /// When the conversation cannot start, which fails the test.
    pub async fn start_conversation(&self, text: &str) -> ConversationId {
        self.runtime
            .start_conversation(StartConversation::new(text))
            .await
            .expect("the conversation starts")
            .conversation_id
    }

    /// The tasks of `conversation`, once `done` holds for them.
    ///
    /// # Panics
    ///
    /// When it does not within two minutes, which fails the test.
    pub async fn tasks_when(
        &self,
        conversation: &ConversationId,
        done: impl Fn(&[TaskView]) -> bool,
    ) -> Vec<TaskView> {
        let wait = async {
            loop {
                let snapshot = self
                    .store
                    .snapshot(conversation, 100, 10_000)
                    .await
                    .expect("the store reads")
                    .expect("the conversation exists");
                let tasks: Vec<TaskView> = snapshot
                    .turns
                    .iter()
                    .flat_map(|turn| &turn.parts)
                    .filter_map(|part| match &part.content {
                        PartContent::Task { call } => call.task.clone(),
                        _other => None,
                    })
                    .collect();
                if done(&tasks) {
                    return tasks;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(120), wait)
            .await
            .expect("the tasks reach the state within two minutes")
    }

    /// Waits until `conversation` has no active run.
    ///
    /// # Panics
    ///
    /// When it still has one after two minutes, which fails the test.
    pub async fn wait_for_idle(&self, conversation: &ConversationId) {
        let wait = async {
            loop {
                let summary = self
                    .store
                    .conversation_summary(conversation)
                    .await
                    .expect("the store reads")
                    .expect("the conversation exists");
                if !summary.has_active_run {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(120), wait)
            .await
            .expect("the run ends within two minutes");
    }

    /// Answers the call that `task` waits on with `choice`.
    ///
    /// # Panics
    ///
    /// When the task waits on nothing, which fails the test.
    pub async fn decide(&self, task: &TaskView, choice: ApprovalChoice) -> Result<(), ToolsError> {
        let approval = task.approval.clone().expect("the task waits on a call");
        let request = approval.call.request.expect("the call asks the owner");
        self.runtime
            .decide_approval(DecideApproval {
                conversation_id: task.conversation_id.clone(),
                part_id: approval.part_id,
                digest: request.digest,
                choice,
            })
            .await
    }

    /// Allows once the call that `task` waits on.
    ///
    /// # Panics
    ///
    /// When the task waits on nothing, which fails the test.
    pub async fn allow_once(&self, task: &TaskView) -> Result<(), ToolsError> {
        self.decide(task, ApprovalChoice::AllowOnce).await
    }
}

/// A worker with `options`, and a master paired with it once it shows as
/// connected.
///
/// # Panics
///
/// When they cannot pair, which fails the test.
pub async fn paired(options: LinkedWorkerOptions) -> (LinkedWorker, LinkedMaster) {
    let worker = LinkedWorker::with_options(options).await;
    let master = LinkedMaster::start().await;
    master
        .pair(&worker.address, &worker.new_code())
        .await
        .expect("the worker pairs");
    master.wait_for_status(WorkerStatus::Connected).await;
    (worker, master)
}

/// The arguments of `delegate_task`: a task in the foreground for the test
/// worker, `build-01`.
pub fn delegation(instruction: &str) -> String {
    delegation_to("build-01", instruction)
}

/// The arguments of `delegate_task`: a task in the foreground for `worker`.
pub fn delegation_to(worker: &str, instruction: &str) -> String {
    serde_json::json!({
        "activity": "Delegating",
        "worker": worker,
        "title": "Build the backend",
        "instruction": instruction,
        "background": false,
        "read_only": false,
    })
    .to_string()
}

/// The credential of the master's profile, which the tests look for where
/// it must never be.
pub const PLANTED_CREDENTIAL: &str = "sk-planted-lease-7f3a9c";

/// Whether a file under `folder` contains `text`.
pub fn files_contain(folder: &Path, text: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        if path.is_dir() {
            files_contain(&path, text)
        } else {
            std::fs::read(&path).is_ok_and(|bytes| {
                bytes
                    .windows(text.len())
                    .any(|window| window == text.as_bytes())
            })
        }
    })
}

/// Calls `Tasks.Get` on the worker at `address` with `key`, expecting the
/// worker's pinned key `pin`, and returns the status of the call: what a
/// master that is not the pinned one gets past pairing.
///
/// # Panics
///
/// When the TLS configuration cannot be built, which fails the test.
pub async fn get_with_key(key: &LinkKey, address: &str, pin: KeyPin) -> tonic::Code {
    let config = tls::client_config(key, Some(pin)).expect("the TLS configuration");
    let Some((channel, _pin)) = mannie_workers::hub::test_channel(address, config).await else {
        return tonic::Code::Unavailable;
    };
    let mut client = TasksServiceClient::new(channel);
    match client
        .get(GetRequest {
            task_id: "0199e0f2-0000-7000-a000-000000000001".to_owned(),
        })
        .await
    {
        Ok(_response) => tonic::Code::Ok,
        Err(status) => status.code(),
    }
}
