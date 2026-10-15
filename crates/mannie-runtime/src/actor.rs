//! The run actor (ADR-0012, decisions 1 and 2): one task per run. It waits
//! for a model slot, prepares each call, streams the answer into parts, runs
//! the model's tool calls, retries transient failures, compacts when the
//! context runs low, and ends the run. Stop and Shutdown arrive through its
//! mailbox and take effect at every wait.
//!
//! A run can give tasks to child agents, which run as runs of their own
//! (ADR-0024): `tasks.rs` holds the parent's side, and `child.rs` the side of
//! a run that works on a task.

mod child;
mod operations;
mod tasks;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use mannie_provider::{EventStream, ProviderError, ProviderEvent};
use mannie_store::{
    Applied, Change, Durability, NewArchive, NewPart, ProviderItem, RuntimeStoreError,
};
use mannie_tools::{result_limit, split_activity};
use mannie_types::clock::UtcMillis;
use mannie_types::ids::{ConversationId, PartId, ProfileId, RunId, TurnId};
use mannie_types::run::{FinishReason, RunActivity, RunFailure, RunOutcome, RunState, Usage};
use mannie_types::tasks::TaskPartDocument;
use mannie_types::tools::{NotRunReason, ToolCallDocument, ToolCallStatus};
use mannie_types::transcript::{PartKind, PartState};
use tokio::sync::{OwnedSemaphorePermit, mpsc, watch};
use tokio::time::Instant;

pub(crate) use self::child::ChildLink;
use self::child::ChildRun;
use self::tasks::TaskLane;
use crate::budget::{Ledger, estimate_text, usage};
use crate::calls::{CallError, CallOptions, Prepared, Setup};
use crate::compaction::{self, ContextSize, Trigger};
use crate::retry::RetryDecision;
use crate::{Busy, RunHandle, Runtime, lock, texts};

/// How many commands a run's mailbox holds.
const MAILBOX: usize = 8;

/// The bytes at the end of a model call's text that the early-stop checks
/// read.
const TEXT_TAIL_BYTES: usize = 512;

/// The most characters of a parser's message about invalid arguments that
/// the journal and the model keep.
const MAX_DETAIL_CHARS: usize = 1_000;

/// Signs of a tool call that a model wrote as text instead of calling the
/// tool, for servers whose quirks do not turn such text into calls.
const TEXT_CALL_SIGNS: [&str; 5] = [
    "<tool_call>",
    "<function_call>",
    "<function=",
    "[TOOL_CALLS]",
    "<|tool_call",
];

/// What a run's mailbox carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    /// The owner stopped the reply, or the parent run ended.
    Stop,
    /// Mannie quits.
    Shutdown,
}

/// Why a run ends without completing.
#[derive(Debug)]
enum Ending {
    /// A failure that the owner sees.
    Failed(RunFailure),
    /// The owner stopped it.
    Stopped,
    /// Mannie quit.
    Interrupted,
    /// The conversation was deleted; nothing is left to write.
    Gone,
    /// The store failed. The run ends as failed if the store still takes it.
    Store(RuntimeStoreError),
}

impl From<RuntimeStoreError> for Ending {
    fn from(error: RuntimeStoreError) -> Self {
        match error {
            RuntimeStoreError::ConversationNotFound => Self::Gone,
            other => Self::Store(other),
        }
    }
}

impl From<CallError> for Ending {
    fn from(error: CallError) -> Self {
        match error {
            CallError::Failed(failure) => Self::Failed(failure),
            CallError::Store(error) => Self::from(error),
        }
    }
}

/// How a model call ended.
#[derive(Debug)]
enum StepEnd {
    Finished(FinishReason),
    Failed(ProviderError),
}

/// What comes after a model call that finished.
#[derive(Debug)]
enum AfterStep {
    /// The reply is complete.
    End,
    /// Another model call follows, after a note.
    Continue,
    /// The model's tool calls run, then another model call follows.
    RunTools(Vec<StepCall>),
    /// The reply would end, but tasks it gave still run or reported unread:
    /// the run waits for them, without a model slot.
    AwaitTasks,
}

/// The part that is streaming.
#[derive(Debug)]
struct OpenPart {
    id: PartId,
    kind: PartKind,
    /// Text that is not stored yet.
    unstored: String,
}

/// A call of the current model call, of a tool or of a task tool. Calls
/// never stream into `RunActor::open`: their arguments arrive whole.
#[derive(Debug)]
struct StepCall {
    part_id: PartId,
    /// The journal record of the call (ADR-0013, decision 1).
    document: ToolCallDocument,
    /// The arguments that the tool reads, once the call is complete: as the
    /// model wrote them, without the activity.
    arguments: Option<String>,
    /// The arguments as the model wrote them, with the activity: the part's
    /// text.
    text: String,
    /// The task section of a call of a task tool, which makes its part a
    /// `task` part; `None` for a tool call.
    task: Option<TaskPartDocument>,
}

impl StepCall {
    /// The change that stores the call's documents as they are now, with
    /// outputs to archive.
    fn update(&self, archives: Vec<NewArchive>) -> Change {
        match &self.task {
            None => Change::UpdatePart {
                part_id: self.part_id.clone(),
                tool_call: self.document.clone(),
                archives,
            },
            Some(section) => Change::UpdateTaskPart {
                part_id: self.part_id.clone(),
                tool_call: self.document.clone(),
                task: Box::new(section.clone()),
                archives,
            },
        }
    }
}

/// What one model call has produced so far. Each call starts from nothing.
#[derive(Debug, Default)]
struct StepState {
    /// Whether the call wrote visible text.
    visible: bool,
    /// The end of the call's text, for the early-stop checks.
    text_tail: String,
    /// The call's whole text, kept only by a child agent, whose last call's
    /// text is its report.
    text: String,
    /// The token counts the call reported.
    usage: Option<Usage>,
    /// The estimated tokens of the call's text.
    output: u32,
}

/// The runtime's side of a stored run, while it is active.
pub(crate) struct RunActor {
    runtime: Runtime,
    conversation: ConversationId,
    /// The conversation whose workspace, project, grants and rules the run's
    /// tools work under: the parent's for a child agent, the run's own
    /// otherwise (ADR-0024, decision 3).
    authority: ConversationId,
    turn: TurnId,
    run: RunId,
    profile: ProfileId,
    agent: String,
    commands: mpsc::Receiver<Command>,
    /// The binding that provider-private data of this run is stored with.
    binding: String,
    /// The task tools that the run is offered, whose calls become `task`
    /// parts.
    task_tools: Vec<String>,
    /// The model calls so far. Each call is an attempt with its own number,
    /// so a failed one can be discarded without touching the others.
    attempt: u32,
    /// Whether the current attempt stored a part.
    attempt_stored: bool,
    /// Changes that wait for the next flush.
    pending: Vec<Change>,
    /// When the pending changes are stored.
    flush_due: Option<Instant>,
    open: Option<OpenPart>,
    /// What the current model call has produced.
    step: StepState,
    /// The calls of the current model call, in order.
    calls: Vec<StepCall>,
    /// The model calls of this run that made tool calls.
    tool_rounds: u32,
    /// The tool calls of this run that counted against its budget.
    tool_calls: u32,
    /// The token counts of every call of the run together.
    usage: Option<Usage>,
    notes: u32,
    compacted_after_overflow: bool,
    /// Whether the server reported a shorter request than a call sent.
    prompt_cut: bool,
    /// The tasks that the run gave to child agents.
    tasks: TaskLane,
    /// The task that the run works on, when it is a child agent's.
    child: Option<ChildRun>,
}

/// Waits until `due`, or forever without a time.
async fn until(due: Option<Instant>) {
    match due {
        Some(due) => tokio::time::sleep_until(due).await,
        None => std::future::pending().await,
    }
}

fn halt(command: Option<Command>) -> Ending {
    match command {
        Some(Command::Stop) => Ending::Stopped,
        // A closed mailbox means the runtime is gone.
        Some(Command::Shutdown) | None => Ending::Interrupted,
    }
}

fn add_usage(total: Option<Usage>, call: Usage) -> Usage {
    let Some(total) = total else {
        return call;
    };
    let sum = |left: Option<u32>, right: Option<u32>| match (left, right) {
        (None, None) => None,
        (left, right) => Some(left.unwrap_or(0).saturating_add(right.unwrap_or(0))),
    };
    Usage {
        input: total.input.saturating_add(call.input),
        output: total.output.saturating_add(call.output),
        cached_input: sum(total.cached_input, call.cached_input),
        reasoning: sum(total.reasoning, call.reasoning),
    }
}

/// Keeps the last `TEXT_TAIL_BYTES` bytes of a text, at a character boundary.
fn keep_tail(text: &mut String) {
    if text.len() <= TEXT_TAIL_BYTES {
        return;
    }
    let mut start = text.len() - TEXT_TAIL_BYTES;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text.drain(..start);
}

/// The bytes of a context budget in tokens, the inverse of the estimate of
/// two tokens for every five bytes.
fn budget_bytes(tokens: u32) -> usize {
    usize::try_from(u64::from(tokens) * 5 / 2).unwrap_or(usize::MAX)
}

impl Runtime {
    /// Starts the actor of a stored run, and returns its mailbox. The
    /// conversation was reserved by the command that stored the run, or it
    /// is the task conversation of the parent run that `link` names.
    pub(crate) fn spawn_run(
        &self,
        conversation: ConversationId,
        turn: TurnId,
        run: RunId,
        profile: ProfileId,
        agent: String,
        link: Option<ChildLink>,
    ) -> mpsc::Sender<Command> {
        let (mailbox, commands) = mpsc::channel(MAILBOX);
        let (ended_sender, ended) = watch::channel(false);
        let handle = RunHandle {
            run_id: run.clone(),
            profile_id: profile.clone(),
            parent: link.as_ref().map(|link| link.parent.clone()),
            mailbox: mailbox.clone(),
            ended,
        };
        lock(&self.inner.busy).insert(conversation.clone(), Busy::Running(handle));
        let (authority, child) = match link {
            Some(link) => (link.parent, Some(ChildRun::new(link.guard))),
            None => (conversation.clone(), None),
        };
        let actor = RunActor {
            runtime: self.clone(),
            conversation: conversation.clone(),
            authority,
            turn: turn.clone(),
            run: run.clone(),
            profile,
            agent,
            commands,
            binding: String::new(),
            task_tools: Vec::new(),
            attempt: 0,
            attempt_stored: false,
            pending: Vec::new(),
            flush_due: None,
            open: None,
            step: StepState::default(),
            calls: Vec::new(),
            tool_rounds: 0,
            tool_calls: 0,
            usage: None,
            notes: 0,
            compacted_after_overflow: false,
            prompt_cut: false,
            tasks: TaskLane::default(),
            child,
        };
        let task = tokio::spawn(actor.run());
        let runtime = self.clone();
        // The supervisor: a run that panics ends as failed, and every run
        // leaves the busy map when its task ends.
        tokio::spawn(async move {
            if let Err(error) = task.await {
                tracing::error!(run = %run, panicked = error.is_panic(), "a run ended unexpectedly");
                // Its end did not stop its children, so they are stopped
                // here; each ends on its own.
                runtime.stop_children_of(&conversation);
                runtime.fail_abandoned_run(&conversation, &turn, &run).await;
            }
            runtime.release_run(&conversation, &run);
            ended_sender.send_replace(true);
        });
        mailbox
    }

    /// Removes a run from the busy map, if it is still there.
    fn release_run(&self, conversation: &ConversationId, run: &RunId) {
        let mut busy = lock(&self.inner.busy);
        let is_this_run =
            matches!(busy.get(conversation), Some(Busy::Running(handle)) if handle.run_id == *run);
        if is_this_run {
            busy.remove(conversation);
        }
    }

    /// Stops the runs of a conversation's children.
    fn stop_children_of(&self, parent: &ConversationId) {
        let busy = lock(&self.inner.busy);
        for entry in busy.values() {
            if let Busy::Running(handle) = entry
                && handle.parent.as_ref() == Some(parent)
            {
                // A full mailbox already holds a command that ends the run.
                let _delivered = handle.mailbox.try_send(Command::Stop);
            }
        }
    }

    /// Ends a run whose task ended without ending it.
    async fn fail_abandoned_run(&self, conversation: &ConversationId, turn: &TurnId, run: &RunId) {
        let failure = RunFailure::Internal;
        let changes = vec![
            Change::AddPart {
                part: NewPart {
                    id: self.inner.ids.part(),
                    turn_id: turn.clone(),
                    run_id: run.clone(),
                    attempt: 1,
                    kind: PartKind::Error,
                    failure: Some(failure.clone()),
                    tool_call: None,
                },
                text: String::new(),
            },
            Change::EndRun {
                run_id: run.clone(),
                state: RunState::Failed,
                outcome: RunOutcome {
                    failure: Some(failure),
                    ..RunOutcome::default()
                },
                task: None,
            },
        ];
        let ended = self
            .inner
            .hubs
            .apply(&self.inner.store, conversation, changes, Durability::Full)
            .await;
        match ended {
            Ok(_)
            | Err(
                RuntimeStoreError::ConversationNotFound | RuntimeStoreError::RunNotActive { .. },
            ) => {}
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), run = %run, "could not end a run that ended unexpectedly");
            }
        }
    }
}

impl RunActor {
    async fn run(mut self) {
        let result = self.drive().await;
        self.end(result).await;
    }

    /// Starts the run: a child's slot first, then a model slot, the stored
    /// start, and the setup of its calls.
    async fn begin(&mut self) -> Result<(Setup, OwnedSemaphorePermit), Ending> {
        if self.runtime.is_shutting_down() {
            return Err(Ending::Interrupted);
        }
        // A child takes its slot among the children before a model slot, so
        // that it never holds a model slot while it waits for one.
        if self.child.is_some() {
            self.acquire_child_slot().await?;
        }
        let permit = self.acquire_slot().await?;
        let preparing = self.activity(RunActivity::Preparing);
        self.apply(
            vec![
                Change::StartRun {
                    run_id: self.run.clone(),
                },
                preparing,
            ],
            Durability::Full,
        )
        .await?;
        self.child_started();
        let setup = {
            let runtime = self.runtime.clone();
            let profile = self.profile.clone();
            let agent = self.agent.clone();
            let task = self.child.is_some();
            self.wait(async move { runtime.load_setup(&profile, &agent, task).await })
                .await?
                .map_err(Ending::Failed)?
        };
        self.binding.clone_from(&setup.binding);
        self.task_tools = setup
            .task_definitions
            .iter()
            .map(|definition| definition.name.clone())
            .collect();
        self.tasks
            .set_result_limit(result_limit(budget_bytes(setup.budget)));
        tracing::info!(run = %self.run, conversation = %self.conversation, binding = %setup.binding, tools = setup.tools.len(), task_tools = self.task_tools.len(), "a run started");
        Ok((setup, permit))
    }

    async fn drive(&mut self) -> Result<FinishReason, Ending> {
        let (setup, permit) = self.begin().await?;
        let mut permit = Some(permit);
        let mut tries = 0;
        loop {
            if permit.is_none() {
                permit = Some(self.acquire_slot().await?);
            }
            let prepared = self.prepare(&setup).await?;
            self.attempt += 1;
            self.attempt_stored = false;
            tries += 1;
            let writing = self.activity(RunActivity::Writing);
            self.apply(vec![writing], Durability::Normal).await?;
            tracing::debug!(run = %self.run, attempt = self.attempt, tokens = prepared.size.projected, "calling the model");
            let stream = setup
                .adapter
                .stream(&setup.target, prepared.request.clone());
            match self.stream_step(stream).await? {
                StepEnd::Finished(finish) => {
                    tries = 0;
                    self.record_step(&setup, &prepared);
                    match self.after_step(&setup, finish).await? {
                        AfterStep::End => return Ok(finish),
                        AfterStep::Continue => {}
                        AfterStep::RunTools(calls) => {
                            // Tools and tasks run without a model slot.
                            permit = None;
                            Box::pin(self.run_calls(&setup, calls)).await?;
                            self.compacted_after_overflow = false;
                        }
                        AfterStep::AwaitTasks => {
                            // Children may need the model slot that the
                            // reply would hold while it waits for them.
                            permit = None;
                            if !self.await_reply_tasks(&setup).await? {
                                return Ok(finish);
                            }
                        }
                    }
                }
                StepEnd::Failed(ProviderError::Transient { cause, retry_after }) => {
                    permit = Some(
                        self.back_off(permit.take(), tries, cause, retry_after)
                            .await?,
                    );
                }
                StepEnd::Failed(ProviderError::ContextOverflow) => {
                    if self.compacted_after_overflow {
                        return Err(Ending::Failed(RunFailure::ContextOverflow));
                    }
                    self.compacted_after_overflow = true;
                    self.discard_attempt().await?;
                    tries = 0;
                    // The provider counted more than the budget, whatever the
                    // projection said.
                    let size = ContextSize {
                        projected: prepared.size.projected.max(setup.budget),
                        ..prepared.size
                    };
                    if !self
                        .compact(&setup, &prepared, Trigger::Overflow, size)
                        .await?
                    {
                        return Err(Ending::Failed(RunFailure::RequestTooLarge));
                    }
                }
                StepEnd::Failed(ProviderError::Permanent(failure)) => {
                    return Err(Ending::Failed(failure));
                }
            }
        }
    }

    /// Decides what follows a model call that finished: a note that sends
    /// the model on, its tool calls, a wait for the reply's tasks, or the
    /// end of the reply.
    async fn after_step(
        &mut self,
        setup: &Setup,
        finish: FinishReason,
    ) -> Result<AfterStep, Ending> {
        let calls = self.take_calls();
        if calls.is_empty() {
            if let Some(note) = self.note_for(setup, finish) {
                self.add_note(note, true).await?;
                return Ok(AfterStep::Continue);
            }
            if self.confirmation_due(finish) {
                self.confirm().await?;
                return Ok(AfterStep::Continue);
            }
            return self.end_of_reply().await;
        }
        if !setup.uses_tools() || self.tools_used_up(setup) {
            // The request offered no tools, or asked for none, and the server
            // sent calls anyway: they do not run, and the reply ends.
            let reason = if setup.uses_tools() {
                NotRunReason::OverBudget
            } else {
                NotRunReason::ToolsOff
            };
            for call in calls {
                self.end_call(call, reason.clone());
            }
            self.flush(Durability::Full).await?;
            return self.end_of_reply().await;
        }
        self.tool_rounds += 1;
        // The calls are stored before any of them runs.
        self.flush(Durability::Full).await?;
        Ok(AfterStep::RunTools(calls))
    }

    /// The end of the reply, or a wait for the tasks it gave first.
    async fn end_of_reply(&mut self) -> Result<AfterStep, Ending> {
        if !self.tasks.has_work_left() {
            return Ok(AfterStep::End);
        }
        // What the model wrote is stored before the wait, which can be long.
        self.flush(Durability::Full).await?;
        Ok(AfterStep::AwaitTasks)
    }

    /// Waits for `future`, unless a command arrives first. Meanwhile, the
    /// states that the run's children publish are stored.
    async fn wait<F: Future>(&mut self, future: F) -> Result<F::Output, Ending> {
        let mut future = std::pin::pin!(future);
        let board = Arc::clone(self.tasks.board());
        loop {
            tokio::select! {
                biased;
                command = self.commands.recv() => return Err(halt(command)),
                output = &mut future => return Ok(output),
                () = board.changed() => self.mirror_children().await?,
            }
        }
    }

    async fn apply(&self, changes: Vec<Change>, durability: Durability) -> Result<Applied, Ending> {
        let applied = self
            .runtime
            .inner
            .hubs
            .apply(
                &self.runtime.inner.store,
                &self.conversation,
                changes,
                durability,
            )
            .await?;
        Ok(applied)
    }

    /// The change that shows what the run does now: `activity`, unless a
    /// child waits for the owner while the run's own work does not
    /// (docs/features/child-agents/tasks.md, what the parent shows).
    fn activity(&mut self, activity: RunActivity) -> Change {
        self.tasks.set_activity(activity);
        self.shown_activity()
    }

    /// The change that shows what the run does now, as the run's own work
    /// and its children's approvals make it.
    fn shown_activity(&self) -> Change {
        Change::SetActivity {
            run_id: self.run.clone(),
            activity: self.tasks.shown_activity(),
        }
    }

    fn now(&self) -> UtcMillis {
        self.runtime.inner.clock.now()
    }

    async fn acquire_slot(&mut self) -> Result<OwnedSemaphorePermit, Ending> {
        let slots = Arc::clone(&self.runtime.inner.slots);
        if let Ok(permit) = Arc::clone(&slots).try_acquire_owned() {
            return Ok(permit);
        }
        let waiting = self.activity(RunActivity::WaitingForSlot);
        self.apply(vec![waiting], Durability::Normal).await?;
        // The runtime never closes the semaphore.
        self.wait(slots.acquire_owned())
            .await?
            .map_err(|_closed| Ending::Interrupted)
    }

    /// Whether the reply used up its tool rounds or its tool calls.
    fn tools_used_up(&self, setup: &Setup) -> bool {
        self.tool_rounds >= setup.agent.max_tool_rounds
            || self.tool_calls >= setup.agent.max_tool_calls
    }

    /// Prepares a call, and compacts first when the context runs low. When a
    /// compacted context still runs low, the latest turn's older tool results
    /// are left out.
    async fn prepare(&mut self, setup: &Setup) -> Result<Prepared, Ending> {
        let options = CallOptions {
            tools_used_up: setup.uses_tools() && self.tools_used_up(setup),
            leave_out_results: false,
        };
        let mut prepared = self
            .runtime
            .prepare_call(&self.conversation, &self.authority, setup, options)
            .await?;
        if !compaction::needed(prepared.size.projected, setup.budget) {
            return Ok(prepared);
        }
        if self
            .compact(setup, &prepared, Trigger::Automatic, prepared.size)
            .await?
        {
            prepared = self
                .runtime
                .prepare_call(&self.conversation, &self.authority, setup, options)
                .await?;
        }
        if setup.uses_tools() && compaction::needed(prepared.size.projected, setup.budget) {
            let leave_out = CallOptions {
                leave_out_results: true,
                ..options
            };
            prepared = self
                .runtime
                .prepare_call(&self.conversation, &self.authority, setup, leave_out)
                .await?;
        }
        Ok(prepared)
    }

    async fn compact(
        &mut self,
        setup: &Setup,
        prepared: &Prepared,
        trigger: Trigger,
        size: ContextSize,
    ) -> Result<bool, Ending> {
        let compacting = self.activity(RunActivity::Compacting);
        self.apply(vec![compacting], Durability::Normal).await?;
        let runtime = self.runtime.clone();
        let conversation = self.conversation.clone();
        let written = self
            .wait(runtime.write_checkpoint(&conversation, setup, prepared, trigger, size))
            .await??;
        let preparing = self.activity(RunActivity::Preparing);
        self.apply(vec![preparing], Durability::Normal).await?;
        Ok(written)
    }

    /// Streams one call into parts until it finishes or fails.
    async fn stream_step(&mut self, mut stream: EventStream) -> Result<StepEnd, Ending> {
        self.step = StepState::default();
        let board = Arc::clone(self.tasks.board());
        loop {
            let flush_due = self.flush_due;
            tokio::select! {
                biased;
                command = self.commands.recv() => return Err(halt(command)),
                () = until(flush_due) => self.flush(Durability::Normal).await?,
                () = board.changed() => self.mirror_children().await?,
                item = stream.next() => match item {
                    Some(Ok(event)) => {
                        if let Some(finish) = self.on_event(event) {
                            return Ok(StepEnd::Finished(finish));
                        }
                    }
                    Some(Err(error)) => return Ok(StepEnd::Failed(error)),
                    None => {
                        return Ok(StepEnd::Failed(ProviderError::Transient {
                            cause: RunFailure::BrokenStream,
                            retry_after: None,
                        }));
                    }
                },
            }
        }
    }

    fn on_event(&mut self, event: ProviderEvent) -> Option<FinishReason> {
        match event {
            ProviderEvent::TextDelta(text) => {
                if !text.trim().is_empty() {
                    self.step.visible = true;
                }
                self.step.output = self.step.output.saturating_add(estimate_text(&text));
                self.step.text_tail.push_str(&text);
                keep_tail(&mut self.step.text_tail);
                if self.child.is_some() {
                    self.step.text.push_str(&text);
                }
                self.append(PartKind::Text, &text);
            }
            ProviderEvent::ReasoningDelta(text) => self.append(PartKind::Reasoning, &text),
            ProviderEvent::ProviderItem(item) => self.attach_item(item),
            ProviderEvent::ToolCallStarted { call_id, name } => self.start_call(call_id, name),
            ProviderEvent::ToolCall {
                call_id,
                name,
                arguments,
            } => self.complete_arguments(&call_id, &name, arguments),
            ProviderEvent::Usage(usage) => self.step.usage = Some(usage),
            ProviderEvent::Finished(finish) => {
                self.close_open(PartState::Complete, None);
                return Some(finish);
            }
        }
        None
    }

    fn append(&mut self, kind: PartKind, text: &str) {
        if text.is_empty() {
            return;
        }
        let continues_part = self.open.as_ref().is_some_and(|open| open.kind == kind);
        if !continues_part {
            self.close_open(PartState::Complete, None);
            self.open_part(kind);
        }
        if let Some(open) = &mut self.open {
            open.unstored.push_str(text);
        }
        self.schedule_flush();
    }

    fn open_part(&mut self, kind: PartKind) {
        let id = self.runtime.inner.ids.part();
        self.pending.push(Change::StartPart {
            part: NewPart {
                id: id.clone(),
                turn_id: self.turn.clone(),
                run_id: self.run.clone(),
                attempt: self.attempt,
                kind,
                failure: None,
                tool_call: None,
            },
        });
        self.open = Some(OpenPart {
            id,
            kind,
            unstored: String::new(),
        });
    }

    /// Starts the part of a call that the model began to write: a `task`
    /// part for a task tool that the run is offered, a tool call otherwise.
    fn start_call(&mut self, call_id: String, name: String) {
        self.close_open(PartState::Complete, None);
        let part_id = self.runtime.inner.ids.part();
        let is_task = self.task_tools.contains(&name);
        let document = ToolCallDocument::receiving(call_id, name);
        self.pending.push(Change::StartPart {
            part: NewPart {
                id: part_id.clone(),
                turn_id: self.turn.clone(),
                run_id: self.run.clone(),
                attempt: self.attempt,
                kind: if is_task {
                    PartKind::Task
                } else {
                    PartKind::ToolCall
                },
                failure: None,
                tool_call: Some(document.clone()),
            },
        });
        self.calls.push(StepCall {
            part_id,
            document,
            arguments: None,
            text: String::new(),
            task: is_task.then(TaskPartDocument::default),
        });
        self.schedule_flush();
    }

    /// Stores a call's complete arguments as its part's text, and its
    /// activity in its document.
    fn complete_arguments(&mut self, call_id: &str, name: &str, arguments: String) {
        if !self
            .calls
            .iter()
            .any(|call| call.document.call_id == call_id)
        {
            self.start_call(call_id.to_owned(), name.to_owned());
        }
        self.step.output = self.step.output.saturating_add(estimate_text(&arguments));
        let Some(call) = self
            .calls
            .iter_mut()
            .find(|call| call.document.call_id == call_id)
        else {
            return;
        };
        // The part keeps the arguments as the model wrote them, so the model
        // reads its calls back unchanged. The tool reads them without the
        // activity, which is for the owner (docs/features/system-tools/
        // tool-calls.md, the activity of a call).
        let split = split_activity(&arguments);
        call.document.activity = split.activity;
        call.document.status = ToolCallStatus::Pending;
        call.text.clone_from(&arguments);
        call.arguments = Some(split.arguments);
        self.pending.push(Change::Delta {
            part_id: call.part_id.clone(),
            text: arguments,
        });
        let update = call.update(Vec::new());
        self.pending.push(update);
        self.schedule_flush();
    }

    /// The complete calls of the call that finished. A call without its
    /// arguments was cut off and does not run.
    fn take_calls(&mut self) -> Vec<StepCall> {
        let mut complete = Vec::new();
        for call in std::mem::take(&mut self.calls) {
            if call.arguments.is_some() {
                complete.push(call);
            } else {
                self.end_call(call, NotRunReason::CutOff);
            }
        }
        complete
    }

    /// Ends a call that never starts.
    fn end_call(&mut self, mut call: StepCall, reason: NotRunReason) {
        tracing::info!(run = %self.run, part = %call.part_id, tool = %call.document.tool, reason = ?reason, "a tool call does not run");
        call.document.status = ToolCallStatus::NotRun;
        call.document.not_run = Some(reason);
        call.document.ended_at = Some(self.now());
        self.finish_call(&call);
    }

    /// Stores a call's final documents and finishes its part.
    fn finish_call(&mut self, call: &StepCall) {
        self.pending.push(call.update(Vec::new()));
        self.pending.push(Change::FinishPart {
            part_id: call.part_id.clone(),
            state: PartState::Complete,
            tail: String::new(),
            provider_item: None,
        });
        self.schedule_flush();
    }

    fn close_open(&mut self, state: PartState, provider_item: Option<ProviderItem>) {
        if let Some(open) = self.open.take() {
            self.pending.push(Change::FinishPart {
                part_id: open.id,
                state,
                tail: open.unstored,
                provider_item,
            });
            self.schedule_flush();
        }
    }

    /// Stores provider-private data with the reasoning it belongs to. The
    /// item ends that reasoning; encrypted reasoning without a summary gets a
    /// reasoning part of its own.
    fn attach_item(&mut self, item: String) {
        let reasoning_open = self
            .open
            .as_ref()
            .is_some_and(|open| open.kind == PartKind::Reasoning);
        if !reasoning_open {
            self.close_open(PartState::Complete, None);
            self.open_part(PartKind::Reasoning);
        }
        let item = ProviderItem {
            binding: self.binding.clone(),
            item,
        };
        self.close_open(PartState::Complete, Some(item));
    }

    fn schedule_flush(&mut self) {
        if self.flush_due.is_none() {
            self.flush_due = Some(Instant::now() + self.runtime.inner.config.flush_interval);
        }
    }

    /// Stores the pending changes and the text of the open part. A batch that
    /// finishes a part is stored with full durability.
    async fn flush(&mut self, durability: Durability) -> Result<(), Ending> {
        self.flush_applied(durability).await.map(drop)
    }

    /// Stores what [`Self::flush`] stores, and returns what the store made of
    /// it.
    async fn flush_applied(&mut self, durability: Durability) -> Result<Applied, Ending> {
        self.flush_due = None;
        let mut changes = std::mem::take(&mut self.pending);
        if let Some(open) = &mut self.open
            && !open.unstored.is_empty()
        {
            changes.push(Change::Delta {
                part_id: open.id.clone(),
                text: std::mem::take(&mut open.unstored),
            });
        }
        if changes.is_empty() {
            return Ok(Applied::default());
        }
        let finishes = changes
            .iter()
            .any(|change| matches!(change, Change::FinishPart { .. } | Change::AddPart { .. }));
        if changes
            .iter()
            .any(|change| matches!(change, Change::StartPart { .. }))
        {
            self.attempt_stored = true;
        }
        let durability = if finishes {
            Durability::Full
        } else {
            durability
        };
        self.apply(changes, durability).await
    }

    /// Records a finished call: its token counts and the context they show.
    fn record_step(&mut self, setup: &Setup, prepared: &Prepared) {
        let checkpoint = prepared
            .checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.id.as_str().to_owned());
        let (ledger, shown) = if let Some(call) = self.step.usage {
            // Servers that cut requests beyond their context instead of
            // failing, such as Ollama beyond a model's `num_ctx`, give the
            // model less than the transcript shows (docs/features/chat/
            // providers.md, quirk profiles).
            let cut = setup.target.quirks.silent_truncation
                && prepared.ledger.shows_cut(
                    &setup.binding,
                    checkpoint.as_deref(),
                    prepared.size.projected,
                    call.input,
                );
            if cut {
                self.prompt_cut = true;
                tracing::warn!(
                    run = %self.run,
                    reported_input = call.input,
                    projected_input = prepared.size.projected,
                    "the server reported a shorter request than was sent; it may have cut the start of the conversation"
                );
            }
            self.usage = Some(add_usage(self.usage, call));
            let ledger = Ledger {
                binding: Some(setup.binding.clone()),
                checkpoint,
                reported_input: Some(call.input),
                estimated_input: Some(prepared.size.estimate),
            };
            let used = call.input.saturating_add(call.output);
            (ledger, usage(used, setup.budget, false))
        } else {
            let used = prepared.size.projected.saturating_add(self.step.output);
            (prepared.ledger.clone(), usage(used, setup.budget, true))
        };
        self.pending.push(Change::UpdateContext {
            ledger: ledger.to_json(),
            usage: shown,
        });
    }

    /// The note that sends the model on after a model call without tool
    /// calls, if one is due (docs/architecture/agent-runtime.md, model quirks
    /// and early stops): after an empty answer; after a tool call written as
    /// text; and, once tools were used, after an answer that stops at a
    /// colon. Plain chat without tools is as it was. A child's notes speak of
    /// its task (docs/features/child-agents/tasks.md, the text the model
    /// sees).
    fn note_for(&self, setup: &Setup, finish: FinishReason) -> Option<&'static str> {
        let may_continue = matches!(finish, FinishReason::Stop | FinishReason::Other);
        if !may_continue || self.notes >= setup.agent.max_continuation_notes {
            return None;
        }
        if !self.step.visible {
            return Some(texts::continuation_note(setup.requester()));
        }
        let tail = &self.step.text_tail;
        if setup.uses_tools() && TEXT_CALL_SIGNS.iter().any(|sign| tail.contains(sign)) {
            return Some(texts::USE_TOOL_INTERFACE);
        }
        if self.tool_rounds > 0 && tail.trim_end().ends_with(':') {
            return Some(texts::continue_note(setup.requester()));
        }
        None
    }

    /// Adds a note of Mannie's own that only the model reads, and continues.
    /// A counted note is a continuation note, of which a turn has a limited
    /// number; the confirmation and the note about unread reports have their
    /// own limits.
    async fn add_note(&mut self, note: &str, counted: bool) -> Result<(), Ending> {
        if counted {
            self.notes += 1;
        }
        let part = NewPart {
            id: self.runtime.inner.ids.part(),
            turn_id: self.turn.clone(),
            run_id: self.run.clone(),
            attempt: self.attempt,
            kind: PartKind::ContinuationNote,
            failure: None,
            tool_call: None,
        };
        self.pending.push(Change::AddPart {
            part,
            text: note.to_owned(),
        });
        let preparing = self.activity(RunActivity::Preparing);
        self.pending.push(preparing);
        tracing::debug!(run = %self.run, notes = self.notes, counted, "continuing with a note");
        self.flush(Durability::Full).await
    }

    /// Forgets what the failed attempt streamed, and discards the parts it
    /// stored.
    fn forget_attempt(&mut self) -> Option<Change> {
        self.pending.clear();
        self.open = None;
        self.calls.clear();
        self.flush_due = None;
        let stored = std::mem::replace(&mut self.attempt_stored, false);
        stored.then(|| Change::DiscardAttempt {
            run_id: self.run.clone(),
            attempt: self.attempt,
        })
    }

    async fn discard_attempt(&mut self) -> Result<(), Ending> {
        match self.forget_attempt() {
            Some(discard) => self.apply(vec![discard], Durability::Full).await.map(drop),
            None => Ok(()),
        }
    }

    /// Waits before the next attempt of a call, without holding a model slot.
    async fn back_off(
        &mut self,
        permit: Option<OwnedSemaphorePermit>,
        tries: u32,
        cause: RunFailure,
        retry_after: Option<Duration>,
    ) -> Result<OwnedSemaphorePermit, Ending> {
        let policy = self.runtime.inner.config.retry;
        let RetryDecision::After(delay) = policy.decide(tries, retry_after, self.runtime.random())
        else {
            return Err(Ending::Failed(cause));
        };
        drop(permit);
        let next_attempt_at = self.runtime.inner.clock.now().saturating_add(delay);
        let mut changes: Vec<Change> = self.forget_attempt().into_iter().collect();
        tracing::info!(
            run = %self.run,
            attempt = tries,
            delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
            cause = cause.code(),
            "retrying a model call"
        );
        changes.push(self.activity(RunActivity::Retrying {
            attempt: tries.saturating_add(1),
            max_attempts: policy.max_attempts,
            next_attempt_at,
            cause,
        }));
        self.apply(changes, Durability::Full).await?;
        self.wait(tokio::time::sleep(delay)).await?;
        let permit = self.acquire_slot().await?;
        let preparing = self.activity(RunActivity::Preparing);
        self.apply(vec![preparing], Durability::Normal).await?;
        Ok(permit)
    }

    /// Ends the run: stops its children and waits for them, finishes its
    /// open part, stores what is pending, adds an error part for a failure,
    /// and stores the final state, in one batch. The store ends the calls
    /// that are still open: an operation that was executing becomes
    /// uncertain, every other call canceled.
    async fn end(mut self, result: Result<FinishReason, Ending>) {
        // A run ends only after its children ended (ADR-0024, decision 9).
        self.end_children(&result).await;
        let (state, part_state, failure, finish) = match result {
            Ok(finish) => (RunState::Completed, PartState::Complete, None, Some(finish)),
            Err(Ending::Failed(failure)) => {
                (RunState::Failed, PartState::Incomplete, Some(failure), None)
            }
            Err(Ending::Stopped) => (RunState::Stopped, PartState::Stopped, None, None),
            Err(Ending::Interrupted) => (RunState::Interrupted, PartState::Interrupted, None, None),
            Err(Ending::Gone) => {
                tracing::info!(run = %self.run, "the conversation of a run was deleted");
                // A child's parent learns of the end from the child's guard,
                // when the actor is dropped.
                self.runtime.release_run(&self.conversation, &self.run);
                return;
            }
            Err(Ending::Store(error)) => {
                tracing::error!(%error, error.code = error.code(), run = %self.run, "a run failed because the store failed");
                (
                    RunState::Failed,
                    PartState::Incomplete,
                    Some(RunFailure::Internal),
                    None,
                )
            }
        };
        self.close_open(part_state, None);
        let mut changes = std::mem::take(&mut self.pending);
        if let Some(failure) = &failure {
            changes.push(Change::AddPart {
                part: NewPart {
                    id: self.runtime.inner.ids.part(),
                    turn_id: self.turn.clone(),
                    run_id: self.run.clone(),
                    attempt: self.attempt.max(1),
                    kind: PartKind::Error,
                    failure: Some(failure.clone()),
                    tool_call: None,
                },
                text: String::new(),
            });
        }
        let failure_code = failure.as_ref().map(RunFailure::code);
        // A child's task ends with its run, with the report when it has one.
        let task = self
            .child
            .is_some()
            .then(|| child::task_ending(state, failure.as_ref(), &self.step.text));
        let end_run = Change::EndRun {
            run_id: self.run.clone(),
            state,
            outcome: RunOutcome {
                failure,
                finish,
                usage: self.usage,
                prompt_cut: self.prompt_cut,
            },
            task: task.clone(),
        };
        changes.push(end_run.clone());
        let mut stored = true;
        if let Err(error) = self.apply(changes, Durability::Full).await {
            // What was pending may be what the store refused; the final
            // state alone still ends the run.
            tracing::warn!(error = ?error, run = %self.run, "could not store the end of a run with its parts");
            if let Err(error) = self.apply(vec![end_run], Durability::Full).await {
                stored = false;
                tracing::error!(error = ?error, run = %self.run, "could not store the end of a run; the next start marks it as interrupted");
            }
        }
        tracing::info!(
            run = %self.run,
            state = state.as_str(),
            failure = failure_code,
            attempts = self.attempt,
            tool_rounds = self.tool_rounds,
            tool_calls = self.tool_calls,
            input_tokens = self.usage.map(|usage| usage.input),
            output_tokens = self.usage.map(|usage| usage.output),
            "a run ended"
        );
        // The busy map lets the next command in once the end is stored. A
        // child frees its slot and its conversation before its parent learns
        // that it ended, so that a follow-up finds both free.
        let child = self.child.take();
        self.runtime.release_run(&self.conversation, &self.run);
        if let Some(child) = child {
            child.finish(task.filter(|_ending| stored));
        }
    }
}
