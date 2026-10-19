//! Test tools: a credential vault in memory, a fake model that plays scripted
//! replies and writes summaries on its own, a runtime on a temporary store, a
//! worker's runtime with its host and ceiling, a port that joins a master and
//! a worker in one process with faults to inject, plugins that start and
//! stop as the plugin host makes them, the services that the runtime binds
//! to a plugin, a fake executor, and a transcript that a subscriber builds
//! from updates (docs/architecture/agent-runtime.md, testing).
//!
//! Only the `testing` feature compiles this module. The desktop app and the
//! worker never enable it.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::{FutureExt, StreamExt};
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::host::{ContributionRefusal, add_contributions, remove_contributions};
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::points::executors::{
    ExecutorRefusal, ExecutorSection, FollowUpPlan, PlanCall, PlanError, SubmitError, TaskExecutor,
    TaskPlan, TaskSubmission,
};
use mannie_plugin_sdk::points::tools::ToolSpec;
use mannie_plugin_sdk::services::credentials::CredentialsApi;
use mannie_plugin_sdk::services::executor_host::ExecutorHostApi;
use mannie_provider::testing::Step;
use mannie_provider::{
    Content, EventStream, HttpClient, ModelRequest, Provider, ProviderError, ProviderEvent,
    ProviderRegistry, Target,
};
use mannie_store::{PluginModule, Store, StoreOptions};
use mannie_tools::{Ceiling, MutationLock, SystemFacts, ToolRegistry};
use mannie_types::ceiling::CeilingMode;
use mannie_types::chat::{
    ChatError, CheckpointRef, EarlierTurns, LoadEarlierTurns, SubscribeConversation,
};
use mannie_types::clock::{Clock, SystemClock};
use mannie_types::ids::{ConversationId, PartId, ProfileId, RunId, TaskId, TurnId};
use mannie_types::provider::{
    ApiInterface, CreateProviderProfile, CredentialStoreProblem, ProfileDraft, ProviderProfile,
    ReasoningEffort,
};
use mannie_types::run::{FinishReason, RunFailure, RunState, RunView, Usage};
use mannie_types::secret::Secret;
use mannie_types::tasks::{
    AssignedTaskState, ProfileSnapshot, TaskAssignment, TaskCallView, TaskExecutorRef, TaskState,
    TaskView,
};
use mannie_types::tools::{
    CommandRule, DecideApproval, ProcessRecord, ReadToolOutput, ToolCallView, ToolOutputPage,
    ToolsError,
};
use mannie_types::transcript::{
    Checkpoint, ConversationEvent, ConversationUpdate, PartContent, PartState, TurnView,
};
use tokio::sync::watch;

use crate::{
    AgentDefinition, AgentRegistry, CredentialVault, FileMentions, GENERAL, InstructionRegistry,
    InstructionSection, MachineSettings, MentionRegistry, Requester, RetryPolicy, Runtime,
    RuntimeConfig, RuntimeDeps, ToolsDeps, VaultError, checkpoint_section, texts,
};

/// How long a helper waits for a state before it fails the test. Shared CI
/// runners run the tests several times slower than a workstation, and a
/// command there can take seconds to start, so the limit only catches a
/// hang, as the helpers of mannie-link do.
const WAIT_LIMIT: Duration = Duration::from_secs(120);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Default)]
struct VaultState {
    entries: BTreeMap<String, Secret>,
    failing_saves: Option<CredentialStoreProblem>,
    failing_reads: Option<CredentialStoreProblem>,
    failing_deletes: Option<CredentialStoreProblem>,
}

/// A credential vault in memory, whose calls can be made to fail. One made
/// with [`MemoryVault::with_arrivals`] tells waiting runs when a credential
/// arrives, as a worker's lease vault does.
#[derive(Debug)]
pub struct MemoryVault {
    state: Mutex<VaultState>,
    /// Counts the credentials saved, for the waits of runs.
    saved: watch::Sender<u64>,
    announces_arrivals: bool,
}

impl Default for MemoryVault {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            saved: watch::Sender::new(0),
            announces_arrivals: false,
        }
    }
}

impl MemoryVault {
    /// An empty vault.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty vault that tells waiting runs when a credential arrives.
    pub fn with_arrivals() -> Self {
        Self {
            announces_arrivals: true,
            ..Self::default()
        }
    }

    /// The references of every stored credential, in order.
    pub fn references(&self) -> Vec<String> {
        lock(&self.state).entries.keys().cloned().collect()
    }

    /// The credential stored under `reference`.
    pub fn credential(&self, reference: &str) -> Option<String> {
        lock(&self.state)
            .entries
            .get(reference)
            .map(|secret| secret.expose().to_owned())
    }

    /// Stores a credential without going through a profile.
    pub fn insert(&self, reference: &str, credential: &str) {
        lock(&self.state)
            .entries
            .insert(reference.to_owned(), Secret::new(credential.to_owned()));
        self.saved.send_modify(|count| *count += 1);
    }

    /// Removes a credential without going through a profile.
    pub fn remove(&self, reference: &str) {
        lock(&self.state).entries.remove(reference);
    }

    /// Makes saves fail with `problem`, or succeed again with `None`.
    pub fn fail_saves(&self, problem: Option<CredentialStoreProblem>) {
        lock(&self.state).failing_saves = problem;
    }

    /// Makes reads fail with `problem`, or succeed again with `None`.
    pub fn fail_reads(&self, problem: Option<CredentialStoreProblem>) {
        lock(&self.state).failing_reads = problem;
    }

    /// Makes deletes fail with `problem`, or succeed again with `None`.
    pub fn fail_deletes(&self, problem: Option<CredentialStoreProblem>) {
        lock(&self.state).failing_deletes = problem;
    }
}

impl CredentialVault for MemoryVault {
    fn save(&self, reference: &str, credential: &Secret) -> Result<(), VaultError> {
        let mut state = lock(&self.state);
        if let Some(problem) = state.failing_saves {
            return Err(VaultError::Unavailable(problem));
        }
        state
            .entries
            .insert(reference.to_owned(), credential.clone());
        drop(state);
        self.saved.send_modify(|count| *count += 1);
        Ok(())
    }

    fn read(&self, reference: &str) -> Result<Secret, VaultError> {
        let state = lock(&self.state);
        if let Some(problem) = state.failing_reads {
            return Err(VaultError::Unavailable(problem));
        }
        state
            .entries
            .get(reference)
            .cloned()
            .ok_or(VaultError::Missing)
    }

    fn delete(&self, reference: &str) -> Result<(), VaultError> {
        let mut state = lock(&self.state);
        if let Some(problem) = state.failing_deletes {
            return Err(VaultError::Unavailable(problem));
        }
        state
            .entries
            .remove(reference)
            .map(|_removed| ())
            .ok_or(VaultError::Missing)
    }

    fn arrival(&self, _reference: &str) -> Option<BoxFuture<'static, ()>> {
        if !self.announces_arrivals {
            return None;
        }
        // The receiver counts what was saved so far, so a save after this
        // call ends the wait, even one before the wait is polled.
        let mut saved = self.saved.subscribe();
        Some(Box::pin(async move {
            let _changed = saved.changed().await;
        }))
    }
}

#[derive(Debug, Default)]
struct ModelState {
    context_window: Option<u32>,
    full_summaries: bool,
    replies: VecDeque<Vec<Step>>,
    /// Scripts for the reply calls whose first message holds a marker, in
    /// order for each marker.
    routed: Vec<(String, VecDeque<Vec<Step>>)>,
    summaries: VecDeque<Vec<Step>>,
    requests: Vec<ModelRequest>,
    summaries_written: u32,
    credentials: Vec<Option<String>>,
}

/// A model that plays scripted replies in order and records every request.
/// It writes summaries on its own when a compaction asks for one, unless a
/// summary was scripted.
#[derive(Debug, Clone)]
pub struct FakeModel {
    interface: ApiInterface,
    state: Arc<Mutex<ModelState>>,
}

impl FakeModel {
    /// A model behind `interface` without scripted replies.
    pub fn new(interface: ApiInterface) -> Self {
        Self {
            interface,
            state: Arc::default(),
        }
    }

    /// Makes calls whose estimated input and output exceed `window` fail
    /// with a context overflow, as a server with that window would.
    pub fn enforce_context_window(&self, window: u32) {
        lock(&self.state).context_window = Some(window);
    }

    /// Makes the summaries it writes on its own as long as their output
    /// limit allows, as a verbose model's would be.
    pub fn write_full_summaries(&self) {
        lock(&self.state).full_summaries = true;
    }

    /// Adds the script of the next reply call.
    pub fn push_reply(&self, steps: Vec<Step>) {
        lock(&self.state).replies.push_back(steps);
    }

    /// Adds the script of the next reply call whose first message contains
    /// `marker`, such as a child agent's instruction. A call whose first
    /// message holds no marker with a script left plays the next script of
    /// [`Self::push_reply`].
    pub fn push_reply_for(&self, marker: &str, steps: Vec<Step>) {
        let mut state = lock(&self.state);
        if let Some((_marker, scripts)) = state
            .routed
            .iter_mut()
            .find(|(known, _scripts)| known == marker)
        {
            scripts.push_back(steps);
        } else {
            state
                .routed
                .push((marker.to_owned(), VecDeque::from([steps])));
        }
    }

    /// The requests of reply calls whose first message contains `marker`.
    pub fn requests_for(&self, marker: &str) -> Vec<ModelRequest> {
        self.reply_requests()
            .into_iter()
            .filter(|request| first_message(request).contains(marker))
            .collect()
    }

    /// Adds the script of the next summary call.
    pub fn push_summary(&self, steps: Vec<Step>) {
        lock(&self.state).summaries.push_back(steps);
    }

    /// Every request so far, in order.
    pub fn requests(&self) -> Vec<ModelRequest> {
        lock(&self.state).requests.clone()
    }

    /// The requests of replies, without the summary calls.
    pub fn reply_requests(&self) -> Vec<ModelRequest> {
        self.requests()
            .into_iter()
            .filter(|request| !is_summary(request))
            .collect()
    }

    /// The requests of summary calls.
    pub fn summary_requests(&self) -> Vec<ModelRequest> {
        self.requests().into_iter().filter(is_summary).collect()
    }

    /// The credential each call carried, in order.
    pub fn credentials(&self) -> Vec<Option<String>> {
        lock(&self.state).credentials.clone()
    }

    /// How many scripted replies have not been played.
    pub fn remaining_replies(&self) -> usize {
        lock(&self.state).replies.len()
    }
}

fn is_summary(request: &ModelRequest) -> bool {
    request.instructions == texts::COMPACTION_INSTRUCTIONS
}

/// The text of a request's first message: the owner's first message, or a
/// child agent's instruction.
fn first_message(request: &ModelRequest) -> String {
    request
        .messages
        .first()
        .map(|message| {
            message
                .content
                .iter()
                .filter_map(|content| match content {
                    Content::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Mannie's estimate of a request's input, in tokens.
pub fn estimate(request: &ModelRequest) -> u32 {
    crate::budget::estimate_request(request)
}

/// The steps of a reply that writes `text` and finishes.
pub fn reply(text: &str) -> Vec<Step> {
    vec![
        Step::Event(ProviderEvent::TextDelta(text.to_owned())),
        Step::Event(ProviderEvent::Finished(FinishReason::Stop)),
    ]
}

/// The steps of a reply that writes `text`, reports usage and finishes.
pub fn reply_with_usage(text: &str, input: u32, output: u32) -> Vec<Step> {
    vec![
        Step::Event(ProviderEvent::TextDelta(text.to_owned())),
        Step::Event(ProviderEvent::Usage(Usage {
            input,
            output,
            cached_input: None,
            reasoning: None,
        })),
        Step::Event(ProviderEvent::Finished(FinishReason::Stop)),
    ]
}

/// The steps of a call that fails at once.
pub fn failure(error: ProviderError) -> Vec<Step> {
    vec![Step::Error(error)]
}

/// The steps of a model call that calls tools: each `(call ID, tool,
/// arguments)` in order, then the finish that servers send after tool calls.
pub fn tool_calls(calls: &[(&str, &str, &str)]) -> Vec<Step> {
    let mut steps = Vec::new();
    for (call_id, name, arguments) in calls {
        steps.push(Step::Event(ProviderEvent::ToolCallStarted {
            call_id: (*call_id).to_owned(),
            name: (*name).to_owned(),
        }));
        steps.push(Step::Event(ProviderEvent::ToolCall {
            call_id: (*call_id).to_owned(),
            name: (*name).to_owned(),
            arguments: (*arguments).to_owned(),
        }));
    }
    steps.push(Step::Event(ProviderEvent::Finished(FinishReason::Other)));
    steps
}

/// The system tools for a test runtime, with `protected` as Mannie's data
/// folder.
///
/// # Panics
///
/// When the folder cannot be resolved, which fails the test.
pub fn test_tools(protected: &Path) -> ToolsDeps {
    let protected = mannie_tools::canonical_folder(protected).expect("the data folder exists");
    ToolsDeps {
        registry: ToolRegistry::with_system_tools(),
        protected: vec![PathBuf::from(protected)],
        mutations: MutationLock::new(),
        system: Arc::new(SystemFacts::detect()),
    }
}

/// A transient server error.
pub fn server_error() -> ProviderError {
    ProviderError::Transient {
        cause: RunFailure::ServerError { status: 503 },
        retry_after: None,
    }
}

impl Provider for FakeModel {
    fn interface(&self) -> ApiInterface {
        self.interface
    }

    fn stream(&self, target: &Target, request: ModelRequest) -> EventStream {
        let summary = is_summary(&request);
        let needed = estimate(&request).saturating_add(request.max_output_tokens);
        let max_output_tokens = request.max_output_tokens;
        let first = first_message(&request);
        let steps = {
            let mut state = lock(&self.state);
            let overflows = state.context_window.is_some_and(|window| needed > window);
            state.requests.push(request);
            state.credentials.push(
                target
                    .credential
                    .as_ref()
                    .map(|credential| credential.expose().to_owned()),
            );
            if overflows {
                failure(ProviderError::ContextOverflow)
            } else if summary {
                let scripted = state.summaries.pop_front();
                scripted.unwrap_or_else(|| {
                    state.summaries_written += 1;
                    let mut text = format!("Summary {}.", state.summaries_written);
                    if state.full_summaries {
                        // The estimate counts two tokens for every five bytes.
                        let bytes =
                            usize::try_from(max_output_tokens).unwrap_or(usize::MAX) * 5 / 2;
                        let padding = bytes.saturating_sub(text.len() + 1);
                        text.push(' ');
                        text.push_str(&"s".repeat(padding));
                    }
                    reply(&text)
                })
            } else {
                let routed = state
                    .routed
                    .iter_mut()
                    .find(|(marker, scripts)| {
                        !scripts.is_empty() && first.contains(marker.as_str())
                    })
                    .and_then(|(_marker, scripts)| scripts.pop_front());
                // A reply without a script fails, so that a test that makes
                // more calls than it expects notices.
                routed
                    .or_else(|| state.replies.pop_front())
                    .unwrap_or_else(|| failure(ProviderError::Permanent(RunFailure::Internal)))
            }
        };
        Box::pin(futures_util::stream::unfold(
            VecDeque::from(steps),
            |mut steps| async move {
                loop {
                    match steps.pop_front()? {
                        Step::Event(event) => return Some((Ok(event), steps)),
                        Step::Error(error) => {
                            steps.clear();
                            return Some((Err(error), steps));
                        }
                        Step::Delay(duration) => tokio::time::sleep(duration).await,
                        Step::Hang => std::future::pending::<()>().await,
                    }
                }
            },
        ))
    }
}

/// How a test runtime is set up.
#[derive(Debug, Clone)]
pub struct HarnessOptions {
    /// The interface of the profile and the fake model.
    pub interface: ApiInterface,
    /// The profile's context window.
    pub context_window: u32,
    /// The profile's output limit.
    pub output_limit: u32,
    /// The profile's credential, if any.
    pub credential: Option<String>,
    /// Whether the fake model refuses requests beyond the context window.
    pub enforce_context_window: bool,
    /// Whether the fake model's summaries use their whole output limit.
    pub full_summaries: bool,
    /// Whether the profile uses tools. Off by default, so the tests of chat
    /// see what a profile without tools sends, which is what M1 sent.
    pub use_tools: bool,
    /// The agent definitions, such as a conversation agent with smaller
    /// budgets.
    pub agents: AgentRegistry,
    /// The runtime's configuration.
    pub config: RuntimeConfig,
    /// The plugins' modules of the store, which the store checks, migrates
    /// and registers as the composition root's (ADR-0030, decision 5;
    /// ADR-0034, decision 7).
    pub store_modules: Vec<PluginModule>,
}

impl Default for HarnessOptions {
    fn default() -> Self {
        Self {
            interface: ApiInterface::ChatCompletions,
            context_window: 128_000,
            output_limit: 16_384,
            credential: Some("sk-test-credential".to_owned()),
            enforce_context_window: false,
            full_summaries: false,
            use_tools: false,
            agents: AgentRegistry::with_builtin(),
            config: fast_config(),
            store_modules: Vec::new(),
        }
    }
}

/// A configuration with short delays, so that tests of retries and flushes
/// run in milliseconds.
pub fn fast_config() -> RuntimeConfig {
    RuntimeConfig {
        retry: RetryPolicy {
            max_attempts: 6,
            base: Duration::from_millis(2),
            cap: Duration::from_millis(10),
            max_retry_after: Duration::from_secs(600),
        },
        flush_interval: Duration::from_millis(5),
        random_seed: Some(7),
        ..RuntimeConfig::default()
    }
}

/// A runtime on a temporary store, with a fake model and a profile.
#[derive(Debug)]
pub struct Harness {
    /// The runtime.
    pub runtime: Runtime,
    /// Its store.
    pub store: Store,
    /// The model behind the profile.
    pub model: FakeModel,
    /// The credential vault.
    pub vault: Arc<MemoryVault>,
    /// The profile, which is the default.
    pub profile: ProviderProfile,
    options: HarnessOptions,
    directory: tempfile::TempDir,
}

/// Opens the store of a harness directory.
///
/// # Panics
///
/// When the store cannot be opened, which fails the test.
pub fn open_store(database: &Path) -> Store {
    open_store_with(database, Vec::new())
}

/// Opens the store of a harness directory with these plugins' modules
/// besides the core's.
///
/// # Panics
///
/// When the store cannot be opened, which fails the test.
pub fn open_store_with(database: &Path, modules: Vec<PluginModule>) -> Store {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let options =
        StoreOptions::new(database.with_file_name("backups"), clock).with_plugin_modules(modules);
    Store::open(database, &options).expect("the test store opens")
}

/// Waits up to five seconds until the shell of a command that outlived a
/// killed Mannie has ended. On Linux and FreeBSD such a command runs on until
/// the recovery ends its tree, in the background, at the next start
/// (docs/features/system-tools/commands.md, crash recovery), so a crash test
/// counts what the command did only once its shell is gone. On Windows the
/// command's job ended with Mannie.
#[cfg_attr(
    not(any(target_os = "linux", target_os = "freebsd")),
    expect(
        clippy::unused_async,
        reason = "the command's job ended with Mannie, so there is nothing to wait for"
    )
)]
pub async fn wait_for_leftover_command(process: Option<&ProcessRecord>) {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    if let Some(process) = process {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while process_exists(process.pid) && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    let _ = process;
}

/// Whether a process exists, as a zombie too.
#[cfg(target_os = "linux")]
fn process_exists(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// Whether a process exists, as a zombie too. FreeBSD mounts no `/proc` by
/// default, so `ps` answers.
#[cfg(target_os = "freebsd")]
fn process_exists(pid: u32) -> bool {
    std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "pid="])
        .output()
        .is_ok_and(|output| !String::from_utf8_lossy(&output.stdout).trim().is_empty())
}

/// Starts a runtime on `store` with `provider` behind every profile of its
/// interface, and the system tools without protected folders.
///
/// # Panics
///
/// When the runtime cannot start, which fails the test.
pub async fn start_runtime(
    store: Store,
    provider: Arc<dyn Provider>,
    vault: Arc<MemoryVault>,
    config: RuntimeConfig,
) -> Runtime {
    let tools = ToolsDeps {
        registry: ToolRegistry::with_system_tools(),
        protected: Vec::new(),
        mutations: MutationLock::new(),
        system: Arc::new(SystemFacts::detect()),
    };
    start_runtime_with_tools(
        store,
        provider,
        vault,
        config,
        tools,
        AgentRegistry::with_builtin(),
    )
    .await
}

/// Starts a runtime as [`start_runtime`] does, with these tools and agents.
///
/// # Panics
///
/// When the runtime cannot start, which fails the test.
pub async fn start_runtime_with_tools(
    store: Store,
    provider: Arc<dyn Provider>,
    vault: Arc<MemoryVault>,
    config: RuntimeConfig,
    tools: ToolsDeps,
    agents: AgentRegistry,
) -> Runtime {
    start_master(store, provider, vault, config, tools, agents).await
}

/// The core's mention providers, as the desktop app registers them.
///
/// # Panics
///
/// Never: the registry starts empty, so the ID is free.
pub fn core_mentions() -> MentionRegistry {
    let mut mentions = MentionRegistry::new();
    mentions
        .register(Arc::new(FileMentions::new()))
        .expect("the registry starts empty");
    mentions
}

/// Starts the runtime of the machine that the owner works on, as
/// [`start_runtime_with_tools`] does. A test adds executors to it afterwards.
///
/// # Panics
///
/// When the runtime cannot start, which fails the test.
pub async fn start_master(
    store: Store,
    provider: Arc<dyn Provider>,
    vault: Arc<MemoryVault>,
    config: RuntimeConfig,
    tools: ToolsDeps,
    agents: AgentRegistry,
) -> Runtime {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let client = HttpClient::new(Arc::clone(&clock)).expect("the HTTP client builds");
    let mut registry = ProviderRegistry::new();
    registry
        .register(provider)
        .expect("one adapter per interface");
    let deps = RuntimeDeps {
        store,
        client,
        registry,
        vault,
        clock,
        agents,
        instructions: InstructionRegistry::with_builtin(),
        tools,
        machine: MachineSettings::default(),
        mentions: core_mentions(),
    };
    Runtime::start(deps, config)
        .await
        .expect("the runtime starts")
}

impl Harness {
    /// A runtime with the default options.
    pub async fn start() -> Self {
        Self::with_options(HarnessOptions::default()).await
    }

    /// A runtime with `options`.
    ///
    /// # Panics
    ///
    /// When the runtime or its profile cannot be set up, which fails the
    /// test.
    pub async fn with_options(options: HarnessOptions) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let store = open_store_with(
            &directory.path().join("mannie.sqlite"),
            options.store_modules.clone(),
        );
        let model = FakeModel::new(options.interface);
        if options.enforce_context_window {
            model.enforce_context_window(options.context_window);
        }
        if options.full_summaries {
            model.write_full_summaries();
        }
        let vault = Arc::new(MemoryVault::new());
        let runtime = start_master(
            store.clone(),
            Arc::new(model.clone()),
            Arc::clone(&vault),
            options.config.clone(),
            test_tools(directory.path()),
            options.agents.clone(),
        )
        .await;
        let profile = runtime
            .create_profile(CreateProviderProfile {
                draft: ProfileDraft {
                    name: "Test".to_owned(),
                    interface: options.interface,
                    base_url: "http://127.0.0.1:9/v1".to_owned(),
                    model: "test-model".to_owned(),
                    context_window: options.context_window,
                    output_limit: options.output_limit,
                    reasoning_effort: ReasoningEffort::ServerDefault,
                    use_tools: options.use_tools,
                },
                credential: options.credential.clone().map(Secret::new),
            })
            .await
            .expect("the test profile is valid");
        Self {
            runtime,
            store,
            model,
            vault,
            profile,
            options,
            directory,
        }
    }

    /// The database file.
    pub fn database(&self) -> PathBuf {
        self.directory.path().join("mannie.sqlite")
    }

    /// Shuts the runtime down and starts a new one on the same store, as a
    /// restart of Mannie does.
    pub async fn restart(mut self) -> Self {
        self.runtime.shutdown().await;
        self.runtime = start_master(
            self.store.clone(),
            Arc::new(self.model.clone()),
            Arc::clone(&self.vault),
            self.options.config.clone(),
            test_tools(self.directory.path()),
            self.options.agents.clone(),
        )
        .await;
        self
    }

    /// The folder that holds the store, which the policy protects as
    /// Mannie's data folder.
    pub fn data_folder(&self) -> &Path {
        self.directory.path()
    }

    /// Follows a conversation from its first event until `done` holds, and
    /// returns the transcript then. Replayed events show the current state of
    /// what they name, so `done` is checked once the replay is applied.
    ///
    /// # Panics
    ///
    /// When the conversation does not exist or `done` does not hold within
    /// two minutes, which fails the test.
    pub async fn wait_until(
        &self,
        conversation: &ConversationId,
        done: impl Fn(&Transcript) -> bool,
    ) -> Transcript {
        let mut subscription = self
            .runtime
            .subscribe(SubscribeConversation {
                conversation_id: conversation.clone(),
                after_seq: Some(0),
            })
            .await
            .expect("the conversation exists");
        let mut transcript = Transcript::default();
        let wait = async {
            while let Some(update) = subscription.next().await {
                transcript.apply(&update);
                if !subscription.is_replaying() && done(&transcript) {
                    return;
                }
            }
        };
        tokio::time::timeout(WAIT_LIMIT, wait)
            .await
            .expect("the condition holds within two minutes");
        transcript
    }

    /// Follows a conversation until the run `run` has ended, and returns the
    /// transcript then.
    ///
    /// # Panics
    ///
    /// When the conversation does not exist or the run does not end within
    /// two minutes, which fails the test.
    pub async fn wait_for_run(&self, conversation: &ConversationId, run: &RunId) -> Transcript {
        self.wait_until(conversation, |transcript| {
            transcript
                .run(run)
                .is_some_and(|view| !view.state.is_active())
        })
        .await
    }

    /// The transcript of a fresh snapshot.
    ///
    /// # Panics
    ///
    /// When the conversation cannot be read, which fails the test.
    pub async fn snapshot(&self, conversation: &ConversationId) -> Transcript {
        let snapshot = self
            .store
            .snapshot(conversation, 1_000, 100_000)
            .await
            .expect("the snapshot reads")
            .expect("the conversation exists");
        let mut transcript = Transcript::default();
        transcript.apply(&ConversationUpdate::Snapshot { snapshot });
        transcript
    }
}

/// The credential that a test worker's leases carry.
pub const LEASE_CREDENTIAL: &str = "sk-lease-credential";

/// How a test worker is set up.
#[derive(Debug, Clone)]
pub struct WorkerOptions {
    /// The mode of the host's ceiling.
    pub mode: CeilingMode,
    /// The host's command rules.
    pub rules: Vec<CommandRule>,
    /// The text of the host instructions file, or `None` for no file.
    pub host_instructions: Option<String>,
    /// How many tasks run at once.
    pub max_parallel_tasks: u32,
    /// Whether the profile that comes with the tasks uses tools.
    pub use_tools: bool,
    /// Whether the profile has a credential, which comes as a lease.
    pub credential: bool,
    /// The runtime's configuration.
    pub config: RuntimeConfig,
    /// The vault of the leases, in place of the harness's own: the link's
    /// lease vault, in its tests.
    pub vault: Option<Arc<dyn CredentialVault>>,
    /// Whether a conversation in full access on the master may answer the
    /// host's requests.
    pub accepts_master_full_access: bool,
}

impl Default for WorkerOptions {
    fn default() -> Self {
        Self {
            mode: CeilingMode::Ask,
            rules: Vec::new(),
            host_instructions: None,
            max_parallel_tasks: 4,
            use_tools: true,
            credential: true,
            config: fast_config(),
            vault: None,
            accepts_master_full_access: true,
        }
    }
}

/// A worker's runtime on a temporary store, with a fake model, a lease vault
/// in memory, and a host whose allowed folder is a temporary folder.
#[derive(Debug)]
pub struct WorkerHarness {
    /// The runtime.
    pub runtime: Runtime,
    /// Its store.
    pub store: Store,
    /// The model behind the tasks' profile.
    pub model: FakeModel,
    /// The lease vault.
    pub vault: Arc<MemoryVault>,
    /// The allowed folder, which is the tasks' workspace.
    pub allowed: PathBuf,
    /// A folder outside the allowed one.
    pub outside: PathBuf,
    /// The host's name.
    pub host_name: String,
    options: WorkerOptions,
    directory: tempfile::TempDir,
}

/// A new UUIDv7 for a test.
fn new_id<T>(parse: fn(&str) -> Result<T, mannie_types::ids::InvalidId>) -> T {
    parse(&uuid::Uuid::now_v7().to_string()).expect("a UUIDv7 is a valid ID")
}

/// Waits until a task's state satisfies `done`, through a watch that starts
/// after the task's current state was read.
async fn wait_for_assigned(
    runtime: &Runtime,
    task: &TaskId,
    done: impl Fn(&AssignedTaskState) -> bool,
) -> AssignedTaskState {
    let wait = async {
        let mut watch = runtime.watch_assigned_tasks();
        // A state that holds already, or that the task reached before the
        // watch began, is read from the runtime.
        if let Some(state) = runtime.assigned_task(task).await.expect("the store reads")
            && done(&state)
        {
            return state;
        }
        loop {
            let state = watch
                .next()
                .await
                .expect("the watch of assigned tasks goes on");
            if state.task_id == *task && done(&state) {
                return state;
            }
        }
    };
    tokio::time::timeout(WAIT_LIMIT, wait)
        .await
        .expect("the task's state holds within two minutes")
}

impl WorkerHarness {
    /// A worker with the default options.
    pub async fn start() -> Self {
        Self::with_options(WorkerOptions::default()).await
    }

    /// A worker with `options`.
    ///
    /// # Panics
    ///
    /// When the folders or the runtime cannot be set up, which fails the
    /// test.
    pub async fn with_options(options: WorkerOptions) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        for folder in ["data", "work", "outside"] {
            std::fs::create_dir_all(directory.path().join(folder)).expect("a test folder");
        }
        let store = open_store(&directory.path().join("data").join("worker.sqlite"));
        let model = FakeModel::new(ApiInterface::ChatCompletions);
        let vault = Arc::new(MemoryVault::with_arrivals());
        let allowed = PathBuf::from(
            mannie_tools::canonical_folder(&directory.path().join("work"))
                .expect("the allowed folder exists"),
        );
        let outside = PathBuf::from(
            mannie_tools::canonical_folder(&directory.path().join("outside"))
                .expect("the outside folder exists"),
        );
        let host_name = "build-01".to_owned();
        let runtime = start_worker_runtime(
            store.clone(),
            &model,
            &vault,
            &options,
            directory.path(),
            &allowed,
            &host_name,
        )
        .await;
        Self {
            runtime,
            store,
            model,
            vault,
            allowed,
            outside,
            host_name,
            options,
            directory,
        }
    }

    /// The profile that comes with the tasks, as the master would send it.
    ///
    /// # Panics
    ///
    /// Never: its ID is a valid UUID.
    pub fn profile(&self) -> ProfileSnapshot {
        ProfileSnapshot {
            id: ProfileId::parse("0199e0f2-0000-7000-8000-00000000c0de").expect("a valid ID"),
            config_revision: 1,
            interface: ApiInterface::ChatCompletions,
            base_url: "http://127.0.0.1:9/v1".to_owned(),
            model: "test-model".to_owned(),
            context_window: 128_000,
            output_limit: 16_384,
            reasoning_effort: ReasoningEffort::ServerDefault,
            server: None,
            use_tools: self.options.use_tools,
            has_credential: self.options.credential,
        }
    }

    /// A new task in a new conversation, as the master assigns it.
    pub fn assignment(&self, instruction: &str) -> TaskAssignment {
        TaskAssignment {
            task_id: new_id(TaskId::parse),
            conversation_id: new_id(ConversationId::parse),
            follow_up: false,
            title: "Build the backend".to_owned(),
            instruction: instruction.to_owned(),
            read_only: false,
            profile: self.profile(),
        }
    }

    /// A follow-up of `earlier` in its conversation.
    pub fn follow_up(earlier: &TaskAssignment, instruction: &str) -> TaskAssignment {
        TaskAssignment {
            task_id: new_id(TaskId::parse),
            follow_up: true,
            instruction: instruction.to_owned(),
            ..earlier.clone()
        }
    }

    /// Delivers the task's lease, as the master does once the worker stored
    /// the task.
    ///
    /// # Panics
    ///
    /// When the worker refuses the lease, which fails the test.
    pub async fn deliver_lease(&self, task: &TaskId) {
        self.runtime
            .deliver_lease(
                task,
                &self.profile().binding(),
                Secret::new(LEASE_CREDENTIAL.to_owned()),
            )
            .await
            .expect("the worker takes the lease");
    }

    /// Assigns a task and delivers its lease.
    ///
    /// # Panics
    ///
    /// When the worker refuses the task or its lease, which fails the test.
    pub async fn assign(&self, assignment: TaskAssignment) -> AssignedTaskState {
        let state = self
            .runtime
            .assign_task(assignment)
            .await
            .expect("the worker takes the task");
        if self.options.credential {
            self.deliver_lease(&state.task_id).await;
        }
        state
    }

    /// Waits until a task's state satisfies `done`.
    ///
    /// # Panics
    ///
    /// When it does not within two minutes, which fails the test.
    pub async fn wait_for_task(
        &self,
        task: &TaskId,
        done: impl Fn(&AssignedTaskState) -> bool,
    ) -> AssignedTaskState {
        wait_for_assigned(&self.runtime, task, done).await
    }

    /// Waits until a task has ended.
    ///
    /// # Panics
    ///
    /// When it does not end within two minutes, which fails the test.
    pub async fn wait_for_end(&self, task: &TaskId) -> AssignedTaskState {
        self.wait_for_task(task, |state| !state.state.is_active())
            .await
    }

    /// The transcript of a fresh snapshot of a task's conversation.
    ///
    /// # Panics
    ///
    /// When the conversation cannot be read, which fails the test.
    pub async fn snapshot(&self, conversation: &ConversationId) -> Transcript {
        let snapshot = self
            .store
            .snapshot(conversation, 1_000, 100_000)
            .await
            .expect("the snapshot reads")
            .expect("the conversation exists");
        let mut transcript = Transcript::default();
        transcript.apply(&ConversationUpdate::Snapshot { snapshot });
        transcript
    }

    /// The worker's data folder, which the policy protects.
    pub fn data_folder(&self) -> PathBuf {
        self.directory.path().join("data")
    }

    /// Shuts the runtime down and starts a new one on the same store, as a
    /// restart of the worker does. Leases do not survive it.
    pub async fn restart(mut self) -> Self {
        self.runtime.shutdown().await;
        self.vault = Arc::new(MemoryVault::with_arrivals());
        self.runtime = start_worker_runtime(
            self.store.clone(),
            &self.model,
            &self.vault,
            &self.options,
            self.directory.path(),
            &self.allowed,
            &self.host_name,
        )
        .await;
        self
    }
}

/// The name of the harness's worker agent, which runs the tasks that a
/// master assigns.
pub const HARNESS_WORKER_AGENT: &str = "worker";

/// The harness's worker agent: every system tool but the move into a
/// project, with the budgets of the worker service's agent.
pub const HARNESS_WORKER: AgentDefinition = AgentDefinition {
    name: HARNESS_WORKER_AGENT,
    requester: Requester::Agent,
    instructions: "",
    tools: GENERAL.tools,
    max_continuation_notes: 2,
    max_tool_rounds: 100,
    max_tool_calls: 250,
    max_tasks: 0,
};

/// The first line of the harness worker's prompt.
pub const HARNESS_WORKER_BASE: &str = "You are the harness's worker agent.";

/// The line of the harness worker's prompt that names the ceiling's mode.
pub fn ceiling_mode_line(mode: CeilingMode) -> String {
    format!("Ceiling mode: {}", mode.as_str())
}

/// The harness worker's sections: its base line, what an assigned run works
/// under, and the checkpoint.
fn harness_worker_sections() -> InstructionRegistry {
    let mut registry = InstructionRegistry::new();
    let sections = [
        InstructionSection::new("base", 0, |_context| Some(HARNESS_WORKER_BASE.to_owned())),
        InstructionSection::new("machine", 250, |context| {
            context.machine.map(|machine| {
                let folders: Vec<String> = machine
                    .allowed_folders
                    .iter()
                    .map(|folder| folder.display().to_string())
                    .collect();
                format!(
                    "{}\nAllowed folders: {}\nInstructions: {}",
                    ceiling_mode_line(machine.mode),
                    folders.join(", "),
                    machine.instructions.unwrap_or("none")
                )
            })
        }),
        checkpoint_section(),
    ];
    for section in sections {
        registry
            .register(section)
            .expect("the harness's sections have unique IDs");
    }
    registry
}

/// Starts a worker's runtime: the harness's worker agent and sections, the
/// system tools, and a ceiling whose allowed folder is `allowed`.
async fn start_worker_runtime(
    store: Store,
    model: &FakeModel,
    vault: &Arc<MemoryVault>,
    options: &WorkerOptions,
    directory: &Path,
    allowed: &Path,
    host_name: &str,
) -> Runtime {
    let instructions_file = options.host_instructions.as_ref().map(|text| {
        let file = directory.join("data").join("host.md");
        std::fs::write(&file, text).expect("the host instructions are written");
        file
    });
    let system = SystemFacts::detect();
    let machine = MachineSettings {
        ceiling: Some(Ceiling {
            mode: options.mode,
            allowed_folders: vec![allowed.to_path_buf()],
            rules: options.rules.clone(),
        }),
        name: Some(host_name.to_owned()),
        instructions_file,
        task_slots: Some(options.max_parallel_tasks),
        assigned_agent: Some(HARNESS_WORKER_AGENT),
        accepts_assigner_full_access: options.accepts_master_full_access,
    };
    let mut agents = AgentRegistry::new();
    agents
        .register(HARNESS_WORKER)
        .expect("the harness registers one agent");
    let protected =
        mannie_tools::canonical_folder(&directory.join("data")).expect("the data folder exists");
    let tools = ToolsDeps {
        registry: ToolRegistry::with_system_tools(),
        protected: vec![PathBuf::from(protected)],
        mutations: MutationLock::new(),
        system: Arc::new(system),
    };
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let client = HttpClient::new(Arc::clone(&clock)).expect("the HTTP client builds");
    let mut registry = ProviderRegistry::new();
    registry
        .register(Arc::new(model.clone()))
        .expect("one adapter per interface");
    let deps = RuntimeDeps {
        store,
        client,
        registry,
        vault: options
            .vault
            .clone()
            .unwrap_or_else(|| Arc::<MemoryVault>::clone(vault)),
        clock,
        agents,
        instructions: harness_worker_sections(),
        tools,
        machine,
        // A worker has no message box.
        mentions: MentionRegistry::new(),
    };
    Runtime::start(deps, options.config.clone())
        .await
        .expect("the worker's runtime starts")
}

/// Takes a plugin's contributions as the plugin host does when the plugin
/// starts: through the SDK's coordinator and the runtime's consumers, all or
/// none (ADR-0034, decision 6).
///
/// # Panics
///
/// When `plugin` is not a plugin ID, which fails the test.
pub fn add_plugin(
    runtime: &Runtime,
    plugin: &str,
    contributions: Contributions,
) -> Result<(), ContributionRefusal> {
    let plugin = PluginId::parse(plugin).expect("the test names a plugin ID");
    add_contributions(&runtime.contribution_consumers(), &plugin, contributions)
}

/// Drops a plugin's contributions as the plugin host does when the plugin
/// stops. Returns whether the runtime held them.
///
/// # Panics
///
/// When `plugin` is not a plugin ID, which fails the test.
pub fn remove_plugin(runtime: &Runtime, plugin: &str) -> bool {
    let plugin = PluginId::parse(plugin).expect("the test names a plugin ID");
    let registered = runtime
        .contributions()
        .generation_of(plugin.as_str())
        .is_some();
    remove_contributions(&runtime.contribution_consumers(), &plugin);
    registered
}

/// The executor host that the runtime binds to a plugin, as its provider
/// binds the service `executor-host` when the plugin host starts a plugin
/// that declares it (ADR-0034, decision 5), without a manifest.
///
/// # Panics
///
/// When `plugin` is not a plugin ID, which fails the test.
pub fn executor_host(runtime: &Runtime, plugin: &str) -> Arc<dyn ExecutorHostApi> {
    let plugin = PluginId::parse(plugin).expect("the test names a plugin ID");
    runtime.bound_executor_host(plugin.as_str())
}

/// The credentials that the runtime binds to a plugin, as its provider binds
/// the service `credentials` when the plugin host starts a plugin that
/// declares it (ADR-0034, decision 5), without a manifest.
///
/// # Panics
///
/// When `plugin` is not a plugin ID, which fails the test.
pub fn credentials(runtime: &Runtime, plugin: &str) -> Arc<dyn CredentialsApi> {
    let plugin = PluginId::parse(plugin).expect("the test names a plugin ID");
    runtime.bound_credentials(plugin.as_str())
}

/// The kind of the [`FakeExecutor`].
pub const FAKE_EXECUTOR_KIND: &str = "fake";

/// The tool of the [`FakeExecutor`].
pub const FAKE_EXECUTOR_TOOL: &str = "give_task";

/// What the text that the model reads calls the fake executor's targets.
pub const FAKE_EXECUTOR_NOUN: &str = "the box";

/// The fake executor's instruction section.
pub const FAKE_EXECUTOR_SECTION: &str = "Boxes you can give tasks to with give_task: box-01.";

/// The agent that the runtime records for the fake executor's tasks.
pub const FAKE_EXECUTOR_AGENT: &str = "remote";

/// What a call of the fake executor's tool names.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FakeArguments {
    target: String,
    title: String,
    instruction: String,
    background: bool,
}

#[derive(Debug, Default)]
struct FakeExecutorState {
    /// The refusal of the next calls of the tool.
    refusal: Option<ExecutorRefusal>,
    /// The refusal of the next submissions.
    submit_refusal: Option<ExecutorRefusal>,
    /// Whether the next submissions end as if the plugin stopped during
    /// them.
    submit_stopped: bool,
    submissions: Vec<TaskSubmission>,
    follow_ups: Vec<TaskExecutorRef>,
    cancels: Vec<TaskId>,
    decisions: Vec<DecideApproval>,
    /// Whether the target can be reached for the reads of its conversations.
    unreachable: bool,
}

/// An executor of a plugin for the tests of the runtime's side of executors
/// (ADR-0031): it takes calls of its tool `give_task`, keeps what it is asked,
/// and lets the test publish its tasks' states with
/// [`Runtime::publish_task_state`].
#[derive(Debug, Default)]
pub struct FakeExecutor {
    state: Mutex<FakeExecutorState>,
}

impl FakeExecutor {
    /// An executor that takes every call.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Refuses the next calls of the tool with `refusal`.
    pub fn refuse_plans(&self, refusal: ExecutorRefusal) {
        lock(&self.state).refusal = Some(refusal);
    }

    /// Refuses the next submissions with `refusal`.
    pub fn refuse_submissions(&self, refusal: ExecutorRefusal) {
        lock(&self.state).submit_refusal = Some(refusal);
    }

    /// Ends the next submissions as if the plugin stopped before the target
    /// confirmed them.
    pub fn stop_submissions(&self) {
        lock(&self.state).submit_stopped = true;
    }

    /// Makes the target unreachable for the reads of its conversations.
    pub fn set_unreachable(&self, unreachable: bool) {
        lock(&self.state).unreachable = unreachable;
    }

    /// The tasks submitted so far.
    pub fn submissions(&self) -> Vec<TaskSubmission> {
        lock(&self.state).submissions.clone()
    }

    /// The targets of the follow-ups planned so far.
    pub fn follow_ups(&self) -> Vec<TaskExecutorRef> {
        lock(&self.state).follow_ups.clone()
    }

    /// The tasks that the runtime asked to cancel.
    pub fn cancels(&self) -> Vec<TaskId> {
        lock(&self.state).cancels.clone()
    }

    /// The owner's decisions that reached the executor.
    pub fn decisions(&self) -> Vec<DecideApproval> {
        lock(&self.state).decisions.clone()
    }

    /// A target of the fake executor.
    pub fn target(name: &str) -> TaskExecutorRef {
        TaskExecutorRef {
            kind: FAKE_EXECUTOR_KIND.to_owned(),
            target_id: format!("target-{name}"),
            target_name: name.to_owned(),
            noun: FAKE_EXECUTOR_NOUN.to_owned(),
        }
    }

    /// A state of a submitted task, for the test to publish.
    pub fn state(submission: &TaskSubmission, version: u64, state: TaskState) -> AssignedTaskState {
        AssignedTaskState {
            task_id: submission.task_id.clone(),
            conversation_id: submission.conversation_id.clone(),
            version,
            state,
            started_at: None,
            activity: None,
            tool_calls: 0,
            approval: None,
            waits_for_lease: false,
            report: None,
            failure: None,
            ended_at: None,
        }
    }

    /// A state of a submitted task that reported.
    pub fn reported(submission: &TaskSubmission, version: u64, report: &str) -> AssignedTaskState {
        AssignedTaskState {
            report: Some(report.to_owned()),
            ..Self::state(submission, version, TaskState::Reported)
        }
    }

    fn unreachable(&self) -> bool {
        lock(&self.state).unreachable
    }
}

fn fake_unreachable() -> ChatError {
    ChatError::TaskTargetUnreachable {
        kind: FAKE_EXECUTOR_KIND.to_owned(),
        target: "box-01".to_owned(),
    }
}

impl TaskExecutor for FakeExecutor {
    fn kind(&self) -> &'static str {
        FAKE_EXECUTOR_KIND
    }

    fn agent(&self) -> &'static str {
        FAKE_EXECUTOR_AGENT
    }

    /// The spec leaves out the activity argument, which the runtime adds.
    fn tool(&self) -> Option<ToolSpec> {
        Some(ToolSpec::new(
            FAKE_EXECUTOR_TOOL,
            "Gives a task to a box.",
            serde_json::json!({
                "additionalProperties": false,
                "properties": {
                    "background": { "type": "boolean" },
                    "instruction": { "type": "string" },
                    "target": { "type": "string" },
                    "title": { "type": "string" },
                },
                "required": ["background", "instruction", "target", "title"],
                "type": "object",
            }),
        ))
    }

    fn section(&self) -> Option<ExecutorSection> {
        Some(ExecutorSection::new(310, FAKE_EXECUTOR_SECTION))
    }

    fn outlives_reply(&self) -> bool {
        true
    }

    fn plan(&self, call: PlanCall<'_>) -> Result<TaskPlan, PlanError> {
        let arguments: FakeArguments =
            serde_json::from_str(call.arguments).map_err(|error| PlanError::InvalidArguments {
                detail: error.to_string(),
            })?;
        let refusal = lock(&self.state).refusal.clone();
        if let Some(refusal) = refusal {
            return Err(PlanError::Refused(refusal));
        }
        Ok(TaskPlan::new(
            Self::target(&arguments.target),
            arguments.title,
            arguments.instruction,
            arguments.background,
            serde_json::json!({}),
        ))
    }

    fn plan_follow_up(
        &self,
        earlier: &TaskExecutorRef,
        _profile: &ProfileSnapshot,
    ) -> Result<FollowUpPlan, ExecutorRefusal> {
        let mut state = lock(&self.state);
        if let Some(refusal) = state.refusal.clone() {
            return Err(refusal);
        }
        state.follow_ups.push(earlier.clone());
        Ok(FollowUpPlan::new(earlier.clone(), serde_json::json!({})))
    }

    fn document(&self, _submission: &TaskSubmission) -> Option<String> {
        Some(r#"{"version":1}"#.to_owned())
    }

    fn submit(
        &self,
        submission: TaskSubmission,
    ) -> BoxFuture<'static, Result<AssignedTaskState, SubmitError>> {
        let mut state = lock(&self.state);
        let answer = if state.submit_stopped {
            Err(SubmitError::Stopped)
        } else {
            match state.submit_refusal.clone() {
                Some(refusal) => Err(SubmitError::Refused(refusal)),
                None => Ok(Self::state(&submission, 1, TaskState::Accepted)),
            }
        };
        state.submissions.push(submission);
        futures_util::future::ready(answer).boxed()
    }

    fn cancel(&self, task: &TaskId) -> BoxFuture<'static, ()> {
        lock(&self.state).cancels.push(task.clone());
        futures_util::future::ready(()).boxed()
    }

    fn subscribe(
        &self,
        _target: &TaskExecutorRef,
        _conversation: &ConversationId,
        _after_seq: Option<u32>,
    ) -> BoxFuture<'static, Result<BoxStream<'static, ConversationUpdate>, ChatError>> {
        let answer = if self.unreachable() {
            Err(fake_unreachable())
        } else {
            Ok(futures_util::stream::empty().boxed())
        };
        futures_util::future::ready(answer).boxed()
    }

    fn earlier_turns(
        &self,
        _target: &TaskExecutorRef,
        _input: LoadEarlierTurns,
    ) -> BoxFuture<'static, Result<EarlierTurns, ChatError>> {
        let answer = if self.unreachable() {
            Err(fake_unreachable())
        } else {
            Ok(EarlierTurns {
                turns: Vec::new(),
                has_earlier_turns: false,
            })
        };
        futures_util::future::ready(answer).boxed()
    }

    fn checkpoint(
        &self,
        _target: &TaskExecutorRef,
        _input: CheckpointRef,
    ) -> BoxFuture<'static, Result<Checkpoint, ChatError>> {
        let answer = if self.unreachable() {
            Err(fake_unreachable())
        } else {
            Err(ChatError::CheckpointNotFound)
        };
        futures_util::future::ready(answer).boxed()
    }

    fn tool_output(
        &self,
        _target: &TaskExecutorRef,
        _input: ReadToolOutput,
    ) -> BoxFuture<'static, Result<ToolOutputPage, ToolsError>> {
        futures_util::future::ready(Err(ToolsError::OutputNotFound)).boxed()
    }

    fn decide_approval(
        &self,
        _target: &TaskExecutorRef,
        input: DecideApproval,
    ) -> BoxFuture<'static, Result<(), ToolsError>> {
        lock(&self.state).decisions.push(input);
        futures_util::future::ready(Ok(())).boxed()
    }
}

/// One part as a subscriber sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenPart {
    /// The part.
    pub id: PartId,
    /// The run that wrote it, or `None` for the owner's message.
    pub run_id: Option<RunId>,
    /// Its state.
    pub state: PartState,
    /// Its content, with the text that streamed so far.
    pub content: PartContent,
}

/// One turn as a subscriber sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenTurn {
    /// The turn.
    pub id: TurnId,
    /// Its current run.
    pub run: Option<RunView>,
    /// Its visible parts by position.
    pub parts: BTreeMap<u32, SeenPart>,
}

/// The conversation that a subscriber builds from its updates, the way the
/// conversation view does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transcript {
    /// The turns by position.
    pub turns: BTreeMap<u32, SeenTurn>,
    /// The sequence numbers of the events applied, in order.
    pub seqs: Vec<u32>,
    /// The sequence number that the last snapshot reflected.
    pub snapshot_seq: Option<u32>,
    /// Why the subscription ended, if it did.
    pub ended: Option<mannie_types::transcript::SubscriptionEnd>,
    /// Every event applied, in order.
    pub events: Vec<ConversationEvent>,
}

fn append_text(content: &mut PartContent, delta: &str) {
    match content {
        PartContent::UserText { text }
        | PartContent::Text { text }
        | PartContent::Reasoning { text } => {
            text.push_str(delta);
        }
        // A call's arguments arrive with the part's next update, and a
        // mention or an image is complete when its turn starts.
        PartContent::ToolCall { .. }
        | PartContent::Task { .. }
        | PartContent::Error { .. }
        | PartContent::Mention { .. }
        | PartContent::Image { .. }
        | PartContent::Unknown { .. } => {}
    }
}

impl Transcript {
    /// Applies one update.
    pub fn apply(&mut self, update: &ConversationUpdate) {
        match update {
            ConversationUpdate::Snapshot { snapshot } => {
                self.turns.clear();
                for turn in &snapshot.turns {
                    self.insert_turn(turn);
                }
                self.snapshot_seq = Some(snapshot.through_seq);
            }
            ConversationUpdate::Event { seq, event } => {
                self.seqs.push(*seq);
                self.apply_event(event);
                self.events.push(event.clone());
            }
            ConversationUpdate::Ended { reason } => self.ended = Some(*reason),
        }
    }

    fn insert_turn(&mut self, turn: &TurnView) {
        let parts = turn
            .parts
            .iter()
            .map(|part| {
                let seen = SeenPart {
                    id: part.id.clone(),
                    run_id: part.run_id.clone(),
                    state: part.state,
                    content: part.content.clone(),
                };
                (part.position, seen)
            })
            .collect();
        self.turns.insert(
            turn.position,
            SeenTurn {
                id: turn.id.clone(),
                run: turn.run.clone(),
                parts,
            },
        );
    }

    fn apply_event(&mut self, event: &ConversationEvent) {
        match event {
            ConversationEvent::TurnStarted { turn } => self.insert_turn(turn),
            ConversationEvent::RunStarted { run, kept_run_ids } => {
                for turn in self.turns.values_mut() {
                    if turn.id == run.turn_id {
                        turn.run = Some(run.clone());
                        // A retry replaces every part but the owner's message
                        // and the parts of runs that made tool calls.
                        turn.parts.retain(|position, part| {
                            let kept = part
                                .run_id
                                .as_ref()
                                .is_some_and(|run| kept_run_ids.contains(run));
                            *position == 0 || kept
                        });
                    }
                }
            }
            ConversationEvent::RunUpdated { run } => {
                for turn in self.turns.values_mut() {
                    if turn
                        .run
                        .as_ref()
                        .is_some_and(|current| current.id == run.id)
                    {
                        turn.run = Some(run.clone());
                    }
                }
            }
            ConversationEvent::PartStarted { part }
            | ConversationEvent::PartUpdated { part }
            | ConversationEvent::PartFinished { part } => {
                for turn in self.turns.values_mut() {
                    let belongs = turn.id == part.turn_id
                        && turn
                            .run
                            .as_ref()
                            .is_some_and(|run| part.run_id.as_ref() == Some(&run.id));
                    if !belongs {
                        continue;
                    }
                    if part.state == PartState::Discarded {
                        turn.parts.remove(&part.position);
                    } else {
                        let seen = SeenPart {
                            id: part.id.clone(),
                            run_id: part.run_id.clone(),
                            state: part.state,
                            content: part.content.clone(),
                        };
                        turn.parts.insert(part.position, seen);
                    }
                }
            }
            ConversationEvent::PartDelta { part_id, text } => {
                for turn in self.turns.values_mut() {
                    for part in turn.parts.values_mut() {
                        if part.id == *part_id {
                            append_text(&mut part.content, text);
                        }
                    }
                }
            }
            ConversationEvent::AttemptDiscarded { part_ids, .. } => {
                for turn in self.turns.values_mut() {
                    turn.parts.retain(|_, part| !part_ids.contains(&part.id));
                }
            }
            ConversationEvent::CheckpointCreated { .. }
            | ConversationEvent::ContextUsageChanged { .. }
            | ConversationEvent::ConversationChanged { .. }
            | ConversationEvent::Unknown { .. } => {}
        }
    }

    /// The run with this ID, as last seen.
    pub fn run(&self, run: &RunId) -> Option<&RunView> {
        self.turns
            .values()
            .filter_map(|turn| turn.run.as_ref())
            .find(|view| view.id == *run)
    }

    /// The state of the run with this ID.
    pub fn run_state(&self, run: &RunId) -> Option<RunState> {
        self.run(run).map(|view| view.state)
    }

    /// The parts of the turn at `position`, in order.
    pub fn parts(&self, position: u32) -> Vec<SeenPart> {
        self.turns
            .get(&position)
            .map(|turn| turn.parts.values().cloned().collect())
            .unwrap_or_default()
    }

    /// The text of the assistant's reply in the turn at `position`.
    pub fn reply_text(&self, position: u32) -> String {
        self.parts(position)
            .iter()
            .filter_map(|part| match &part.content {
                PartContent::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The tool calls of the turn at `position`, in order.
    pub fn tool_calls(&self, position: u32) -> Vec<ToolCallView> {
        self.parts(position)
            .into_iter()
            .filter_map(|part| match part.content {
                PartContent::ToolCall { call } => Some(*call),
                _ => None,
            })
            .collect()
    }

    /// The calls of task tools of the turn at `position`, in order.
    pub fn task_calls(&self, position: u32) -> Vec<TaskCallView> {
        self.parts(position)
            .into_iter()
            .filter_map(|part| match part.content {
                PartContent::Task { call } => Some(*call),
                _ => None,
            })
            .collect()
    }

    /// The tasks that the calls of the turn at `position` assigned, in
    /// order.
    pub fn tasks(&self, position: u32) -> Vec<TaskView> {
        self.task_calls(position)
            .into_iter()
            .filter_map(|call| call.task)
            .collect()
    }

    /// The transcript without what only a live subscriber knows: the
    /// sequence numbers and the end of the subscription.
    pub fn content(&self) -> BTreeMap<u32, SeenTurn> {
        self.turns.clone()
    }
}
