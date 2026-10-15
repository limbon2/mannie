//! The lifecycle of one tool call (ADR-0013; docs/features/system-tools/
//! operations-and-approvals.md, the lifecycle).
//!
//! A call is prepared, which yields its accesses; the policy decides on them,
//! or the owner does. After the owner's approval the call is prepared again,
//! and only the accesses the owner saw may run. An operation with side
//! effects is stored as executing, durably, before it starts, so a crash
//! never repeats it: the recovery marks it uncertain. The calls of a
//! conversation run one at a time, and a run holds no model slot while its
//! tools run or wait.

use std::path::PathBuf;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use mannie_store::{Change, Durability, NewArchive};
use mannie_tools::{
    CommandEnvironment, Done, Execution, Grants, PermissionClass, PolicyInputs, PrepareError,
    ProjectFacts, StopReason, StopSignal, Tool, ToolContext, Verdict, decide, digest, result_limit,
    saved_rule,
};
use mannie_types::ids::ProjectId;
use mannie_types::run::{RunActivity, RunFailure};
use mannie_types::tools::{
    Access, ApprovalChoice, ApprovalRequest, ArchiveRef, CommandRule, Decision, GrantScope,
    NotRunReason, ProcessRecord, ToolCallStatus, ToolResult, ToolsSettings,
};
use tokio::sync::oneshot;

use super::{Ending, RunActor, StepCall, halt};
use crate::calls::Setup;
use crate::projects::Moves;

/// The most characters of a parser's message about invalid arguments that
/// the journal and the model keep.
const MAX_DETAIL_CHARS: usize = 1_000;

/// How a tool call ended, for the calls after it in the same step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallEnd {
    /// It ran, failed or did not run; the next call goes on.
    Ran,
    /// The owner denied it; the rest of the step does not run.
    Denied,
}

/// What preparing a call found.
enum Preparation {
    /// The accesses of the operation.
    Accesses(Vec<Access>),
    /// The call never runs.
    NotRun(NotRunReason),
    /// The call cannot run; the failure is its result.
    Failed(Done),
}

/// What a call needs from its surroundings, read again for every call, so a
/// change of the project, the settings, a grant or a rule applies from the
/// next call.
struct Surroundings {
    context: ToolContext,
    settings: ToolsSettings,
    /// The conversation's own grants.
    grants: Vec<GrantScope>,
    /// The grants for every conversation of its project.
    project_grants: Vec<GrantScope>,
    /// The grants for every conversation.
    always_grants: Vec<GrantScope>,
    rules: Vec<CommandRule>,
    /// The conversation's project.
    project: Option<ProjectId>,
}

/// The bytes of a context budget in tokens, the inverse of the estimate of
/// two tokens for every five bytes.
fn budget_bytes(tokens: u32) -> usize {
    usize::try_from(u64::from(tokens) * 5 / 2).unwrap_or(usize::MAX)
}

/// A store failure that ends the run.
fn internal(error: &dyn std::fmt::Display, what: &str) -> Ending {
    tracing::error!(%error, "could not read {what} for a tool call");
    Ending::Failed(RunFailure::Internal)
}

impl RunActor {
    /// Runs the complete calls of a model call one at a time. A denial skips
    /// the rest of them, and calls beyond the reply's budget do not run.
    pub(super) async fn run_calls(
        &mut self,
        setup: &Setup,
        calls: Vec<StepCall>,
    ) -> Result<(), Ending> {
        let mut denied = false;
        for call in calls {
            if denied {
                self.end_call(call, NotRunReason::EarlierCallDenied);
                continue;
            }
            if self.tool_calls >= setup.agent.max_tool_calls {
                self.end_call(call, NotRunReason::OverBudget);
                continue;
            }
            self.tool_calls += 1;
            if self.run_call(setup, call).await? == CallEnd::Denied {
                denied = true;
            }
        }
        self.pending.push(self.activity(RunActivity::Preparing));
        self.flush(Durability::Full).await
    }

    async fn run_call(&mut self, setup: &Setup, mut call: StepCall) -> Result<CallEnd, Ending> {
        let Some(tool) = setup.tool(&call.document.tool).map(Arc::clone) else {
            self.end_call(call, NotRunReason::UnknownTool);
            return Ok(CallEnd::Ran);
        };
        let arguments = call.arguments.clone().unwrap_or_default();
        let mut surroundings = self.surroundings(setup).await?;
        let mut accesses = match self
            .prepare_tool(&tool, &surroundings.context, &arguments)
            .await?
        {
            Preparation::Accesses(accesses) => accesses,
            Preparation::NotRun(reason) => {
                self.end_call(call, reason);
                return Ok(CallEnd::Ran);
            }
            Preparation::Failed(done) => {
                self.complete_call(call, done);
                return Ok(CallEnd::Ran);
            }
        };
        let moves = accesses
            .iter()
            .any(|access| matches!(access, Access::Project { .. }));
        let owner_request = if moves {
            self.owner_request(&call).await?
        } else {
            None
        };
        loop {
            let protected = &self.runtime.inner.tools.protected;
            let verdict = decide(
                &accesses,
                &PolicyInputs {
                    workspace: surroundings.context.workspace.as_deref(),
                    allow_changes_in_workspace: surroundings.settings.allow_changes_in_workspace,
                    protected,
                    grants: Grants {
                        conversation: &surroundings.grants,
                        project: &surroundings.project_grants,
                        always: &surroundings.always_grants,
                    },
                    rules: &surroundings.rules,
                    project: surroundings.project.as_ref(),
                    owner_request: owner_request.as_deref(),
                },
            );
            let request = match verdict {
                Verdict::Unrestricted => break,
                Verdict::Allowed(decision) => {
                    call.document.decision = Some(decision);
                    call.document.decided_at = Some(self.now());
                    break;
                }
                Verdict::Ask(request) => request,
            };
            let choice = self.ask(&mut call, &accesses, &request).await?;
            let decision = Decision::of_choice(&choice);
            call.document.decision = Some(decision);
            call.document.decided_at = Some(self.now());
            if !decision.allows() {
                tracing::info!(run = %self.run, part = %call.part_id, tool = %call.document.tool, "the owner denied a tool call");
                call.document.status = ToolCallStatus::Denied;
                call.document.ended_at = Some(self.now());
                self.finish_call(&call);
                self.flush(Durability::Full).await?;
                return Ok(CallEnd::Denied);
            }
            self.remember(&choice, &request).await;
            // The approval covers the accesses that the owner saw. Anything
            // that changed since goes back to the policy, and an earlier
            // approval no longer counts.
            surroundings = self.surroundings(setup).await?;
            match self
                .prepare_tool(&tool, &surroundings.context, &arguments)
                .await?
            {
                Preparation::Accesses(again) => {
                    let unchanged = digest(&again) == request.digest;
                    accesses = again;
                    if unchanged {
                        break;
                    }
                    tracing::info!(run = %self.run, part = %call.part_id, "the accesses of an approved tool call changed; asking the policy again");
                }
                Preparation::NotRun(reason) => {
                    self.end_call(call, reason);
                    return Ok(CallEnd::Ran);
                }
                Preparation::Failed(done) => {
                    self.complete_call(call, done);
                    return Ok(CallEnd::Ran);
                }
            }
        }
        self.execute(&tool, call, surroundings.context, arguments, accesses)
            .await
    }

    /// Runs an authorized call and stores its result.
    async fn execute(
        &mut self,
        tool: &Arc<dyn Tool>,
        mut call: StepCall,
        context: ToolContext,
        arguments: String,
        accesses: Vec<Access>,
    ) -> Result<CallEnd, Ending> {
        let side_effects = tool.class() == PermissionClass::SideEffects;
        let started_at = self.now();
        call.document.accesses.clone_from(&accesses);
        call.document.started_at = Some(started_at);
        // Only an operation with side effects is journaled as executing; a
        // read that Mannie's end interrupts was never anything but canceled.
        call.document.status = if side_effects {
            ToolCallStatus::Executing
        } else {
            ToolCallStatus::Running
        };
        self.pending.push(update(&call));
        self.pending.push(self.activity(RunActivity::RunningTool {
            part_id: call.part_id.clone(),
            started_at,
        }));
        // The journal: the executing status is durable before the side
        // effect starts (ADR-0013, decision 5).
        self.flush(Durability::Full).await?;
        tracing::info!(run = %self.run, part = %call.part_id, tool = tool.name(), "a tool call runs");
        tracing::debug!(part = %call.part_id, accesses = ?accesses, "the accesses of a tool call");
        let stop = StopSignal::new();
        let (process_sender, process) = oneshot::channel();
        let execution = tool.execute(Execution {
            context,
            arguments,
            accesses,
            stop: stop.clone(),
            process: Some(process_sender),
        });
        let (done, halted) = self.watch(execution, &stop, process, &mut call).await?;
        match halted {
            None => {
                self.complete_call(call, done);
                self.flush(Durability::Full).await?;
                Ok(CallEnd::Ran)
            }
            Some(ending) => {
                // A change that ran keeps its result; a read that was stopped
                // is canceled when the run ends.
                if side_effects {
                    self.complete_call(call, done);
                }
                Err(ending)
            }
        }
    }

    /// Waits for a running tool. Stop and Shutdown tell it to stop, and the
    /// run ends once it has; a command reports its process for the journal
    /// as soon as it runs.
    async fn watch(
        &mut self,
        mut execution: BoxFuture<'static, Done>,
        stop: &StopSignal,
        mut process: oneshot::Receiver<ProcessRecord>,
        call: &mut StepCall,
    ) -> Result<(Done, Option<Ending>), Ending> {
        let mut halted: Option<Ending> = None;
        let mut process_pending = true;
        loop {
            tokio::select! {
                done = &mut execution => return Ok((done, halted)),
                command = self.commands.recv(), if halted.is_none() => {
                    let ending = halt(command);
                    let reason = match ending {
                        Ending::Stopped => StopReason::Stopped,
                        _ => StopReason::Interrupted,
                    };
                    tracing::info!(run = %self.run, part = %call.part_id, "stopping a running tool");
                    stop.stop(reason);
                    halted = Some(ending);
                }
                record = &mut process, if process_pending => {
                    process_pending = false;
                    if let Ok(record) = record {
                        call.document.process = Some(record);
                        self.pending.push(update(call));
                        self.flush(Durability::Full).await?;
                    }
                }
            }
        }
    }

    /// Asks the owner and waits for the answer. Stop and Shutdown end the
    /// wait, and the request is withdrawn.
    async fn ask(
        &mut self,
        call: &mut StepCall,
        accesses: &[Access],
        request: &ApprovalRequest,
    ) -> Result<ApprovalChoice, Ending> {
        call.document.status = ToolCallStatus::AwaitingApproval;
        call.document.accesses = accesses.to_vec();
        call.document.request = Some(request.clone());
        // The request is registered before any window can see it, so a
        // decision never arrives before it.
        let (waiting, answer) = self
            .runtime
            .ask_owner(&self.conversation, &call.part_id, request);
        self.pending.push(update(call));
        self.pending
            .push(self.activity(RunActivity::AwaitingApproval {
                part_id: call.part_id.clone(),
            }));
        self.flush(Durability::Full).await?;
        tracing::info!(run = %self.run, part = %call.part_id, tool = %call.document.tool, reasons = ?request.reasons, "a tool call waits for the owner");
        let answer = self.wait(answer).await;
        drop(waiting);
        answer?.map_err(|_withdrawn| Ending::Interrupted)
    }

    /// The owner's message of the turn, while no tool call of the turn came
    /// before this one: content that the model read since can never stand in
    /// for the owner's words (ADR-0023, decision 3).
    async fn owner_request(&self, call: &StepCall) -> Result<Option<String>, Ending> {
        let store = &self.runtime.inner.store;
        if store.tool_calls_before(&call.part_id).await? > 0 {
            return Ok(None);
        }
        Ok(store.turn_request(&self.turn).await?)
    }

    /// Saves what the owner's choice allows beyond this call: the request's
    /// grants, for the conversation, its project or every conversation, or a
    /// rule built from the request's command. A failure to save leaves this
    /// call allowed, and the next call asks again.
    async fn remember(&self, choice: &ApprovalChoice, request: &ApprovalRequest) {
        let store = &self.runtime.inner.store;
        let ids = &self.runtime.inner.ids;
        let grants = || -> Vec<_> {
            request
                .grants
                .iter()
                .map(|scope| (ids.grant(), scope.clone()))
                .collect()
        };
        match choice {
            ApprovalChoice::AllowForConversation if !request.grants.is_empty() => {
                if let Err(error) = store.add_grants(&self.conversation, grants()).await {
                    tracing::warn!(%error, "could not save the grants of an approval");
                }
            }
            ApprovalChoice::AllowForProject if !request.grants.is_empty() => {
                let Some(project) = &request.project else {
                    return;
                };
                if let Err(error) = store.add_shared_grants(Some(project), grants()).await {
                    tracing::warn!(%error, "could not save the project grants of an approval");
                }
            }
            ApprovalChoice::AllowAlways if !request.grants.is_empty() => {
                if let Err(error) = store.add_shared_grants(None, grants()).await {
                    tracing::warn!(%error, "could not save the grants of an approval for every conversation");
                }
            }
            ApprovalChoice::SaveRule {
                prefix_words,
                limit_to_folder,
            } => {
                let rule = request
                    .rule
                    .as_ref()
                    .map(|proposal| saved_rule(proposal, *prefix_words, *limit_to_folder));
                if let Some(Ok((words, folder))) = rule
                    && let Err(error) = store.add_rule(&ids.rule(), words, folder).await
                {
                    tracing::warn!(%error, "could not save the rule of an approval");
                }
            }
            _ => {}
        }
    }

    /// Prepares a call. Stop and Shutdown end the wait.
    async fn prepare_tool(
        &mut self,
        tool: &Arc<dyn Tool>,
        context: &ToolContext,
        arguments: &str,
    ) -> Result<Preparation, Ending> {
        let prepared = self
            .wait(tool.prepare(context.clone(), arguments.to_owned()))
            .await?;
        Ok(match prepared {
            Ok(prepared) => Preparation::Accesses(prepared.accesses),
            Err(PrepareError::InvalidArguments { detail }) => {
                Preparation::NotRun(NotRunReason::InvalidArguments {
                    detail: detail.chars().take(MAX_DETAIL_CHARS).collect(),
                })
            }
            Err(PrepareError::Failed(failure)) => Preparation::Failed(Done::failed(failure)),
        })
    }

    /// Reads what the policy and the tool need, fresh for this call.
    async fn surroundings(&self, setup: &Setup) -> Result<Surroundings, Ending> {
        let inner = &self.runtime.inner;
        let summary = inner.store.conversation_summary(&self.conversation).await?;
        let workspace = summary
            .as_ref()
            .and_then(|summary| summary.workspace.as_ref())
            .map(PathBuf::from);
        let project = summary.and_then(|summary| summary.project_id);
        let projects = inner.store.projects().await?;
        let mut project_grants = Vec::new();
        let mut always_grants = Vec::new();
        let shared = inner
            .store
            .shared_grants()
            .await
            .map_err(|error| internal(&error, "the shared grants"))?;
        for grant in shared {
            match &grant.project_id {
                None => always_grants.push(grant.scope),
                Some(owner) if Some(owner) == project.as_ref() => project_grants.push(grant.scope),
                Some(_other) => {}
            }
        }
        let current = project
            .as_ref()
            .and_then(|id| projects.iter().find(|candidate| candidate.id == *id))
            .cloned();
        let settings = inner
            .store
            .load_settings::<ToolsSettings>()
            .await
            .map_err(|error| internal(&error, "the tools settings"))?;
        let grants = inner
            .store
            .grants(&self.conversation)
            .await
            .map_err(|error| internal(&error, "the grants"))?
            .into_iter()
            .map(|grant| grant.scope)
            .collect();
        let rules = inner
            .store
            .rules()
            .await
            .map_err(|error| internal(&error, "the rules"))?;
        let runtime = self.runtime.clone();
        let context = ToolContext {
            conversation_id: self.conversation.clone(),
            workspace,
            store: inner.store.clone(),
            result_limit: result_limit(budget_bytes(setup.budget)),
            environment: Arc::new(CommandEnvironment::from_process(
                &settings.command_environment,
            )),
            mutations: inner.tools.mutations.clone(),
            archive_ids: Arc::new(move || runtime.inner.ids.archive()),
            system: Arc::clone(&inner.tools.system),
            projects: ProjectFacts {
                current,
                all: Arc::from(projects),
                moves: Arc::new(Moves {
                    runtime: self.runtime.clone(),
                }),
            },
        };
        Ok(Surroundings {
            context,
            settings,
            grants,
            project_grants,
            always_grants,
            rules,
            project,
        })
    }

    /// Stores a call's result and finishes its part. Archived outputs are
    /// stored with it.
    fn complete_call(&mut self, mut call: StepCall, done: Done) {
        let references = done
            .archives
            .iter()
            .map(|archive| ArchiveRef {
                id: archive.id.clone(),
                name: archive.name.clone(),
                bytes: u32::try_from(archive.content.len()).unwrap_or(u32::MAX),
                lines: u32::try_from(archive.content.lines().count()).unwrap_or(u32::MAX),
            })
            .collect();
        let archives = done
            .archives
            .into_iter()
            .map(|archive| NewArchive {
                id: archive.id,
                name: archive.name,
                content: archive.content,
            })
            .collect();
        call.document.status = ToolCallStatus::Completed;
        call.document.result = Some(ToolResult {
            outcome: done.outcome,
            output: done.output,
            archives: references,
        });
        call.document.ended_at = Some(self.now());
        tracing::info!(run = %self.run, part = %call.part_id, tool = %call.document.tool, "a tool call completed");
        self.pending.push(Change::UpdatePart {
            part_id: call.part_id.clone(),
            tool_call: call.document.clone(),
            archives,
        });
        self.pending.push(Change::FinishPart {
            part_id: call.part_id,
            state: mannie_types::transcript::PartState::Complete,
            tail: String::new(),
            provider_item: None,
        });
        self.schedule_flush();
    }
}

/// The change that stores a call's document as it is now.
fn update(call: &StepCall) -> Change {
    Change::UpdatePart {
        part_id: call.part_id.clone(),
        tool_call: call.document.clone(),
        archives: Vec::new(),
    }
}
