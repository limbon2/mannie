//! The run actor (ADR-0012, decisions 1 and 2): one task per run. It waits
//! for a model slot, prepares each call, streams the answer into parts, runs
//! the model's tool calls, retries transient failures, compacts when the
//! context runs low, and ends the run. Stop and Shutdown arrive through its
//! mailbox and take effect at every wait.

mod operations;

use std::future::Future;
use std::time::Duration;

use futures_util::StreamExt;
use mannie_provider::{EventStream, ProviderError, ProviderEvent};
use mannie_store::{Change, Durability, NewPart, ProviderItem, RuntimeStoreError};
use mannie_tools::split_activity;
use mannie_types::clock::UtcMillis;
use mannie_types::ids::{ConversationId, PartId, ProfileId, RunId, TurnId};
use mannie_types::run::{FinishReason, RunActivity, RunFailure, RunOutcome, RunState, Usage};
use mannie_types::tools::{NotRunReason, ToolCallDocument, ToolCallStatus};
use mannie_types::transcript::{PartKind, PartState};
use tokio::sync::{OwnedSemaphorePermit, mpsc, watch};
use tokio::time::Instant;

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
    /// The owner stopped the reply.
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
}

/// The part that is streaming.
#[derive(Debug)]
struct OpenPart {
    id: PartId,
    kind: PartKind,
    /// Text that is not stored yet.
    unstored: String,
}

/// A tool call of the current model call. Tool calls never stream into
/// `RunActor::open`: their arguments arrive whole.
#[derive(Debug)]
struct StepCall {
    part_id: PartId,
    /// The journal record of the call (ADR-0013, decision 1).
    document: ToolCallDocument,
    /// The arguments that the tool reads, once the call is complete: as the
    /// model wrote them, without the activity.
    arguments: Option<String>,
}

/// What one model call has produced so far. Each call starts from nothing.
#[derive(Debug, Default)]
struct StepState {
    /// Whether the call wrote visible text.
    visible: bool,
    /// The end of the call's text, for the early-stop checks.
    text_tail: String,
    /// The token counts the call reported.
    usage: Option<Usage>,
    /// The estimated tokens of the call's text.
    output: u32,
}

/// The runtime's side of a stored run, while it is active.
pub(crate) struct RunActor {
    runtime: Runtime,
    conversation: ConversationId,
    turn: TurnId,
    run: RunId,
    profile: ProfileId,
    agent: String,
    commands: mpsc::Receiver<Command>,
    /// The binding that provider-private data of this run is stored with.
    binding: String,
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
    /// The tool calls of the current model call, in order.
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

impl Runtime {
    /// Starts the actor of a stored run. The conversation was reserved by the
    /// command that stored the run.
    pub(crate) fn spawn_run(
        &self,
        conversation: ConversationId,
        turn: TurnId,
        run: RunId,
        profile: ProfileId,
        agent: String,
    ) {
        let (mailbox, commands) = mpsc::channel(MAILBOX);
        let (ended_sender, ended) = watch::channel(false);
        let handle = RunHandle {
            run_id: run.clone(),
            profile_id: profile.clone(),
            mailbox,
            ended,
        };
        lock(&self.inner.busy).insert(conversation.clone(), Busy::Running(handle));
        let actor = RunActor {
            runtime: self.clone(),
            conversation: conversation.clone(),
            turn: turn.clone(),
            run: run.clone(),
            profile,
            agent,
            commands,
            binding: String::new(),
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
        };
        let task = tokio::spawn(actor.run());
        let runtime = self.clone();
        // The supervisor: a run that panics ends as failed, and every run
        // leaves the busy map when its task ends.
        tokio::spawn(async move {
            if let Err(error) = task.await {
                tracing::error!(run = %run, panicked = error.is_panic(), "a run ended unexpectedly");
                runtime.fail_abandoned_run(&conversation, &turn, &run).await;
            }
            runtime.release_run(&conversation, &run);
            ended_sender.send_replace(true);
        });
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

    async fn drive(&mut self) -> Result<FinishReason, Ending> {
        if self.runtime.is_shutting_down() {
            return Err(Ending::Interrupted);
        }
        let mut permit = Some(self.acquire_slot().await?);
        self.apply(
            vec![
                Change::StartRun {
                    run_id: self.run.clone(),
                },
                self.activity(RunActivity::Preparing),
            ],
            Durability::Full,
        )
        .await?;
        let setup = {
            let runtime = self.runtime.clone();
            let profile = self.profile.clone();
            let agent = self.agent.clone();
            self.wait(async move { runtime.load_setup(&profile, &agent).await })
                .await?
                .map_err(Ending::Failed)?
        };
        self.binding.clone_from(&setup.binding);
        tracing::info!(run = %self.run, conversation = %self.conversation, binding = %setup.binding, tools = setup.tools.len(), "a run started");
        let mut tries = 0;
        loop {
            if permit.is_none() {
                permit = Some(self.acquire_slot().await?);
            }
            let prepared = self.prepare(&setup).await?;
            self.attempt += 1;
            self.attempt_stored = false;
            tries += 1;
            self.apply(
                vec![self.activity(RunActivity::Writing)],
                Durability::Normal,
            )
            .await?;
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
                            // Tools run without a model slot.
                            permit = None;
                            self.run_calls(&setup, calls).await?;
                            self.compacted_after_overflow = false;
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
    /// the model on, its tool calls, or the end of the reply.
    async fn after_step(
        &mut self,
        setup: &Setup,
        finish: FinishReason,
    ) -> Result<AfterStep, Ending> {
        let calls = self.take_calls();
        if calls.is_empty() {
            return Ok(match self.note_for(setup, finish) {
                Some(note) => {
                    self.add_note(note).await?;
                    AfterStep::Continue
                }
                None => AfterStep::End,
            });
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
            return Ok(AfterStep::End);
        }
        self.tool_rounds += 1;
        // The calls are stored before any of them runs.
        self.flush(Durability::Full).await?;
        Ok(AfterStep::RunTools(calls))
    }

    /// Waits for `future`, unless a command arrives first.
    async fn wait<F: Future>(&mut self, future: F) -> Result<F::Output, Ending> {
        tokio::select! {
            biased;
            command = self.commands.recv() => Err(halt(command)),
            output = future => Ok(output),
        }
    }

    async fn apply(&self, changes: Vec<Change>, durability: Durability) -> Result<(), Ending> {
        self.runtime
            .inner
            .hubs
            .apply(
                &self.runtime.inner.store,
                &self.conversation,
                changes,
                durability,
            )
            .await?;
        Ok(())
    }

    fn activity(&self, activity: RunActivity) -> Change {
        Change::SetActivity {
            run_id: self.run.clone(),
            activity,
        }
    }

    fn now(&self) -> UtcMillis {
        self.runtime.inner.clock.now()
    }

    async fn acquire_slot(&mut self) -> Result<OwnedSemaphorePermit, Ending> {
        let slots = std::sync::Arc::clone(&self.runtime.inner.slots);
        if let Ok(permit) = std::sync::Arc::clone(&slots).try_acquire_owned() {
            return Ok(permit);
        }
        self.apply(
            vec![self.activity(RunActivity::WaitingForSlot)],
            Durability::Normal,
        )
        .await?;
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
            .prepare_call(&self.conversation, setup, options)
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
                .prepare_call(&self.conversation, setup, options)
                .await?;
        }
        if setup.uses_tools() && compaction::needed(prepared.size.projected, setup.budget) {
            let leave_out = CallOptions {
                leave_out_results: true,
                ..options
            };
            prepared = self
                .runtime
                .prepare_call(&self.conversation, setup, leave_out)
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
        self.apply(
            vec![self.activity(RunActivity::Compacting)],
            Durability::Normal,
        )
        .await?;
        let runtime = self.runtime.clone();
        let conversation = self.conversation.clone();
        let written = self
            .wait(runtime.write_checkpoint(&conversation, setup, prepared, trigger, size))
            .await??;
        self.apply(
            vec![self.activity(RunActivity::Preparing)],
            Durability::Normal,
        )
        .await?;
        Ok(written)
    }

    /// Streams one call into parts until it finishes or fails.
    async fn stream_step(&mut self, mut stream: EventStream) -> Result<StepEnd, Ending> {
        self.step = StepState::default();
        loop {
            let flush_due = self.flush_due;
            tokio::select! {
                biased;
                command = self.commands.recv() => return Err(halt(command)),
                () = until(flush_due) => self.flush(Durability::Normal).await?,
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

    /// Starts the part of a tool call that the model began to write.
    fn start_call(&mut self, call_id: String, name: String) {
        self.close_open(PartState::Complete, None);
        let part_id = self.runtime.inner.ids.part();
        let document = ToolCallDocument::receiving(call_id, name);
        self.pending.push(Change::StartPart {
            part: NewPart {
                id: part_id.clone(),
                turn_id: self.turn.clone(),
                run_id: self.run.clone(),
                attempt: self.attempt,
                kind: PartKind::ToolCall,
                failure: None,
                tool_call: Some(document.clone()),
            },
        });
        self.calls.push(StepCall {
            part_id,
            document,
            arguments: None,
        });
        self.schedule_flush();
    }

    /// Stores a tool call's complete arguments as its part's text, and its
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
        self.pending.push(Change::Delta {
            part_id: call.part_id.clone(),
            text: arguments,
        });
        self.pending.push(Change::UpdatePart {
            part_id: call.part_id.clone(),
            tool_call: call.document.clone(),
            archives: Vec::new(),
        });
        call.arguments = Some(split.arguments);
        self.schedule_flush();
    }

    /// The complete tool calls of the call that finished. A call without its
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

    /// Ends a tool call that never starts.
    fn end_call(&mut self, mut call: StepCall, reason: NotRunReason) {
        tracing::info!(run = %self.run, part = %call.part_id, tool = %call.document.tool, reason = ?reason, "a tool call does not run");
        call.document.status = ToolCallStatus::NotRun;
        call.document.not_run = Some(reason);
        call.document.ended_at = Some(self.now());
        self.finish_call(&call);
    }

    /// Stores a tool call's final document and finishes its part.
    fn finish_call(&mut self, call: &StepCall) {
        self.pending.push(Change::UpdatePart {
            part_id: call.part_id.clone(),
            tool_call: call.document.clone(),
            archives: Vec::new(),
        });
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
            return Ok(());
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
    /// colon. Plain chat without tools is as it was.
    fn note_for(&self, setup: &Setup, finish: FinishReason) -> Option<&'static str> {
        let may_continue = matches!(finish, FinishReason::Stop | FinishReason::Other);
        if !may_continue || self.notes >= setup.agent.max_continuation_notes {
            return None;
        }
        if !self.step.visible {
            return Some(texts::CONTINUATION_NOTE);
        }
        let tail = &self.step.text_tail;
        if setup.uses_tools() && TEXT_CALL_SIGNS.iter().any(|sign| tail.contains(sign)) {
            return Some(texts::USE_TOOL_INTERFACE);
        }
        if self.tool_rounds > 0 && tail.trim_end().ends_with(':') {
            return Some(texts::CONTINUE_NOTE);
        }
        None
    }

    async fn add_note(&mut self, note: &'static str) -> Result<(), Ending> {
        self.notes += 1;
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
        self.pending.push(self.activity(RunActivity::Preparing));
        tracing::debug!(run = %self.run, notes = self.notes, "the model stopped early; continuing with a note");
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
            Some(discard) => self.apply(vec![discard], Durability::Full).await,
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
        self.apply(
            vec![self.activity(RunActivity::Preparing)],
            Durability::Normal,
        )
        .await?;
        Ok(permit)
    }

    /// Ends the run: finishes its open part, stores what is pending, adds an
    /// error part for a failure, and stores the final state, in one batch.
    /// The store ends the tool calls that are still open: an operation that
    /// was executing becomes uncertain, every other call canceled.
    async fn end(mut self, result: Result<FinishReason, Ending>) {
        let (state, part_state, failure, finish) = match result {
            Ok(finish) => (RunState::Completed, PartState::Complete, None, Some(finish)),
            Err(Ending::Failed(failure)) => {
                (RunState::Failed, PartState::Incomplete, Some(failure), None)
            }
            Err(Ending::Stopped) => (RunState::Stopped, PartState::Stopped, None, None),
            Err(Ending::Interrupted) => (RunState::Interrupted, PartState::Interrupted, None, None),
            Err(Ending::Gone) => {
                tracing::info!(run = %self.run, "the conversation of a run was deleted");
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
        let end_run = Change::EndRun {
            run_id: self.run.clone(),
            state,
            outcome: RunOutcome {
                failure,
                finish,
                usage: self.usage,
                prompt_cut: self.prompt_cut,
            },
        };
        changes.push(end_run.clone());
        if let Err(error) = self.apply(changes, Durability::Full).await {
            // What was pending may be what the store refused; the final
            // state alone still ends the run.
            tracing::warn!(error = ?error, run = %self.run, "could not store the end of a run with its parts");
            if let Err(error) = self.apply(vec![end_run], Durability::Full).await {
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
        // The busy map lets the next command in once the end is stored.
        self.runtime.release_run(&self.conversation, &self.run);
    }
}
