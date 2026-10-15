//! Preparing model calls (docs/architecture/agent-runtime.md, the loop): what
//! a run reads once at its start, the context of each call, and the calls
//! that write checkpoints.

use std::collections::HashSet;
use std::sync::Arc;

use futures_util::StreamExt;
use mannie_provider::{
    ModelRequest, Provider, ProviderError, ProviderEvent, Target, ToolChoice, ToolDefinition,
    check_base_url, quirks_for,
};
use mannie_store::{
    Change, Durability, ModelTurn, NewCheckpoint, RuntimeStoreError, StoredCheckpoint,
    StoredProfile,
};
use mannie_tools::{Availability, Tool, with_activity};
use mannie_types::chat::{ChatSettings, Instructions};
use mannie_types::ids::{ConversationId, PartId, ProfileId};
use mannie_types::provider::{ApiInterface, ReasoningEffort, context_budget};
use mannie_types::run::RunFailure;
use mannie_types::tools::ToolCallStatus;
use mannie_types::transcript::PartKind;

use crate::agents::{AgentDefinition, Requester};
use crate::budget::{Ledger, estimate_request, usage};
use crate::compaction::{self, ContextSize, Trigger};
use crate::context::{RequestShape, build_request};
use crate::instructions::{InstructionContext, ToolEnvironment};
use crate::profiles::{VaultError, in_vault};
use crate::retry::RetryDecision;
use crate::{Runtime, lock, tasks, texts};

/// The newest tool results of the latest turn that stay in the context when
/// older ones are left out to save space.
const KEPT_RESULTS: usize = 3;

/// The tool that moves a general conversation into a project.
const MOVE_TO_PROJECT: &str = "move_to_project";

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
    pub(crate) async fn setup_for(
        &self,
        profile: &StoredProfile,
        agent: &str,
        task: bool,
    ) -> Result<Setup, RunFailure> {
        let Some(agent) = self.inner.agents.get(agent).cloned() else {
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
        let tools: Vec<Arc<dyn Tool>> = if profile.use_tools {
            agent
                .tools
                .iter()
                .filter_map(|name| self.inner.tools.registry.get(name))
                .collect()
        } else {
            Vec::new()
        };
        let definitions = tools
            .iter()
            .map(|tool| ToolDefinition {
                name: tool.name().to_owned(),
                description: tool.description().to_owned(),
                // Every call names itself for the owner (docs/features/
                // system-tools/tool-calls.md, the activity of a call).
                parameters: with_activity(tool.parameters()),
            })
            .collect();
        let child_agents: Vec<&'static str> = self
            .inner
            .agents
            .child_agents()
            .map(|definition| definition.name)
            .collect();
        // A child never gives tasks of its own (T-22), and an agent without a
        // task budget or anyone to give tasks to is not offered the tools.
        let offers_tasks =
            profile.use_tools && !task && agent.max_tasks > 0 && !child_agents.is_empty();
        let task_definitions = if offers_tasks {
            tasks::definitions(&child_agents)
                .into_iter()
                .filter(|definition| agent.tools.contains(&definition.name.as_str()))
                .collect()
        } else {
            Vec::new()
        };
        let credential = match profile.credential_ref.clone() {
            None => None,
            Some(reference) => {
                let read = in_vault(&self.inner.vault, move |vault| vault.read(&reference)).await;
                Some(read.map_err(|error| match error {
                    VaultError::Missing => RunFailure::CredentialMissing,
                    VaultError::Unavailable(_) => RunFailure::CredentialStoreUnavailable,
                })?)
            }
        };
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
        let owner = self.owner_instructions().await?;
        let checkpoint = self.current_checkpoint(conversation).await?;
        let covered = checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.covers_through_turn);
        let turns = self.inner.store.model_turns(conversation, covered).await?;
        let (project_id, projects) = if setup.uses_tools() {
            let summary = self.inner.store.conversation_summary(authority).await?;
            let projects = self.inner.store.projects().await?;
            (summary.and_then(|summary| summary.project_id), projects)
        } else {
            (None, Vec::new())
        };
        let project = project_id
            .as_ref()
            .and_then(|id| projects.iter().find(|project| project.id == *id));
        // Which tools a call is offered depends on the conversation: a
        // general conversation can move into a project, one in a project
        // cannot (docs/architecture/tools.md, availability).
        let availability = Availability {
            project,
            projects: &projects,
        };
        let mut definitions: Vec<ToolDefinition> = setup
            .tools
            .iter()
            .zip(&setup.definitions)
            .filter(|(tool, _definition)| tool.offered(&availability))
            .map(|(_tool, definition)| definition.clone())
            .collect();
        definitions.extend(setup.task_definitions.iter().cloned());
        // Only a run that can move into a project learns the owner's
        // projects; a child works in its parent's workspace.
        let movable: &[_] = if setup.tool(MOVE_TO_PROJECT).is_some() {
            &projects
        } else {
            &[]
        };
        let system = &self.inner.tools.system;
        let instructions = self.inner.instructions.assemble(&InstructionContext {
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
            }),
            offers_tasks: setup.offers_tasks(),
        });
        let left_out = if options.leave_out_results {
            older_results(&turns)
        } else {
            HashSet::new()
        };
        let shape = RequestShape {
            instructions: &instructions,
            model: &setup.model,
            interface: setup.interface,
            binding: &setup.binding,
            max_output_tokens: setup.output_limit,
            reasoning_effort: setup.reasoning_effort,
            tools: &definitions,
            tool_choice: if options.tools_used_up {
                ToolChoice::None
            } else {
                ToolChoice::Auto
            },
            note: (options.tools_used_up && setup.uses_tools())
                .then(|| texts::tool_budget_used(setup.requester())),
            left_out: &left_out,
            requester: setup.requester(),
        };
        let request = build_request(shape, &turns);
        let estimate = estimate_request(&request);
        let ledger = Ledger::parse(
            self.inner
                .store
                .context_ledger(conversation)
                .await?
                .as_deref(),
        );
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

    async fn owner_instructions(&self) -> Result<Instructions, CallError> {
        match self.inner.store.load_settings::<ChatSettings>().await {
            Ok(settings) => Ok(settings.instructions),
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
                    Some(Ok(ProviderEvent::Finished(_))) => break Ok(()),
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
                Ok(()) => return Ok(text.trim().to_owned()),
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
