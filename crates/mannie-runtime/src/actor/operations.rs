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
use mannie_store::{Change, Durability, NewArchive, NewToolImage};
use mannie_tools::{
    Ceiling, CommandEnvironment, Done, Execution, Grants, PermissionClass, PolicyInputs,
    PrepareError, ProjectFacts, SavedRule, StopReason, StopSignal, Tool, ToolContext, Verdict,
    decide, digest, result_limit, saved_rule,
};
use mannie_types::ceiling::CeilingMode;
use mannie_types::ids::ProjectId;
use mannie_types::run::{RunActivity, RunFailure};
use mannie_types::tools::{
    Access, ApprovalChoice, ApprovalRequest, ArchiveRef, CommandRule, Decision, GrantScope,
    Interruption, NotRunReason, ProcessRecord, ProgramRule, Refusal, RemoteCommandRule,
    ToolCallStatus, ToolImageRef, ToolOutcome, ToolResult, ToolsSettings,
};
use tokio::sync::oneshot;

use super::{Ending, MAX_DETAIL_CHARS, RunActor, StepCall, halt};
use crate::approvals::Answer;
use crate::background::BackgroundPort;
use crate::budget::budget_bytes;
use crate::calls::Setup;
use crate::policy::PolicyGrants;
use crate::projects::Moves;
use crate::tool_images;

/// How a tool call ended, for the calls after it in the same step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallEnd {
    /// It ran, failed or did not run; the next call goes on.
    Ran,
    /// The owner denied it; the rest of the step does not run.
    Denied,
}

/// How the wait for a running tool ended.
enum Watched {
    /// The tool returned, by itself or after Stop or Shutdown told it to.
    Ended {
        /// What the tool returned, boxed because it is far larger than an
        /// abort.
        done: Box<Done>,
        /// How the run ends, when a command halted it meanwhile.
        halted: Option<Ending>,
    },
    /// Turning the tool's plugin off aborted the execution, which was dropped.
    Aborted,
}

/// Waits until turning a plugin off aborts an execution of its tool, or
/// forever for a call without such a signal.
async fn aborted(abort: Option<&StopSignal>) {
    match abort {
        Some(abort) => abort.stopped().await,
        None => std::future::pending().await,
    }
}

/// Why a prepared call does not run.
enum Unprepared {
    /// The call never runs.
    NotRun(NotRunReason),
    /// The call cannot run; the failure is its result.
    Failed(Done),
}

/// The owner's choice and the decision that an answer to an approval gives.
fn decision_of(answer: Answer) -> (Option<ApprovalChoice>, Decision) {
    match answer {
        Answer::Owner(choice) => {
            let decision = Decision::of_choice(&choice);
            (Some(choice), decision)
        }
        // Full access allows this call once; it grants nothing longer and
        // saves no rule (ADR-0029, decision 5).
        Answer::FullAccess => (None, Decision::FullAccess),
    }
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
    /// The rules for commands on remote hosts (ADR-0036).
    remote_rules: Vec<RemoteCommandRule>,
    /// The rules for runs of installed programs (ADR-0038).
    program_rules: Vec<ProgramRule>,
    /// The conversation's project.
    project: Option<ProjectId>,
    /// On a worker, the ceiling of the task, narrowed as the task asks.
    ceiling: Option<Ceiling>,
    /// On a worker, its name, for its approval requests.
    host: Option<String>,
    /// Whether the conversation, or a child's parent, is in full access
    /// (ADR-0029). The switch is read again for every call.
    full_access: bool,
}

/// Why a call of a tool that the run does not have does not run. A read-only
/// task is not offered the tools that change things, and a call of one that
/// comes anyway meets the ceiling's refusal (ADR-0028, decision 5).
fn missing_tool_reason(setup: &Setup, tool: &str) -> NotRunReason {
    let read_only = setup
        .assigned
        .as_ref()
        .is_some_and(|assigned| assigned.mode == CeilingMode::ReadOnly);
    if read_only && setup.agent.tools.contains(&tool) {
        NotRunReason::Refused {
            refusal: Refusal::ReadOnlyHost,
        }
    } else {
        NotRunReason::UnknownTool
    }
}

/// A store failure that ends the run.
fn internal(error: &dyn std::fmt::Display, what: &str) -> Ending {
    tracing::error!(%error, "could not read {what} for a tool call");
    Ending::Failed(RunFailure::Internal)
}

impl RunActor {
    /// Runs the complete calls of a model call in their order. A tool call
    /// runs to its end before the next call starts; a start or a follow-up
    /// of a task does not hold up the step, and the step ends when its tasks
    /// that do not run in the background reported (ADR-0024, decision 5). A
    /// denial skips the rest of the calls, and calls beyond the reply's
    /// budget do not run.
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
            self.publish_progress(call.document.activity.clone());
            if call.task.is_some() {
                Box::pin(self.run_task_call(setup, call)).await?;
            } else if self.run_call(setup, call).await? == CallEnd::Denied {
                denied = true;
            }
        }
        self.await_foreground_tasks().await?;
        // The ends of background commands reach the model before its next
        // call (ADR-0041).
        self.note_command_ends().await?;
        self.publish_progress(None);
        let preparing = self.activity(RunActivity::Preparing);
        self.pending.push(preparing);
        self.flush(Durability::Full).await
    }

    /// The tool of a call, or why the call does not run before its tool is
    /// prepared: the run was not offered the tool, or the tool's plugin
    /// stopped since the run started. A run keeps the tools it started with,
    /// and a call into a stopped plugin does not run (ADR-0030, decision 8).
    /// The run holds the tool of the plugin's start that offered it, so a
    /// plugin that was turned off and on again since is off for the call too:
    /// its new start has a generation of its own.
    fn tool_to_run(&self, setup: &Setup, name: &str) -> Result<Arc<dyn Tool>, NotRunReason> {
        let Some(tool) = setup.tool(name).map(Arc::clone) else {
            return Err(missing_tool_reason(setup, name));
        };
        if let Some(offered) = setup.plugin_tool(name)
            && self.runtime.contributions().generation_of(&offered.plugin)
                != Some(offered.generation)
        {
            return Err(NotRunReason::PluginOff {
                plugin: offered.plugin.clone(),
            });
        }
        Ok(tool)
    }

    async fn run_call(&mut self, setup: &Setup, mut call: StepCall) -> Result<CallEnd, Ending> {
        let tool = match self.tool_to_run(setup, &call.document.tool) {
            Ok(tool) => tool,
            Err(reason) => {
                self.end_call(call, reason);
                return Ok(CallEnd::Ran);
            }
        };
        // The card of a plugin's call is its plugin's to describe (ADR-0039),
        // so the call's record names the plugin from its first write on.
        call.document.plugin = setup
            .plugin_tool(&call.document.tool)
            .map(|offered| offered.plugin.clone());
        let arguments = call.arguments.clone().unwrap_or_default();
        let mut surroundings = self.surroundings(setup).await?;
        let mut accesses = match self
            .prepare_tool(&tool, &surroundings.context, &arguments)
            .await?
        {
            Ok(accesses) => accesses,
            Err(unprepared) => return Ok(self.end_unprepared(call, unprepared)),
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
            let verdict = self.verdict(&accesses, &surroundings, owner_request.as_deref());
            let request = match verdict {
                Verdict::Unrestricted => break,
                Verdict::Allowed(decision) => {
                    call.document.decision = Some(decision);
                    call.document.decided_at = Some(self.now());
                    break;
                }
                Verdict::Ask(request) => *request,
                Verdict::Refused(refusal) => {
                    tracing::info!(run = %self.run, part = %call.part_id, tool = %call.document.tool, refusal = ?refusal, "the worker's ceiling refused a tool call");
                    self.end_call(call, NotRunReason::Refused { refusal });
                    return Ok(CallEnd::Ran);
                }
            };
            let (choice, decision) = decision_of(self.ask(&mut call, &accesses, &request).await?);
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
            if let Some(choice) = &choice {
                self.remember(choice, &request).await;
            }
            // The owner may have turned off the tool's plugin while the call
            // waited, and a plugin that is off prepares nothing again
            // (ADR-0030, decision 8).
            if let Err(reason) = self.tool_to_run(setup, &call.document.tool) {
                self.end_call(call, reason);
                return Ok(CallEnd::Ran);
            }
            // The approval covers the accesses that the owner saw. Anything
            // that changed since goes back to the policy, and an earlier
            // approval no longer counts.
            surroundings = self.surroundings(setup).await?;
            match self
                .prepare_tool(&tool, &surroundings.context, &arguments)
                .await?
            {
                Ok(again) => {
                    let unchanged = digest(&again) == request.digest;
                    accesses = again;
                    if unchanged {
                        break;
                    }
                    tracing::info!(run = %self.run, part = %call.part_id, "the accesses of an approved tool call changed; asking the policy again");
                }
                Err(unprepared) => return Ok(self.end_unprepared(call, unprepared)),
            }
        }
        self.execute(setup, &tool, call, surroundings, arguments, accesses)
            .await
    }

    /// Ends a call whose preparation left it nothing to run.
    fn end_unprepared(&mut self, call: StepCall, unprepared: Unprepared) -> CallEnd {
        match unprepared {
            Unprepared::NotRun(reason) => self.end_call(call, reason),
            Unprepared::Failed(done) => self.complete_call(call, done),
        }
        CallEnd::Ran
    }

    /// What the policy decides about a call's accesses in its surroundings.
    fn verdict(
        &self,
        accesses: &[Access],
        surroundings: &Surroundings,
        owner_request: Option<&str>,
    ) -> Verdict {
        decide(
            accesses,
            &PolicyInputs {
                workspace: surroundings.context.workspace.as_deref(),
                allow_changes_in_workspace: surroundings.settings.allow_changes_in_workspace,
                protected: &self.runtime.inner.tools.protected,
                grants: Grants {
                    conversation: &surroundings.grants,
                    project: &surroundings.project_grants,
                    always: &surroundings.always_grants,
                },
                rules: &surroundings.rules,
                remote_rules: &surroundings.remote_rules,
                program_rules: &surroundings.program_rules,
                project: surroundings.project.as_ref(),
                owner_request,
                ceiling: surroundings.ceiling.as_ref(),
                host: surroundings.host.as_deref(),
                full_access: surroundings.full_access,
            },
        )
    }

    /// Runs an authorized call and stores its result. A call of a plugin's
    /// tool runs only while the plugin's start that offered the tool to the
    /// run is on, and turning the plugin off waits for it (ADR-0034,
    /// decision 8).
    async fn execute(
        &mut self,
        setup: &Setup,
        tool: &Arc<dyn Tool>,
        mut call: StepCall,
        surroundings: Surroundings,
        arguments: String,
        accesses: Vec<Access>,
    ) -> Result<CallEnd, Ending> {
        let side_effects = tool.class() == PermissionClass::SideEffects;
        let offered = setup.plugin_tool(&call.document.tool);
        let execution_guard = match offered {
            None => None,
            Some(offered) => {
                let Some(guard) = self.runtime.begin_plugin_execution(offered) else {
                    let plugin = offered.plugin.clone();
                    self.end_call(call, NotRunReason::PluginOff { plugin });
                    return Ok(CallEnd::Ran);
                };
                Some(guard)
            }
        };
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
        self.pending.push(call.update(Vec::new()));
        let running = self.activity(RunActivity::RunningTool {
            part_id: call.part_id.clone(),
            started_at,
        });
        self.pending.push(running);
        // The journal: the executing status is durable before the side
        // effect starts (ADR-0013, decision 5).
        self.flush(Durability::Full).await?;
        tracing::info!(run = %self.run, part = %call.part_id, tool = tool.name(), "a tool call runs");
        tracing::debug!(part = %call.part_id, accesses = ?accesses, "the accesses of a tool call");
        let stop = StopSignal::new();
        let (process_sender, process) = oneshot::channel();
        let execution = tool.execute(Execution {
            context: surroundings.context,
            arguments,
            accesses,
            stop: stop.clone(),
            process: Some(process_sender),
            part_id: Some(call.part_id.clone()),
            activity: call.document.activity.clone(),
        });
        let abort = execution_guard.as_ref().map(|guard| guard.abort.clone());
        let watched = self
            .watch(execution, &stop, process, &mut call, abort.as_ref())
            .await;
        // After the plugin's work is done or dropped, so a plugin that turns
        // off waits for it.
        drop(execution_guard);
        let (done, halted) = match watched? {
            Watched::Ended { done, halted } => (*done, halted),
            Watched::Aborted => {
                let plugin = offered.map_or_else(String::new, |offered| offered.plugin.clone());
                self.abandon_aborted(call, side_effects, plugin);
                self.flush(Durability::Full).await?;
                return Ok(CallEnd::Ran);
            }
        };
        match halted {
            None => {
                self.complete_call(call, done);
                self.flush(Durability::Full).await?;
                Ok(CallEnd::Ran)
            }
            Some(ending) => {
                // The core let a plugin's work go without its result
                // (ADR-0034, decision 3), so nobody knows how far it got: the
                // call stays executing, which the end of the run makes
                // uncertain, and says why.
                let let_go = !matches!(done.outcome, ToolOutcome::Plugin { .. });
                if side_effects && offered.is_some() && let_go {
                    if !matches!(ending, Ending::Interrupted) {
                        call.document.interruption = Some(Interruption::Stopped);
                    }
                    self.pending.push(call.update(Vec::new()));
                } else if side_effects {
                    // A change that ran keeps its result; a read that was
                    // stopped is canceled when the run ends.
                    self.complete_call(call, done);
                }
                Err(ending)
            }
        }
    }

    /// Ends a call of a plugin's tool whose execution turning the plugin off
    /// aborted, while the reply goes on: an operation may have taken effect,
    /// so it is uncertain; a read did not finish, so it did not run.
    fn abandon_aborted(&mut self, mut call: StepCall, side_effects: bool, plugin: String) {
        tracing::info!(run = %self.run, part = %call.part_id, tool = %call.document.tool, "turning a plugin off aborted a tool call");
        if !side_effects {
            self.end_call(call, NotRunReason::PluginOff { plugin });
            return;
        }
        call.document.status = ToolCallStatus::Uncertain;
        call.document.interruption = Some(Interruption::PluginOff { plugin });
        call.document.ended_at = Some(self.now());
        self.finish_call(&call);
    }

    /// Waits for a running tool. Stop and Shutdown tell it to stop, and the
    /// run ends once it has; a command reports its process for the journal
    /// as soon as it runs. When turning its plugin off aborts a plugin's
    /// tool, the execution is dropped. Meanwhile, the states that the run's
    /// children publish are stored.
    async fn watch(
        &mut self,
        mut execution: BoxFuture<'static, Done>,
        stop: &StopSignal,
        mut process: oneshot::Receiver<ProcessRecord>,
        call: &mut StepCall,
        abort: Option<&StopSignal>,
    ) -> Result<Watched, Ending> {
        let mut halted: Option<Ending> = None;
        let mut process_pending = true;
        let board = Arc::clone(self.tasks.board());
        loop {
            tokio::select! {
                done = &mut execution => return Ok(Watched::Ended { done: Box::new(done), halted }),
                () = aborted(abort), if halted.is_none() => return Ok(Watched::Aborted),
                command = self.commands.recv(), if halted.is_none() => {
                    let ending = halt(command);
                    // A revoked lease ends the run on purpose, as a stop does.
                    let reason = match ending {
                        Ending::Stopped | Ending::Failed(_) => StopReason::Stopped,
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
                        self.pending.push(call.update(Vec::new()));
                        self.flush(Durability::Full).await?;
                    }
                }
                () = board.changed(), if halted.is_none() => self.mirror_children().await?,
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
    ) -> Result<Answer, Ending> {
        call.document.status = ToolCallStatus::AwaitingApproval;
        call.document.accesses = accesses.to_vec();
        call.document.request = Some(request.clone());
        // The request is registered before any window can see it, so a
        // decision never arrives before it.
        let (waiting, answer) = self
            .runtime
            .ask_owner(&self.conversation, &call.part_id, request);
        self.pending.push(call.update(Vec::new()));
        let awaiting = self.activity(RunActivity::AwaitingApproval {
            part_id: call.part_id.clone(),
        });
        self.pending.push(awaiting);
        let applied = self.flush_applied(Durability::Full).await?;
        // A child's request shows in its parent's conversation, where the
        // owner answers it (docs/features/child-agents/tasks.md, what the
        // parent shows).
        self.publish_approval(call, &applied);
        tracing::info!(run = %self.run, part = %call.part_id, tool = %call.document.tool, reasons = ?request.reasons, "a tool call waits for the owner");
        let answer = self.wait(answer).await;
        drop(waiting);
        self.withdraw_approval();
        answer?.map_err(|_withdrawn| Ending::Interrupted)
    }

    /// The owner's message of the turn, while no tool call of the turn came
    /// before this one: content that the model read since can never stand in
    /// for the owner's words (ADR-0023, decision 3). A child's turn holds
    /// another agent's instruction, which never counts as the owner's words.
    async fn owner_request(&self, call: &StepCall) -> Result<Option<String>, Ending> {
        if self.child.is_some() {
            return Ok(None);
        }
        let store = &self.runtime.inner.store;
        if store.tool_calls_before(&call.part_id).await? > 0 {
            return Ok(None);
        }
        Ok(store.turn_request(&self.turn).await?)
    }

    /// Saves what the owner's choice allows beyond this call: the request's
    /// grants, for the conversation, its project or every conversation, or a
    /// rule built from the request's command. A failure to save leaves this
    /// call allowed, and the next call asks again. A child's grants "for this
    /// conversation" belong to its parent, so they cover the parent and all
    /// its children.
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
                if let Err(error) = store.add_grants(&self.authority, grants()).await {
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
                let saved = match rule {
                    Some(Ok(SavedRule::Local {
                        words,
                        folder,
                        background,
                    })) => store
                        .add_rule(&ids.rule(), words, folder, background)
                        .await
                        .map(drop),
                    Some(Ok(SavedRule::Remote {
                        target,
                        words,
                        background,
                    })) => store
                        .add_remote_rule(&ids.rule(), target, words, background)
                        .await
                        .map(drop),
                    Some(Ok(SavedRule::Program {
                        target,
                        words,
                        folder,
                    })) => store
                        .add_program_rule(&ids.rule(), target, words, folder)
                        .await
                        .map(drop),
                    Some(Err(_)) | None => Ok(()),
                };
                if let Err(error) = saved {
                    tracing::warn!(%error, "could not save the rule of an approval");
                }
            }
            _ => {}
        }
    }

    /// Prepares a call, which yields the accesses of its operation or why it
    /// does not run. Stop and Shutdown end the wait.
    async fn prepare_tool(
        &mut self,
        tool: &Arc<dyn Tool>,
        context: &ToolContext,
        arguments: &str,
    ) -> Result<Result<Vec<Access>, Unprepared>, Ending> {
        let prepared = self
            .wait(tool.prepare(context.clone(), arguments.to_owned()))
            .await?;
        Ok(match prepared {
            Ok(prepared) => Ok(prepared.accesses),
            Err(PrepareError::InvalidArguments { detail }) => {
                Err(Unprepared::NotRun(NotRunReason::InvalidArguments {
                    detail: detail.chars().take(MAX_DETAIL_CHARS).collect(),
                }))
            }
            Err(PrepareError::Failed(failure)) => Err(Unprepared::Failed(Done::failed(failure))),
            Err(PrepareError::Ended(done)) => Err(Unprepared::Failed(*done)),
        })
    }

    /// Reads what the policy and the tool need, fresh for this call: the
    /// workspace, the project and the grants of the authority conversation,
    /// which for a child agent is its parent (ADR-0024, decision 3).
    async fn surroundings(&self, setup: &Setup) -> Result<Surroundings, Ending> {
        let inner = &self.runtime.inner;
        let summary = inner.store.conversation_summary(&self.authority).await?;
        let workspace = summary
            .as_ref()
            .and_then(|summary| summary.workspace.as_ref())
            .map(PathBuf::from);
        let full_access = summary.as_ref().is_some_and(|summary| summary.full_access);
        let project = summary.and_then(|summary| summary.project_id);
        let projects = inner.store.projects().await?;
        let PolicyGrants {
            settings,
            conversation: grants,
            project: project_grants,
            always: always_grants,
            rules,
            remote_rules,
            program_rules,
        } = self
            .runtime
            .policy_grants(Some(&self.authority), project.as_ref())
            .await
            .map_err(|failure| internal(&failure.error, failure.what))?;
        let current = project
            .as_ref()
            .and_then(|id| projects.iter().find(|candidate| candidate.id == *id))
            .cloned();
        // A child knows only its parent's project and never moves a
        // conversation.
        let projects = if self.child.is_some() {
            ProjectFacts {
                all: Arc::from(current.clone().into_iter().collect::<Vec<_>>()),
                current,
                ..ProjectFacts::none()
            }
        } else {
            ProjectFacts {
                current,
                all: Arc::from(projects),
                moves: Arc::new(Moves {
                    runtime: self.runtime.clone(),
                }),
            }
        };
        let (workspace, ceiling, host) = match self.ceiling() {
            Some((ceiling, host)) => (
                ceiling.allowed_folders.first().cloned(),
                Some(ceiling),
                host,
            ),
            None => (workspace, None, None),
        };
        let runtime = self.runtime.clone();
        // Archives belong to the conversation of the call, also a child's.
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
            projects,
            // The commands belong to the call's conversation, also a
            // child's (ADR-0041).
            background: Arc::new(BackgroundPort {
                runtime: self.runtime.clone(),
            }),
        };
        Ok(Surroundings {
            context,
            settings,
            grants,
            project_grants,
            always_grants,
            rules,
            remote_rules,
            program_rules,
            project,
            ceiling,
            host,
            full_access,
        })
    }

    /// On a machine with a ceiling, the ceiling of the run's calls, narrowed
    /// as its task asks, with the machine's name; the first allowed folder is
    /// the workspace (ADR-0028, decision 5). `None` on a machine without one.
    fn ceiling(&self) -> Option<(Ceiling, Option<String>)> {
        let machine = self.runtime.machine();
        let ceiling = machine.ceiling.as_ref()?;
        let read_only = self
            .assigned
            .as_ref()
            .is_some_and(|assigned| assigned.read_only);
        let ceiling = ceiling.narrowed(read_only.then_some(CeilingMode::ReadOnly));
        Some((ceiling, machine.name.clone()))
    }

    /// Stores a call's result and finishes its part. Archived outputs and the
    /// images that reach the model are stored with it, the images checked and
    /// without their metadata (ADR-0044).
    fn complete_call(&mut self, mut call: StepCall, done: Done) {
        let offered = done.images.len();
        let (output, images) = tool_images::clean_result(done.output, &done.images);
        if images.len() < offered {
            tracing::info!(run = %self.run, part = %call.part_id, offered, kept = images.len(), "images of a tool's result were left out");
        }
        let image_refs = images
            .iter()
            .map(|(position, image)| ToolImageRef {
                position: *position,
                format: image.format,
                width: image.width,
                height: image.height,
                bytes: u32::try_from(image.data.len()).unwrap_or(u32::MAX),
            })
            .collect();
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
            output,
            archives: references,
            images: image_refs,
        });
        call.document.ended_at = Some(self.now());
        tracing::info!(run = %self.run, part = %call.part_id, tool = %call.document.tool, "a tool call completed");
        self.pending.push(Change::UpdatePart {
            part_id: call.part_id.clone(),
            tool_call: call.document.clone(),
            archives,
        });
        if !images.is_empty() {
            self.pending.push(Change::AddToolImages {
                part_id: call.part_id.clone(),
                images: images
                    .into_iter()
                    .map(|(position, image)| NewToolImage {
                        position,
                        format: image.format,
                        width: image.width,
                        height: image.height,
                        data: image.data,
                    })
                    .collect(),
            });
        }
        self.pending.push(Change::FinishPart {
            part_id: call.part_id,
            state: mannie_types::transcript::PartState::Complete,
            tail: String::new(),
            provider_item: None,
        });
        self.schedule_flush();
    }
}
