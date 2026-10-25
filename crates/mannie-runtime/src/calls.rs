//! Preparing model calls (docs/architecture/agent-runtime.md, the loop): what
//! a run reads once at its start, the context of each call, and the calls
//! that write checkpoints.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use futures_util::StreamExt;
use mannie_plugin_sdk::points::executors::{Executors, TaskExecutor};
use mannie_plugin_sdk::points::tools::ToolSpec;
use mannie_provider::{
    ImageInput, ModelRequest, Provider, ProviderError, ProviderEvent, Target, ToolChoice,
    ToolDefinition, check_base_url, quirks_for,
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
use mannie_types::provider::{ApiInterface, ReasoningEffort, context_budget};
use mannie_types::run::{FinishReason, RunFailure};
use mannie_types::secret::Secret;
use mannie_types::tools::ToolCallStatus;
use mannie_types::transcript::PartKind;

use crate::agents::{AgentDefinition, Requester};
use crate::assigned::{AssignedRun, lease_reference};
use crate::budget::{Ledger, estimate_request, usage};
use crate::compaction::{self, ContextSize, Trigger};
use crate::context::{RequestShape, SentImages, build_request, chosen_context};
use crate::instructions::{
    InstructionContext, InstructionSection, MachineContext, SourcesContext, ToolEnvironment,
    plugin_sections,
};
use crate::plugin_tools::{OfferedPluginTool, offered_tools};
use crate::plugins::Snapshot;
use crate::profiles::{VaultError, in_vault};
use crate::retry::RetryDecision;
use crate::{Runtime, lock, tasks, texts};

/// The newest tool results of the latest turn that stay in the context when
/// older ones are left out to save space.
const KEPT_RESULTS: usize = 3;

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

/// What the calls of a run use, read once when the run starts.
#[derive(Debug, Clone)]
pub(crate) struct Setup {
    /// The agent that makes the calls.
    pub(crate) agent: AgentDefinition,
    /// The adapter of the profile's interface.
    pub(crate) adapter: Arc<dyn Provider>,
    /// The server, the credential and the server's quirks.
    pub(crate) target: Target,
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
    /// The child agents that `start_task` can start.
    pub(crate) child_agents: Vec<&'static str>,
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
        !self.tools.is_empty() || self.offers_tasks()
    }

    /// Whether the run is offered the task tools.
    pub(crate) fn offers_tasks(&self) -> bool {
        !self.task_definitions.is_empty()
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
    /// works, followed by the task tools.
    fn definitions_for(&self, availability: &Availability<'_>) -> Vec<ToolDefinition> {
        let mut definitions: Vec<ToolDefinition> = self
            .tools
            .iter()
            .zip(&self.definitions)
            .filter(|(tool, _definition)| tool.offered(availability))
            .map(|(_tool, definition)| definition.clone())
            .collect();
        definitions.extend(self.task_definitions.iter().cloned());
        definitions
    }
}

/// How a call is prepared.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CallOptions {
    /// The reply used up its tool calls: the model may not call tools, and a
    /// note asks it to answer.
    pub(crate) tools_used_up: bool,
    /// Even after compaction the context runs low: the latest turn's older
    /// tool results are left out (docs/features/system-tools/tool-calls.md,
    /// context).
    pub(crate) leave_out_results: bool,
}

/// One call, ready to be made.
#[derive(Debug, Clone)]
pub(crate) struct Prepared {
    /// The request.
    pub(crate) request: ModelRequest,
    /// The size of the request's context.
    pub(crate) size: ContextSize,
    /// The checkpoint that stands in for older turns.
    pub(crate) checkpoint: Option<StoredCheckpoint>,
    /// The turns after the checkpoint, as the model reads them.
    pub(crate) turns: Vec<ModelTurn>,
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
    ) -> Result<Setup, RunFailure> {
        let profile = match self.inner.store.profile(profile_id).await {
            Ok(Some(profile)) => profile,
            Ok(None) => return Err(RunFailure::ProfileMissing),
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), "could not read the profile of a run");
                return Err(RunFailure::Internal);
            }
        };
        self.setup_for(&profile, agent, task).await
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
    ) -> Result<Setup, RunFailure> {
        let snapshot = self.contributions();
        let Some(agent) = self.agent_in(&snapshot, agent) else {
            tracing::error!(agent, "a run names an agent that is not registered");
            return Err(RunFailure::Internal);
        };
        let Some(adapter) = self.inner.registry.adapter(profile.interface) else {
            tracing::error!(
                interface = profile.interface.as_str(),
                "no adapter is registered for the profile's interface"
            );
            return Err(RunFailure::Internal);
        };
        let base_url = check_base_url(&profile.base_url).map_err(|error| {
            tracing::error!(error.code = error.code(), "a stored base URL is not valid");
            RunFailure::Internal
        })?;
        let (plugin_tools, adapters) = if profile.use_tools {
            self.plugin_tools_for(&snapshot, &agent)
        } else {
            (Vec::new(), Vec::new())
        };
        let tools: Vec<Arc<dyn Tool>> = if profile.use_tools {
            with_follow_ups_only_with_starts(
                agent
                    .tools
                    .iter()
                    .filter_map(|name| self.inner.tools.registry.get(name))
                    .chain(adapters)
                    .collect(),
            )
        } else {
            Vec::new()
        };
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
        let child_agents = self.child_agent_names(&snapshot);
        // A child never gives tasks of its own (T-22), and an agent without a
        // task budget or anyone to give tasks to is not offered the tools.
        let offers_tasks =
            profile.use_tools && !task && agent.max_tasks > 0 && !child_agents.is_empty();
        let mut task_definitions: Vec<ToolDefinition> = if offers_tasks {
            tasks::definitions(&child_agents)
                .into_iter()
                .filter(|definition| agent.tools.contains(&definition.name.as_str()))
                .collect()
        } else {
            Vec::new()
        };
        let mut executors = Vec::new();
        let mut plugin_sections = plugin_sections(&snapshot);
        if offers_tasks {
            let offered_names: Vec<String> = definitions
                .iter()
                .chain(&task_definitions)
                .map(|definition| definition.name.clone())
                .collect();
            let offer = Self::executor_offer(&snapshot, offered_names);
            executors = offer.executors;
            task_definitions.extend(offer.definitions);
            plugin_sections.extend(offer.sections);
        }
        let credential = self.read_credential(profile).await?;
        Ok(Setup {
            agent,
            adapter,
            target: Target {
                base_url,
                credential,
                quirks: quirks_for(profile.interface, profile.server),
            },
            interface: profile.interface,
            model: profile.model.clone(),
            binding: profile.binding(),
            output_limit: profile.output_limit,
            budget: context_budget(profile.context_window, profile.output_limit),
            reasoning_effort: profile.reasoning_effort,
            tools,
            definitions,
            task_definitions,
            child_agents,
            assigned: None,
            executors,
            plugin_tools,
            plugin_sections,
        })
    }

    /// The plugins' tools that join the core's that the agent names, each
    /// for the agents that its selector offers it to (ADR-0034, decision 6),
    /// within the room of a request beside the core's tools, the task tools
    /// and the executors' tools, which join later (ADR-0043, decision 2).
    fn plugin_tools_for(
        &self,
        snapshot: &Snapshot,
        agent: &AgentDefinition,
    ) -> (Vec<OfferedPluginTool>, Vec<Arc<dyn Tool>>) {
        let core = agent
            .tools
            .iter()
            .filter(|name| self.inner.tools.registry.get(name).is_some())
            .count();
        let others = core + tasks::TASK_TOOLS.len() + snapshot.items::<Executors>().len();
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
            id: snapshot.id.clone(),
            name: String::new(),
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
        let mut setup = self.setup_for(&profile, agent, true).await?;
        let narrower = assigned.read_only.then_some(CeilingMode::ReadOnly);
        let mode = ceiling.mode.narrowed_by(narrower);
        if mode == CeilingMode::ReadOnly {
            // A read-only task is offered only the tools that read, so the
            // model rarely meets a refusal (ADR-0028, decision 5). It starts
            // no background command, so it reads none either (ADR-0041).
            let (tools, definitions): (Vec<_>, Vec<_>) = setup
                .tools
                .into_iter()
                .zip(setup.definitions)
                .filter(|(tool, _definition)| tool.class() == PermissionClass::ReadOnly)
                .filter(|(tool, _definition)| !COMMAND_FOLLOW_UPS.contains(&tool.name()))
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
        options: CallOptions,
    ) -> Result<Prepared, CallError> {
        // An assigned task never receives the owner's global instructions,
        // which belong to the conversations of the machine that the owner
        // works on.
        let owner = if setup.assigned.is_some() {
            String::new()
        } else {
            self.owner_instructions().await?
        };
        let checkpoint = self.current_checkpoint(conversation).await?;
        let covered = checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.covers_through_turn);
        let turns = self.inner.store.model_turns(conversation, covered).await?;
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
        // Which tools a call is offered depends on the conversation: a
        // general conversation can move into a project, one in a project
        // cannot (docs/architecture/tools.md, availability), and plugins'
        // tools may need its attachments (ADR-0052).
        let availability = Availability {
            project,
            projects: &projects,
            attachments: &attachments,
            has_home: home.is_some(),
        };
        let definitions = setup.definitions_for(&availability);
        let offered: Vec<&str> = definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect();
        // Only a run that can move into a project learns the owner's
        // projects; a child works in its parent's workspace.
        let movable: &[_] = if setup.tool(MOVE_TO_PROJECT).is_some() {
            &projects
        } else {
            &[]
        };
        let system = &self.inner.tools.system;
        let full_context = chosen_context(&turns);
        let attached = self.attached_sources(&attachments, home.as_ref()).await;
        let context = InstructionContext {
            agent: &setup.agent,
            owner_instructions: owner.as_str(),
            checkpoint: checkpoint
                .as_ref()
                .map(|checkpoint| checkpoint.summary.as_str()),
            tools: setup.uses_tools().then(|| ToolEnvironment {
                system: &system.system,
                shell: system.shell.as_ref().map(|shell| shell.name.as_str()),
                project,
                projects: movable,
                full_access,
                offered: &offered,
            }),
            offers_tasks: setup.offers_tasks(),
            machine: self.machine_context(setup),
            sources: attached.context(!full_context.is_empty()),
        };
        let instructions = self
            .inner
            .instructions
            .assemble_with(&context, &setup.plugin_sections);
        let left_out = if options.leave_out_results {
            older_results(&turns)
        } else {
            HashSet::new()
        };
        // A model without tools reads the images of results as lines, so a
        // request without tools reads none of their bytes.
        let tool_images = if definitions.is_empty() {
            SentImages::new()
        } else {
            self.sent_tool_images(conversation, &turns, &left_out)
                .await?
        };
        let shape = Shaping {
            options,
            instructions: &instructions,
            definitions: &definitions,
            left_out: &left_out,
            tool_images: &tool_images,
            full_context: &full_context,
        };
        let request = call_request(setup, &shape, &turns);
        let estimate = estimate_request(&request);
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
            ledger,
        })
    }

    /// The ledger of a conversation's context sizes, as the store keeps it.
    async fn ledger_of(&self, conversation: &ConversationId) -> Result<Ledger, CallError> {
        let stored = self.inner.store.context_ledger(conversation).await?;
        Ok(Ledger::parse(stored.as_deref()))
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

    async fn owner_instructions(&self) -> Result<String, CallError> {
        match self.inner.store.load_settings::<ChatSettings>().await {
            Ok(settings) => Ok(settings.instructions.as_str().to_owned()),
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), "could not read the owner's instructions");
                Err(CallError::Failed(RunFailure::Internal))
            }
        }
    }

    /// The newest checkpoint whose hash still matches the turns it covers. A
    /// checkpoint is checked once per process.
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
            let after = previous.map(|previous| previous.covers_through_turn);
            let turns = self.inner.store.model_turns(conversation, after).await?;
            let covered: Vec<ModelTurn> = turns
                .into_iter()
                .filter(|turn| turn.position <= checkpoint.covers_through_turn)
                .collect();
            let hash = compaction::content_hash(
                previous.map(|previous| previous.content_hash.as_str()),
                &covered,
            );
            if hash == checkpoint.content_hash {
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

    /// Summarizes the turns that a compaction plan covers and stores the
    /// checkpoint. `Ok(false)` when nothing is worth compacting. The caller
    /// holds a model slot.
    pub(crate) async fn write_checkpoint(
        &self,
        conversation: &ConversationId,
        setup: &Setup,
        prepared: &Prepared,
        trigger: Trigger,
        size: ContextSize,
    ) -> Result<bool, CallError> {
        // The latest turn holds the request being answered, which is never
        // compacted.
        let candidates = prepared
            .turns
            .split_last()
            .map_or(&[][..], |(_latest, earlier)| earlier);
        let allowance = compaction::summary_allowance(setup.budget, setup.output_limit);
        let Some(plan) = compaction::plan(candidates, size, allowance, trigger, setup.requester())
        else {
            return Ok(false);
        };
        let covered = &candidates[..plan.covered];
        let Some(last) = covered.last() else {
            return Ok(false);
        };
        let previous = prepared.checkpoint.as_ref();
        let summary = self
            .summarize(
                setup,
                previous.map(|previous| previous.summary.as_str()),
                covered,
                allowance,
            )
            .await?;
        let id = self.inner.ids.checkpoint();
        let ledger = Ledger {
            binding: Some(setup.binding.clone()),
            checkpoint: Some(id.as_str().to_owned()),
            reported_input: None,
            estimated_input: None,
        };
        let changes = vec![
            Change::AddCheckpoint {
                checkpoint: NewCheckpoint {
                    id: id.clone(),
                    covers_through_turn: last.position,
                    content_hash: compaction::content_hash(
                        previous.map(|previous| previous.content_hash.as_str()),
                        covered,
                    ),
                    summary,
                    tokens_before: size.projected,
                    tokens_after: plan.after,
                    trigger: trigger.as_str().to_owned(),
                },
            },
            Change::UpdateContext {
                ledger: ledger.to_json(),
                usage: usage(plan.after, setup.budget, true),
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
            turns = covered.len(),
            trigger = trigger.as_str(),
            tokens_before = size.projected,
            tokens_after = plan.after,
            "wrote a checkpoint"
        );
        Ok(true)
    }

    /// Summarizes turns, folding them in chunks that each fit one call.
    async fn summarize(
        &self,
        setup: &Setup,
        previous: Option<&str>,
        covered: &[ModelTurn],
        allowance: u32,
    ) -> Result<String, CallError> {
        let rendered = compaction::render_turns(covered, setup.requester());
        let overhead = compaction::request_overhead(previous, allowance);
        let chunk_tokens = setup
            .budget
            .saturating_sub(allowance)
            .saturating_sub(overhead)
            .max(MIN_CHUNK_TOKENS);
        let mut summary = previous.map(str::to_owned);
        for chunk in compaction::chunks(&rendered, chunk_tokens) {
            let chunk = compaction::fit(&chunk, chunk_tokens);
            let request =
                compaction::summary_request(&setup.model, summary.as_deref(), &chunk, allowance);
            summary = Some(self.summary_call(setup, request).await?);
        }
        summary
            .filter(|summary| !summary.trim().is_empty())
            .ok_or(CallError::Failed(RunFailure::InvalidResponse))
    }

    /// One summary call, retried like the calls of a run.
    async fn summary_call(
        &self,
        setup: &Setup,
        request: ModelRequest,
    ) -> Result<String, CallError> {
        let policy = self.inner.config.retry;
        let mut tries = 0;
        loop {
            tries += 1;
            let mut stream = setup.adapter.stream(&setup.target, request.clone());
            let mut text = String::new();
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
            match ended {
                // A summary cut at the output limit lost its end, where the
                // latest of what it covers and the next steps go, and a model
                // that reasons may have spent the limit before it wrote
                // anything. It never becomes a checkpoint.
                Ok(FinishReason::OutputLimit) => {
                    tracing::warn!("a summary reached the output limit; no checkpoint is written");
                    return Err(CallError::Failed(RunFailure::SummaryCut));
                }
                Ok(_finish) => return Ok(text.trim().to_owned()),
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
    options: CallOptions,
    instructions: &'a str,
    definitions: &'a [ToolDefinition],
    /// The older results of the latest turn, left out when the context runs
    /// low.
    left_out: &'a HashSet<PartId>,
    /// The images of the newest results that the request sends.
    tool_images: &'a SentImages,
    /// The parts `context` whose excerpts the request sends in full.
    full_context: &'a HashSet<PartId>,
}

/// The request of a call: its instructions, the tools that it offers and the
/// turns that the model reads, with the results and the images that the
/// shaping leaves out.
fn call_request(setup: &Setup, shaping: &Shaping<'_>, turns: &[ModelTurn]) -> ModelRequest {
    let options = shaping.options;
    let shape = RequestShape {
        instructions: shaping.instructions,
        model: &setup.model,
        interface: setup.interface,
        binding: &setup.binding,
        max_output_tokens: setup.output_limit,
        reasoning_effort: setup.reasoning_effort,
        tools: shaping.definitions,
        tool_choice: if options.tools_used_up {
            ToolChoice::None
        } else {
            ToolChoice::Auto
        },
        note: (options.tools_used_up && setup.uses_tools())
            .then(|| texts::tool_budget_used(setup.requester())),
        left_out: shaping.left_out,
        requester: setup.requester(),
        tool_images: shaping.tool_images,
        full_context: shaping.full_context,
    };
    build_request(shape, turns)
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

/// The calls of the latest turn whose results can be left out: every
/// completed tool call and every task call that returned reports, but the
/// newest three.
fn older_results(turns: &[ModelTurn]) -> HashSet<PartId> {
    let Some(latest) = turns.last() else {
        return HashSet::new();
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
            part.tool_call
                .as_ref()
                .is_some_and(|call| call.status == ToolCallStatus::Completed)
        })
        .map(|part| &part.id)
        .collect();
    let older = completed.len().saturating_sub(KEPT_RESULTS);
    completed[..older].iter().map(|id| (*id).clone()).collect()
}
