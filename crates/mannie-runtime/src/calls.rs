//! Preparing model calls (docs/architecture/agent-runtime.md, the loop): what
//! a run reads once at its start, the context of each call, and the calls
//! that write checkpoints.

use std::borrow::Cow;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use futures_util::StreamExt;
use mannie_plugin_sdk::points::executors::{Executors, TaskExecutor};
use mannie_plugin_sdk::points::tools::ToolSpec;
use mannie_provider::{
    ImageInput, ModelRequest, Provider, ProviderError, ProviderEvent, ResponsesShape, Target,
    ToolChoice, ToolDefinition, Url, chatgpt_plan_quirks, check_base_url, quirks_for,
};
use mannie_store::{
    Change, Durability, ModelTurn, NewCheckpoint, RuntimeStoreError, StoredCheckpoint,
    StoredProfile,
};
use mannie_tools::{Availability, PermissionClass, Tool, with_activity};
use mannie_types::ceiling::CeilingMode;
use mannie_types::chat::ChatSettings;
use mannie_types::clock::UtcMillis;
use mannie_types::context::{Attachment, AttachmentRef};
use mannie_types::ids::{ConversationId, PartId, ProfileId, ProjectId};
use mannie_types::projects::ProjectView;
use mannie_types::provider::{ApiInterface, ProfileAccess, ReasoningEffort, context_budget};
use mannie_types::run::{FinishReason, RunFailure};
use mannie_types::secret::Secret;
use mannie_types::tools::ToolCallStatus;
use mannie_types::transcript::PartKind;

use crate::agents::{AgentDefinition, Requester};
use crate::assigned::{AssignedRun, lease_reference};
use crate::budget::{Ledger, estimate_request, estimate_scaled_image, estimate_text, usage};
use crate::chatgpt::AccountKey;
use crate::compaction::{self, CheckpointCursor, ContextSize, Plan, Trigger};
use crate::context::{RequestShape, SentImages, build_request, chosen_context};
use crate::instructions::{
    InstructionContext, InstructionSection, MachineContext, SourcesContext, ToolEnvironment,
    plugin_sections,
};
use crate::plugin_tools::{OfferedPluginTool, offered_tools};
use crate::plugins::Snapshot;
use crate::profiles::{VaultError, effort_for_model, in_vault, summary_reasoning_effort};
use crate::retry::RetryDecision;
use crate::self_management::MANNIE_INSPECT;
use crate::{Runtime, lock, plans, questions, tasks, texts};

/// The newest tool results of the latest turn that stay in the context when
/// older ones are pruned to save space.
const KEPT_RESULTS: usize = 3;

/// Output reserve cap for ChatGPT plan profiles, which omit `max_output_tokens`
/// on the wire (ADR-0062, decision 10; ADR-0083).
const CHATGPT_PLAN_OUTPUT_RESERVE: u32 = 32_768;

/// Context budget for a profile: ChatGPT plan profiles omit `max_output_tokens`
/// on the wire (ADR-0062, decision 10), so their output reserve is capped rather
/// than subtracting the full advertised `output_limit` (ADR-0083).
fn profile_budget(access: ProfileAccess, context_window: u32, output_limit: u32) -> u32 {
    let output_reserve = match access {
        ProfileAccess::ChatGptPlan => output_limit.min(CHATGPT_PLAN_OUTPUT_RESERVE),
        ProfileAccess::Endpoint => output_limit,
    };

    context_budget(context_window, output_reserve)
}

/// The tool that moves a general conversation into a project.
const MOVE_TO_PROJECT: &str = "move_to_project";

/// The tool that starts a command in the background.
const START_COMMAND: &str = "start_command";

/// The tools that read and stop the commands that a run started in the
/// background, which a run is offered only with a tool that starts them
/// (ADR-0041).
const COMMAND_FOLLOW_UPS: [&str; 2] = ["read_command_output", "stop_command"];

/// A run's tools without the follow-ups of background commands when none of
/// its tools starts one.
fn with_follow_ups_only_with_starts(tools: Vec<Arc<dyn Tool>>) -> Vec<Arc<dyn Tool>> {
    if tools.iter().any(|tool| tool.name() == START_COMMAND) {
        return tools;
    }
    tools
        .into_iter()
        .filter(|tool| !COMMAND_FOLLOW_UPS.contains(&tool.name()))
        .collect()
}

/// Whether a tool only reads, for a run that is offered only what reads. It
/// starts no background command, so it reads none either (ADR-0041).
fn only_reads(tool: &Arc<dyn Tool>) -> bool {
    tool.class() == PermissionClass::ReadOnly && !COMMAND_FOLLOW_UPS.contains(&tool.name())
}

/// The most bytes of the machine's instructions that a run of an assigned
/// task reads; the rest is left out.
const MAX_MACHINE_INSTRUCTIONS_BYTES: usize = 64 * 1024;

/// The most tools that one request offers: the most functions that OpenAI's
/// interfaces take. A run that would offer more leaves out the tools that the
/// plugins' sources published, so every other tool fits (ADR-0043,
/// decision 2).
const MAX_REQUEST_TOOLS: usize = 128;

/// Why a call could not be prepared or made.
#[derive(Debug)]
pub(crate) enum CallError {
    /// The call cannot be made, or it failed: the cause for the owner.
    Failed(RunFailure),
    /// The store failed.
    Store(RuntimeStoreError),
}

impl From<RuntimeStoreError> for CallError {
    fn from(error: RuntimeStoreError) -> Self {
        Self::Store(error)
    }
}

/// Where the credential of a run's calls comes from.
#[derive(Debug, Clone)]
pub(crate) enum CallCredential {
    /// The profile's own credential, read once when the run starts, which
    /// the target holds.
    Profile,
    /// A token of the ChatGPT account that the run started with, got right
    /// before each call, since tokens expire within the hour (ADR-0062,
    /// decision 8).
    ChatGptAccount {
        /// The account.
        key: AccountKey,
    },
}

/// Where a run's calls go, and how they are bound.
struct Route {
    /// The API interface.
    interface: ApiInterface,
    /// The server's base URL.
    base_url: Url,
    /// Where the calls' credential comes from.
    credential: CallCredential,
    /// The binding of the run's provider-private data.
    binding: String,
}

/// The target of one call, with the serial of the ChatGPT token it carries.
#[derive(Debug, Clone)]
pub(crate) struct CallTarget {
    /// The server, the credential and the server's quirks.
    pub(crate) target: Target,
    /// The serial of the session whose token the call carries, for a call
    /// on the ChatGPT plan.
    pub(crate) issued: Option<u64>,
}

/// What the calls of a run use, read once when the run starts.
#[derive(Debug, Clone)]
pub(crate) struct Setup {
    /// The agent that makes the calls.
    pub(crate) agent: AgentDefinition,
    /// The adapter of the profile's interface.
    pub(crate) adapter: Arc<dyn Provider>,
    /// The server, the credential and the server's quirks. On the ChatGPT
    /// plan it has no credential; each call gets a token.
    pub(crate) target: Target,
    /// Where the calls' credential comes from.
    pub(crate) credential: CallCredential,
    /// The interface.
    pub(crate) interface: ApiInterface,
    /// The model.
    pub(crate) model: String,
    /// The profile and its configuration revision.
    pub(crate) binding: String,
    /// The output limit of each call.
    pub(crate) output_limit: u32,
    /// The context budget of each call.
    pub(crate) budget: u32,
    /// The reasoning effort.
    pub(crate) reasoning_effort: ReasoningEffort,
    /// The lowest explicit reasoning effort supported by the model, used for
    /// compaction summary calls (ADR-0083).
    pub(crate) summary_reasoning_effort: ReasoningEffort,
    /// The tools of the run: those its agent names that the registry holds,
    /// or none when the profile does not use tools.
    pub(crate) tools: Vec<Arc<dyn Tool>>,
    /// Their definitions, as requests carry them when the tools are
    /// offered.
    pub(crate) definitions: Vec<ToolDefinition>,
    /// The definitions of the task tools that the run is offered: those its
    /// agent names, when its profile uses tools and it is not a task itself
    /// (ADR-0024, decision 1).
    pub(crate) task_definitions: Vec<ToolDefinition>,
    /// The definition of `ask_owner`, for a run whose agent names it, whose
    /// profile uses tools and that is not a task itself (ADR-0058,
    /// decision 1). It stays apart from the task tools, whose list says
    /// whether a run gives tasks and makes their calls `task` parts.
    pub(crate) question_definition: Option<ToolDefinition>,
    /// The definition of `save_plan`, for a run whose agent names it, whose
    /// profile uses tools and that is not a task itself (ADR-0060, decision 2).
    pub(crate) plan_definition: Option<ToolDefinition>,
    /// The child agents that `start_task` can start.
    pub(crate) child_agents: Vec<String>,
    /// For a run of an assigned task, what it works under.
    pub(crate) assigned: Option<AssignedSetup>,
    /// The executors whose tools the run is offered, as they were when the
    /// run started (ADR-0031, decision 4).
    pub(crate) executors: Vec<OfferedExecutor>,
    /// The plugins' tools that the run is offered, each with its plugin and
    /// the generation of the plugin's start, so a call of one whose plugin
    /// stopped since, or stopped and started again, does not run (ADR-0030,
    /// decision 8).
    pub(crate) plugin_tools: Vec<OfferedPluginTool>,
    /// The plugins' instruction sections and those of the executors whose
    /// tools the run is offered, as they were when the run started, so the
    /// prompt stays the same while the run lasts.
    pub(crate) plugin_sections: Vec<InstructionSection>,
}

/// An executor whose tool a run is offered.
#[derive(Debug, Clone)]
pub(crate) struct OfferedExecutor {
    /// The tool's name.
    pub(crate) tool: String,
    /// The executor.
    pub(crate) executor: Arc<dyn TaskExecutor>,
}

/// What the plugins' executors offer a run that is offered the task tools:
/// the executors with their tools' definitions and their sections.
struct ExecutorOffer {
    executors: Vec<OfferedExecutor>,
    definitions: Vec<ToolDefinition>,
    sections: Vec<InstructionSection>,
}

/// The plugins' tools of a run that fit in a request beside `others` tools:
/// every static tool, and the published ones in their order until the room
/// is full. The rest are left out, which the log records (ADR-0043,
/// decision 2).
fn within_request_room(
    offered: Vec<(OfferedPluginTool, Arc<dyn Tool>)>,
    others: usize,
) -> Vec<(OfferedPluginTool, Arc<dyn Tool>)> {
    let fixed = offered.iter().filter(|(tool, _)| !tool.published).count();
    let room = MAX_REQUEST_TOOLS.saturating_sub(others + fixed);
    let mut published = 0;
    let mut left_out = 0;
    let mut kept = Vec::with_capacity(offered.len());
    for (tool, adapter) in offered {
        if tool.published {
            if published == room {
                left_out += 1;
                continue;
            }
            published += 1;
        }
        kept.push((tool, adapter));
    }
    if left_out > 0 {
        tracing::warn!(
            left_out,
            max = MAX_REQUEST_TOOLS,
            "a run leaves out published tools that do not fit in a request"
        );
    }
    kept
}

/// The definition that the model gets of an executor's tool. The runtime
/// adds the activity argument here, as it does for every other tool, so an
/// executor's spec leaves it out (docs/features/system-tools/tool-calls.md,
/// the activity of a call).
fn executor_definition(spec: ToolSpec) -> ToolDefinition {
    ToolDefinition {
        parameters: with_activity(&spec.parameters),
        strict: spec.strict,
        name: spec.name,
        description: spec.description,
    }
}

/// What a run of an assigned task reads of its machine once, when it starts.
#[derive(Debug, Clone)]
pub(crate) struct AssignedSetup {
    /// The mode of the task's ceiling.
    pub(crate) mode: CeilingMode,
    /// The machine's instructions, as the file said when the run started.
    pub(crate) instructions: Option<String>,
}

impl Setup {
    /// The run's tool with this name.
    pub(crate) fn tool(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.iter().find(|tool| tool.name() == name)
    }

    /// Whether the run has tools.
    pub(crate) fn uses_tools(&self) -> bool {
        !self.tools.is_empty()
            || self.offers_tasks()
            || self.offers_questions()
            || self.offers_plans()
    }

    /// Whether the run's credential may be lent to an executor's task. A
    /// run on the ChatGPT plan keeps its tokens on this machine (ADR-0062,
    /// decision 13).
    pub(crate) fn lends_credential(&self) -> bool {
        matches!(self.credential, CallCredential::Profile)
    }

    /// Whether the run is offered the task tools.
    pub(crate) fn offers_tasks(&self) -> bool {
        !self.task_definitions.is_empty()
    }

    /// Whether the run is offered `ask_owner`.
    pub(crate) fn offers_questions(&self) -> bool {
        self.question_definition.is_some()
    }

    /// Whether the run is offered `save_plan`.
    pub(crate) fn offers_plans(&self) -> bool {
        self.plan_definition.is_some()
    }

    /// The child agents that `start_task` can start, as a call reads them.
    pub(crate) fn child_agent_names(&self) -> Vec<&str> {
        self.child_agents.iter().map(String::as_str).collect()
    }

    /// Whom the run's agent answers to.
    pub(crate) fn requester(&self) -> Requester {
        self.agent.requester
    }

    /// A tool of the run that a plugin contributed, with its plugin's start.
    pub(crate) fn plugin_tool(&self, tool: &str) -> Option<&OfferedPluginTool> {
        self.plugin_tools
            .iter()
            .find(|offered| offered.name == tool)
    }

    /// The executor whose tool has this name, among those the run is
    /// offered.
    pub(crate) fn executor_for(&self, tool: &str) -> Option<Arc<dyn TaskExecutor>> {
        self.executors
            .iter()
            .find(|offered| offered.tool == tool)
            .map(|offered| Arc::clone(&offered.executor))
    }

    /// The definitions of the tools that a call is offered where the run
    /// works, followed by `ask_owner` and the task tools.
    fn definitions_for(&self, availability: &Availability<'_>) -> Vec<ToolDefinition> {
        let mut definitions: Vec<ToolDefinition> = self
            .tools
            .iter()
            .zip(&self.definitions)
            .filter(|(tool, _definition)| tool.offered(availability))
            .map(|(_tool, definition)| definition.clone())
            .collect();
        definitions.extend(self.question_definition.iter().cloned());
        definitions.extend(self.plan_definition.iter().cloned());
        definitions.extend(self.task_definitions.iter().cloned());
        definitions
    }
}

/// How a call is prepared.
#[derive(Debug, Clone, Default)]
pub(crate) struct CallOptions {
    /// When set, prune eligible completed tool and task results of the latest
    /// turn up through this part (inclusive), keeping the trimmed prefix fixed
    /// across steps for prompt-prefix caching (ADR-0083).
    pub(crate) prune_through: Option<PartId>,
}

/// One call, ready to be made.
#[derive(Debug, Clone)]
pub(crate) struct Prepared {
    /// The request.
    pub(crate) request: ModelRequest,
    /// The size of the request's context.
    pub(crate) size: ContextSize,
    /// The checkpoint that stands in for older turns or steps.
    pub(crate) checkpoint: Option<StoredCheckpoint>,
    /// The uncovered turns after the checkpoint, as the model reads them.
    pub(crate) turns: Vec<ModelTurn>,
    /// The full turns loaded from the store from the checkpoint's turn onward,
    /// used to build candidate slices and resumed-turn anchors.
    pub(crate) full_turns: Vec<ModelTurn>,
    /// The system prompt without the checkpoint section, used to evaluate
    /// candidate cuts and post-compaction request sizes without re-querying
    /// the store.
    pub(crate) base_instructions: String,
    /// The tool definitions offered to this call.
    pub(crate) definitions: Vec<ToolDefinition>,
    /// The tool result images loaded for this call.
    pub(crate) tool_images: SentImages,
    /// Whether older tool results of the latest turn were pruned from this
    /// request (ADR-0083).
    pub(crate) results_pruned: bool,
    /// Mannie's uncalibrated estimate of the same call with no results pruned,
    /// when `results_pruned` is true.
    pub(crate) unpruned_estimate: Option<u32>,
    /// The ledger of the conversation's last call.
    pub(crate) ledger: Ledger,
}

/// What a run learns of its conversation for one call.
#[derive(Debug, Default)]
struct ConversationFacts {
    /// The conversation's project, for a run with tools.
    project_id: Option<ProjectId>,
    /// The owner's projects, the oldest first, for a run with tools.
    projects: Vec<ProjectView>,
    /// Whether the conversation, or a child's parent, is in full access.
    full_access: bool,
    /// The attachments of the conversation, or of a child's parent, which
    /// the instructions outline and plugins' tools may be offered for
    /// (ADR-0049, ADR-0052).
    attachments: Vec<Attachment>,
    /// The attachment that the conversation, or a child's parent, belongs to
    /// (ADR-0051).
    home: Option<AttachmentRef>,
}

/// What the instructions say of a conversation's attachments, owned for one
/// call, which the section of sources borrows.
#[derive(Debug, Default)]
struct AttachedSources {
    /// Whether the conversation holds attachments at all.
    attached: bool,
    /// What the attachments hold, as their providers outline it.
    outline: String,
    /// The label of the attachment that the conversation belongs to.
    home: Option<String>,
}

impl AttachedSources {
    /// The section of sources of the instructions, or `None` without
    /// attachments. `excerpts` says whether a message of the request holds
    /// excerpts.
    fn context(&self, excerpts: bool) -> Option<SourcesContext<'_>> {
        self.attached.then(|| SourcesContext {
            home: self.home.as_deref(),
            outline: &self.outline,
            excerpts,
        })
    }
}

impl Runtime {
    /// The setup of a run that uses the profile `profile_id`. A run that
    /// works on a task is never offered the task tools.
    pub(crate) async fn load_setup(
        &self,
        profile_id: &ProfileId,
        agent: &str,
        task: bool,
        account: Option<&AccountKey>,
        effort: ReasoningEffort,
    ) -> Result<Setup, RunFailure> {
        let mut profile = match self.inner.store.profile(profile_id).await {
            Ok(Some(profile)) => profile,
            Ok(None) => return Err(RunFailure::ProfileMissing),
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), "could not read the profile of a run");
                return Err(RunFailure::Internal);
            }
        };

        // The profile may name another model by the time a slot is available.
        profile.reasoning_effort = effort_for_model(&profile, effort);
        self.setup_for(&profile, agent, task, account).await
    }

    /// The setup of calls with a profile. Reads the credential, which may
    /// wait for the owner to unlock the credential store.
    ///
    /// Everything that plugins contribute comes from one snapshot of the
    /// registry, so a run gets all of a plugin's contributions or none, even
    /// while the plugin starts or stops (ADR-0034, decision 6).
    pub(crate) async fn setup_for(
        &self,
        profile: &StoredProfile,
        agent: &str,
        task: bool,
        account: Option<&AccountKey>,
    ) -> Result<Setup, RunFailure> {
        let snapshot = self.contributions();
        let Some(agent) = self.agent_in(&snapshot, agent) else {
            // The owner deleted a custom agent after the run was queued, or a
            // plugin's agent is gone (ADR-0059, decision 12).
            tracing::warn!(agent, "a run names an agent that is not registered");
            return Err(RunFailure::AgentMissing);
        };
        let route = self.route_of(profile, account).await?;
        let Some(adapter) = self.inner.registry.adapter(route.interface) else {
            tracing::error!(
                interface = route.interface.as_str(),
                "no adapter is registered for the profile's interface"
            );
            return Err(RunFailure::Internal);
        };
        let self_management = if profile.use_tools && !task && agent.requester == Requester::Owner {
            self.self_management_tools(agent.read_only)
        } else {
            Vec::new()
        };
        let (plugin_tools, adapters) = if profile.use_tools {
            self.plugin_tools_for(&snapshot, &agent, self_management.len())
        } else {
            (Vec::new(), Vec::new())
        };
        let mut tools = if profile.use_tools {
            self.tools_of(&agent, adapters)
        } else {
            Vec::new()
        };
        tools.extend(self_management);
        let definitions: Vec<ToolDefinition> = tools
            .iter()
            .map(|tool| ToolDefinition {
                name: tool.name().to_owned(),
                description: tool.description().to_owned(),
                // Every call names itself for the owner (docs/features/
                // system-tools/tool-calls.md, the activity of a call).
                parameters: with_activity(tool.parameters()),
                strict: tool.strict(),
            })
            .collect();
        // A task's run never asks the owner: it reports open questions to the
        // agent that gave it the task (ADR-0058, decision 9).
        let question_definition =
            (profile.use_tools && !task && agent.tools.contains(&questions::ASK_OWNER))
                .then(questions::definition);
        // A task's run saves no plan: only an agent that answers to the owner
        // has someone to read it.
        let plan_definition =
            (profile.use_tools && !task && agent.tools.contains(&plans::SAVE_PLAN))
                .then(plans::definition);
        let child_agents = self.roster_of(&snapshot, &agent);
        // A child never gives tasks of its own (T-22), and an agent without a
        // task budget or anyone to give tasks to is not offered the tools.
        let offers_tasks =
            profile.use_tools && !task && agent.max_tasks > 0 && !child_agents.is_empty();
        let mut task_definitions: Vec<ToolDefinition> = if offers_tasks {
            tasks::definitions(&child_agents.iter().map(String::as_str).collect::<Vec<_>>())
                .into_iter()
                .filter(|definition| agent.tools.contains(&definition.name.as_str()))
                .collect()
        } else {
            Vec::new()
        };
        let mut executors = Vec::new();
        let mut plugin_sections = plugin_sections(&snapshot, &agent);
        // An executor's task runs elsewhere with tools of its own, so an agent
        // that only reads gives it none. The ChatGPT plan's tokens stay on this
        // machine, so a run on the plan gives executors no tasks (ADR-0062,
        // decision 13).
        if offers_tasks && !agent.read_only && profile.access.is_lendable() {
            let offered_names: Vec<String> = definitions
                .iter()
                .chain(&task_definitions)
                .chain(&question_definition)
                .chain(&plan_definition)
                .map(|definition| definition.name.clone())
                .collect();
            let offer = Self::executor_offer(&snapshot, offered_names);
            executors = offer.executors;
            task_definitions.extend(offer.definitions);
            plugin_sections.extend(offer.sections);
        }
        let target = self.target_of(profile, &route).await?;
        let agent = self.with_roster_named(&snapshot, agent, &child_agents, offers_tasks);
        let budget = profile_budget(profile.access, profile.context_window, profile.output_limit);
        Ok(Setup {
            agent,
            adapter,
            target,
            credential: route.credential,
            interface: route.interface,
            model: profile.model.clone(),
            binding: route.binding,
            output_limit: profile.output_limit,
            budget,
            reasoning_effort: profile.reasoning_effort,
            summary_reasoning_effort: summary_reasoning_effort(profile),
            tools,
            definitions,
            task_definitions,
            question_definition,
            plan_definition,
            child_agents,
            assigned: None,
            executors,
            plugin_tools,
            plugin_sections,
        })
    }

    /// The tools that a run of this agent may call, from the registry and the
    /// plugins' adapters.
    fn tools_of(
        &self,
        agent: &AgentDefinition,
        adapters: Vec<Arc<dyn Tool>>,
    ) -> Vec<Arc<dyn Tool>> {
        let mut tools = with_follow_ups_only_with_starts(
            agent
                .tools
                .iter()
                .filter_map(|name| self.inner.tools.registry.get(name))
                .chain(adapters)
                .collect(),
        );
        // For every run, because a stored definition or a plugin's tool can
        // change after the definition was saved (ADR-0059, decision 5; T-86).
        if agent.read_only {
            tools.retain(only_reads);
        }
        tools
    }

    /// The agent, with its roster in its instructions when it gives tasks and
    /// the roster is not the default one, which the tool's words cover
    /// (ADR-0059, decision 11). A plugin's agent may have no description.
    fn with_roster_named(
        &self,
        snapshot: &Snapshot,
        mut agent: AgentDefinition,
        roster: &[String],
        offers_tasks: bool,
    ) -> AgentDefinition {
        let names: Vec<&str> = roster.iter().map(String::as_str).collect();
        if !offers_tasks || tasks::is_default_roster(&names) {
            return agent;
        }
        let definitions: Vec<(&str, String)> = names
            .iter()
            .map(|name| {
                let description = self
                    .agent_in(snapshot, name)
                    .map(|definition| definition.description.into_owned())
                    .unwrap_or_default();
                (*name, description)
            })
            .collect();
        let entries: Vec<(&str, &str)> = definitions
            .iter()
            .map(|(name, description)| (*name, description.as_str()))
            .collect();
        let roster = texts::roster_section(&entries);
        let instructions = if agent.instructions.is_empty() {
            roster
        } else {
            format!("{}\n\n{roster}", agent.instructions)
        };
        agent.instructions = Cow::Owned(instructions);
        agent
    }

    /// The plugins' tools that join the core's that the agent names, each
    /// for the agents that its selector offers it to (ADR-0034, decision 6),
    /// within the room of a request beside the core's tools, the task tools
    /// and the executors' tools, which join later (ADR-0043, decision 2).
    fn plugin_tools_for(
        &self,
        snapshot: &Snapshot,
        agent: &AgentDefinition,
        self_management_count: usize,
    ) -> (Vec<OfferedPluginTool>, Vec<Arc<dyn Tool>>) {
        let core = agent
            .tools
            .iter()
            .filter(|name| self.inner.tools.registry.get(name).is_some())
            .count();
        let asks = usize::from(agent.tools.contains(&questions::ASK_OWNER));
        let saves = usize::from(agent.tools.contains(&plans::SAVE_PLAN));
        let others = core
            + asks
            + saves
            + self_management_count
            + tasks::TASK_TOOLS.len()
            + snapshot.items::<Executors>().len();
        within_request_room(offered_tools(snapshot, agent, self), others)
            .into_iter()
            .unzip()
    }

    /// The tools of the plugins' executors that join the task tools of a run
    /// that is offered them, each while its executor offers it, with their
    /// sections, so Mannie without plugins offers what it offered before
    /// (ADR-0031). `snapshot` holds the executors as the run reads them when
    /// it starts, and `taken` the names of the run's other tools.
    fn executor_offer(snapshot: &Snapshot, mut taken: Vec<String>) -> ExecutorOffer {
        let mut offer = ExecutorOffer {
            executors: Vec::new(),
            definitions: Vec::new(),
            sections: Vec::new(),
        };
        for registered in snapshot.items::<Executors>() {
            let executor = Arc::clone(&registered.entry);
            let Some(spec) = executor.tool() else {
                continue;
            };
            // An executor names its tool anew for each run, so the tool that
            // it offers now may have the name of a plugin's tool that was
            // checked while the executor offered none, or of another
            // executor's tool. The run offers each name once, so the model
            // never gets two definitions of one name, and leaves the
            // executor's section out with its tool.
            if taken.contains(&spec.name) {
                tracing::warn!(
                    kind = executor.kind(),
                    tool = spec.name,
                    "a run leaves out an executor's tool that has the name of another of its tools"
                );
                continue;
            }
            taken.push(spec.name.clone());
            if let Some(section) = executor.section() {
                let text = section.text;
                offer.sections.push(InstructionSection::new(
                    executor.kind(),
                    section.position,
                    move |_context| Some(text.clone()),
                ));
            }
            offer.executors.push(OfferedExecutor {
                tool: spec.name.clone(),
                executor,
            });
            offer.definitions.push(executor_definition(spec));
        }
        offer
    }

    /// Where a run's calls go (ADR-0062, decisions 8 and 9): on the ChatGPT
    /// plan, OpenAI's Responses API with the tokens of the account that is
    /// signed in now, its provider-private data bound to that account too;
    /// otherwise the profile's interface, server and credential.
    async fn route_of(
        &self,
        profile: &StoredProfile,
        expected: Option<&AccountKey>,
    ) -> Result<Route, RunFailure> {
        if (profile.access == ProfileAccess::ChatGptPlan) != expected.is_some() {
            return Err(RunFailure::ProfileAccessChanged);
        }
        if let Some(expected) = expected {
            let key = self.chatgpt_key().await?;

            if expected != &key {
                return Err(RunFailure::ChatGptAccountChanged);
            }
            if let Some(account) = &profile.catalog_account
                && (!profile.catalog_available || account != key.as_str())
            {
                return Err(RunFailure::ModelNotFound {
                    model: profile.model.clone(),
                });
            }
            return Ok(Route {
                interface: ApiInterface::Responses,
                base_url: self.chatgpt_endpoints().api_base,
                binding: format!("{}:{}", profile.binding(), key.as_str()),
                credential: CallCredential::ChatGptAccount { key },
            });
        }
        let base_url = check_base_url(&profile.base_url).map_err(|error| {
            tracing::error!(error.code = error.code(), "a stored base URL is not valid");
            RunFailure::Internal
        })?;
        Ok(Route {
            interface: profile.interface,
            base_url,
            credential: CallCredential::Profile,
            binding: profile.binding(),
        })
    }

    /// The target of a run's calls. A profile's own credential is read here,
    /// which may wait for the owner to unlock the credential store; on the
    /// ChatGPT plan each call gets its token instead.
    async fn target_of(
        &self,
        profile: &StoredProfile,
        route: &Route,
    ) -> Result<Target, RunFailure> {
        let target = match route.credential {
            CallCredential::ChatGptAccount { .. } => Target {
                base_url: route.base_url.clone(),
                credential: None,
                quirks: chatgpt_plan_quirks(),
            },
            CallCredential::Profile => Target {
                base_url: route.base_url.clone(),
                credential: self.read_credential(profile).await?,
                quirks: quirks_for(profile.interface, profile.server),
            },
        };
        Ok(target)
    }

    /// The target of one call: the setup's, or on the ChatGPT plan, the
    /// setup's with a token that is valid now.
    pub(crate) async fn call_target(&self, setup: &Setup) -> Result<CallTarget, ProviderError> {
        match &setup.credential {
            CallCredential::Profile => Ok(CallTarget {
                target: setup.target.clone(),
                issued: None,
            }),
            CallCredential::ChatGptAccount { key } => {
                let issued = self.chatgpt_token(key).await?;
                Ok(CallTarget {
                    target: Target {
                        credential: Some(issued.token),
                        ..setup.target.clone()
                    },
                    issued: Some(issued.serial),
                })
            }
        }
    }

    /// The credential of a profile, which may wait for the owner to unlock
    /// the credential store, or none for a profile without one.
    async fn read_credential(&self, profile: &StoredProfile) -> Result<Option<Secret>, RunFailure> {
        let Some(reference) = profile.credential_ref.clone() else {
            return Ok(None);
        };
        let read = in_vault(&self.inner.vault, move |vault| vault.read(&reference)).await;
        read.map(Some).map_err(|error| match error {
            VaultError::Missing => RunFailure::CredentialMissing,
            VaultError::Unavailable(_) => RunFailure::CredentialStoreUnavailable,
        })
    }

    /// The setup of a run of a task that another machine assigned (ADR-0028,
    /// decision 4): the profile that came with the task, whose credential the
    /// vault holds under the task's reference, the tools that the task's mode
    /// allows, and the machine's instructions as they are now.
    pub(crate) async fn assigned_setup(
        &self,
        assigned: &AssignedRun,
        agent: &str,
    ) -> Result<Setup, RunFailure> {
        let Some(ceiling) = self.machine().ceiling.clone() else {
            tracing::error!("a run of an assigned task started on a machine without a ceiling");
            return Err(RunFailure::Internal);
        };
        let snapshot = &assigned.profile;
        let profile = StoredProfile {
            catalog_account: None,
            catalog_available: false,
            advertised_efforts: None,
            id: snapshot.id.clone(),
            name: String::new(),
            // A task's profile is never on the ChatGPT plan, whose tokens are
            // never lent (ADR-0062, decision 13).
            access: ProfileAccess::Endpoint,
            interface: snapshot.interface,
            base_url: snapshot.base_url.clone(),
            model: snapshot.model.clone(),
            credential_ref: snapshot
                .has_credential
                .then(|| lease_reference(&assigned.task)),
            is_default: false,
            config_revision: snapshot.config_revision,
            context_window: snapshot.context_window,
            output_limit: snapshot.output_limit,
            reasoning_effort: snapshot.reasoning_effort,
            use_tools: snapshot.use_tools,
            server: snapshot.server,
            created_at: UtcMillis::EPOCH,
            updated_at: UtcMillis::EPOCH,
        };
        let mut setup = self.setup_for(&profile, agent, true, None).await?;
        let narrower = assigned.read_only.then_some(CeilingMode::ReadOnly);
        let mode = ceiling.mode.narrowed_by(narrower);
        if mode == CeilingMode::ReadOnly {
            // A read-only task is offered only the tools that read, so the
            // model rarely meets a refusal (ADR-0028, decision 5).
            let (tools, definitions): (Vec<_>, Vec<_>) = setup
                .tools
                .into_iter()
                .zip(setup.definitions)
                .filter(|(tool, _definition)| only_reads(tool))
                .unzip();
            setup.tools = tools;
            setup.definitions = definitions;
        }
        setup.assigned = Some(AssignedSetup {
            mode,
            instructions: read_machine_instructions(self.machine().instructions_file.clone()).await,
        });
        Ok(setup)
    }

    /// What a run learns of `authority` for its next call: its attachments
    /// and home, and for a run with tools, its project, the owner's projects
    /// and whether it is in full access.
    async fn conversation_facts(
        &self,
        authority: &ConversationId,
        setup: &Setup,
    ) -> Result<ConversationFacts, CallError> {
        let summary = self.inner.store.conversation_summary(authority).await?;
        let (attachments, home) = summary
            .as_ref()
            .map(|summary| (summary.attachments.clone(), summary.home.clone()))
            .unwrap_or_default();
        if !setup.uses_tools() {
            return Ok(ConversationFacts {
                attachments,
                home,
                ..ConversationFacts::default()
            });
        }
        let projects = self.inner.store.projects().await?;
        Ok(ConversationFacts {
            full_access: summary.as_ref().is_some_and(|summary| summary.full_access),
            project_id: summary.and_then(|summary| summary.project_id),
            projects,
            attachments,
            home,
        })
    }

    /// The next call of a conversation: the instructions, the checkpoint and
    /// the turns after it, and the size of it all. `authority` is the
    /// conversation whose workspace and project the tools work in: the
    /// parent's for a child agent, the conversation itself otherwise.
    pub(crate) async fn prepare_call(
        &self,
        conversation: &ConversationId,
        authority: &ConversationId,
        setup: &Setup,
        options: &CallOptions,
    ) -> Result<Prepared, CallError> {
        let owner = self.owner_instructions_of(setup).await?;
        let checkpoint = self.current_checkpoint(conversation).await?;
        let cursor = checkpoint.as_ref().map(CheckpointCursor::from_stored);
        let after = cursor.as_ref().and_then(CheckpointCursor::load_after_turn);
        let full_turns = self.inner.store.model_turns(conversation, after).await?;
        let (turns, anchor) =
            compaction::uncovered_turns(full_turns.clone(), cursor.as_ref(), setup.budget);
        let ConversationFacts {
            project_id,
            projects,
            full_access,
            attachments,
            home,
        } = self.conversation_facts(authority, setup).await?;
        let project = project_id
            .as_ref()
            .and_then(|id| projects.iter().find(|project| project.id == *id));
        let skills = self.run_skills(setup, project).await;
        // Which tools a call is offered depends on the conversation: a
        // general conversation can move into a project, one in a project
        // cannot (docs/architecture/tools.md, availability), and plugins'
        // tools may need its attachments (ADR-0052).
        let availability = Availability {
            project,
            projects: &projects,
            attachments: &attachments,
            has_home: home.is_some(),
            skills: skills.shelf.has_skills_on(),
        };
        let definitions = setup.definitions_for(&availability);
        let offered: Vec<&str> = definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect();
        let full_context = chosen_context(&turns);
        let attached = self.attached_sources(&attachments, home.as_ref()).await;
        let skills_section = skills.section();
        let mannie_state = self.mannie_state_for_setup(setup);
        let base_context = InstructionContext {
            agent: &setup.agent,
            owner_instructions: owner.as_str(),
            checkpoint: None,
            tools: self.tool_environment(setup, project, &projects, full_access, &offered),
            offers_tasks: setup.offers_tasks(),
            machine: self.machine_context(setup),
            sources: attached.context(!full_context.is_empty()),
            skills: skills_section.as_deref(),
            mannie_state: mannie_state.as_deref(),
        };
        let (base_instructions, instructions) =
            self.call_instructions(base_context, &setup.plugin_sections, checkpoint.as_ref());
        let left_out = left_out_through(&turns, options.prune_through.as_ref());
        let results_pruned = !left_out.is_empty();
        // A model without tools reads the images of results as lines, so a
        // request without tools reads none of their bytes.
        let tool_images = if definitions.is_empty() {
            SentImages::new()
        } else {
            self.sent_tool_images(conversation, &turns, &left_out)
                .await?
        };
        let anchor_ref = anchor.as_ref().map(|(id, text)| (id, text.as_str()));
        let shape = Shaping {
            instructions: &instructions,
            definitions: &definitions,
            left_out: &left_out,
            tool_images: &tool_images,
            full_context: &full_context,
            resumed_turn_anchor: anchor_ref,
        };
        let request = call_request(setup, &shape, &turns);
        let estimate = estimate_request(&request);
        let unpruned_estimate =
            results_pruned.then(|| unpruned_request_estimate(setup, &shape, &turns, &tool_images));
        let ledger = self.ledger_of(conversation).await?;
        let checkpoint_id = checkpoint.as_ref().map(|checkpoint| checkpoint.id.as_str());
        let projected = ledger.project(&setup.binding, checkpoint_id, estimate);
        Ok(Prepared {
            request,
            size: ContextSize {
                budget: setup.budget,
                projected,
                estimate,
            },
            checkpoint,
            turns,
            full_turns,
            base_instructions,
            definitions,
            tool_images,
            results_pruned,
            unpruned_estimate,
            ledger,
        })
    }

    /// Builds the tool environment context section for a call when tools are
    /// enabled; only runs that can move into a project learn the project list.
    fn tool_environment<'a>(
        &'a self,
        setup: &Setup,
        project: Option<&'a ProjectView>,
        projects: &'a [ProjectView],
        full_access: bool,
        offered: &'a [&'a str],
    ) -> Option<ToolEnvironment<'a>> {
        if !setup.uses_tools() {
            return None;
        }
        let movable: &[ProjectView] = if setup.tool(MOVE_TO_PROJECT).is_some() {
            projects
        } else {
            &[]
        };
        let system = &self.inner.tools.system;
        Some(ToolEnvironment {
            system: &system.system,
            shell: system.shell.as_ref().map(|shell| shell.name.as_str()),
            project,
            projects: movable,
            full_access,
            offered,
        })
    }

    /// Assembles the base system instructions (without checkpoint) and the full
    /// instructions (including the active checkpoint summary when present).
    fn call_instructions(
        &self,
        base_context: InstructionContext<'_>,
        plugin_sections: &[InstructionSection],
        checkpoint: Option<&StoredCheckpoint>,
    ) -> (String, String) {
        let base = self
            .inner
            .instructions
            .assemble_with(&base_context, plugin_sections);
        let full = match checkpoint {
            Some(cp) if !cp.summary.trim().is_empty() => {
                let with_checkpoint = InstructionContext {
                    checkpoint: Some(cp.summary.as_str()),
                    ..base_context
                };
                self.inner
                    .instructions
                    .assemble_with(&with_checkpoint, plugin_sections)
            }
            _ => base.clone(),
        };

        (base, full)
    }

    /// The ledger of a conversation's context sizes, as the store keeps it.
    async fn ledger_of(&self, conversation: &ConversationId) -> Result<Ledger, CallError> {
        let stored = self.inner.store.context_ledger(conversation).await?;
        Ok(Ledger::parse(stored.as_deref()))
    }

    fn mannie_state_for_setup(&self, setup: &Setup) -> Option<String> {
        let eligible = setup.assigned.is_none()
            && setup.agent.requester == Requester::Owner
            && setup.tool(MANNIE_INSPECT).is_some();
        if eligible {
            self.mannie_state_section()
        } else {
            None
        }
    }

    /// What the instructions say of a conversation's attachments: the
    /// outlines of what they hold, which the model reads before it searches
    /// them, and the label of its home (ADR-0049, decision 3).
    async fn attached_sources(
        &self,
        attachments: &[Attachment],
        home: Option<&AttachmentRef>,
    ) -> AttachedSources {
        if attachments.is_empty() {
            return AttachedSources::default();
        }
        let outline = self.outline_attachments(attachments).await;
        let home = home.and_then(|home| {
            attachments
                .iter()
                .find(|attachment| {
                    attachment.provider == home.provider && attachment.key == home.key
                })
                .map(|attachment| attachment.label.clone())
        });
        AttachedSources {
            attached: true,
            outline,
            home,
        }
    }

    /// The images of the newest results that a request sends, read from the
    /// store (ADR-0044, decision 5).
    async fn sent_tool_images(
        &self,
        conversation: &ConversationId,
        turns: &[ModelTurn],
        left_out: &HashSet<PartId>,
    ) -> Result<SentImages, CallError> {
        let chosen = chosen_tool_images(turns, left_out);
        if chosen.is_empty() {
            return Ok(SentImages::new());
        }
        let parts: Vec<PartId> = chosen
            .iter()
            .map(|(part, _positions)| part.clone())
            .collect();
        let stored = self.inner.store.tool_images(conversation, &parts).await?;
        let mut sent = SentImages::new();
        for (part, images) in stored {
            let Some((_part, positions)) =
                chosen.iter().find(|(chosen, _positions)| *chosen == part)
            else {
                continue;
            };
            let kept: Vec<(u32, ImageInput)> = images
                .into_iter()
                .filter(|image| positions.contains(&image.position))
                .map(|image| {
                    (
                        image.position,
                        ImageInput {
                            format: image.format,
                            width: image.width,
                            height: image.height,
                            data: image.data,
                        },
                    )
                })
                .collect();
            if !kept.is_empty() {
                sent.insert(part, kept);
            }
        }
        Ok(sent)
    }

    /// For a run of an assigned task, what it works under, as the sections
    /// of its machine show it.
    fn machine_context<'a>(&'a self, setup: &'a Setup) -> Option<MachineContext<'a>> {
        let assigned = setup.assigned.as_ref()?;
        let ceiling = self.machine().ceiling.as_ref()?;
        Some(MachineContext {
            mode: assigned.mode,
            allowed_folders: &ceiling.allowed_folders,
            instructions: assigned.instructions.as_deref(),
        })
    }

    /// The owner's global instructions for a run. An assigned task never
    /// receives them, because they belong to the conversations of the machine
    /// that the owner works on.
    async fn owner_instructions_of(&self, setup: &Setup) -> Result<String, CallError> {
        if setup.assigned.is_some() {
            return Ok(String::new());
        }
        self.owner_instructions().await
    }

    async fn owner_instructions(&self) -> Result<String, CallError> {
        match self.inner.store.load_settings::<ChatSettings>().await {
            Ok(settings) => Ok(settings.instructions.as_str().to_owned()),
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), "could not read the owner's instructions");
                Err(CallError::Failed(RunFailure::Internal))
            }
        }
    }

    /// The newest checkpoint whose hash still matches the turns or step prefix
    /// it covers. A checkpoint is checked once per process.
    async fn current_checkpoint(
        &self,
        conversation: &ConversationId,
    ) -> Result<Option<StoredCheckpoint>, RuntimeStoreError> {
        let checkpoints = self.inner.store.checkpoints(conversation).await?;
        for (index, checkpoint) in checkpoints.iter().enumerate() {
            if lock(&self.inner.verified_checkpoints).contains(&checkpoint.id) {
                return Ok(Some(checkpoint.clone()));
            }
            let previous = checkpoints.get(index + 1);
            let previous_cursor = previous.map(CheckpointCursor::from_stored);
            let this_cursor = CheckpointCursor::from_stored(checkpoint);
            let after = previous_cursor
                .as_ref()
                .and_then(CheckpointCursor::load_after_turn);
            let turns = self.inner.store.model_turns(conversation, after).await?;
            let covered = compaction::slice_between(&turns, previous_cursor.as_ref(), &this_cursor);
            let hash_matches = covered.is_some_and(|covered| {
                let hash = compaction::content_hash(
                    previous.map(|previous| previous.content_hash.as_str()),
                    &covered,
                );
                hash == checkpoint.content_hash
            });
            if hash_matches {
                lock(&self.inner.verified_checkpoints).insert(checkpoint.id.clone());
                return Ok(Some(checkpoint.clone()));
            }
            tracing::warn!(
                conversation = %conversation,
                checkpoint = %checkpoint.id,
                "a checkpoint no longer matches the turns it covers; the one before it is used"
            );
        }
        Ok(None)
    }

    /// Plans a compaction for `prepared` without performing any model call or
    /// store write, so callers avoid switching UI status when nothing is worth
    /// compacting (ADR-0083).
    pub(crate) fn plan_checkpoint(
        setup: &Setup,
        prepared: &Prepared,
        trigger: Trigger,
        size: ContextSize,
        allow_active_turn_end: bool,
    ) -> Option<Plan> {
        let allowance = compaction::summary_allowance(setup.budget, setup.output_limit);
        compaction::plan_compaction(
            &prepared.turns,
            size,
            allowance,
            trigger,
            allow_active_turn_end,
            |cursor| estimate_after_cursor(setup, prepared, cursor, None),
        )
    }

    /// Summarizes the turns or step prefix in `plan`, measures the actual
    /// post-compaction context estimate, and stores the checkpoint. The caller
    /// holds a model slot.
    pub(crate) async fn write_checkpoint_plan(
        &self,
        conversation: &ConversationId,
        setup: &Setup,
        prepared: &Prepared,
        plan: Plan,
        trigger: Trigger,
        size: ContextSize,
    ) -> Result<(), CallError> {
        let allowance = compaction::summary_allowance(setup.budget, setup.output_limit);
        let previous = prepared.checkpoint.as_ref();
        let summary = self
            .summarize(
                setup,
                previous.map(|previous| previous.summary.as_str()),
                &plan.covered,
                allowance,
            )
            .await?;
        let id = self.inner.ids.checkpoint();
        let ledger = prepared
            .ledger
            .after_checkpoint(&setup.binding, id.as_str());
        let actual_estimate =
            estimate_after_cursor(setup, prepared, &plan.cursor, Some(summary.as_str()));
        let tokens_after = ledger.project(&setup.binding, Some(id.as_str()), actual_estimate);
        let changes = vec![
            Change::AddCheckpoint {
                checkpoint: NewCheckpoint {
                    id: id.clone(),
                    covers_through_turn: plan.cursor.turn,
                    covers_through_part: plan.cursor.part.clone(),
                    content_hash: compaction::content_hash(
                        previous.map(|previous| previous.content_hash.as_str()),
                        &plan.covered,
                    ),
                    summary,
                    tokens_before: size.projected,
                    tokens_after,
                    trigger: trigger.as_str().to_owned(),
                },
            },
            Change::UpdateContext {
                ledger: ledger.to_json(),
                usage: usage(tokens_after, setup.budget, true),
            },
        ];
        self.inner
            .hubs
            .apply(&self.inner.store, conversation, changes, Durability::Full)
            .await?;
        lock(&self.inner.verified_checkpoints).insert(id.clone());
        tracing::info!(
            conversation = %conversation,
            checkpoint = %id,
            turns = plan.covered.len(),
            covers_through_turn = plan.cursor.turn,
            step_boundary = plan.cursor.part.is_some(),
            trigger = trigger.as_str(),
            tokens_before = size.projected,
            tokens_after,
            "wrote a checkpoint"
        );
        Ok(())
    }

    /// Summarizes turns, folding them in chunks that each fit one call.
    async fn summarize(
        &self,
        setup: &Setup,
        previous: Option<&str>,
        covered: &[ModelTurn],
        allowance: u32,
    ) -> Result<String, CallError> {
        let overhead = compaction::request_overhead(previous, allowance);
        let chunk_tokens = setup
            .budget
            .saturating_sub(allowance)
            .saturating_sub(overhead)
            .max(MIN_CHUNK_TOKENS);
        let rendered = compaction::render_turns(covered, setup.requester(), chunk_tokens);
        let mut summary = previous.map(str::to_owned);
        for chunk in compaction::chunks(&rendered, chunk_tokens) {
            let chunk = compaction::fit(&chunk, chunk_tokens);
            let request = compaction::summary_request(
                &setup.model,
                summary.as_deref(),
                &chunk,
                allowance,
                setup.summary_reasoning_effort,
            );
            summary = Some(self.summary_call(setup, request).await?);
        }
        summary
            .filter(|summary| !summary.trim().is_empty())
            .ok_or(CallError::Failed(RunFailure::InvalidResponse))
    }

    /// One summary call, retried like the calls of a run. On the ChatGPT
    /// plan each try gets a token that is valid at its moment, and a refused
    /// token is refreshed once (ADR-0062, decision 8).
    async fn summary_call(
        &self,
        setup: &Setup,
        request: ModelRequest,
    ) -> Result<String, CallError> {
        let policy = self.inner.config.retry;
        let mut tries = 0;
        let mut refused_once = false;
        loop {
            tries += 1;
            let mut text = String::new();
            let (ended, issued) = match self.call_target(setup).await {
                Err(error) => (Err(error), None),
                Ok(call) => {
                    let mut stream = setup.adapter.stream(&call.target, request.clone());
                    let ended = loop {
                        match stream.next().await {
                            Some(Ok(ProviderEvent::TextDelta(delta))) => text.push_str(&delta),
                            Some(Ok(ProviderEvent::Finished(finish))) => break Ok(finish),
                            Some(Ok(_)) => {}
                            Some(Err(error)) => break Err(error),
                            None => {
                                break Err(ProviderError::Transient {
                                    cause: RunFailure::BrokenStream,
                                    retry_after: None,
                                });
                            }
                        }
                    };
                    (ended, call.issued)
                }
            };
            match ended {
                // A summary cut at the output limit lost its end, where the
                // latest of what it covers and the next steps go, and a model
                // that reasons may have spent the limit before it wrote
                // anything. It never becomes a checkpoint.
                Ok(FinishReason::OutputLimit) => {
                    tracing::warn!("a summary reached the output limit; no checkpoint is written");
                    return Err(CallError::Failed(RunFailure::SummaryCut));
                }
                Ok(_finish) => return Ok(fitted_summary(setup, &request, text.trim())),
                Err(ProviderError::Permanent(RunFailure::InvalidCredential))
                    if issued.is_some() && !refused_once =>
                {
                    refused_once = true;
                    self.reject_chatgpt_token(issued);
                }
                Err(ProviderError::Transient { cause, retry_after }) => {
                    match policy.decide(tries, retry_after, self.random()) {
                        RetryDecision::After(delay) => tokio::time::sleep(delay).await,
                        RetryDecision::GiveUp => return Err(CallError::Failed(cause)),
                    }
                }
                Err(error) => return Err(CallError::Failed(error.failure())),
            }
        }
    }
}

/// The smallest transcript chunk of a summary call.
const MIN_CHUNK_TOKENS: u32 = 256;

/// A summary that fits its allowance. The ChatGPT plan takes no output limit,
/// so a summary there can run longer than its request allows; it is cut to
/// its head and tail, as a long chunk is (ADR-0062, decision 10).
fn fitted_summary(setup: &Setup, request: &ModelRequest, summary: &str) -> String {
    let unbounded = setup.target.quirks.responses == ResponsesShape::ChatGptPlan;
    if unbounded && estimate_text(summary) > request.max_output_tokens {
        tracing::warn!("a summary on the ChatGPT plan was longer than its allowance and was cut");
        return compaction::fit(summary, request.max_output_tokens);
    }
    summary.to_owned()
}

/// The machine's instructions, read for every assigned task so a change
/// applies from the next task. A missing file means that there are none; a
/// file that cannot be read leaves them out, with a warning in the log, and a
/// longer one is cut.
async fn read_machine_instructions(path: Option<PathBuf>) -> Option<String> {
    let path = path?;
    let read = tokio::task::spawn_blocking(move || std::fs::read(&path)).await;
    let bytes = match read {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Ok(Err(error)) => {
            tracing::warn!(%error, "could not read the machine's instructions; the task runs without them");
            return None;
        }
        Err(error) => {
            tracing::warn!(%error, "could not read the machine's instructions; the task runs without them");
            return None;
        }
    };
    if bytes.len() > MAX_MACHINE_INSTRUCTIONS_BYTES {
        tracing::warn!(
            bytes = bytes.len(),
            limit = MAX_MACHINE_INSTRUCTIONS_BYTES,
            "the machine's instructions are longer than the limit; the rest is left out"
        );
    }
    let mut end = bytes.len().min(MAX_MACHINE_INSTRUCTIONS_BYTES);
    // A cut in the middle of a character is moved back to its start.
    while end > 0 && end < bytes.len() && (bytes[end] & 0b1100_0000) == 0b1000_0000 {
        end -= 1;
    }
    let text = String::from_utf8_lossy(&bytes[..end]).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// What a call's request is built from, besides its run's setup and the
/// turns.
struct Shaping<'a> {
    instructions: &'a str,
    definitions: &'a [ToolDefinition],
    /// The older results of the latest turn, left out when the context runs
    /// low.
    left_out: &'a HashSet<PartId>,
    /// The images of the newest results that the request sends.
    tool_images: &'a SentImages,
    /// The parts `context` whose excerpts the request sends in full.
    full_context: &'a HashSet<PartId>,
    /// When a step-boundary checkpoint ends inside a turn, the user anchor
    /// message emitted before that turn's remaining steps.
    resumed_turn_anchor: Option<(&'a mannie_types::ids::TurnId, &'a str)>,
}

/// The request of a call: its instructions, the tools that it offers and the
/// turns that the model reads, with the results and the images that the
/// shaping leaves out.
fn call_request(setup: &Setup, shaping: &Shaping<'_>, turns: &[ModelTurn]) -> ModelRequest {
    let shape = RequestShape {
        instructions: shaping.instructions,
        model: &setup.model,
        interface: setup.interface,
        binding: &setup.binding,
        max_output_tokens: setup.output_limit,
        reasoning_effort: setup.reasoning_effort,
        tools: shaping.definitions,
        tool_choice: ToolChoice::Auto,
        left_out: shaping.left_out,
        requester: setup.requester(),
        tool_images: shaping.tool_images,
        full_context: shaping.full_context,
        resumed_turn_anchor: shaping.resumed_turn_anchor,
    };
    build_request(shape, turns)
}

/// Estimates the request that would remain after cutting at `cursor`, with an
/// optional newly written checkpoint summary included in the instructions.
fn estimate_after_cursor(
    setup: &Setup,
    prepared: &Prepared,
    cursor: &CheckpointCursor,
    summary: Option<&str>,
) -> u32 {
    let (remaining, anchor) =
        compaction::uncovered_turns(prepared.full_turns.clone(), Some(cursor), setup.budget);
    let full_context = chosen_context(&remaining);
    let empty_left_out = HashSet::new();
    let instructions = match summary.map(str::trim).filter(|text| !text.is_empty()) {
        Some(summary) if prepared.base_instructions.is_empty() => {
            format!("{}\n\n{summary}", texts::CHECKPOINT_HEADING)
        }
        Some(summary) => format!(
            "{}\n\n{}\n\n{summary}",
            prepared.base_instructions,
            texts::CHECKPOINT_HEADING
        ),
        None => prepared.base_instructions.clone(),
    };
    let anchor_ref = anchor.as_ref().map(|(id, text)| (id, text.as_str()));
    let shape = Shaping {
        instructions: &instructions,
        definitions: &prepared.definitions,
        left_out: &empty_left_out,
        tool_images: &prepared.tool_images,
        full_context: &full_context,
        resumed_turn_anchor: anchor_ref,
    };
    let request = call_request(setup, &shape, &remaining);
    estimate_request(&request)
}

/// Extra image tokens of unpruned results that were not loaded into
/// `loaded_images` because they had been pruned from the active call.
fn unpruned_image_tokens(turns: &[ModelTurn], loaded_images: &SentImages) -> u32 {
    let chosen = chosen_tool_images(turns, &HashSet::new());
    let mut extra = 0_u32;
    for (part_id, positions) in chosen {
        let already = loaded_images.get(&part_id);
        let Some(part) = turns
            .iter()
            .flat_map(|turn| &turn.parts)
            .find(|part| part.id == part_id)
        else {
            continue;
        };
        let images = part
            .tool_call
            .as_ref()
            .and_then(|call| call.result.as_ref())
            .map_or(&[][..], |result| result.images.as_slice());
        for position in positions {
            let loaded = already.is_some_and(|list| list.iter().any(|(pos, _)| *pos == position));
            if loaded {
                continue;
            }
            if let Some(image) = images.iter().find(|image| image.position == position) {
                extra = extra.saturating_add(estimate_scaled_image(image.width, image.height));
            }
        }
    }
    extra
}

/// Advances the epoch tool-result pruning watermark for the latest turn in a
/// coarse batch until calibrated context fits within the compaction target
/// (keeping at least the newest [`KEPT_RESULTS`] results), so the pruned prefix
/// stays stable across subsequent steps for prompt caching (ADR-0083).
pub(crate) fn next_prune_watermark(
    setup: &Setup,
    prepared: &Prepared,
    current: Option<&PartId>,
) -> Option<PartId> {
    let prunable = prunable_results(&prepared.turns);
    let start_index = match current {
        Some(mark) => prunable
            .iter()
            .position(|id| *id == mark)
            .map_or(0, |pos| pos + 1),
        None => 0,
    };
    if start_index >= prunable.len() {
        return None;
    }

    let target = compaction::share(setup.budget, compaction::TARGET_PERCENT);
    let full_context = chosen_context(&prepared.turns);
    let anchor = prepared
        .checkpoint
        .as_ref()
        .map(CheckpointCursor::from_stored)
        .filter(|cursor| cursor.part.is_some())
        .and_then(|cursor| {
            compaction::uncovered_turns(prepared.full_turns.clone(), Some(&cursor), setup.budget).1
        });
    let anchor_ref = anchor.as_ref().map(|(id, text)| (id, text.as_str()));

    for candidate_index in start_index..prunable.len() {
        let left_out: HashSet<PartId> = prunable[..=candidate_index]
            .iter()
            .map(|id| (*id).clone())
            .collect();
        let shape = Shaping {
            instructions: &prepared.request.instructions,
            definitions: &prepared.definitions,
            left_out: &left_out,
            tool_images: &prepared.tool_images,
            full_context: &full_context,
            resumed_turn_anchor: anchor_ref,
        };
        let req = call_request(setup, &shape, &prepared.turns);
        let est = estimate_request(&req);
        let checkpoint_id = prepared
            .checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.id.as_str());
        let projected = prepared.ledger.project(&setup.binding, checkpoint_id, est);
        if projected <= target {
            return Some(prunable[candidate_index].clone());
        }
    }

    prunable.last().map(|id| (*id).clone())
}

/// The most results whose images a request sends, and the most images in all
/// (ADR-0044, decision 5). Every request sends again what the context holds,
/// so older images become lines instead of being sent again and again.
const MAX_SENT_IMAGE_RESULTS: usize = 3;
const MAX_SENT_TOOL_IMAGES: usize = 8;

/// Which images of results a request sends, by part and position: those of
/// the newest tool calls whose results hold images and that the context
/// keeps whole, until [`MAX_SENT_IMAGE_RESULTS`] results or
/// [`MAX_SENT_TOOL_IMAGES`] images.
fn chosen_tool_images(turns: &[ModelTurn], left_out: &HashSet<PartId>) -> Vec<(PartId, Vec<u32>)> {
    let mut chosen = Vec::new();
    let mut room = MAX_SENT_TOOL_IMAGES;
    let newest_first = turns.iter().rev().flat_map(|turn| turn.parts.iter().rev());
    for part in newest_first {
        if chosen.len() == MAX_SENT_IMAGE_RESULTS || room == 0 {
            break;
        }
        if part.kind != Some(PartKind::ToolCall) || left_out.contains(&part.id) {
            continue;
        }
        let images = part
            .tool_call
            .as_ref()
            .and_then(|call| call.result.as_ref())
            .map_or(&[][..], |result| result.images.as_slice());
        if images.is_empty() {
            continue;
        }
        let positions: Vec<u32> = images
            .iter()
            .take(room)
            .map(|image| image.position)
            .collect();
        room -= positions.len();
        chosen.push((part.id.clone(), positions));
    }
    chosen
}

/// Estimates the full unpruned token count of a call when tool results are
/// currently trimmed by an epoch watermark.
fn unpruned_request_estimate(
    setup: &Setup,
    shape: &Shaping<'_>,
    turns: &[ModelTurn],
    tool_images: &SentImages,
) -> u32 {
    let empty_left_out = HashSet::new();
    let unpruned_shape = Shaping {
        left_out: &empty_left_out,
        ..*shape
    };
    let unpruned_req = call_request(setup, &unpruned_shape, turns);
    let extra_images = unpruned_image_tokens(turns, tool_images);

    estimate_request(&unpruned_req).saturating_add(extra_images)
}

/// Eligible completed tool and task calls of the latest turn whose results can
/// be pruned, in order, excluding the newest [`KEPT_RESULTS`], owner question
/// replies (ADR-0058, decision 7), and saved plans (ADR-0060).
fn prunable_results(turns: &[ModelTurn]) -> Vec<&PartId> {
    let Some(latest) = turns.last() else {
        return Vec::new();
    };
    let completed: Vec<&PartId> = latest
        .parts
        .iter()
        .filter(|part| match part.kind {
            Some(PartKind::ToolCall) => true,
            Some(PartKind::Task) => part.task.as_ref().is_some_and(tasks::returned_reports),
            _ => false,
        })
        .filter(|part| {
            part.tool_call.as_ref().is_some_and(|call| {
                call.status == ToolCallStatus::Completed
                    && !questions::holds_reply(call)
                    && !plans::holds_plan(call)
            })
        })
        .map(|part| &part.id)
        .collect();
    let older = completed.len().saturating_sub(KEPT_RESULTS);
    completed[..older].to_vec()
}

/// The prunable calls of the latest turn up through `watermark` (inclusive).
fn left_out_through(turns: &[ModelTurn], watermark: Option<&PartId>) -> HashSet<PartId> {
    let Some(mark) = watermark else {
        return HashSet::new();
    };
    let prunable = prunable_results(turns);
    let Some(end) = prunable.iter().position(|id| *id == mark) else {
        return HashSet::new();
    };
    prunable[..=end].iter().map(|id| (*id).clone()).collect()
}

#[cfg(test)]
mod tests {
    use mannie_store::{ModelPart, ModelTurn};
    use mannie_types::ids::{PartId, TurnId};
    use mannie_types::provider::ProfileAccess;
    use mannie_types::tools::ToolResult;
    use mannie_types::tools::{QuestionAnswer, ToolCallDocument, ToolCallStatus, ToolOutcome};
    use mannie_types::transcript::{PartKind, PartState};

    use super::{left_out_through, profile_budget, prunable_results};

    fn part_id(number: u32) -> PartId {
        PartId::parse(&format!("0199e0f2-0000-7000-8000-{number:012}")).unwrap()
    }

    /// A completed call of the latest turn with this outcome.
    fn completed(number: u32, outcome: ToolOutcome) -> ModelPart {
        let mut call = ToolCallDocument::receiving(format!("call_{number}"), "tool".to_owned());
        call.status = ToolCallStatus::Completed;
        call.result = Some(ToolResult {
            outcome,
            output: "The result.".to_owned(),
            archives: Vec::new(),
            images: Vec::new(),
        });
        ModelPart {
            id: part_id(number),
            run_id: None,
            kind: Some(PartKind::ToolCall),
            kind_name: "tool_call".to_owned(),
            state: PartState::Complete,
            text: "{}".to_owned(),
            provider_item: None,
            tool_call: Some(call),
            task: None,
            attempt: 1,
            image: None,
            context: None,
        }
    }

    #[test]
    fn the_owners_replies_stay_while_older_results_are_prunable() {
        let answered = ToolOutcome::QuestionAnswered {
            answers: vec![QuestionAnswer {
                selected: vec![0],
                own_words: None,
            }],
        };
        let turn = ModelTurn {
            turn_id: TurnId::parse("0199e0f2-0000-7000-8000-000000000100").unwrap(),
            position: 0,
            run_id: None,
            run_state: None,
            kept_runs: Vec::new(),
            parts: vec![
                completed(1, answered),
                completed(2, ToolOutcome::QuestionDeclined),
                completed(3, ToolOutcome::Unknown),
                completed(4, ToolOutcome::Unknown),
                completed(5, ToolOutcome::Unknown),
                completed(6, ToolOutcome::Unknown),
                completed(7, ToolOutcome::Unknown),
            ],
        };

        let turns = [turn];
        let prunable = prunable_results(&turns);

        assert_eq!(prunable, vec![&part_id(3), &part_id(4)]);
        let first_epoch = left_out_through(&turns, Some(&part_id(3)));
        assert_eq!(first_epoch.len(), 1);
        assert!(first_epoch.contains(&part_id(3)));
        let second_epoch = left_out_through(&turns, Some(&part_id(4)));
        assert_eq!(second_epoch.len(), 2);
        assert!(second_epoch.contains(&part_id(3)));
        assert!(second_epoch.contains(&part_id(4)));
    }

    #[test]
    fn chatgpt_plan_profile_caps_output_reserve_in_budget() {
        // 400K window with 128K output limit: endpoint keeps full 128K reserve
        // (252K budget), while ChatGPT plan caps reserve at 32,768 (347,232).
        assert_eq!(
            profile_budget(ProfileAccess::Endpoint, 400_000, 128_000),
            252_000
        );
        assert_eq!(
            profile_budget(ProfileAccess::ChatGptPlan, 400_000, 128_000),
            347_232
        );
        // Smaller output limits stay unchanged on both access modes.
        assert_eq!(
            profile_budget(ProfileAccess::ChatGptPlan, 128_000, 16_384),
            profile_budget(ProfileAccess::Endpoint, 128_000, 16_384)
        );
    }
}
