//! A terminal while it runs
//! (plugins/terminal/docs/adr/0002-keep-a-headless-emulator-of-each-terminal-in-rust.md).
//!
//! Each terminal has a thread of its own that owns its emulator, so a flood
//! never holds up the async runtime that runs replies. A task pumps the
//! core's output into the thread's inbox, which holds at most 4 MiB of output
//! before the pump waits, and the thread turns markers into commands, keeps
//! the views current and publishes the terminal's state. Every wait is driven
//! by events: output, markers, acknowledgments and the views' timers. Nothing
//! polls.
//!
//! An agent types a command only at an idle prompt whose user and folder are
//! those that its approval named, and never while the owner types or a
//! command runs (plugins/terminal/docs/adr/0001-mark-commands-with-shell-integration.md).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use futures_util::StreamExt as _;
use mannie_plugin_sdk::BoxStream;
use mannie_plugin_sdk::contracts::Clock;
use mannie_plugin_sdk::services::terminals::{
    Terminal, TerminalEnding, TerminalIoError, TerminalOutput, TerminalSize,
};
use tokio::sync::{Notify, oneshot, watch};

use crate::blocks::{Author, Block, BlockText, MAX_BLOCKS, text_of};
use crate::cite;
use crate::commands::{BlockView, Count, EndingView, TerminalPhase, TerminalUpdate};
use crate::emulator::{Emulator, PlacedMark, Position};
use crate::marks::Mark;
use crate::views::View;

/// The most bytes of output that wait in a terminal's inbox before the pump
/// waits for the thread.
const INBOX_BYTES: usize = 4 * 1024 * 1024;

/// The most bytes of one terminal's text log.
const LOG_BYTES: usize = 2 * 1024 * 1024;

/// The most bytes of every terminal's text log together.
const ALL_LOGS_BYTES: usize = 32 * 1024 * 1024;

/// How long after the owner's last key a prompt counts as being typed at.
const TYPING_GRACE: Duration = Duration::from_millis(1500);

/// The most selections that a terminal keeps for mentions.
const MAX_SELECTIONS: usize = 20;

/// How long closing a terminal waits for its shell after the hang-up.
pub(crate) const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// What a terminal does, as its thread publishes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Status {
    /// What it does now.
    pub(crate) phase: TerminalPhase,
    /// Whether its shell marks its commands.
    pub(crate) marking: bool,
    /// The user and folder that it reported last, such as `root in /etc`.
    pub(crate) state: Option<String>,
    /// The folder that it reported last.
    pub(crate) folder: Option<String>,
    /// How it ended.
    pub(crate) ending: Option<TerminalEnding>,
    /// How many pieces of output arrived, which the typed setup waits on.
    pub(crate) outputs: u64,
    /// The number of its latest command.
    pub(crate) latest_block: u32,
}

impl Status {
    fn starting() -> Self {
        Self {
            phase: TerminalPhase::Starting,
            marking: false,
            state: None,
            folder: None,
            ending: None,
            outputs: 0,
            latest_block: 0,
        }
    }
}

/// How an agent's command went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BlockOutcome {
    /// The command's block, ended.
    pub(crate) block: Block,
    /// Its output.
    pub(crate) text: BlockText,
}

/// Why an agent cannot type into a terminal now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The terminal ended.
    Ended,
    /// Its shell does not mark its commands.
    NotMarking,
    /// A command runs, or the shell has not reached its prompt.
    Busy,
    /// The owner is typing at the prompt.
    OwnerTyping,
    /// The user or folder is not the one that the approval named.
    Changed {
        /// What the terminal reports now.
        now: Option<String>,
    },
}

/// What the thread receives.
enum Message {
    Output(Vec<u8>),
    Ended(TerminalEnding),
    Resize { columns: u16, rows: u16 },
    OwnerInput { enter: bool },
    Subscribe(View),
    Acknowledge { view: String, seq: u64 },
    Run(Box<dyn FnOnce(&mut Core) + Send>),
    Stop,
}

#[derive(Default)]
struct InboxState {
    messages: VecDeque<Message>,
    output_bytes: usize,
    stopped: bool,
}

/// The thread's inbox: messages in order, with the output's bytes counted.
#[derive(Default)]
struct Inbox {
    state: Mutex<InboxState>,
    ready: Condvar,
    drained: Notify,
}

impl Inbox {
    fn lock(&self) -> std::sync::MutexGuard<'_, InboxState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn push(&self, message: Message) {
        let mut state = self.lock();
        if state.stopped {
            return;
        }
        state.messages.push_back(message);
        drop(state);
        self.ready.notify_one();
    }

    /// Pushes output, after waiting while the inbox holds too much.
    async fn push_output(&self, bytes: Vec<u8>) {
        loop {
            let drained = self.drained.notified();
            tokio::pin!(drained);
            drained.as_mut().enable();
            {
                let mut state = self.lock();
                if state.stopped {
                    return;
                }
                if state.output_bytes < INBOX_BYTES {
                    state.output_bytes += bytes.len();
                    state.messages.push_back(Message::Output(bytes));
                    drop(state);
                    self.ready.notify_one();
                    return;
                }
            }
            drained.await;
        }
    }

    /// Takes every message, waiting until one arrives or `deadline` passes.
    fn take(&self, deadline: Option<Instant>) -> Vec<Message> {
        let mut state = self.lock();
        while state.messages.is_empty() && !state.stopped {
            match deadline {
                Some(deadline) => {
                    let now = Instant::now();
                    if deadline <= now {
                        break;
                    }
                    state = self
                        .ready
                        .wait_timeout(state, deadline - now)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                }
                None => {
                    state = self
                        .ready
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            }
        }
        let messages: Vec<Message> = state.messages.drain(..).collect();
        state.output_bytes = 0;
        drop(state);
        self.drained.notify_waiters();
        messages
    }

    fn stop(&self) {
        let mut state = self.lock();
        state.stopped = true;
        state.messages.clear();
        drop(state);
        self.ready.notify_one();
        self.drained.notify_waiters();
    }
}

/// The core's terminal, which the session's handle and its thread share: the
/// thread marks whether a command runs.
type SharedTerminal = Arc<Mutex<Option<Box<dyn Terminal>>>>;

/// What a session starts with.
pub(crate) struct SessionStart {
    /// The session's own ID, which its cite links and mentions name.
    pub(crate) id: String,
    /// The nonce of its markers.
    pub(crate) nonce: String,
    /// The terminal's size.
    pub(crate) size: TerminalSize,
    /// The clock of the commands' times.
    pub(crate) clock: Arc<dyn Clock>,
    /// How many terminals run, which shares the logs' bytes.
    pub(crate) running: Arc<AtomicUsize>,
}

/// An agent's command, from the moment the agent claims the prompt to the
/// marker that starts it.
#[derive(Debug)]
struct AgentClaim {
    command: String,
    reply: oneshot::Sender<BlockOutcome>,
    /// Where the prompt ended, after which the shell echoes the command,
    /// for a shell that gives no start marker.
    prompt_end: Position,
}

/// A command that the owner entered at a prompt, for shells that give no
/// start marker: its line is read at the next prompt, once the shell echoed
/// it after the prompt's end.
#[derive(Debug)]
struct OwnerEnter {
    prompt_end: Position,
}

/// The most rows of a command line that wraps.
const MAX_COMMAND_ROWS: u64 = 64;

/// The thread's state.
struct Core {
    emulator: Emulator,
    id: String,
    clock: Arc<dyn Clock>,
    terminal: SharedTerminal,
    running_terminals: Arc<AtomicUsize>,
    log_cap: usize,
    marking: bool,
    phase: TerminalPhase,
    prompt_end: Option<Position>,
    /// The command line of an `E` marker, for the command that starts next.
    next_line: Option<String>,
    blocks: VecDeque<Block>,
    next_block: u32,
    /// The running command's number.
    running: Option<u32>,
    /// Who waits for the running command, if an agent ran it.
    running_reply: Option<oneshot::Sender<BlockOutcome>>,
    agent: Option<AgentClaim>,
    owner_enter: Option<OwnerEnter>,
    folder: Option<String>,
    user: Option<String>,
    owner_typed_at: Option<Instant>,
    views: Vec<View>,
    status: watch::Sender<Status>,
    /// The end of the last output when it stopped in a UTF-8 character.
    carry: Vec<u8>,
    last_output: Option<Instant>,
    ending: Option<TerminalEnding>,
    outputs: u64,
    selections: VecDeque<(String, String)>,
    /// The phase, marking and state that the views heard last.
    sent_status: Option<(TerminalPhase, bool, Option<String>)>,
    stopped: bool,
}

impl Core {
    /// The user and folder as an approval names them.
    fn state(&self) -> Option<String> {
        match (&self.user, &self.folder) {
            (Some(user), Some(folder)) => Some(format!("{user} in {folder}")),
            (None, Some(folder)) => Some(folder.clone()),
            (Some(user), None) => Some(user.clone()),
            (None, None) => None,
        }
    }

    fn publish(&self) {
        let status = Status {
            phase: self.phase,
            marking: self.marking,
            state: self.state(),
            folder: self.folder.clone(),
            ending: self.ending.clone(),
            outputs: self.outputs,
            latest_block: self.next_block.saturating_sub(1),
        };
        self.status.send_if_modified(|current| {
            let changed = *current != status;
            if changed {
                *current = status;
            }
            changed
        });
    }

    fn mark_running(&self, running: bool) {
        let terminal = self.terminal.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(terminal) = terminal.as_ref() {
            terminal.set_running(running);
        }
    }

    fn set_phase(&mut self, phase: TerminalPhase) {
        if self.phase == phase {
            return;
        }
        let was_running = self.phase == TerminalPhase::Running;
        self.phase = phase;
        let is_running = phase == TerminalPhase::Running;
        if was_running != is_running {
            self.mark_running(is_running);
        }
    }

    /// Tells the views what the terminal does, once a pass changed any of
    /// it: a marker can set the phase before the next one sets the marking.
    fn send_status_if_changed(&mut self) {
        let status = (self.phase, self.marking, self.state());
        if self.sent_status.as_ref() == Some(&status) {
            return;
        }
        for view in &self.views {
            view.send(TerminalUpdate::Status {
                phase: status.0,
                marking: status.1,
                state: status.2.clone(),
            });
        }
        self.sent_status = Some(status);
    }

    fn block_view(&self, block: &Block) -> BlockView {
        let (first_row, end_row) = block.rows(self.emulator.cursor());
        BlockView {
            number: block.number,
            author: block.author,
            command: block.command.clone(),
            exit_code: block.exit_code,
            running: block.end.is_none() && self.ending.is_none(),
            started_at: block.started_at,
            duration_ms: block
                .duration_ms()
                .map(|milliseconds| u32::try_from(milliseconds).unwrap_or(u32::MAX)),
            first_row: Count(first_row),
            end_row: Count(end_row),
            key: cite::block_key(&self.id, block.number),
        }
    }

    fn send_block(&self, number: u32) {
        let Some(block) = self.blocks.iter().find(|block| block.number == number) else {
            return;
        };
        let view = self.block_view(block);
        for subscriber in &self.views {
            subscriber.send(TerminalUpdate::Block {
                block: view.clone(),
            });
        }
    }

    fn start_block(&mut self, author: Author, command: String, start: Position) -> u32 {
        let number = self.next_block;
        self.next_block += 1;
        self.blocks.push_back(Block {
            number,
            author,
            command,
            start,
            end: None,
            exit_code: None,
            started_at: self.clock.now(),
            ended_at: None,
            folder: self.folder.clone(),
        });
        while self.blocks.len() > MAX_BLOCKS {
            self.blocks.pop_front();
        }
        number
    }

    /// Ends the running command at `end`, and hands an agent its output.
    fn end_block(&mut self, end: Position, exit: Option<i32>) {
        let Some(number) = self.running.take() else {
            return;
        };
        let now = self.clock.now();
        if let Some(block) = self.blocks.iter_mut().find(|block| block.number == number) {
            block.end = Some(end);
            block.exit_code = exit;
            block.ended_at = Some(now);
        }
        let head = self.emulator.take_head();
        if let Some(reply) = self.running_reply.take()
            && let Some(block) = self
                .blocks
                .iter()
                .find(|block| block.number == number)
                .cloned()
        {
            let head_rows = head.map(|head| head.rows).unwrap_or_default();
            let text = self.text_with_head(&block, &head_rows);
            let _delivered = reply.send(BlockOutcome { block, text });
        }
        self.send_block(number);
    }

    fn text_with_head(
        &mut self,
        block: &Block,
        head: &[(u64, crate::emulator::RowText)],
    ) -> BlockText {
        let (from, to) = block.rows(self.emulator.cursor());
        let rows = self.emulator.rows_text(from, to);
        text_of(rows, from, block.start.column, block.end, head)
    }

    /// The text of a command of this terminal.
    pub(crate) fn block_text(&mut self, number: u32) -> Option<(Block, BlockText)> {
        let block = self
            .blocks
            .iter()
            .find(|block| block.number == number)?
            .clone();
        let text = self.text_with_head(&block, &[]);
        Some((block, text))
    }

    fn on_marks(&mut self, marks: Vec<PlacedMark>) {
        for placed in marks {
            match placed.mark {
                Mark::PromptStart => {
                    self.marking = true;
                    self.set_phase(TerminalPhase::Prompt);
                }
                Mark::PromptEnd => {
                    self.marking = true;
                    self.prompt_end = Some(placed.at);
                    self.set_phase(TerminalPhase::Prompt);
                }
                Mark::CommandLine(line) => self.next_line = Some(line),
                Mark::CommandStart => self.on_command_start(placed),
                Mark::CommandEnd { exit } => self.on_command_end(placed.at, exit),
                Mark::Folder(folder) => self.folder = Some(folder),
                Mark::User(user) => self.user = Some(user),
            }
        }
    }

    fn on_command_start(&mut self, placed: PlacedMark) {
        // A command that started without ending, such as one that a shell
        // replaced, ends where this one starts.
        self.end_block(placed.at, None);
        self.owner_enter = None;
        let line = self.next_line.take().or(placed.line).unwrap_or_default();
        let number = if let Some(claim) = self.agent.take() {
            self.running_reply = Some(claim.reply);
            self.start_block(Author::Agent, claim.command, placed.at)
        } else {
            self.start_block(Author::Owner, line, placed.at)
        };
        self.running = Some(number);
        self.emulator.start_head(placed.at.row);
        self.set_phase(TerminalPhase::Running);
        self.send_block(number);
    }

    fn on_command_end(&mut self, at: Position, exit: Option<i32>) {
        if self.running.is_some() {
            self.end_block(at, exit);
        } else if let Some(claim) = self.agent.take() {
            // A shell without a start marker, such as sh or cmd: the command
            // ran from the row after its line to this prompt.
            let (_line, start) = self.command_line(claim.prompt_end);
            let number = self.start_block(Author::Agent, claim.command, start);
            self.running = Some(number);
            self.running_reply = Some(claim.reply);
            self.end_block(at, exit);
        } else if let Some(entered) = self.owner_enter.take() {
            let (line, start) = self.command_line(entered.prompt_end);
            if !line.is_empty() {
                let number = self.start_block(Author::Owner, line, start);
                self.running = Some(number);
                self.end_block(at, exit);
            }
        }
        self.next_line = None;
        self.set_phase(TerminalPhase::Prompt);
    }

    fn on_owner_input(&mut self, enter: bool) {
        self.owner_typed_at = Some(Instant::now());
        if !enter || self.phase != TerminalPhase::Prompt || self.agent.is_some() {
            return;
        }
        let Some(prompt_end) = self.prompt_end else {
            return;
        };
        self.owner_enter = Some(OwnerEnter { prompt_end });
    }

    /// The command line that a shell echoed after a prompt's end, through the
    /// rows that it wrapped onto, and where the output after it starts.
    fn command_line(&mut self, prompt_end: Position) -> (String, Position) {
        let rows = self
            .emulator
            .rows_text(prompt_end.row, prompt_end.row + MAX_COMMAND_ROWS);
        let mut line = String::new();
        let mut row = prompt_end.row;
        for (offset, text) in rows.into_iter().enumerate() {
            row = prompt_end.row + offset as u64;
            let Some(text) = text else {
                break;
            };
            if offset == 0 {
                line.extend(text.text.chars().skip(usize::from(prompt_end.column)));
            } else {
                line.push_str(&text.text);
            }
            if !text.wrapped {
                break;
            }
        }
        let start = Position {
            row: row + 1,
            column: 0,
        };
        (line.trim().to_owned(), start)
    }

    fn owner_typing(&self) -> bool {
        let recent = self
            .owner_typed_at
            .is_some_and(|at| at.elapsed() < TYPING_GRACE);
        let moved = self
            .prompt_end
            .is_none_or(|prompt_end| prompt_end != self.emulator.cursor());
        recent || moved
    }

    /// Claims the prompt for an agent's command.
    fn claim(
        &mut self,
        expected: Option<&str>,
        command: String,
    ) -> Result<oneshot::Receiver<BlockOutcome>, Refusal> {
        if self.ending.is_some() {
            return Err(Refusal::Ended);
        }
        if !self.marking {
            return Err(Refusal::NotMarking);
        }
        let busy = self.phase != TerminalPhase::Prompt
            || self.running.is_some()
            || self.agent.is_some()
            || self.owner_enter.is_some();
        if busy {
            return Err(Refusal::Busy);
        }
        if self.owner_typing() {
            return Err(Refusal::OwnerTyping);
        }
        if let Some(expected) = expected
            && self.state().as_deref() != Some(expected)
        {
            return Err(Refusal::Changed { now: self.state() });
        }
        let (reply, outcome) = oneshot::channel();
        let prompt_end = self.prompt_end.unwrap_or_else(|| self.emulator.cursor());
        self.agent = Some(AgentClaim {
            command,
            reply,
            prompt_end,
        });
        self.set_phase(TerminalPhase::Running);
        Ok(outcome)
    }

    fn on_output(&mut self, bytes: &[u8]) {
        self.outputs += 1;
        self.last_output = Some(Instant::now());
        let marks = self.emulator.process(bytes);
        self.on_marks(marks);
        let text = self.decode(bytes);
        if !text.is_empty() {
            let top_row = self.emulator.top_row();
            for view in &mut self.views {
                view.output(&text, top_row);
            }
        }
    }

    /// The output as text, with a character that a piece split kept for the
    /// next piece.
    fn decode(&mut self, bytes: &[u8]) -> String {
        let mut joined = std::mem::take(&mut self.carry);
        joined.extend_from_slice(bytes);
        let complete = complete_utf8_len(&joined);
        self.carry = joined.split_off(complete);
        String::from_utf8_lossy(&joined).into_owned()
    }

    fn on_ended(&mut self, ending: &TerminalEnding) {
        if self.ending.is_some() {
            return;
        }
        let cursor = self.emulator.cursor();
        self.end_block(cursor, None);
        // An agent that claimed the prompt hears nothing more.
        self.agent = None;
        self.owner_enter = None;
        self.ending = Some(ending.clone());
        self.running_terminals.fetch_sub(1, Ordering::Relaxed);
        self.set_phase(TerminalPhase::Ended);
        let view = ending_view(ending);
        for subscriber in &self.views {
            subscriber.send(TerminalUpdate::Ended {
                ending: view.clone(),
            });
        }
    }

    fn subscribe(&mut self, mut view: View) {
        view.send_snapshot(&self.emulator);
        let blocks = self
            .blocks
            .iter()
            .map(|block| self.block_view(block))
            .collect();
        view.send(TerminalUpdate::Blocks { blocks });
        view.send(TerminalUpdate::Status {
            phase: self.phase,
            marking: self.marking,
            state: self.state(),
        });
        if let Some(ending) = &self.ending {
            view.send(TerminalUpdate::Ended {
                ending: ending_view(ending),
            });
        }
        self.views.push(view);
    }

    /// Shares the logs' bytes between the terminals that run.
    fn fit_log(&mut self) {
        let running = self.running_terminals.load(Ordering::Relaxed).max(1);
        let cap = LOG_BYTES.min(ALL_LOGS_BYTES / running);
        if cap != self.log_cap {
            self.log_cap = cap;
            self.emulator.set_log_cap(cap);
        }
    }

    fn handle(&mut self, message: Message) {
        match message {
            Message::Output(bytes) => self.on_output(&bytes),
            Message::Ended(ending) => self.on_ended(&ending),
            Message::Resize { columns, rows } => self.emulator.resize(columns, rows),
            Message::OwnerInput { enter } => self.on_owner_input(enter),
            Message::Subscribe(view) => self.subscribe(view),
            Message::Acknowledge { view, seq } => {
                if let Some(subscriber) =
                    self.views.iter_mut().find(|candidate| candidate.id == view)
                {
                    subscriber.acknowledge(seq);
                }
            }
            Message::Run(job) => job(self),
            Message::Stop => self.stopped = true,
        }
    }

    /// Runs the views' timers, and returns when the next one is due. A view
    /// whose channel filled up starts over as a new one would.
    fn tick(&mut self) -> Option<Instant> {
        self.views.retain(|view| !view.is_closed());
        let mut index = 0;
        while index < self.views.len() {
            if self.views[index].must_start_over() {
                let mut view = self.views.remove(index);
                view.start_over();
                self.subscribe(view);
            } else {
                index += 1;
            }
        }
        let now = Instant::now();
        let last_output = self.last_output;
        let emulator = &self.emulator;
        self.views
            .iter_mut()
            .filter_map(|view| view.tick(emulator, now, last_output))
            .min()
    }

    fn run(mut self, inbox: &Inbox) {
        let mut deadline = None;
        while !self.stopped {
            let messages = inbox.take(deadline);
            for message in messages {
                self.handle(message);
            }
            self.fit_log();
            self.publish();
            self.send_status_if_changed();
            deadline = self.tick();
        }
    }
}

/// How many bytes from the start of `bytes` end on a character's boundary:
/// all of them, but a character that the end cuts.
fn complete_utf8_len(bytes: &[u8]) -> usize {
    let start = bytes.len().saturating_sub(3);
    for index in (start..bytes.len()).rev() {
        let byte = bytes[index];
        if byte & 0b1100_0000 == 0b1000_0000 {
            continue;
        }
        let needed = match byte {
            0x00..=0x7f => 1,
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            _ => return bytes.len(),
        };
        return if bytes.len() - index >= needed {
            bytes.len()
        } else {
            index
        };
    }
    bytes.len()
}

/// How a terminal ended, as its views word it.
pub(crate) fn ending_view(ending: &TerminalEnding) -> EndingView {
    match ending {
        TerminalEnding::Exited { code } => EndingView::Exited { code: *code },
        TerminalEnding::Lost { code } => EndingView::Lost { code: code.clone() },
        TerminalEnding::PluginOff | TerminalEnding::ShuttingDown => EndingView::Stopped,
        _ => EndingView::Closed,
    }
}

/// A terminal: what the registry, the tools and the channels hold of it.
pub(crate) struct Session {
    /// The session's own ID, which its cite links and mentions name.
    pub(crate) id: String,
    terminal: SharedTerminal,
    inbox: Arc<Inbox>,
    status: watch::Receiver<Status>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Session")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Starts the thread and the pump of a terminal that the core opened.
    pub(crate) fn start(mut terminal: Box<dyn Terminal>, start: SessionStart) -> Arc<Self> {
        let output = terminal.output();
        let shared: SharedTerminal = Arc::new(Mutex::new(Some(terminal)));
        let inbox = Arc::new(Inbox::default());
        let (status_sender, status) = watch::channel(Status::starting());
        start.running.fetch_add(1, Ordering::Relaxed);
        let core = Core {
            emulator: Emulator::new(start.size.columns, start.size.rows, &start.nonce, LOG_BYTES),
            id: start.id.clone(),
            clock: start.clock,
            terminal: Arc::clone(&shared),
            running_terminals: start.running,
            log_cap: LOG_BYTES,
            marking: false,
            phase: TerminalPhase::Starting,
            prompt_end: None,
            next_line: None,
            blocks: VecDeque::new(),
            next_block: 1,
            running: None,
            running_reply: None,
            agent: None,
            owner_enter: None,
            folder: None,
            user: None,
            owner_typed_at: None,
            views: Vec::new(),
            status: status_sender,
            carry: Vec::new(),
            last_output: None,
            ending: None,
            outputs: 0,
            selections: VecDeque::new(),
            sent_status: None,
            stopped: false,
        };
        let thread_inbox = Arc::clone(&inbox);
        let spawned = std::thread::Builder::new()
            .name(format!("terminal-{}", start.id))
            .spawn(move || core.run(&thread_inbox));
        if let Err(error) = spawned {
            tracing::error!(%error, "the thread of a terminal could not start");
        }
        tokio::spawn(pump(output, Arc::clone(&inbox)));
        Arc::new(Self {
            id: start.id,
            terminal: shared,
            inbox,
            status,
        })
    }

    /// The terminal's state as its thread published it last.
    pub(crate) fn status(&self) -> Status {
        self.status.borrow().clone()
    }

    /// Follows the terminal's state.
    pub(crate) fn follow_status(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }

    /// Runs `job` on the terminal's thread and returns its answer, or `None`
    /// when the thread stopped.
    pub(crate) async fn ask<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut CoreAccess<'_>) -> T + Send + 'static,
    ) -> Option<T> {
        let (reply, answer) = oneshot::channel();
        self.inbox.push(Message::Run(Box::new(move |core| {
            let _delivered = reply.send(job(&mut CoreAccess { core }));
        })));
        answer.await.ok()
    }

    /// Writes bytes to the terminal.
    pub(crate) async fn write(&self, bytes: Vec<u8>) -> Result<(), TerminalIoError> {
        let writing = {
            let terminal = self.terminal.lock().unwrap_or_else(PoisonError::into_inner);
            match terminal.as_ref() {
                Some(terminal) => terminal.write(bytes),
                None => return Err(TerminalIoError::Ended),
            }
        };
        writing.await
    }

    /// Writes what the owner typed, which the thread notes for the typing
    /// check and for commands without a start marker.
    pub(crate) async fn write_owner(&self, data: String) -> Result<(), TerminalIoError> {
        let enter = data.contains('\r');
        self.inbox.push(Message::OwnerInput { enter });
        self.write(data.into_bytes()).await
    }

    /// Changes the terminal's size.
    pub(crate) async fn resize(&self, size: TerminalSize) -> Result<(), TerminalIoError> {
        self.inbox.push(Message::Resize {
            columns: size.columns,
            rows: size.rows,
        });
        let resizing = {
            let terminal = self.terminal.lock().unwrap_or_else(PoisonError::into_inner);
            match terminal.as_ref() {
                Some(terminal) => terminal.resize(size),
                None => return Err(TerminalIoError::Ended),
            }
        };
        resizing.await
    }

    /// Adds a view, which gets a snapshot first.
    pub(crate) fn subscribe(&self, view: View) {
        self.inbox.push(Message::Subscribe(view));
    }

    /// Records what a view wrote.
    pub(crate) fn acknowledge(&self, view: String, seq: u64) {
        self.inbox.push(Message::Acknowledge { view, seq });
    }

    /// Ends the terminal: hangs it up and ends what is left after the grace.
    pub(crate) async fn end(&self) {
        let terminal = self
            .terminal
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(terminal) = terminal {
            let ending = terminal.close(CLOSE_GRACE).await;
            self.inbox.push(Message::Ended(ending));
        }
    }

    /// Stops the thread, after the terminal ended.
    pub(crate) fn stop(&self) {
        self.inbox.push(Message::Stop);
        self.inbox.stop();
    }
}

/// What a job on a terminal's thread reaches.
pub(crate) struct CoreAccess<'core> {
    core: &'core mut Core,
}

impl CoreAccess<'_> {
    /// Claims the prompt for an agent's command, if the terminal is idle and
    /// in the state that the approval named.
    pub(crate) fn claim(
        &mut self,
        expected: Option<&str>,
        command: String,
    ) -> Result<oneshot::Receiver<BlockOutcome>, Refusal> {
        self.core.claim(expected, command)
    }

    /// Lets go of a claim whose command could not be typed.
    pub(crate) fn release_claim(&mut self) {
        if self.core.agent.take().is_some() {
            self.core.set_phase(TerminalPhase::Prompt);
        }
    }

    /// The text of a command.
    pub(crate) fn block_text(&mut self, number: u32) -> Option<(Block, BlockText)> {
        self.core.block_text(number)
    }

    /// The commands, the latest last.
    pub(crate) fn blocks(&self) -> Vec<Block> {
        self.core.blocks.iter().cloned().collect()
    }

    /// Keeps a selection under a key, and returns the key.
    pub(crate) fn keep_selection(&mut self, text: String) -> String {
        let key = cite::selection_key(&self.core.id);
        self.core.selections.push_back((key.clone(), text));
        while self.core.selections.len() > MAX_SELECTIONS {
            self.core.selections.pop_front();
        }
        key
    }

    /// A selection that the terminal keeps.
    pub(crate) fn selection(&self, key: &str) -> Option<String> {
        self.core
            .selections
            .iter()
            .find(|(kept, _text)| kept == key)
            .map(|(_kept, text)| text.clone())
    }
}

/// Pumps the core's output into the thread's inbox.
async fn pump(mut output: BoxStream<'static, TerminalOutput>, inbox: Arc<Inbox>) {
    while let Some(item) = output.next().await {
        match item {
            TerminalOutput::Bytes { bytes } => inbox.push_output(bytes).await,
            TerminalOutput::Ended { ending } => {
                inbox.push(Message::Ended(ending));
                return;
            }
            _ => {}
        }
    }
    inbox.push(Message::Ended(TerminalEnding::Closed));
}

#[cfg(test)]
mod tests {
    use super::complete_utf8_len;

    #[test]
    fn a_character_that_the_end_cuts_waits_for_the_next_piece() {
        assert_eq!(complete_utf8_len(b"abc"), 3);
        let euro = "€".as_bytes();
        assert_eq!(complete_utf8_len(&euro[..2]), 0);
        assert_eq!(complete_utf8_len(euro), 3);
        let mut text = b"ok ".to_vec();
        text.extend_from_slice(&"😀".as_bytes()[..3]);
        assert_eq!(complete_utf8_len(&text), 3);
        // A byte that starts nothing is not held back.
        assert_eq!(complete_utf8_len(&[b'a', 0xff]), 2);
    }
}
