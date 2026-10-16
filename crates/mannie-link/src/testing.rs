//! Test tools for the link: a worker's runtime served over the link on
//! loopback, a master's runtime with a hub, the master's key in memory, a
//! relay between them that can cut every connection, lose the worker's
//! answers or sit in the middle with a key of its own, and the steps that
//! the link's tests share (docs/features/workers/pairing-and-link.md,
//! acceptance).
//!
//! Only the `testing` feature compiles this module. The desktop app and the
//! worker never enable it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use mannie_logging::Redactor;
use mannie_runtime::testing::{
    FakeModel, MemoryVault, WorkerHarness, WorkerOptions, fast_config, open_store, start_master,
    test_tools,
};
use mannie_runtime::{AgentRegistry, Delegation, Runtime, RuntimeConfig};
use mannie_store::Store;
use mannie_types::chat::StartConversation;
use mannie_types::clock::{Clock, SystemClock};
use mannie_types::ids::ConversationId;
use mannie_types::provider::{
    ApiInterface, CreateProviderProfile, ProfileDraft, ProviderProfile, ReasoningEffort,
};
use mannie_types::secret::Secret;
use mannie_types::tasks::TaskView;
use mannie_types::tools::{ApprovalChoice, DecideApproval, ToolsError};
use mannie_types::transcript::PartContent;
use mannie_types::workers::{PairWorker, WorkerDetails, WorkerStatus, WorkerView, WorkersError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use crate::keys::LinkKey;
use crate::master::{Hub, HubConfig, KeyStoreError, MasterKeyStore};
use crate::worker::{self, LeaseVault, WorkerLink};
use crate::{levels, tls};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
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

impl MasterKeyStore for MemoryKeyStore {
    fn load(&self) -> Result<Option<LinkKey>, KeyStoreError> {
        if *lock(&self.unavailable) {
            return Err(KeyStoreError::Unavailable);
        }
        lock(&self.key)
            .clone()
            .map(|pkcs8| LinkKey::from_pkcs8(pkcs8).map_err(|_invalid| KeyStoreError::Invalid))
            .transpose()
    }

    fn save(&self, key: &LinkKey) -> Result<(), KeyStoreError> {
        if *lock(&self.unavailable) {
            return Err(KeyStoreError::Unavailable);
        }
        *lock(&self.key) = Some(key.pkcs8().to_vec());
        Ok(())
    }
}

/// How a linked worker is set up.
#[derive(Debug, Clone)]
pub struct LinkedWorkerOptions {
    /// The options of the worker's runtime.
    pub worker: WorkerOptions,
    /// The version that the worker says it runs.
    pub version: String,
}

impl Default for LinkedWorkerOptions {
    fn default() -> Self {
        Self {
            worker: WorkerOptions::default(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}

/// A worker's runtime served over the link on a loopback port.
#[derive(Debug)]
pub struct LinkedWorker {
    /// The worker's runtime and its model.
    pub harness: WorkerHarness,
    /// The address the worker listens on.
    pub address: String,
    /// The worker's leases.
    pub leases: Arc<LeaseVault>,
    /// The redactor that hides the leases.
    pub redactor: Arc<Redactor>,
    /// The worker's key.
    pub key: Arc<LinkKey>,
    shutdown: Option<oneshot::Sender<()>>,
    server: Option<JoinHandle<()>>,
}

impl LinkedWorker {
    /// A worker with the default options.
    pub async fn start() -> Self {
        Self::with_options(LinkedWorkerOptions::default()).await
    }

    /// A worker with `options`, served at once.
    ///
    /// # Panics
    ///
    /// When the worker cannot be set up, which fails the test.
    pub async fn with_options(options: LinkedWorkerOptions) -> Self {
        let redactor = Arc::new(Redactor::new());
        let leases = Arc::new(LeaseVault::new(Arc::clone(&redactor)));
        let vault: Arc<dyn mannie_runtime::CredentialVault> = Arc::<LeaseVault>::clone(&leases);
        let harness = WorkerHarness::with_options(WorkerOptions {
            vault: Some(vault),
            ..options.worker
        })
        .await;
        let key = Arc::new(LinkKey::generate().expect("a key"));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port");
        let address = listener.local_addr().expect("an address").to_string();
        let link = WorkerLink {
            runtime: harness.runtime.clone(),
            key: LinkKey::from_pkcs8(key.pkcs8().to_vec()).expect("the same key"),
            name: harness.host_name.clone(),
            details: WorkerDetails {
                purpose: "Builds and tests the backend".to_owned(),
                facts: None,
                ceiling: None,
                version: Some(options.version.clone()),
                max_parallel_tasks: 4,
            },
            version: options.version,
            data: harness.data_folder(),
            leases: Arc::clone(&leases),
            clock: Arc::new(SystemClock),
        };
        let (shutdown, stopped) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let stopping = async {
                let _stopped = stopped.await;
            };
            if let Err(error) = worker::serve(link, listener, stopping).await {
                tracing::error!(%error, "the test worker's link failed");
            }
        });
        Self {
            harness,
            address,
            leases,
            redactor,
            key,
            shutdown: Some(shutdown),
            server: Some(server),
        }
    }

    /// The data folder, which holds the master's pin and the pairing code.
    pub fn data(&self) -> PathBuf {
        self.harness.data_folder()
    }

    /// A new pairing code, as `mannie-worker pair` writes it.
    ///
    /// # Panics
    ///
    /// When the code cannot be written, which fails the test.
    pub fn new_code(&self) -> String {
        worker::write_code(&self.data(), SystemClock.now())
            .expect("the code is written")
            .display()
    }

    /// Stops serving the link.
    pub async fn stop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _sent = shutdown.send(());
        }
        if let Some(server) = self.server.take() {
            let _ended = server.await;
        }
    }
}

/// A master's runtime with its hub on a temporary store.
#[derive(Debug)]
pub struct LinkedMaster {
    /// The master's runtime, which delegates through the hub.
    pub runtime: Runtime,
    /// The hub.
    pub hub: Hub,
    /// The master's store.
    pub store: Store,
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
        let store = open_store(&directory.path().join("mannie.sqlite"));
        let keys = Arc::new(MemoryKeyStore::default());
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let hub = Hub::start(
            store.clone(),
            Arc::<MemoryKeyStore>::clone(&keys),
            HubConfig {
                version,
                max_retry_delay: TEST_MAX_RETRY_DELAY,
            },
            clock,
        )
        .await
        .expect("the hub starts");
        let model = FakeModel::new(ApiInterface::ChatCompletions);
        let vault = Arc::new(MemoryVault::new());
        let port: Arc<dyn Delegation> = Arc::new(hub.clone());
        let runtime = start_master(
            store.clone(),
            Arc::new(model.clone()),
            Arc::clone(&vault),
            config,
            test_tools(directory.path()),
            AgentRegistry::with_builtin(),
            Some(port),
        )
        .await;
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
            store,
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
    pub fn key_pin(&self) -> crate::keys::KeyPin {
        self.keys
            .load()
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
            .start_conversation(StartConversation {
                text: text.to_owned(),
                profile_id: None,
                project_id: None,
            })
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

/// A relay between the master and a worker: it copies the bytes of each
/// connection, can cut every connection, can refuse new ones while blocked,
/// can lose what the worker sends, and can send new connections to another
/// address.
#[derive(Debug)]
pub struct Relay {
    /// The address the master connects to.
    pub address: String,
    /// Cuts the current connections when it changes.
    cut: watch::Sender<u64>,
    blocked: Arc<Mutex<bool>>,
    answers_lost: Arc<AtomicBool>,
    target: Arc<Mutex<String>>,
    task: JoinHandle<()>,
}

/// Copies one connection both ways until either side ends it. While
/// `answers_lost` is set, what the worker sends is read and dropped, as when
/// a link goes down after a call reached the worker and before its answer
/// reached the master.
async fn relay_connection(inbound: TcpStream, outbound: TcpStream, answers_lost: Arc<AtomicBool>) {
    let (mut from_master, mut to_master) = inbound.into_split();
    let (mut from_worker, mut to_worker) = outbound.into_split();
    let requests = tokio::io::copy(&mut from_master, &mut to_worker);
    let answers = async {
        let mut buffer = vec![0_u8; 16 * 1024];
        loop {
            let read = from_worker.read(&mut buffer).await?;
            if read == 0 {
                return Ok::<(), std::io::Error>(());
            }
            if !answers_lost.load(Ordering::SeqCst) {
                to_master.write_all(&buffer[..read]).await?;
            }
        }
    };
    tokio::select! {
        _copied = requests => {}
        _copied = answers => {}
    }
}

impl Relay {
    /// A relay to `target`.
    ///
    /// # Panics
    ///
    /// When it cannot listen, which fails the test.
    pub async fn start(target: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port");
        let address = listener.local_addr().expect("an address").to_string();
        let cut = watch::Sender::new(0);
        let blocked = Arc::new(Mutex::new(false));
        let answers_lost = Arc::new(AtomicBool::new(false));
        let target = Arc::new(Mutex::new(target.to_owned()));
        let task = {
            let cut = cut.clone();
            let blocked = Arc::clone(&blocked);
            let answers_lost = Arc::clone(&answers_lost);
            let target = Arc::clone(&target);
            tokio::spawn(async move {
                while let Ok((inbound, _address)) = listener.accept().await {
                    if *lock(&blocked) {
                        drop(inbound);
                        continue;
                    }
                    let target = lock(&target).clone();
                    let mut cut = cut.subscribe();
                    let answers_lost = Arc::clone(&answers_lost);
                    tokio::spawn(async move {
                        let Ok(outbound) = TcpStream::connect(&target).await else {
                            return;
                        };
                        tokio::select! {
                            () = relay_connection(inbound, outbound, answers_lost) => {}
                            _cut = cut.changed() => {}
                        }
                    });
                }
            })
        };
        Self {
            address,
            cut,
            blocked,
            answers_lost,
            target,
            task,
        }
    }

    /// Loses what the worker sends on every connection from now on, or
    /// stops losing it.
    pub fn set_answers_lost(&self, lost: bool) {
        self.answers_lost.store(lost, Ordering::SeqCst);
    }

    /// Ends every connection and lets new ones through as they come, so the
    /// master connects again: the link drops and comes back.
    pub fn drop_link(&self) {
        self.set_answers_lost(false);
        self.cut();
    }

    /// Sends new connections to `target`.
    pub fn set_target(&self, target: &str) {
        target.clone_into(&mut lock(&self.target));
    }

    /// Cuts every current connection.
    pub fn cut(&self) {
        self.cut.send_modify(|count| *count += 1);
    }

    /// Refuses new connections, or lets them through again.
    pub fn set_blocked(&self, blocked: bool) {
        *lock(&self.blocked) = blocked;
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A relay in the middle with a key of its own: it ends the master's TLS
/// with its key and opens its own TLS to the worker with the same key, then
/// copies the plaintext both ways, as an attacker in the middle of a pairing
/// would.
#[derive(Debug)]
pub struct KeyInTheMiddle {
    /// The address the master connects to.
    pub address: String,
    task: JoinHandle<()>,
}

impl KeyInTheMiddle {
    /// A relay to `target` with a new key.
    ///
    /// # Panics
    ///
    /// When it cannot listen, which fails the test.
    pub async fn start(target: &str) -> Self {
        let key = LinkKey::generate().expect("a key");
        let server = tls::server_config(&key).expect("the relay's TLS");
        let client = tls::client_config(&key, None).expect("the relay's TLS");
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port");
        let address = listener.local_addr().expect("an address").to_string();
        let target = target.to_owned();
        let task = tokio::spawn(async move {
            let acceptor = tokio_rustls::TlsAcceptor::from(server);
            while let Ok((inbound, _address)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let client = Arc::clone(&client);
                let target = target.clone();
                tokio::spawn(async move {
                    let Ok(mut inbound) = acceptor.accept(inbound).await else {
                        return;
                    };
                    let Ok((mut outbound, _pin)) = tls::connect(&target, client).await else {
                        return;
                    };
                    let _copied = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                });
            }
        });
        Self { address, task }
    }
}

impl Drop for KeyInTheMiddle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// This version's levels, for tests that simulate another version.
pub fn current_levels() -> std::collections::HashMap<String, u32> {
    levels::levels()
}

/// Calls `Tasks.Get` on the worker at `address` with `key`, expecting the
/// worker's pinned key `pin`, and returns the status of the call: what a
/// master that is not the pinned one gets past pairing.
///
/// # Panics
///
/// When the TLS configuration cannot be built, which fails the test.
pub async fn get_with_key(key: &LinkKey, address: &str, pin: crate::keys::KeyPin) -> tonic::Code {
    let config = tls::client_config(key, Some(pin)).expect("the TLS configuration");
    let Ok((channel, _pin)) = crate::master::test_channel(address, config).await else {
        return tonic::Code::Unavailable;
    };
    let mut client = crate::proto::tasks::tasks_service_client::TasksServiceClient::new(channel);
    match client
        .get(crate::proto::tasks::GetRequest {
            task_id: "0199e0f2-0000-7000-a000-000000000001".to_owned(),
        })
        .await
    {
        Ok(_response) => tonic::Code::Ok,
        Err(status) => status.code(),
    }
}
