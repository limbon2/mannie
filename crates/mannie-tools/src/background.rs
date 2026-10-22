//! The registry of background commands as the tools see it (ADR-0041,
//! docs/features/system-tools/background-commands.md): a port that the
//! runtime of each machine implements, as it implements the move into a
//! project, through which `start_command`, `read_command_output` and
//! `stop_command` reach the commands of their conversation.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::ids::{ConversationId, PartId};
use mannie_types::tools::{BackgroundCommandView, ProcessRecord, ToolFailure};
use regex::Regex;
use tokio::sync::oneshot;

use crate::stop::StopSignal;
use crate::supervisor::{BackgroundSpec, LogLines};

/// The longest wait of `read_command_output`, in seconds.
pub const MAX_WAIT_SECONDS: u32 = 600;

/// A command that `start_command` starts in the background.
#[derive(Debug)]
pub struct StartRequest {
    /// The conversation whose call starts it.
    pub conversation: ConversationId,
    /// The part of the call that starts it.
    pub part_id: PartId,
    /// The call's activity, which names the command.
    pub activity: Option<String>,
    /// The command, its shell, its working directory and its environment.
    pub spec: BackgroundSpec,
    /// Receives the shell's process as soon as it runs, so the call's
    /// journal holds it before the command's own record does.
    pub process: Option<oneshot::Sender<ProcessRecord>>,
}

/// A command that started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Started {
    /// Its number in its conversation.
    pub number: u32,
}

/// What `read_command_output` reads.
#[derive(Debug, Clone)]
pub struct ReadRequest {
    /// The conversation whose call reads.
    pub conversation: ConversationId,
    /// The command's number.
    pub number: u32,
    /// The first line to read, which leaves the read position where it is;
    /// or `None` for the lines after the read position, which then moves
    /// past them.
    pub from_line: Option<u64>,
    /// The most lines that a read from `from_line` returns.
    pub max_lines: usize,
    /// How long to wait for the command's end, or for a new line that
    /// matches `until`, before the read.
    pub wait: Duration,
    /// The pattern of a line that ends the wait early.
    pub until: Option<Regex>,
}

/// What a read found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRead {
    /// The command as it is now.
    pub view: BackgroundCommandView,
    /// How long it ran, or has run so far.
    pub ran_for: Duration,
    /// The lines read, with the lines among them that the log let go.
    pub lines: LogLines,
    /// The lines that the command wrote so far.
    pub total_lines: u64,
    /// The new line that matched `until`, when one did.
    pub matched: Option<u64>,
    /// Whether the wait ended because the call was stopped.
    pub stopped: bool,
    /// Whether bytes of its output that are not valid UTF-8 were replaced.
    pub replaced: bool,
    /// Whether its output was lost, because Mannie ended before it could
    /// archive it.
    pub lost: bool,
}

/// What `stop_command` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandStop {
    /// The command as it is now, ended.
    pub view: BackgroundCommandView,
    /// How long it ran.
    pub ran_for: Duration,
    /// Whether it had ended already, so the call changed nothing.
    pub had_ended: bool,
    /// How many of its lines the model has not read.
    pub unread: u64,
    /// Whether it ran on a remote host, whose processes may outlive it.
    pub remote: bool,
}

/// The background commands of a machine, as the tools reach them.
pub trait BackgroundCommands: Send + Sync + fmt::Debug {
    /// Starts a command, records it durably, and follows it.
    fn start(&self, request: StartRequest) -> BoxFuture<'static, Result<Started, ToolFailure>>;

    /// Reads a command's output, after waiting as the request says or until
    /// `stop` fires.
    fn read(
        &self,
        request: ReadRequest,
        stop: StopSignal,
    ) -> BoxFuture<'static, Result<CommandRead, ToolFailure>>;

    /// Ends a command that the agent no longer needs, and waits for its end.
    fn stop(
        &self,
        conversation: ConversationId,
        number: u32,
    ) -> BoxFuture<'static, Result<CommandStop, ToolFailure>>;
}

/// Surroundings without background commands, in which every call fails, such
/// as the tests of tools that need none.
pub fn no_background_commands() -> Arc<dyn BackgroundCommands> {
    Arc::new(NoBackgroundCommands)
}

#[derive(Debug)]
struct NoBackgroundCommands;

fn unavailable() -> ToolFailure {
    ToolFailure::IoError {
        kind: "background commands are not available here".to_owned(),
    }
}

impl BackgroundCommands for NoBackgroundCommands {
    fn start(&self, _request: StartRequest) -> BoxFuture<'static, Result<Started, ToolFailure>> {
        async { Err(unavailable()) }.boxed()
    }

    fn read(
        &self,
        _request: ReadRequest,
        _stop: StopSignal,
    ) -> BoxFuture<'static, Result<CommandRead, ToolFailure>> {
        async { Err(unavailable()) }.boxed()
    }

    fn stop(
        &self,
        _conversation: ConversationId,
        _number: u32,
    ) -> BoxFuture<'static, Result<CommandStop, ToolFailure>> {
        async { Err(unavailable()) }.boxed()
    }
}
