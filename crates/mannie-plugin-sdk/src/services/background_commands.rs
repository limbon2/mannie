//! The service `background-commands` (ADR-0041, decision 11): a plugin's tool
//! that starts a command on a remote host in the background hands it to the
//! core, which follows it as it follows the commands that run on the owner's
//! machine. The core keeps the command's output for the model's reads,
//! records its end, and ends it when the agent or the owner stops it, when
//! the plugin goes off and when Mannie quits.
//!
//! The core puts a ticket in the execution of a call whose authorized
//! accesses hold a remote command in the background, good for one start
//! while the call executes. The command's host and text come from that
//! access, never from the plugin. The plugin hands the core the command as a
//! trait object (decision 3): a stream of its lines that ends with how it
//! ended, and a stop that ends it and resolves once it ended.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::service::Service;
use crate::{BoxFuture, BoxStream};

/// The code of the item of a plugin's work in progress that counts the
/// background commands that its tools started and that still run.
pub const BACKGROUND_COMMANDS_WORK: &str = "core.background_commands";

/// The service `background-commands`, major 1. A command that outlives its
/// call keeps acting on a host with the owner's rights, so the service is
/// sensitive.
#[derive(Debug)]
pub enum BackgroundCommands {}

impl Service for BackgroundCommands {
    const NAME: &'static str = "background-commands";
    const MAJOR: u16 = 1;
    const SENSITIVE: bool = true;
    type Api = dyn BackgroundCommandsApi;
}

/// The core's registry of background commands, as a plugin reaches it.
pub trait BackgroundCommandsApi: Send + Sync + fmt::Debug {
    /// Hands the core a command that the call of `ticket` started, and
    /// returns its number in the call's conversation. Once handed over, the
    /// command is the core's: a start that the core refuses ends it before
    /// the future resolves.
    fn start(
        &self,
        ticket: BackgroundTicket,
        command: Box<dyn RemoteCommand>,
    ) -> BoxFuture<'static, Result<StartedCommand, BackgroundError>>;
}

/// A command that a plugin's tool started on a remote host and hands the
/// core.
pub trait RemoteCommand: Send + fmt::Debug {
    /// The command's output: its lines in the order the host sent them,
    /// then how it ended. The core takes it once, right after the start,
    /// and reads it until it ends. A stream that ends without saying how the
    /// command ended lets the command go.
    fn output(&mut self) -> BoxStream<'static, RemoteOutput>;

    /// Ends the command and resolves once it ended, with how it ended. The
    /// core calls it when the agent or the owner stops the command, when the
    /// plugin goes off and when Mannie quits; the output then ends.
    fn stop(self: Box<Self>) -> BoxFuture<'static, RemoteEnding>;
}

/// What a remote command's output carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum RemoteOutput {
    /// Whole lines of the output, without their line breaks, with what the
    /// plugin keeps secret already replaced.
    Lines {
        /// The lines.
        lines: Vec<String>,
    },
    /// How the command ended. Nothing follows.
    Ended {
        /// The end.
        ending: RemoteEnding,
    },
}

impl RemoteOutput {
    /// Lines of the output.
    pub fn lines(lines: Vec<String>) -> Self {
        Self::Lines { lines }
    }

    /// How the command ended.
    pub fn ended(ending: RemoteEnding) -> Self {
        Self::Ended { ending }
    }
}

/// How a remote command ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum RemoteEnding {
    /// The host reported the command's exit status.
    Exited {
        /// The status.
        status: u32,
    },
    /// The host reported the signal that ended the command, such as `TERM`.
    Signaled {
        /// The signal's name, without `SIG`.
        signal: String,
    },
    /// The host closed the command's channel without saying how it ended.
    Closed,
    /// The plugin closed its session to the host, as when the owner edits or
    /// removes the host.
    SessionClosed,
}

/// The ticket of a call that may hand the core one command that it starts
/// in the background. Only the core makes one.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BackgroundTicket(String);

impl BackgroundTicket {
    /// A ticket with this text. Only the core makes one.
    #[cfg(feature = "host")]
    pub fn new(text: String) -> Self {
        Self(text)
    }

    /// The ticket's text.
    #[cfg(feature = "host")]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A command that the core took.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct StartedCommand {
    /// Its number in its conversation, which the model names to read or
    /// stop it.
    pub number: u32,
}

impl StartedCommand {
    /// A command that the core took. Only the core makes one.
    #[cfg(feature = "host")]
    pub fn new(number: u32) -> Self {
        Self { number }
    }
}

/// Which limit of running commands a start reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundScope {
    /// The commands that run at once in one conversation.
    Conversation,
    /// The commands that run at once on the machine.
    Machine,
}

/// Why the core did not take a command, which it ended. The core adds
/// reasons by addition, so a plugin treats a reason that it does not know as
/// a failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum BackgroundError {
    /// No call of the plugin that executes now holds the ticket, or the
    /// ticket started a command already.
    #[error("the ticket does not let a command start")]
    TicketInvalid,
    /// As many commands run as a limit allows.
    #[error("{limit} background commands run already, the most for the {scope:?}")]
    TooMany {
        /// Which limit the start reached.
        scope: BackgroundScope,
        /// The limit.
        limit: u32,
    },
    /// Mannie is quitting.
    #[error("Mannie is quitting")]
    ShuttingDown,
    /// The core could not record the command. The log has the cause.
    #[error("the command could not be recorded")]
    StorageFailed,
}
