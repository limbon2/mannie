//! The side of a run that gives tasks to child agents
//! (docs/features/child-agents/tasks.md; ADR-0024): starts and follow-ups,
//! waits for reports, the states that children publish, and the end of the
//! children with their parent. The tasks of plugins' executors share the
//! lane, the board and the waits (ADR-0031); `remote.rs` gives them.
//!
//! The run is the only writer of its conversation. A task's part stays open
//! while the task runs, and the run keeps the part's documents here: every
//! change to them is stored at once, in a batch of its own or with the
//! run's pending changes, so a change that a child publishes never meets an
//! older document in the same batch.

use std::sync::Arc;

use mannie_plugin_sdk::points::executors::ExecutorRefusal;
use mannie_store::{
    Change, Durability, NewArchive, NewConversation, NewRun, NewTask, RuntimeStoreError,
    TaskEnding, TaskOrigin,
};
use mannie_types::clock::UtcMillis;
use mannie_types::ids::{ConversationId, PartId, RunId, TaskId, TurnId};
use mannie_types::run::{FinishReason, RunActivity, RunFailure, TaskKindCount};
use mannie_types::tasks::{
    AwaitedTask, INSTRUCTION_PREVIEW_CHARS, MAX_REPORT_VIEW_BYTES, TaskDocument, TaskPartDocument,
    TaskRefusal, TaskState,
};
use mannie_types::tools::{ArchiveRef, NotRunReason, ToolCallDocument, ToolCallStatus};
use mannie_types::transcript::PartState;
use tokio::sync::mpsc;

use super::{ChildLink, Command, Ending, MAX_DETAIL_CHARS, RunActor, StepCall, halt};
use crate::calls::Setup;
use crate::executors::RemoteFollow;
use crate::ids::IdGenerator;
use crate::tasks::{
    ChildEnd, ChildGuard, ChildState, TaskBoard, TaskCall, TaskSink, doer_of, parse_call,
    render_report,
};
use crate::{Runtime, lock, texts};

/// The least bytes of the result limit that each report of a wait gets.
const MIN_REPORT_BYTES: usize = 2_048;

/// A task that the run gave, while its part is open.
#[derive(Debug)]
pub(super) struct LaneTask {
    pub(super) id: TaskId,
    pub(super) part_id: PartId,
    /// The call record of the task's part.
    pub(super) call: ToolCallDocument,
    /// The task section of the part, without the task, which `document`
    /// holds.
    pub(super) section: TaskPartDocument,
    pub(super) document: TaskDocument,
    /// The child's run, which the run stops when it ends; `None` for an
    /// executor's task, which the run cancels through its executor.
    pub(super) mailbox: Option<mpsc::Sender<Command>>,
    /// For an executor's task, the run's hold on its states.
    pub(super) follow: Option<RemoteFollow>,
    /// How the task ended, once the run stored it.
    pub(super) ending: Option<TaskEnding>,
    /// Whether the model read the task's report.
    pub(super) read: bool,
    /// Whether a note asked the model to read the report.
    pub(super) noted: bool,
    /// Whether the run let the task go because its executor's plugin
    /// stopped: the task may still run on its target (ADR-0031, decision
    /// 10).
    pub(super) let_go: bool,
}

/// A task of an earlier reply that an executor still runs, which a wait of
/// this run takes up again (ADR-0027, decision 6).
#[derive(Debug)]
struct FollowedTask {
    id: TaskId,
    /// The kind of the executor that runs it.
    kind: String,
    ending: Option<TaskEnding>,
    /// Whether the executor's plugin stopped while the wait followed the
    /// task.
    let_go: bool,
    _follow: RemoteFollow,
}

impl FollowedTask {
    fn is_active(&self) -> bool {
        self.ending.is_none() && !self.let_go
    }
}

impl LaneTask {
    fn is_active(&self) -> bool {
        self.ending.is_none() && !self.let_go
    }

    /// Whether an executor runs the task.
    fn is_remote(&self) -> bool {
        self.mailbox.is_none()
    }

    /// The kind of the executor that runs the task.
    fn executor_kind(&self) -> Option<&str> {
        self.document
            .executor
            .as_ref()
            .map(|executor| executor.kind.as_str())
    }

    /// The change that stores the task's part as it is now.
    pub(super) fn update(&self, archives: Vec<NewArchive>) -> Change {
        let mut section = self.section.clone();
        section.task = Some(self.document.clone());
        Change::UpdateTaskPart {
            part_id: self.part_id.clone(),
            tool_call: self.call.clone(),
            task: Box::new(section),
            archives,
        }
    }
}

/// The tasks that a run gave, and what its own work does.
#[derive(Debug)]
pub(super) struct TaskLane {
    /// Where the run's children publish their states.
    pub(super) board: Arc<TaskBoard>,
    /// The tasks, in the order the run gave them.
    pub(super) tasks: Vec<LaneTask>,
    /// The tasks of earlier replies that executors run, which a wait
    /// follows.
    followed: Vec<FollowedTask>,
    /// The starts and follow-ups of the reply, against its task budget.
    pub(super) assigned: u32,
    /// What the run's own work does. A child's approval request covers it
    /// while the owner has to answer the request.
    activity: RunActivity,
    /// The result limit of the run's calls, in bytes.
    result_limit: usize,
}

impl Default for TaskLane {
    fn default() -> Self {
        Self {
            board: Arc::default(),
            tasks: Vec::new(),
            followed: Vec::new(),
            assigned: 0,
            activity: RunActivity::Preparing,
            result_limit: MIN_REPORT_BYTES,
        }
    }
}

/// What the run stores of its children's states.
#[derive(Debug, Default)]
struct Mirror {
    changes: Vec<Change>,
    /// Whether a task's state, its approval request or its end changed,
    /// which is stored durably; a child's activity alone is not.
    durable: bool,
    /// Whether a child's approval request appeared or went away, which
    /// changes what the run shows.
    approval_changed: bool,
}

impl TaskLane {
    pub(super) fn board(&self) -> &Arc<TaskBoard> {
        &self.board
    }

    pub(super) fn set_result_limit(&mut self, limit: usize) {
        self.result_limit = limit;
    }

    pub(super) fn set_activity(&mut self, activity: RunActivity) {
        self.activity = activity;
    }

    /// What the run shows that it does: its own work, unless a child waits
    /// for the owner while the run's own work does not. The activity then
    /// names the child's task part, which holds the request.
    pub(super) fn shown_activity(&self) -> RunActivity {
        if matches!(self.activity, RunActivity::AwaitingApproval { .. }) {
            return self.activity.clone();
        }
        match self.child_waiting() {
            Some(part_id) => RunActivity::AwaitingApproval {
                part_id: part_id.clone(),
            },
            None => self.activity.clone(),
        }
    }

    /// The part of the first task whose child waits for the owner.
    fn child_waiting(&self) -> Option<&PartId> {
        self.tasks
            .iter()
            .find(|task| task.document.approval.is_some())
            .map(|task| &task.part_id)
    }

    /// Whether the reply has to wait before it ends: a task still runs, or
    /// a report is unread and no note asked for it yet.
    pub(super) fn has_work_left(&self) -> bool {
        self.tasks
            .iter()
            .any(|task| task.is_active() || (!task.read && !task.noted))
    }

    /// Sends `command` to every child that still works. Executors' tasks
    /// have no mailbox: the run cancels them through their executors.
    fn stop_all(&self, command: Command) {
        for task in self.tasks.iter().filter(|task| task.is_active()) {
            if let Some(mailbox) = &task.mailbox {
                // A full mailbox already holds a command that ends the run.
                let _delivered = mailbox.try_send(command);
            }
        }
    }

    /// The latest task of the child with this number, by its index.
    fn latest(&self, number: u32) -> Option<usize> {
        self.tasks
            .iter()
            .rposition(|task| task.document.number == number)
    }

    /// The numbers of the tasks that ended with reports the model did not
    /// read and was not asked to read yet. They count as asked for from now.
    fn take_unread(&mut self) -> Vec<u32> {
        let mut numbers = Vec::new();
        for task in &mut self.tasks {
            if !task.is_active() && !task.read && !task.noted {
                task.noted = true;
                numbers.push(task.document.number);
            }
        }
        numbers.sort_unstable();
        numbers.dedup();
        numbers
    }

    /// Takes what the children published since the run last looked into the
    /// tasks' documents, and returns the changes that store it. A task that
    /// ended finishes its part, with the report as its result unless it runs
    /// in the background, where `wait_for_tasks` returns the report. A task
    /// whose executor's plugin stopped is let go.
    fn absorb(&mut self, ids: &IdGenerator, now: UtcMillis) -> Mirror {
        let published = self.board.take_changes();
        let waiting_before = self.child_waiting().cloned();
        let mut mirror = Mirror::default();
        for (task_id, state) in published {
            let Some(task) = self
                .tasks
                .iter_mut()
                .find(|task| task.id == task_id && task.is_active())
            else {
                // A task of an earlier reply that a wait follows has no part
                // in this run: only its end matters, or that it was let go.
                let followed = self
                    .followed
                    .iter_mut()
                    .find(|followed| followed.id == task_id && followed.is_active());
                if let Some(followed) = followed {
                    if let Some(end) = &state.end {
                        followed.ending = Some(end.ending.clone());
                        self.board.forget(&task_id);
                    } else if state.let_go {
                        followed.let_go = true;
                        self.board.forget(&task_id);
                    }
                }
                continue;
            };
            if state.let_go && state.end.is_none() {
                mirror.durable = true;
                mirror.changes.extend(let_go(task, now));
                self.board.forget(&task_id);
                continue;
            }
            mirror.durable |= apply_state(&mut task.document, &state);
            match &state.end {
                None => mirror.changes.push(task.update(Vec::new())),
                Some(end) => {
                    mirror.durable = true;
                    mirror
                        .changes
                        .extend(complete(task, end, self.result_limit, ids));
                    self.board.forget(&task_id);
                }
            }
        }
        mirror.approval_changed = self.child_waiting() != waiting_before.as_ref();
        mirror
    }
}

/// Applies a child's state to its task's document. Whether something
/// changed that is stored durably: the task's state or its approval request.
fn apply_state(document: &mut TaskDocument, state: &ChildState) -> bool {
    let before = (document.state, document.approval.clone());
    document.state = state.task_state();
    document.started_at = state.started_at.or(document.started_at);
    document.activity.clone_from(&state.activity);
    document.tool_calls = state.tool_calls;
    document.approval.clone_from(&state.approval);
    if let Some(end) = &state.end {
        document.activity = None;
        document.approval = None;
        document.report = end.ending.report.as_deref().map(report_view);
        document.failure.clone_from(&end.ending.failure);
        document.ended_at = Some(end.ended_at);
    }
    (document.state, document.approval.clone()) != before
}

/// Finishes the part of a task that ended. A task that the run waits for in
/// its step gets the report as its result, archived whole when it is longer
/// than the result limit.
fn complete(task: &mut LaneTask, end: &ChildEnd, limit: usize, ids: &IdGenerator) -> Vec<Change> {
    let mut archives = Vec::new();
    if !task.document.background {
        let output = render_report(
            task.document.number,
            &end.ending,
            doer_of(&task.document),
            limit,
            || ids.archive(),
        );
        task.section.output = Some(output.output);
        if let Some((archive, reference)) = output.archive {
            archives.push(archive);
            task.section.archives.push(reference);
        }
        task.read = true;
    }
    task.call.status = ToolCallStatus::Completed;
    task.call.ended_at = Some(end.ended_at);
    task.ending = Some(end.ending.clone());
    task.follow = None;
    tracing::info!(
        task = %task.id,
        number = task.document.number,
        agent = %task.document.agent,
        executor = task.document.executor.as_ref().map(|executor| executor.kind.as_str()),
        state = end.ending.state.as_str(),
        tool_calls = task.document.tool_calls,
        "a task ended"
    );
    vec![
        task.update(archives),
        Change::FinishPart {
            part_id: task.part_id.clone(),
            state: PartState::Complete,
            tail: String::new(),
            provider_item: None,
        },
    ]
}

/// The result of an executor's task that the run let go: it may still run
/// on its target.
pub(super) fn let_go_text(document: &TaskDocument) -> String {
    let (noun, target) = document
        .executor
        .as_ref()
        .map_or((texts::THE_CHILD, ""), |executor| {
            (executor.noun.as_str(), executor.target_name.as_str())
        });
    texts::task_let_go(document.number, noun, target)
}

/// Lets go of an executor's task whose plugin stopped (ADR-0031, decision
/// 10): its part ends with the result that the task may still run on its
/// target, and a wait takes the task up again once the plugin is back on. A
/// task in the background, whose result the model read when it started,
/// keeps that result, and a wait for the reply's reports says that it was
/// let go.
fn let_go(task: &mut LaneTask, now: UtcMillis) -> Vec<Change> {
    task.let_go = true;
    task.follow = None;
    task.document.detached = true;
    task.document.activity = None;
    task.document.approval = None;
    if task.document.background {
        task.noted = true;
    } else {
        task.section.output = Some(let_go_text(&task.document));
        task.read = true;
    }
    task.call.status = ToolCallStatus::Completed;
    task.call.ended_at = Some(now);
    tracing::info!(
        task = %task.id,
        number = task.document.number,
        executor = task.document.executor.as_ref().map(|executor| executor.kind.as_str()),
        "a run let go of a task whose plugin stopped"
    );
    vec![
        task.update(Vec::new()),
        Change::FinishPart {
            part_id: task.part_id.clone(),
            state: PartState::Complete,
            tail: String::new(),
            provider_item: None,
        },
    ]
}

/// The start of an instruction, which the parent's part keeps for the owner.
fn instruction_preview(instruction: &str) -> String {
    instruction
        .chars()
        .take(INSTRUCTION_PREVIEW_CHARS)
        .collect()
}

/// A report as the parent's part keeps it for the owner, at most
/// [`MAX_REPORT_VIEW_BYTES`] bytes.
fn report_view(report: &str) -> String {
    let mut end = report.len().min(MAX_REPORT_VIEW_BYTES);
    while !report.is_char_boundary(end) {
        end -= 1;
    }
    report[..end].to_owned()
}

/// A task whose report a wait returns.
#[derive(Debug)]
enum Awaited {
    /// A task of this run, by its index.
    Lane(usize),
    /// A child's latest task from an earlier reply, which ended.
    Earlier {
        number: u32,
        title: String,
        ending: TaskEnding,
        doer: String,
    },
    /// An executor's task from an earlier reply that still runs, which the
    /// wait follows to its end.
    Followed {
        number: u32,
        title: String,
        task: TaskId,
        /// The kind of the executor that runs it.
        kind: String,
        doer: String,
        /// The name of the target that runs it.
        target: String,
    },
    /// An executor's task that still runs while its plugin is off, which
    /// no wait can follow (ADR-0031, decision 10).
    PluginOff {
        number: u32,
        kind: String,
        target: String,
    },
    /// A number that no child of the conversation has.
    Unknown(u32),
}

/// The ending of a task that has none stored: it never ended while Mannie
/// ran.
fn interrupted() -> TaskEnding {
    TaskEnding {
        state: TaskState::Interrupted,
        report: None,
        failure: None,
    }
}

impl Runtime {
    /// Whether a conversation has a run, or a command that is starting one.
    fn is_busy(&self, conversation: &ConversationId) -> bool {
        lock(&self.inner.busy).contains_key(conversation)
    }
}

impl RunActor {
    /// Stores what the run's children published since the last time.
    pub(super) async fn mirror_children(&mut self) -> Result<(), Ending> {
        let now = self.now();
        let mirror = self.tasks.absorb(&self.runtime.inner.ids, now);
        let mut changes = mirror.changes;
        if mirror.approval_changed {
            changes.push(self.shown_activity());
        }
        if changes.is_empty() {
            return Ok(());
        }
        let durability = if mirror.durable || mirror.approval_changed {
            Durability::Full
        } else {
            Durability::Normal
        };
        self.apply(changes, durability).await.map(drop)
    }

    /// Waits until the run's tasks that `awaited` picks have ended, and the
    /// tasks of earlier replies that a wait follows, storing what the
    /// children and the workers publish meanwhile. The run holds no model
    /// slot. Stop and Shutdown end the wait.
    async fn await_tasks(&mut self, awaited: impl Fn(&LaneTask) -> bool) -> Result<(), Ending> {
        let board = Arc::clone(&self.tasks.board);
        loop {
            self.mirror_children().await?;
            let waited: Vec<&LaneTask> = self
                .tasks
                .tasks
                .iter()
                .filter(|task| task.is_active() && awaited(task))
                .collect();
            let followed = self
                .tasks
                .followed
                .iter()
                .filter(|followed| followed.is_active())
                .count();
            let count = waited.len() + followed;
            if count == 0 {
                return Ok(());
            }
            let waiting = RunActivity::WaitingForTasks {
                count: u32::try_from(count).unwrap_or(u32::MAX),
                workers: 0,
                kinds: self.waited_kinds(&waited),
            };
            if self.tasks.activity != waiting {
                let change = self.activity(waiting);
                self.apply(vec![change], Durability::Normal).await?;
            }
            tokio::select! {
                biased;
                command = self.commands.recv() => return Err(halt(command)),
                () = board.changed() => {}
            }
        }
    }

    /// How many of the waited tasks, and of the followed ones, each
    /// executor runs.
    fn waited_kinds(&self, waited: &[&LaneTask]) -> Vec<TaskKindCount> {
        let mut kinds: Vec<TaskKindCount> = Vec::new();
        let followed = self
            .tasks
            .followed
            .iter()
            .filter(|followed| followed.is_active())
            .map(|followed| followed.kind.as_str());
        for kind in waited
            .iter()
            .filter_map(|task| task.executor_kind())
            .chain(followed)
        {
            match kinds.iter_mut().find(|counted| counted.kind == kind) {
                Some(counted) => counted.count += 1,
                None => kinds.push(TaskKindCount {
                    kind: kind.to_owned(),
                    count: 1,
                }),
            }
        }
        kinds
    }

    /// Waits for the tasks of the step that do not run in the background:
    /// the step ends when they reported.
    pub(super) async fn await_foreground_tasks(&mut self) -> Result<(), Ending> {
        self.await_tasks(|task| !task.document.background).await
    }

    /// Waits for every task of the reply before it ends. When reports are
    /// unread then, a note of Mannie's own asks the model to read them, once
    /// for each task, and the reply continues: `Ok(true)`. The note names
    /// the tasks and holds none of their text (T-21).
    pub(super) async fn await_reply_tasks(&mut self, setup: &Setup) -> Result<bool, Ending> {
        self.await_tasks(|_task| true).await?;
        // A model without tool calls left cannot call wait_for_tasks.
        if self.tools_used_up(setup) {
            return Ok(false);
        }
        let unread = self.tasks.take_unread();
        if unread.is_empty() {
            return Ok(false);
        }
        tracing::info!(run = %self.run, tasks = ?unread, "a reply has unread reports; asking the model to read them");
        self.add_note(&texts::unread_reports(&unread), false)
            .await?;
        Ok(true)
    }

    /// Stops the run's children that still work and waits until each has
    /// ended, with no time limit: every child's wait listens to its
    /// mailbox. Their ends are stored in their parts when the store still
    /// takes them.
    ///
    /// The tasks of executors whose tasks outlive their reply are an
    /// exception (ADR-0027, decision 6): a stop asks each executor to cancel
    /// them and waits for their targets to confirm up to the cancel timeout;
    /// quitting leaves them running. The store lets go of the parts of those
    /// that still run, and each executor keeps its cancels.
    pub(super) async fn end_children(&mut self, result: &Result<FinishReason, Ending>) {
        if !self.tasks.tasks.iter().any(LaneTask::is_active) {
            return;
        }
        let mut command = if matches!(result, Err(Ending::Interrupted)) {
            Command::Shutdown
        } else {
            Command::Stop
        };
        tracing::info!(run = %self.run, command = ?command, "a run ends; stopping its tasks");
        self.tasks.stop_all(command);
        let deadline = if command == Command::Stop {
            self.cancel_remote_tasks().await;
            Some(tokio::time::Instant::now() + self.runtime.inner.config.executor_cancel_timeout)
        } else {
            None
        };
        let board = Arc::clone(&self.tasks.board);
        let mut storing = !matches!(result, Err(Ending::Gone));
        let mut listening = true;
        loop {
            if storing && let Err(ending) = self.mirror_children().await {
                tracing::warn!(run = %self.run, ending = ?ending, "could not store the ends of a run's tasks");
                storing = false;
            }
            if !storing {
                let now = self.now();
                let _unstored = self.tasks.absorb(&self.runtime.inner.ids, now);
            }
            let active = self.tasks.tasks.iter().filter(|task| task.is_active());
            let (remote, local): (Vec<&LaneTask>, Vec<&LaneTask>) =
                active.partition(|task| self.outlives_reply(task));
            let waits_for_targets = !remote.is_empty()
                && command == Command::Stop
                && deadline.is_some_and(|deadline| tokio::time::Instant::now() < deadline);
            if local.is_empty() && !waits_for_targets {
                if !remote.is_empty() {
                    tracing::info!(run = %self.run, tasks = remote.len(), "the run lets go of tasks that executors still run");
                }
                return;
            }
            tokio::select! {
                command_received = self.commands.recv(), if listening => match command_received {
                    Some(Command::Shutdown) => {
                        command = Command::Shutdown;
                        self.tasks.stop_all(Command::Shutdown);
                    }
                    Some(Command::Stop | Command::LeaseRevoked) => {}
                    None => listening = false,
                },
                () = board.changed() => {}
                () = super::until(deadline), if waits_for_targets => {}
            }
        }
    }

    /// Whether a task outlives the reply that gave it: a task of an
    /// executor that says so, also while its plugin is off.
    fn outlives_reply(&self, task: &LaneTask) -> bool {
        task.is_remote()
            && task
                .executor_kind()
                .and_then(|kind| self.runtime.executor(kind))
                .is_none_or(|executor| executor.outlives_reply())
    }

    /// Runs a call of a task tool. A start or a follow-up returns once the
    /// child's run started; a wait returns once its tasks ended.
    pub(super) async fn run_task_call(
        &mut self,
        setup: &Setup,
        call: StepCall,
    ) -> Result<(), Ending> {
        if let Some(executor) = setup.executor_for(&call.document.tool) {
            return Box::pin(self.give_task(setup, call, executor)).await;
        }
        let arguments = call.arguments.clone().unwrap_or_default();
        let parsed = match parse_call(&call.document.tool, &arguments, &setup.child_agents) {
            Ok(parsed) => parsed,
            Err(detail) => {
                let detail = detail.chars().take(MAX_DETAIL_CHARS).collect();
                self.end_call(call, NotRunReason::InvalidArguments { detail });
                return Ok(());
            }
        };
        match parsed {
            TaskCall::Start {
                title,
                agent,
                instruction,
                background,
            } => {
                self.start_task(setup, call, title, agent, instruction, background)
                    .await
            }
            TaskCall::FollowUp {
                number,
                instruction,
                background,
            } => Box::pin(self.follow_up_task(setup, call, number, instruction, background)).await,
            TaskCall::Wait { numbers } => self.wait_for_tasks(call, &numbers).await,
        }
    }

    /// Whether the reply started all the tasks it may start.
    pub(super) fn task_budget_used(&self, setup: &Setup) -> bool {
        self.tasks.assigned >= setup.agent.max_tasks
    }

    async fn start_task(
        &mut self,
        setup: &Setup,
        call: StepCall,
        title: String,
        agent: String,
        instruction: String,
        background: bool,
    ) -> Result<(), Ending> {
        let Some(call) = self.unless_plugin_off(call, &agent) else {
            return Ok(());
        };
        if self.task_budget_used(setup) {
            self.refuse_task(call, TaskRefusal::BudgetUsed);
            return Ok(());
        }
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
            agent,
            title,
            background,
            follow_up: false,
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
            executor: None,
            detached: false,
        };
        let origin = TaskOrigin {
            parent_id: self.conversation.clone(),
            number,
            agent: document.agent.clone(),
            executor: None,
        };
        self.assign(setup, call, document, instruction, Some(origin))
            .await
    }

    async fn follow_up_task(
        &mut self,
        setup: &Setup,
        call: StepCall,
        number: u32,
        instruction: String,
        background: bool,
    ) -> Result<(), Ending> {
        if self.task_budget_used(setup) {
            self.refuse_task(call, TaskRefusal::BudgetUsed);
            return Ok(());
        }
        let running_here = self
            .tasks
            .latest(number)
            .is_some_and(|index| self.tasks.tasks[index].is_active());
        if running_here {
            self.refuse_task(call, TaskRefusal::StillRunning { number });
            return Ok(());
        }
        let child = self
            .runtime
            .inner
            .store
            .child_by_number(&self.conversation, number)
            .await?;
        let Some(child) = child else {
            self.refuse_task(call, TaskRefusal::UnknownTask { number });
            return Ok(());
        };
        if child.executor.is_some() {
            return self
                .follow_up_on_executor(call, child, instruction, background)
                .await;
        }
        let Some(call) = self.unless_plugin_off(call, &child.agent) else {
            return Ok(());
        };
        let running = child
            .latest
            .as_ref()
            .is_some_and(|task| task.state.is_active())
            || self.runtime.is_busy(&child.conversation_id);
        if running {
            self.refuse_task(call, TaskRefusal::StillRunning { number });
            return Ok(());
        }
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
            executor: None,
            detached: false,
        };
        self.assign(setup, call, document, instruction, None).await
    }

    /// Gives a child its task (docs/features/child-agents/tasks.md, starting
    /// a task): the parent's part records the task first, then one batch in
    /// the child's conversation creates the conversation when it is new and
    /// adds the turn with the instruction, the child's run and the task, and
    /// then the child's run starts. Mannie's end between the first two steps
    /// leaves the part "not started", and nothing ran.
    async fn assign(
        &mut self,
        setup: &Setup,
        mut call: StepCall,
        document: TaskDocument,
        instruction: String,
        origin: Option<TaskOrigin>,
    ) -> Result<(), Ending> {
        self.tasks.assigned += 1;
        call.document.status = ToolCallStatus::Running;
        call.document.started_at = Some(self.now());
        let mut section = call.task.take().unwrap_or_default();
        section.task = Some(document.clone());
        call.task = Some(section);
        self.pending.push(call.update(Vec::new()));
        self.flush(Durability::Full).await?;
        let (changes, turn, run) =
            self.child_batch(setup, &document, &call.part_id, instruction, origin);
        let stored = self
            .runtime
            .inner
            .hubs
            .apply(
                &self.runtime.inner.store,
                &document.conversation_id,
                changes,
                Durability::Full,
            )
            .await;
        match stored {
            Ok(_applied) => {}
            Err(RuntimeStoreError::RunActive) => {
                // The child still works on an earlier task: nothing was
                // stored for this one.
                self.tasks.assigned -= 1;
                if let Some(section) = &mut call.task {
                    section.task = None;
                }
                let refusal = TaskRefusal::StillRunning {
                    number: document.number,
                };
                self.refuse_task(call, refusal);
                return Ok(());
            }
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), run = %self.run, "could not store the task of a child agent");
                return Err(Ending::Failed(RunFailure::Internal));
            }
        }
        self.start_child(call, document, turn, run);
        self.flush(Durability::Full).await
    }

    /// The batch that stores a child's task in the child's conversation: the
    /// conversation, when it is new, and the turn with the instruction, the
    /// child's queued run, with the parent's profile, and the task. Returns
    /// the turn and the run too.
    fn child_batch(
        &self,
        setup: &Setup,
        document: &TaskDocument,
        part_id: &PartId,
        instruction: String,
        origin: Option<TaskOrigin>,
    ) -> (Vec<Change>, TurnId, RunId) {
        let ids = &self.runtime.inner.ids;
        let turn = ids.turn();
        let run = ids.run();
        let mut changes = Vec::with_capacity(2);
        if let Some(origin) = origin {
            changes.push(Change::CreateConversation {
                conversation: NewConversation {
                    id: document.conversation_id.clone(),
                    title: document.title.clone(),
                    profile_id: Some(self.profile.clone()),
                    project_id: None,
                    task: Some(origin),
                },
            });
        }
        changes.push(Change::StartTurn {
            turn_id: turn.clone(),
            user_part_id: ids.part(),
            mentions: Vec::new(),
            images: Vec::new(),
            text: instruction,
            run: NewRun {
                id: run.clone(),
                agent: document.agent.clone(),
                profile_id: self.profile.clone(),
                binding: setup.binding.clone(),
            },
            task: Some(NewTask {
                id: document.task_id.clone(),
                part_id: Some(part_id.clone()),
                assignment: None,
                follow_up: document.follow_up,
                background: document.background,
            }),
        });
        (changes, turn, run)
    }

    /// Starts the run of a child whose task is stored, and keeps the task
    /// until it ends. A task in the background gets its result at once.
    fn start_child(
        &mut self,
        mut call: StepCall,
        document: TaskDocument,
        turn: TurnId,
        run: RunId,
    ) {
        let guard = ChildGuard::new(
            TaskSink::Board(Arc::clone(&self.tasks.board)),
            document.task_id.clone(),
            Arc::clone(&self.runtime.inner.clock),
        );
        let mailbox = self.runtime.spawn_run(
            document.conversation_id.clone(),
            turn,
            run,
            self.profile.clone(),
            document.agent.clone(),
            Some(ChildLink {
                parent: Some(self.conversation.clone()),
                guard,
                assigned: None,
            }),
        );
        tracing::info!(
            run = %self.run,
            task = %document.task_id,
            number = document.number,
            agent = %document.agent,
            follow_up = document.follow_up,
            background = document.background,
            "a child agent got a task"
        );
        let mut section = call.task.take().unwrap_or_default();
        section.task = None;
        let mut task = LaneTask {
            id: document.task_id.clone(),
            part_id: call.part_id,
            call: call.document,
            section,
            document,
            mailbox: Some(mailbox),
            follow: None,
            ending: None,
            read: false,
            noted: false,
            let_go: false,
        };
        if task.document.background {
            // The model reads this result at once; the part stays open until
            // the task ends.
            task.section.output = Some(texts::task_started_in_background(
                task.document.number,
                &task.document.agent,
            ));
            self.pending.push(task.update(Vec::new()));
        }
        self.tasks.tasks.push(task);
    }

    /// The call, unless it gives a task to the agent of a plugin that is off,
    /// which does not run: the run started with the plugin's agent, or the
    /// child was given its first task while the plugin was on (ADR-0030,
    /// decision 8).
    fn unless_plugin_off(&mut self, call: StepCall, agent: &str) -> Option<StepCall> {
        if self.runtime.agent(agent).is_some() {
            return Some(call);
        }
        let Some(plugin) = self.runtime.agent_plugin(agent) else {
            return Some(call);
        };
        self.end_call(call, NotRunReason::PluginOff { plugin });
        None
    }

    /// Ends a call of a task tool that assigns no task, with the reason.
    pub(super) fn refuse_task(&mut self, mut call: StepCall, refusal: TaskRefusal) {
        tracing::info!(run = %self.run, part = %call.part_id, tool = %call.document.tool, refusal = ?refusal, "a task call assigned no task");
        call.task
            .get_or_insert_with(TaskPartDocument::default)
            .refusal = Some(refusal);
        self.complete_task_call(call, Vec::new());
    }

    /// Stores the final documents of a call of a task tool that ended, with
    /// outputs to archive, and finishes its part.
    pub(super) fn complete_task_call(&mut self, mut call: StepCall, archives: Vec<NewArchive>) {
        call.document.status = ToolCallStatus::Completed;
        call.document.ended_at = Some(self.now());
        self.pending.push(call.update(archives));
        self.pending.push(Change::FinishPart {
            part_id: call.part_id,
            state: PartState::Complete,
            tail: String::new(),
            provider_item: None,
        });
        self.schedule_flush();
    }

    /// The tasks that a wait names, or for no numbers, the tasks of the
    /// reply whose reports the model did not read.
    async fn awaited(&self, numbers: &[u32]) -> Result<Vec<Awaited>, Ending> {
        if numbers.is_empty() {
            return Ok(self
                .tasks
                .tasks
                .iter()
                .enumerate()
                .filter(|(_index, task)| !task.read)
                .map(|(index, _task)| Awaited::Lane(index))
                .collect());
        }
        let mut awaited = Vec::with_capacity(numbers.len());
        for &number in numbers {
            // A task that the run let go is taken up again as a task of an
            // earlier reply.
            let in_lane = self
                .tasks
                .latest(number)
                .filter(|index| !self.tasks.tasks[*index].let_go);
            if let Some(index) = in_lane {
                awaited.push(Awaited::Lane(index));
                continue;
            }
            let child = self
                .runtime
                .inner
                .store
                .child_by_number(&self.conversation, number)
                .await?;
            let Some((title, task, executor)) =
                child.and_then(|child| Some((child.title, child.latest?, child.executor)))
            else {
                awaited.push(Awaited::Unknown(number));
                continue;
            };
            let pick = match executor {
                // An executor's task that still runs is followed to its end,
                // while its plugin is on.
                Some(executor) if task.state.is_active() => {
                    if self.runtime.executor(&executor.kind).is_some() {
                        Awaited::Followed {
                            number,
                            title,
                            task: task.id,
                            kind: executor.kind,
                            doer: executor.noun,
                            target: executor.target_name,
                        }
                    } else {
                        Awaited::PluginOff {
                            number,
                            kind: executor.kind,
                            target: executor.target_name,
                        }
                    }
                }
                executor => Awaited::Earlier {
                    number,
                    title,
                    ending: task.ending.unwrap_or_else(interrupted),
                    doer: executor
                        .map_or_else(|| texts::THE_CHILD.to_owned(), |executor| executor.noun),
                },
            };
            awaited.push(pick);
        }
        Ok(awaited)
    }

    /// Takes up the tasks of earlier replies that executors run, which a
    /// wait names: the executors' states reach this run's board from now on,
    /// and a task that ended meanwhile is read from its row.
    async fn follow_earlier(&mut self, awaited: &[Awaited]) -> Result<(), Ending> {
        for pick in awaited {
            let Awaited::Followed { task, kind, .. } = pick else {
                continue;
            };
            let follow = RemoteFollow::new(&self.runtime, task, kind, &self.tasks.board);
            // Read after the hold, so an end stored before it is seen here
            // and one stored after it reaches the board.
            let stored = self.runtime.inner.store.task(task).await?;
            let ending = stored
                .filter(|stored| !stored.state.is_active())
                .map(|stored| {
                    stored.ending.unwrap_or(TaskEnding {
                        state: stored.state,
                        report: None,
                        failure: None,
                    })
                });
            // Checked after the hold too: a plugin that stopped before it
            // could not tell this run to let the task go.
            let let_go = ending.is_none() && self.runtime.executor(kind).is_none();
            tracing::info!(run = %self.run, task = %task, kind = %kind, let_go, "a wait takes up a task of an executor");
            self.tasks.followed.push(FollowedTask {
                id: task.clone(),
                kind: kind.clone(),
                ending,
                let_go,
                _follow: follow,
            });
        }
        Ok(())
    }

    /// The awaited tasks as the wait's part shows them.
    fn awaited_view(&self, awaited: &[Awaited]) -> Vec<AwaitedTask> {
        awaited
            .iter()
            .filter_map(|pick| match pick {
                Awaited::Lane(index) => {
                    let document = &self.tasks.tasks[*index].document;
                    Some(AwaitedTask {
                        number: document.number,
                        title: document.title.clone(),
                        state: document.state,
                    })
                }
                Awaited::Earlier {
                    number,
                    title,
                    ending,
                    ..
                } => Some(AwaitedTask {
                    number: *number,
                    title: title.clone(),
                    state: ending.state,
                }),
                Awaited::Followed { number, title, .. } => Some(AwaitedTask {
                    number: *number,
                    title: title.clone(),
                    state: TaskState::Running,
                }),
                Awaited::PluginOff { .. } | Awaited::Unknown(_) => None,
            })
            .collect()
    }

    /// Waits until the named tasks ended and returns their reports, each
    /// within its share of the result limit. A report that does not fit is
    /// archived whole.
    async fn wait_for_tasks(&mut self, mut call: StepCall, numbers: &[u32]) -> Result<(), Ending> {
        let awaited = self.awaited(numbers).await?;
        // A wait for a task whose plugin is off does not run (ADR-0031,
        // decision 10).
        let plugin_off = awaited.iter().find_map(|pick| match pick {
            Awaited::PluginOff {
                number,
                kind,
                target,
            } => Some((*number, kind.clone(), target.clone())),
            _ => None,
        });
        if let Some((number, kind, target)) = plugin_off {
            let refusal = ExecutorRefusal::new(
                "plugin_off",
                Some(target),
                texts::task_plugin_off(number, &kind),
            );
            self.refuse_by_executor(call, &kind, refusal);
            return Ok(());
        }
        if awaited.is_empty() {
            call.task
                .get_or_insert_with(TaskPartDocument::default)
                .output = Some(texts::NOTHING_TO_WAIT_FOR.to_owned());
            self.complete_task_call(call, Vec::new());
            return Ok(());
        }
        let view = self.awaited_view(&awaited);
        call.task
            .get_or_insert_with(TaskPartDocument::default)
            .awaited = view;
        call.document.status = ToolCallStatus::Running;
        call.document.started_at = Some(self.now());
        self.pending.push(call.update(Vec::new()));
        self.flush(Durability::Full).await?;
        let waited: Vec<TaskId> = awaited
            .iter()
            .filter_map(|pick| match pick {
                Awaited::Lane(index) => Some(self.tasks.tasks[*index].id.clone()),
                Awaited::Earlier { .. }
                | Awaited::Followed { .. }
                | Awaited::PluginOff { .. }
                | Awaited::Unknown(_) => None,
            })
            .collect();
        self.follow_earlier(&awaited).await?;
        let waited_out = self.await_tasks(|task| waited.contains(&task.id)).await;
        let followed: Vec<FollowedTask> = std::mem::take(&mut self.tasks.followed);
        waited_out?;
        let reports = self.waited_reports(&awaited, &followed);
        let view = self.awaited_view(&awaited);
        let section = call.task.get_or_insert_with(TaskPartDocument::default);
        section.awaited = view;
        section.output = Some(reports.outputs.join("\n\n"));
        section.archives = reports.references;
        self.complete_task_call(call, reports.archives);
        Ok(())
    }

    /// The reports of the tasks that a wait waited for, in their order, each
    /// within its share of the result limit. A report that does not fit is
    /// archived whole.
    fn waited_reports(&mut self, awaited: &[Awaited], followed: &[FollowedTask]) -> WaitedReports {
        let limit = (self.tasks.result_limit / awaited.len()).max(MIN_REPORT_BYTES);
        let ids = &self.runtime.inner.ids;
        let mut reports = WaitedReports {
            outputs: Vec::with_capacity(awaited.len()),
            archives: Vec::new(),
            references: Vec::new(),
        };
        let outputs = &mut reports.outputs;
        for pick in awaited {
            let (number, ending, doer) = match pick {
                Awaited::Lane(index) => {
                    let task = &mut self.tasks.tasks[*index];
                    task.read = true;
                    if task.let_go {
                        outputs.push(let_go_text(&task.document));
                        continue;
                    }
                    let ending = task.ending.clone().unwrap_or_else(interrupted);
                    (
                        task.document.number,
                        ending,
                        doer_of(&task.document).to_owned(),
                    )
                }
                Awaited::Earlier {
                    number,
                    ending,
                    doer,
                    ..
                } => (*number, ending.clone(), doer.clone()),
                Awaited::Followed {
                    number,
                    task,
                    doer,
                    target,
                    ..
                } => {
                    let followed = followed.iter().find(|followed| followed.id == *task);
                    if followed.is_some_and(|followed| followed.ending.is_none() && followed.let_go)
                    {
                        outputs.push(texts::task_let_go(*number, doer, target));
                        continue;
                    }
                    let ending = followed
                        .and_then(|followed| followed.ending.clone())
                        .unwrap_or_else(interrupted);
                    (*number, ending, doer.clone())
                }
                Awaited::PluginOff { .. } => continue,
                Awaited::Unknown(number) => {
                    outputs.push(texts::no_such_task(*number));
                    continue;
                }
            };
            let rendered = render_report(number, &ending, &doer, limit, || ids.archive());
            outputs.push(rendered.output);
            if let Some((archive, reference)) = rendered.archive {
                reports.archives.push(archive);
                reports.references.push(reference);
            }
        }
        reports
    }
}

/// What a wait read from the tasks it waited for: one line of the result for
/// each task, and the reports that did not fit, archived whole.
struct WaitedReports {
    outputs: Vec<String>,
    archives: Vec<NewArchive>,
    references: Vec<ArchiveRef>,
}

#[cfg(test)]
mod tests {
    use mannie_types::tasks::MAX_REPORT_VIEW_BYTES;

    use super::{instruction_preview, report_view};

    #[test]
    fn the_part_keeps_the_start_of_an_instruction_and_a_bounded_report() {
        let instruction = "ä".repeat(2_000);
        let report = "é".repeat(MAX_REPORT_VIEW_BYTES);

        assert_eq!(instruction_preview(&instruction).chars().count(), 1_000);
        let view = report_view(&report);
        assert!(view.len() <= MAX_REPORT_VIEW_BYTES);
        assert!(view.len() >= MAX_REPORT_VIEW_BYTES - 1);
        assert_eq!(report_view("short"), "short");
    }
}
