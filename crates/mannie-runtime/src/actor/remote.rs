//! The side of a run that gives tasks to the executors of plugins (ADR-0031,
//! decision 4): the call of an executor's tool, the part, the rows and the
//! submission, follow-ups on the same target, and the cancels of a stop.
//!
//! An executor's task joins the run's lane as a child's task does. Its states
//! come from the executor, which publishes them on the run's board while the
//! run holds the task's [`RemoteFollow`].

use std::sync::Arc;

use mannie_plugin_sdk::points::executors::{
    ExecutorRefusal, PlanCall, PlanError, SubmitError, TaskExecutor, TaskSubmission,
};
use mannie_store::{
    Change, Durability, NewConversation, NewExecutorTask, RuntimeStoreError, StoredChild,
    StoredTask, TaskEnding, TaskOrigin,
};
use mannie_types::ids::TaskId;
use mannie_types::run::RunFailure;
use mannie_types::tasks::{
    INSTRUCTION_PREVIEW_CHARS, ProfileSnapshot, TaskDocument, TaskRefusal, TaskState,
};
use mannie_types::tools::{NotRunReason, ToolCallStatus};
use mannie_types::transcript::PartState;

use super::tasks::{LaneTask, let_go_text};
use super::{Ending, MAX_DETAIL_CHARS, RunActor, StepCall};
use crate::calls::Setup;
use crate::executors::RemoteFollow;
use crate::texts;

/// The start of an instruction, which the parent's part keeps for the owner.
fn instruction_preview(instruction: &str) -> String {
    instruction
        .chars()
        .take(INSTRUCTION_PREVIEW_CHARS)
        .collect()
}

/// What a call of an executor's tool stores and submits, past its checks.
struct Given {
    document: TaskDocument,
    /// The task's conversation, for a new task.
    conversation: Option<NewConversation>,
    instruction: String,
    options: serde_json::Value,
    profile: ProfileSnapshot,
}

impl RunActor {
    /// Runs a call of an executor's tool: lets the executor check the
    /// arguments, checks the budget, lets the executor take the call on, then
    /// gives it the task.
    pub(super) async fn give_task(
        &mut self,
        setup: &Setup,
        call: StepCall,
        offered: Arc<dyn TaskExecutor>,
    ) -> Result<(), Ending> {
        // A run keeps the tools it started with. A call of the tool of a
        // plugin that stopped since does not run (ADR-0030, decision 8), and
        // one of a plugin that started again goes to its new executor.
        let kind = offered.kind();
        let Some(executor) = self.runtime.executor(kind) else {
            let refusal = ExecutorRefusal::new("plugin_off", None, texts::tool_plugin_off(kind));
            self.refuse_by_executor(call, kind, refusal);
            return Ok(());
        };
        let profile = self.profile_snapshot().await?;
        let arguments = call.arguments.clone().unwrap_or_default();
        let planned = executor.plan(PlanCall::new(&arguments, &profile));
        let plan = match planned {
            Err(PlanError::InvalidArguments { detail }) => {
                let detail = detail.chars().take(MAX_DETAIL_CHARS).collect();
                self.end_call(call, NotRunReason::InvalidArguments { detail });
                return Ok(());
            }
            _ if self.task_budget_used(setup) => {
                self.refuse_task(call, TaskRefusal::BudgetUsed);
                return Ok(());
            }
            Err(PlanError::Refused(refusal)) => {
                self.refuse_by_executor(call, executor.kind(), refusal);
                return Ok(());
            }
            Ok(plan) => plan,
        };
        let number = self
            .runtime
            .inner
            .store
            .next_task_number(&self.conversation)
            .await?;
        let ids = &self.runtime.inner.ids;
        let document = TaskDocument {
            task_id: ids.task(),
            number,
            conversation_id: ids.conversation(),
            agent: executor.agent().to_owned(),
            title: plan.title,
            background: plan.background,
            follow_up: false,
            state: TaskState::Accepted,
            instruction: instruction_preview(&plan.instruction),
            activity: None,
            tool_calls: 0,
            approval: None,
            report: None,
            failure: None,
            started_at: None,
            ended_at: None,
            worker: None,
            executor: Some(plan.target.clone()),
            detached: false,
        };
        let conversation = NewConversation {
            id: document.conversation_id.clone(),
            title: document.title.clone(),
            profile_id: Some(self.profile.clone()),
            project_id: None,
            task: Some(TaskOrigin {
                parent_id: self.conversation.clone(),
                number,
                agent: document.agent.clone(),
                executor: Some(plan.target),
            }),
        };
        let given = Given {
            document,
            conversation: Some(conversation),
            instruction: plan.instruction,
            options: plan.options,
            profile,
        };
        self.submit_to_executor(call, &executor, given).await
    }

    /// Gives an executor a further task in the conversation of an earlier
    /// one, once that task ended. The target that ran it runs it again.
    pub(super) async fn follow_up_on_executor(
        &mut self,
        call: StepCall,
        child: StoredChild,
        instruction: String,
        background: bool,
    ) -> Result<(), Ending> {
        let number = child.number;
        let Some(earlier) = child.executor else {
            self.refuse_task(call, TaskRefusal::UnknownTask { number });
            return Ok(());
        };
        let still_running = child
            .latest
            .as_ref()
            .is_some_and(|task| task.state.is_active());
        if still_running {
            self.refuse_task(call, TaskRefusal::StillRunning { number });
            return Ok(());
        }
        let Some(executor) = self.runtime.executor(&earlier.kind) else {
            let refusal = ExecutorRefusal::new(
                "plugin_off",
                Some(earlier.target_name.clone()),
                texts::task_plugin_off(number, &earlier.kind),
            );
            self.refuse_by_executor(call, &earlier.kind, refusal);
            return Ok(());
        };
        let profile = self.profile_snapshot().await?;
        let plan = match executor.plan_follow_up(&earlier, &profile) {
            Ok(plan) => plan,
            Err(refusal) => {
                self.refuse_by_executor(call, executor.kind(), refusal);
                return Ok(());
            }
        };
        let document = TaskDocument {
            task_id: self.runtime.inner.ids.task(),
            number,
            conversation_id: child.conversation_id,
            agent: child.agent,
            title: child.title,
            background,
            follow_up: true,
            state: TaskState::Accepted,
            instruction: instruction_preview(&instruction),
            activity: None,
            tool_calls: 0,
            approval: None,
            report: None,
            failure: None,
            started_at: None,
            ended_at: None,
            worker: None,
            executor: Some(plan.target),
            detached: false,
        };
        let given = Given {
            document,
            conversation: None,
            instruction,
            options: plan.options,
            profile,
        };
        self.submit_to_executor(call, &executor, given).await
    }

    /// Gives an executor its task (ADR-0027, decision 3; ADR-0031): the part
    /// records the task first; then the task's rows are stored, with the
    /// executor's document; then the executor submits the task. Mannie's end
    /// after the part and before the rows leaves the part "not started";
    /// after the rows, the part lets the task go, and the executor learns
    /// what became of it when it starts again.
    async fn submit_to_executor(
        &mut self,
        mut call: StepCall,
        executor: &Arc<dyn TaskExecutor>,
        given: Given,
    ) -> Result<(), Ending> {
        let Given {
            document,
            conversation,
            instruction,
            options,
            profile,
        } = given;
        let Some(target) = document.executor.clone() else {
            return Err(Ending::Failed(RunFailure::Internal));
        };
        let submission = TaskSubmission::new(
            document.task_id.clone(),
            document.conversation_id.clone(),
            self.conversation.clone(),
            document.number,
            target,
            profile,
        )
        .with_title(document.title.clone())
        .with_instruction(instruction)
        .with_options(options)
        .with_follow_up(document.follow_up);
        self.tasks.assigned += 1;
        self.record_in_part(&mut call, &document).await?;
        let Some(row) = self
            .record_rows(&mut call, &document, conversation, executor, &submission)
            .await?
        else {
            self.refuse_task(
                call,
                TaskRefusal::StillRunning {
                    number: document.number,
                },
            );
            return Ok(());
        };
        // The run holds the task's states before the executor can publish
        // any.
        let follow = RemoteFollow::new(&self.runtime, &row.id, executor.kind(), &self.tasks.board);
        let submitted = self.wait(executor.submit(submission)).await;
        let state = match submitted {
            Ok(Ok(state)) => state,
            Ok(Err(SubmitError::Refused(refusal))) => {
                tracing::info!(run = %self.run, task = %row.id, kind = executor.kind(), code = %refusal.code, "an executor did not take a task");
                self.end_unsubmitted(executor.kind(), &row.id).await;
                if let Some(section) = &mut call.task {
                    section.task = None;
                }
                self.refuse_by_executor(call, executor.kind(), refusal);
                return Ok(());
            }
            Ok(Err(SubmitError::Stopped)) => {
                tracing::info!(run = %self.run, task = %row.id, kind = executor.kind(), "the plugin of a task stopped during its submission; the run lets the task go");
                drop(follow);
                self.let_go_unconfirmed(call, document);
                return self.flush(Durability::Full).await;
            }
            Err(ending) => {
                // A stop during the submission cancels the task, which the
                // target may have stored; the executor keeps the cancel.
                if matches!(ending, Ending::Stopped) {
                    executor.cancel(&row.id).await;
                }
                return Err(ending);
            }
        };
        if let Err(error) = self
            .runtime
            .publish_task_state(executor.kind(), state)
            .await
        {
            tracing::error!(%error, error.code = error.code(), task = %row.id, "could not store the first state of an executor's task");
        }
        tracing::info!(
            run = %self.run,
            task = %row.id,
            number = document.number,
            kind = executor.kind(),
            follow_up = document.follow_up,
            background = document.background,
            "an executor got a task"
        );
        self.add_remote_task(call, document, follow);
        self.flush(Durability::Full).await
    }

    /// Records a task in its call's part, durably, before anything else
    /// stores it (step 1 of a submission).
    async fn record_in_part(
        &mut self,
        call: &mut StepCall,
        document: &TaskDocument,
    ) -> Result<(), Ending> {
        call.document.status = ToolCallStatus::Running;
        call.document.started_at = Some(self.now());
        let mut section = call.task.take().unwrap_or_default();
        section.task = Some(document.clone());
        call.task = Some(section);
        self.pending.push(call.update(Vec::new()));
        self.flush(Durability::Full).await
    }

    /// Stores a task's rows, and its conversation's for a new task, with the
    /// executor's document (step 2 of a submission). `None` when the task's
    /// conversation still works on an earlier task: nothing was stored, and
    /// the part no longer records the task.
    async fn record_rows(
        &mut self,
        call: &mut StepCall,
        document: &TaskDocument,
        conversation: Option<NewConversation>,
        executor: &Arc<dyn TaskExecutor>,
        submission: &TaskSubmission,
    ) -> Result<Option<StoredTask>, Ending> {
        let recorded = self
            .runtime
            .inner
            .store
            .record_executor_task(NewExecutorTask {
                id: document.task_id.clone(),
                conversation_id: document.conversation_id.clone(),
                conversation,
                part_id: call.part_id.clone(),
                kind: executor.kind().to_owned(),
                target: submission.target.target_id.clone(),
                document: executor.document(submission),
                follow_up: document.follow_up,
                background: document.background,
            })
            .await;
        match recorded {
            Ok(row) => Ok(Some(row)),
            Err(RuntimeStoreError::TaskActive) => {
                self.tasks.assigned -= 1;
                if let Some(section) = &mut call.task {
                    section.task = None;
                }
                Ok(None)
            }
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), run = %self.run, "could not store a task for an executor");
                Err(Ending::Failed(RunFailure::Internal))
            }
        }
    }

    /// Keeps a task whose submission its plugin's stop cut short in the
    /// lane, let go: the target may have stored it, so its row stays active
    /// and its part says that it may run there, also for a task in the
    /// background, whose result the model reads now.
    fn let_go_unconfirmed(&mut self, mut call: StepCall, mut document: TaskDocument) {
        document.detached = true;
        let mut section = call.task.take().unwrap_or_default();
        section.task = None;
        section.output = Some(let_go_text(&document));
        call.document.status = ToolCallStatus::Completed;
        call.document.ended_at = Some(self.now());
        let task = LaneTask {
            id: document.task_id.clone(),
            part_id: call.part_id,
            call: call.document,
            section,
            document,
            mailbox: None,
            follow: None,
            ending: None,
            read: true,
            noted: true,
            let_go: true,
        };
        self.pending.push(task.update(Vec::new()));
        self.pending.push(Change::FinishPart {
            part_id: task.part_id.clone(),
            state: PartState::Complete,
            tail: String::new(),
            provider_item: None,
        });
        self.tasks.tasks.push(task);
    }

    /// Ends the row of a task that its executor did not take.
    async fn end_unsubmitted(&self, kind: &str, task: &TaskId) {
        let ending = TaskEnding {
            state: TaskState::Interrupted,
            report: None,
            failure: None,
        };
        if let Err(error) = self
            .runtime
            .inner
            .store
            .apply_executor_task_state(task, kind, 0, TaskState::Interrupted, Some(ending))
            .await
        {
            tracing::error!(%error, error.code = error.code(), task = %task, "could not end a task that its executor did not take");
        }
    }

    /// Cancels the run's tasks that executors still run, because the run
    /// stops: each executor keeps its cancels until their targets take them.
    pub(super) async fn cancel_remote_tasks(&mut self) {
        let remote: Vec<(TaskId, String)> = self
            .tasks
            .tasks
            .iter()
            .filter(|task| task.ending.is_none() && task.mailbox.is_none())
            .filter_map(|task| {
                let kind = task.document.executor.as_ref()?.kind.clone();
                Some((task.id.clone(), kind))
            })
            .collect();
        for (task, kind) in &remote {
            match self.runtime.executor(kind) {
                Some(executor) => executor.cancel(task).await,
                None => {
                    tracing::info!(run = %self.run, task = %task, kind = %kind, "a stop found the plugin of a task off; the task runs on");
                }
            }
        }
        if !remote.is_empty() {
            tracing::info!(run = %self.run, tasks = remote.len(), "canceling the run's tasks of executors");
        }
    }

    /// Ends a call of an executor's tool that assigned no task: the part
    /// keeps the text that the model reads and the executor's code for the
    /// owner.
    pub(super) fn refuse_by_executor(
        &mut self,
        mut call: StepCall,
        kind: &str,
        refusal: ExecutorRefusal,
    ) {
        let section = call
            .task
            .get_or_insert_with(mannie_types::tasks::TaskPartDocument::default);
        section.output = Some(refusal.text);
        section.refusal = Some(TaskRefusal::Executor {
            kind: kind.to_owned(),
            code: refusal.code,
            target: refusal.target,
        });
        tracing::info!(run = %self.run, part = %call.part_id, tool = %call.document.tool, kind, "an executor's call assigned no task");
        self.complete_task_call(call, Vec::new());
    }

    /// The snapshot of the run's profile that an executor gets with a task,
    /// without its credential.
    async fn profile_snapshot(&self) -> Result<ProfileSnapshot, Ending> {
        let profile = self
            .runtime
            .inner
            .store
            .profile(&self.profile)
            .await
            .map_err(|error| {
                tracing::error!(%error, error.code = error.code(), "could not read the profile of a run");
                Ending::Failed(RunFailure::Internal)
            })?
            .ok_or(Ending::Failed(RunFailure::ProfileMissing))?;
        Ok(ProfileSnapshot {
            id: profile.id.clone(),
            config_revision: profile.config_revision,
            interface: profile.interface,
            base_url: profile.base_url.clone(),
            model: profile.model.clone(),
            context_window: profile.context_window,
            output_limit: profile.output_limit,
            reasoning_effort: profile.reasoning_effort,
            server: profile.server,
            use_tools: profile.use_tools,
            has_credential: profile.credential_ref.is_some(),
        })
    }

    /// Keeps an executor's task in the lane until it ends. A task in the
    /// background gets its result at once.
    fn add_remote_task(
        &mut self,
        mut call: StepCall,
        document: TaskDocument,
        follow: RemoteFollow,
    ) {
        let mut section = call.task.take().unwrap_or_default();
        section.task = None;
        let mut task = LaneTask {
            id: document.task_id.clone(),
            part_id: call.part_id,
            call: call.document,
            section,
            document,
            mailbox: None,
            follow: Some(follow),
            ending: None,
            read: false,
            noted: false,
            let_go: false,
        };
        if task.document.background
            && let Some(executor) = &task.document.executor
        {
            // The model reads this result at once; the part stays open until
            // the task ends.
            task.section.output = Some(texts::task_started_in_background_on(
                task.document.number,
                &executor.noun,
                &executor.target_name,
            ));
            self.pending.push(task.update(Vec::new()));
        }
        self.tasks.tasks.push(task);
    }
}
