//! Test tools: a credential vault in memory, a fake model that plays scripted
//! replies and writes summaries on its own, a runtime on a temporary store,
//! and a transcript that a subscriber builds from updates
//! (docs/architecture/agent-runtime.md, testing).
//!
//! Only the `testing` feature compiles this module. The desktop app and the
//! worker never enable it.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use mannie_provider::testing::Step;
use mannie_provider::{
    EventStream, HttpClient, ModelRequest, Provider, ProviderError, ProviderEvent,
    ProviderRegistry, Target,
};
use mannie_store::{Store, StoreOptions};
use mannie_tools::{MutationLock, SystemFacts, ToolRegistry};
use mannie_types::clock::{Clock, SystemClock};
use mannie_types::ids::{ConversationId, PartId, RunId, TurnId};
use mannie_types::provider::{
    ApiInterface, CreateProviderProfile, CredentialStoreProblem, ProfileDraft, ProviderProfile,
    ReasoningEffort,
};
use mannie_types::run::{FinishReason, RunFailure, RunState, RunView, Usage};
use mannie_types::secret::Secret;
use mannie_types::tools::ToolCallView;
use mannie_types::transcript::{
    ConversationEvent, ConversationUpdate, PartContent, PartState, TurnView,
};

use crate::{
    AgentRegistry, CredentialVault, InstructionRegistry, RetryPolicy, Runtime, RuntimeConfig,
    RuntimeDeps, ToolsDeps, VaultError, texts,
};

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

/// A credential vault in memory, whose calls can be made to fail.
#[derive(Debug, Default)]
pub struct MemoryVault {
    state: Mutex<VaultState>,
}

impl MemoryVault {
    /// An empty vault.
    pub fn new() -> Self {
        Self::default()
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
}

#[derive(Debug, Default)]
struct ModelState {
    context_window: Option<u32>,
    full_summaries: bool,
    replies: VecDeque<Vec<Step>>,
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
                // A reply without a script fails, so that a test that makes
                // more calls than it expects notices.
                state
                    .replies
                    .pop_front()
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
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let options = StoreOptions::new(database.with_file_name("backups"), clock);
    Store::open(database, &options).expect("the test store opens")
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
        let store = open_store(&directory.path().join("mannie.sqlite"));
        let model = FakeModel::new(options.interface);
        if options.enforce_context_window {
            model.enforce_context_window(options.context_window);
        }
        if options.full_summaries {
            model.write_full_summaries();
        }
        let vault = Arc::new(MemoryVault::new());
        let runtime = start_runtime_with_tools(
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
        self.runtime = start_runtime_with_tools(
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
    /// 20 seconds, which fails the test.
    pub async fn wait_until(
        &self,
        conversation: &ConversationId,
        done: impl Fn(&Transcript) -> bool,
    ) -> Transcript {
        let mut subscription = self
            .runtime
            .subscribe(mannie_types::chat::SubscribeConversation {
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
        tokio::time::timeout(Duration::from_secs(20), wait)
            .await
            .expect("the condition holds within 20 seconds");
        transcript
    }

    /// Follows a conversation until the run `run` has ended, and returns the
    /// transcript then.
    ///
    /// # Panics
    ///
    /// When the conversation does not exist or the run does not end within 20
    /// seconds, which fails the test.
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
        // A tool call's arguments arrive with the part's next update.
        PartContent::ToolCall { .. } | PartContent::Error { .. } | PartContent::Unknown { .. } => {}
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

    /// The transcript without what only a live subscriber knows: the
    /// sequence numbers and the end of the subscription.
    pub fn content(&self) -> BTreeMap<u32, SeenTurn> {
        self.turns.clone()
    }
}
