//! Mannie's agent runtime (ADR-0012): each reply runs as an actor, every
//! change to a conversation is an event in the store before anyone sees it,
//! and windows follow conversations through subscriptions that replay from a
//! cursor.
//!
//! The runtime decides what happens in a run: it assembles the model's
//! context, calls the provider adapters of `mannie-provider`, runs the tool
//! calls of the model through the operation lifecycle of `mannie-tools`,
//! retries transient failures, compacts long conversations into checkpoints,
//! and handles Stop, Shutdown and recovery after a crash. It never runs on a
//! window's behalf; a window only sends commands and follows events.
//!
//! The same runtime serves the owner's conversations and the tasks that
//! another machine assigns, under a ceiling that its composition root sets
//! (ADR-0028). The executors of plugins run tasks somewhere else, such as on
//! a worker (ADR-0031).

mod actor;
mod agents;
mod approvals;
mod assigned;
mod budget;
mod calls;
mod compaction;
mod context;
mod conversations;
mod executors;
mod hub;
mod ids;
mod images;
mod instructions;
mod mentions;
mod plugin_tools;
mod plugins;
mod policy;
mod profiles;
mod projects;
mod retry;
mod tasks;
#[cfg(feature = "testing")]
pub mod testing;
pub mod texts;
mod tools;
#[cfg(feature = "validation")]
mod validation;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use mannie_plugin_sdk::host::ServiceProvider;
use mannie_provider::{HttpClient, ProviderRegistry};
use mannie_store::{RuntimeStoreError, Store};
use mannie_tools::{Ceiling, MutationLock, SystemFacts, ToolRegistry};
use mannie_types::clock::Clock;
use mannie_types::ids::{CheckpointId, ConversationId, ProfileId, ProjectId, RunId};
use mannie_types::tasks::ChildAgentsSettings;
use tokio::sync::{Semaphore, mpsc, watch};

pub use agents::{
    AgentDefinition, AgentRegistry, CONVERSATION, CONVERSATION_AGENT, DuplicateAgent, EXPLORE,
    EXPLORE_AGENT, GENERAL, GENERAL_AGENT, Requester, SYSTEM_TOOLS,
};
pub use assigned::{AssignError, AssignedTaskWatch, LeaseError};
pub use hub::{ConversationSubscription, ListSubscription, SubscriptionLimits};
pub use instructions::{
    DuplicateSection, InstructionContext, InstructionRegistry, InstructionSection, MachineContext,
    SectionText, ToolEnvironment, builtin_sections, checkpoint_section,
};
pub use mentions::{
    DuplicateMentionProvider, FILES, FileMentions, MentionContext, MentionProvider, MentionRegistry,
};
pub use profiles::{CredentialVault, VaultError};
pub use retry::RetryPolicy;
pub use tasks::TASK_TOOLS;

use crate::actor::Command;
use crate::agents::AgentPlugins;
use crate::approvals::Pending;
use crate::assigned::AssignedTasks;
use crate::executors::{CredentialsProvider, ExecutorFollows, ExecutorHostProvider};
use crate::hub::Hubs;
use crate::ids::IdGenerator;
use crate::plugins::Registry;
use crate::tasks::ChildSlots;

/// What the system tools work with. The composition root builds it once.
#[derive(Debug, Clone)]
pub struct ToolsDeps {
    /// The tools that runs may use, of those their agent names.
    pub registry: ToolRegistry,
    /// Mannie's data and log folders, in canonical form. Every access inside
    /// them asks the owner (T-18).
    pub protected: Vec<PathBuf>,
    /// The lock that makes file changes run one at a time on the machine.
    pub mutations: MutationLock,
    /// What the tools report about the machine.
    pub system: Arc<SystemFacts>,
}

/// What a composition root says about the machine that its runtime serves
/// (ADR-0028, ADR-0031). The default is a machine that the owner works on
/// directly: no ceiling, the child agents settings for its task slots, and
/// no tasks from another machine.
#[derive(Debug, Clone, Default)]
pub struct MachineSettings {
    /// The ceiling of every tool call, for a machine whose owner sets one
    /// outside Mannie's interface. Without one, the owner's grants, rules and
    /// full access decide alone.
    pub ceiling: Option<Ceiling>,
    /// The machine's name, which its approval requests show where another
    /// machine presents them to the owner.
    pub name: Option<String>,
    /// A Markdown file of instructions for the machine, which the runtime
    /// reads for every run of an assigned task, so a change applies from the
    /// next one.
    pub instructions_file: Option<PathBuf>,
    /// How many tasks run at once, in place of the child agents settings.
    pub task_slots: Option<u32>,
    /// The agent definition that runs the tasks that another machine assigns,
    /// or `None` for a machine that takes none.
    pub assigned_agent: Option<&'static str>,
    /// Whether the full access of the conversation that assigned a task may
    /// answer the task's approval requests (ADR-0029, decision 5).
    pub accepts_assigner_full_access: bool,
}

/// What the runtime works with. The composition root builds these and hands
/// them over once.
#[derive(Debug)]
pub struct RuntimeDeps {
    /// The store.
    pub store: Store,
    /// The HTTP client of the provider adapters.
    pub client: HttpClient,
    /// The provider adapters.
    pub registry: ProviderRegistry,
    /// Where the credentials of provider profiles are kept.
    pub vault: Arc<dyn CredentialVault>,
    /// The clock of every timestamp and ID.
    pub clock: Arc<dyn Clock>,
    /// The agent definitions.
    pub agents: AgentRegistry,
    /// The instruction sections.
    pub instructions: InstructionRegistry,
    /// The system tools.
    pub tools: ToolsDeps,
    /// The machine that the runtime serves.
    pub machine: MachineSettings,
    /// The core's mention providers (ADR-0032), which a machine without a
    /// message box leaves empty.
    pub mentions: MentionRegistry,
}

/// The runtime's limits and timings.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeConfig {
    /// The most model calls at the same time, across every conversation.
    pub max_concurrent_calls: usize,
    /// How transient failures are retried.
    pub retry: RetryPolicy,
    /// The size of snapshots, the reach of replays and the subscribers'
    /// queues.
    pub subscriptions: SubscriptionLimits,
    /// How often streamed text is stored.
    pub flush_interval: Duration,
    /// How long Stop waits for the run to end.
    pub stop_timeout: Duration,
    /// How long shutting down waits for the runs to end.
    pub shutdown_timeout: Duration,
    /// How long Stop waits for the targets of a reply's executors' tasks to
    /// confirm their cancels before it lets the tasks go (ADR-0027,
    /// decision 6).
    pub executor_cancel_timeout: Duration,
    /// The seed of the retry jitter, for tests; `None` seeds it at random.
    pub random_seed: Option<u64>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_concurrent_calls: 8,
            retry: RetryPolicy::default(),
            subscriptions: SubscriptionLimits::default(),
            flush_interval: Duration::from_millis(50),
            stop_timeout: Duration::from_secs(2),
            shutdown_timeout: Duration::from_secs(3),
            executor_cancel_timeout: Duration::from_secs(10),
            random_seed: None,
        }
    }
}

/// The runtime of the master: conversations, their runs and the provider
/// profiles. Cloning is cheap; clones share everything.
#[derive(Debug, Clone)]
pub struct Runtime {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    store: Store,
    client: HttpClient,
    registry: ProviderRegistry,
    vault: Arc<dyn CredentialVault>,
    clock: Arc<dyn Clock>,
    ids: IdGenerator,
    agents: AgentRegistry,
    instructions: InstructionRegistry,
    tools: ToolsDeps,
    machine: MachineSettings,
    mentions: MentionRegistry,
    /// The runs that follow the tasks of plugins' executors, and the changes
    /// of the executors' outbox.
    executors: ExecutorFollows,
    /// What the plugins that are on contribute, by extension point
    /// (ADR-0034, decision 6).
    contributions: Registry,
    /// The plugin of each agent that a plugin contributed since Mannie
    /// started.
    agent_plugins: AgentPlugins,
    /// The tasks that another machine assigned, while they are active, and
    /// that machine's watches of them.
    assigned: Arc<AssignedTasks>,
    /// Makes assignments one at a time, so a task submitted twice at once is
    /// stored once.
    assigning: tokio::sync::Mutex<()>,
    /// The approvals that runs wait for, one per conversation at most, since
    /// a conversation runs its tool calls one at a time.
    approvals: Mutex<HashMap<ConversationId, Pending>>,
    hubs: Hubs,
    /// What each conversation is busy with. The store refuses a second active
    /// run on its own; this map also covers compactions and the moments before
    /// a run is stored.
    busy: Mutex<HashMap<ConversationId, Busy>>,
    /// Checkpoints whose hash matched the turns they cover in this process.
    verified_checkpoints: Mutex<HashSet<CheckpointId>>,
    /// The projects that are being removed: no conversation starts in them or
    /// moves into them.
    removing_projects: Mutex<HashSet<ProjectId>>,
    slots: Arc<Semaphore>,
    /// The slots of child agents on the machine (docs/features/child-agents/
    /// tasks.md, slots).
    child_slots: ChildSlots,
    /// Makes changes of the child agents settings one at a time, so the pool
    /// always has the limit that was stored last.
    child_settings: tokio::sync::Mutex<()>,
    config: RuntimeConfig,
    shutting_down: AtomicBool,
    random: Mutex<fastrand::Rng>,
}

/// What a conversation is busy with.
#[derive(Debug)]
enum Busy {
    /// A command is storing a new run.
    Starting,
    /// A run is active.
    Running(RunHandle),
    /// The owner's compaction is writing a checkpoint.
    Compacting,
}

/// The runtime's handle on an active run.
#[derive(Debug)]
struct RunHandle {
    run_id: RunId,
    profile_id: ProfileId,
    /// The parent conversation, for a run that works on a task of a child
    /// agent, which its parent's run starts and ends.
    parent: Option<ConversationId>,
    /// Whether the run works on a task that the master assigned to this
    /// worker.
    assigned: bool,
    mailbox: mpsc::Sender<Command>,
    ended: watch::Receiver<bool>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every critical section only reads or replaces map entries, so the data
    // stays consistent even if a holder panicked.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Runtime {
    /// Starts the runtime. Runs that were active when Mannie last ended become
    /// interrupted first, before any command can start a new one.
    pub async fn start(
        deps: RuntimeDeps,
        config: RuntimeConfig,
    ) -> Result<Self, RuntimeStoreError> {
        let recovered = deps.store.recover_interrupted_runs().await?;
        if !recovered.is_empty() {
            tracing::info!(
                runs = recovered.len(),
                "recovered runs that were active when Mannie ended"
            );
        }
        // What is left of commands that ran when Mannie ended is ended in the
        // background, as a best effort; their calls are uncertain already.
        let leftovers: Vec<_> = recovered
            .iter()
            .flat_map(|run| &run.uncertain)
            .filter_map(|call| call.process.clone())
            .collect();
        if !leftovers.is_empty() {
            tokio::spawn(async move {
                for record in &leftovers {
                    mannie_tools::end_leftover_command(record).await;
                }
            });
        }
        let random = config
            .random_seed
            .map_or_else(fastrand::Rng::new, fastrand::Rng::with_seed);
        // A section that cannot be read leaves the default limit, and the
        // settings page shows the failure when the owner opens it.
        // A machine with task slots of its own, such as a worker, takes its
        // limit from its composition root; otherwise the child agents
        // settings limit the children.
        let parallel = match deps.machine.task_slots {
            Some(slots) => slots,
            None => match deps.store.load_settings::<ChildAgentsSettings>().await {
                Ok(settings) => settings.parallel_children.get(),
                Err(error) => {
                    tracing::warn!(%error, error.code = error.code(), "could not read the child agents settings; using the default limit");
                    ChildAgentsSettings::default().parallel_children.get()
                }
            },
        };
        let inner = Inner {
            ids: IdGenerator::new(Arc::clone(&deps.clock)),
            store: deps.store,
            client: deps.client,
            registry: deps.registry,
            vault: deps.vault,
            clock: deps.clock,
            agents: deps.agents,
            instructions: deps.instructions,
            tools: deps.tools,
            machine: deps.machine,
            mentions: deps.mentions,
            executors: ExecutorFollows::default(),
            contributions: Registry::default(),
            agent_plugins: AgentPlugins::default(),
            assigned: Arc::default(),
            assigning: tokio::sync::Mutex::new(()),
            approvals: Mutex::default(),
            hubs: Hubs::default(),
            busy: Mutex::default(),
            verified_checkpoints: Mutex::default(),
            removing_projects: Mutex::default(),
            slots: Arc::new(Semaphore::new(config.max_concurrent_calls)),
            child_slots: ChildSlots::new(parallel),
            child_settings: tokio::sync::Mutex::new(()),
            config,
            shutting_down: AtomicBool::new(false),
            random: Mutex::new(random),
        };
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    /// The providers of the services that the runtime owns, which the plugin
    /// host binds to each plugin that declares them (ADR-0034, decision 5):
    /// the executor host and credentials, each bound to the executor kind
    /// that equals the plugin's ID.
    pub fn service_providers(&self) -> Vec<Arc<dyn ServiceProvider>> {
        vec![
            Arc::new(ExecutorHostProvider::new(self)),
            Arc::new(CredentialsProvider::new(self)),
        ]
    }

    /// How many of the owner's replies are active. The runs of child agents
    /// belong to their parent's reply and do not count of their own, and a
    /// worker's runs work on tasks.
    pub fn active_run_count(&self) -> u32 {
        let busy = lock(&self.inner.busy);
        let active = busy
            .values()
            .filter(|busy| match busy {
                Busy::Starting => true,
                Busy::Running(handle) => handle.parent.is_none() && !handle.assigned,
                Busy::Compacting => false,
            })
            .count();
        u32::try_from(active).unwrap_or(u32::MAX)
    }

    /// Ends every run as interrupted, waits for them up to the shutdown
    /// timeout, ends every subscription, and makes everything stored durable.
    /// No run starts afterwards. The composition root stops the plugins, and
    /// with them their executors, before.
    pub async fn shutdown(&self) {
        self.inner.shutting_down.store(true, Ordering::SeqCst);
        let mut ended = Vec::new();
        {
            let busy = lock(&self.inner.busy);
            for entry in busy.values() {
                if let Busy::Running(handle) = entry {
                    // A full mailbox already holds a Stop, which ends the run
                    // too.
                    let _delivered = handle.mailbox.try_send(Command::Shutdown);
                    ended.push(handle.ended.clone());
                }
            }
        }
        let all_ended = futures_util::future::join_all(
            ended.iter_mut().map(|ended| ended.wait_for(|ended| *ended)),
        );
        if tokio::time::timeout(self.inner.config.shutdown_timeout, all_ended)
            .await
            .is_err()
        {
            tracing::warn!("runs did not end in time; the next start marks them as interrupted");
        }
        self.inner.hubs.shut_down().await;
        self.inner.assigned.shut_down();
        if let Err(error) = self.inner.store.flush().await {
            tracing::warn!(%error, error.code = error.code(), "could not make the store durable while quitting");
        }
    }

    /// Whether an active run uses a profile.
    fn profile_in_use(&self, profile: &ProfileId) -> bool {
        let busy = lock(&self.inner.busy);
        busy.values()
            .any(|busy| matches!(busy, Busy::Running(handle) if handle.profile_id == *profile))
    }

    fn is_shutting_down(&self) -> bool {
        self.inner.shutting_down.load(Ordering::SeqCst)
    }

    /// What the composition root said about the machine.
    fn machine(&self) -> &MachineSettings {
        &self.inner.machine
    }

    /// A number in [0, 1) for the retry jitter.
    fn random(&self) -> f64 {
        lock(&self.inner.random).f64()
    }
}
