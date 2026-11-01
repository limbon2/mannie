//! The service `terminals` (ADR-0053, decision 4): the core's registry of
//! terminals, through which a plugin opens interactive shells on this machine
//! and on the hosts that plugins offer through the point `terminal-hosts`.
//!
//! On this machine the core starts a shell that it lists, attached to a
//! pseudo console and contained as a command is (ADR-0014), with the
//! environment of commands and a setup that the plugin passes: a script that
//! the shell runs after the owner's profile, which the core hands each shell
//! through the shell's own means. On a host that a plugin offers, the core
//! asks that plugin's source to open the terminal, and checks the endpoint
//! and identity that an approval named.
//!
//! Every terminal belongs to a conversation. Deleting the conversation, the
//! plugin going off and Mannie quitting end its terminals, and the terminal's
//! output then ends with how it ended. The core counts the terminals that a
//! plugin marks as running a command, for the warning before Mannie quits.
//!
//! A terminal comes back as a trait object (ADR-0034, decision 3): bytes go
//! in through `write`, the output comes out as a stream, and dropping the
//! terminal ends it.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::service::Service;
use crate::{BoxFuture, BoxStream};

/// The most terminals that one conversation holds at once.
pub const MAX_TERMINALS_PER_CONVERSATION: u32 = 8;

/// The most terminals that run on this machine at once, those on other
/// hosts included.
pub const MAX_TERMINALS_PER_MACHINE: u32 = 32;

/// The most bytes of one write to a terminal.
pub const MAX_TERMINAL_WRITE_BYTES: usize = 64 * 1024;

/// The most bytes of a setup's script.
pub const MAX_SETUP_BYTES: usize = 16 * 1024;

/// The most columns of a terminal.
pub const MAX_TERMINAL_COLUMNS: u16 = 1000;

/// The most rows of a terminal.
pub const MAX_TERMINAL_ROWS: u16 = 500;

/// The longest that closing a terminal waits for its shell to end after the
/// hang-up, before it ends the shell's tree.
pub const MAX_TERMINAL_CLOSE_GRACE: Duration = Duration::from_secs(10);

/// The service `terminals`, major 1. A terminal runs whatever is typed into
/// it with the owner's rights, so the service is sensitive.
#[derive(Debug)]
pub enum Terminals {}

impl Service for Terminals {
    const NAME: &'static str = "terminals";
    const MAJOR: u16 = 1;
    const SENSITIVE: bool = true;
    type Api = dyn TerminalsApi;
}

/// The terminals of a plugin.
pub trait TerminalsApi: Send + Sync + fmt::Debug {
    /// This machine's shells and the hosts that plugins offer. Each item is
    /// the complete list, which replaces the one before; the first comes at
    /// once.
    fn hosts(&self) -> BoxStream<'static, TerminalHostList>;

    /// Opens a terminal, and returns it once its shell runs or its host
    /// answered.
    fn open(
        &self,
        spec: TerminalSpec,
    ) -> BoxFuture<'static, Result<Box<dyn Terminal>, TerminalOpenError>>;
}

/// Where terminals can open.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct TerminalHostList {
    /// This machine's shells, in the order of the menu, the default first.
    pub shells: Vec<LocalShell>,
    /// The hosts that plugins offer, by plugin and then by name.
    pub hosts: Vec<OfferedHost>,
}

impl TerminalHostList {
    /// A list with these shells and hosts. Only the core makes one.
    #[cfg(feature = "host")]
    pub fn new(shells: Vec<LocalShell>, hosts: Vec<OfferedHost>) -> Self {
        Self { shells, hosts }
    }
}

/// A shell of this machine that a terminal can run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct LocalShell {
    /// The canonical path of the shell's program, which names it in a
    /// [`TerminalHostRef::ThisMachine`].
    pub program: PathBuf,
    /// The shell's name, such as `Windows PowerShell` or `bash`.
    pub name: String,
    /// What kind of shell it is, which decides how a setup reaches it.
    pub kind: ShellKind,
    /// Whether a terminal on this machine runs it when the plugin names no
    /// shell.
    pub default: bool,
}

impl LocalShell {
    /// A shell that the core found. Only the core makes one.
    #[cfg(feature = "host")]
    pub fn new(program: PathBuf, name: String, kind: ShellKind, default: bool) -> Self {
        Self {
            program,
            name,
            kind,
            default,
        }
    }
}

/// A host that a plugin offers through the point `terminal-hosts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct OfferedHost {
    /// The plugin that offers the host.
    pub plugin: String,
    /// The plugin's key for the host.
    pub key: String,
    /// The host's name, as the owner gave it.
    pub label: String,
    /// Where terminals run there, such as `deploy@build-box.lan:22`.
    pub endpoint: String,
    /// What identifies the host beyond its address, such as the fingerprint
    /// of its pinned key.
    pub identity: String,
    /// Whether terminals open there now.
    pub state: TerminalHostState,
    /// The host's login shell, when its plugin knows it.
    pub shell: Option<ShellKind>,
}

impl OfferedHost {
    /// A host with these parts. Only the core makes one, from a source's
    /// entry.
    #[cfg(feature = "host")]
    pub fn new(plugin: String, entry: crate::points::terminal_hosts::TerminalHostEntry) -> Self {
        Self {
            plugin,
            key: entry.key,
            label: entry.label,
            endpoint: entry.endpoint,
            identity: entry.identity,
            state: entry.state,
            shell: entry.shell,
        }
    }
}

/// Whether terminals open on a host now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum TerminalHostState {
    /// Terminals open there.
    Ready,
    /// They do not, for the plugin's reason, such as `no_password` or
    /// `key_changed`: 1 to 64 lower-case letters and underscores.
    Unavailable {
        /// The plugin's code.
        code: String,
    },
}

/// The kind of a shell, which decides how a setup reaches it and how its
/// commands split into words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ShellKind {
    /// GNU bash, also Git's bash on Windows.
    Bash,
    /// zsh.
    Zsh,
    /// fish.
    Fish,
    /// A POSIX `sh`, such as dash or FreeBSD's `sh`.
    Sh,
    /// Windows PowerShell or PowerShell 7.
    PowerShell,
    /// Windows' `cmd.exe`.
    Cmd,
    /// Any other shell, which a setup does not reach.
    Other,
}

/// Where a terminal runs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum TerminalHostRef {
    /// This machine, in one of the shells that the core lists.
    ThisMachine {
        /// The shell's program, as [`LocalShell::program`] names it.
        shell: PathBuf,
    },
    /// A host that a plugin offers.
    Plugin {
        /// The plugin that offers the host.
        plugin: String,
        /// The plugin's key for the host.
        key: String,
    },
}

/// A terminal to open.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TerminalSpec {
    /// The conversation that the terminal belongs to, which must exist.
    pub conversation: String,
    /// Where the terminal runs.
    pub host: TerminalHostRef,
    /// The absolute folder that a shell of this machine starts in, which must
    /// exist, or `None` for the owner's home folder. A host's terminal starts
    /// where its login does.
    pub folder: Option<PathBuf>,
    /// The terminal's size.
    pub size: TerminalSize,
    /// The script that a shell of this machine runs after the owner's
    /// profile, or `None` for none. A host's shell gets none from the core.
    pub setup: Option<String>,
    /// For a terminal that an approved command opens on a host, the endpoint
    /// and identity that the approval named; the terminal does not open
    /// where either changed.
    pub expect: Option<ExpectedHost>,
}

impl TerminalSpec {
    /// A terminal of `conversation` on `host`, of `size`, in the default
    /// folder, without a setup.
    pub fn new(conversation: impl Into<String>, host: TerminalHostRef, size: TerminalSize) -> Self {
        Self {
            conversation: conversation.into(),
            host,
            folder: None,
            size,
            setup: None,
            expect: None,
        }
    }

    /// The same terminal, starting in `folder`.
    #[must_use]
    pub fn in_folder(mut self, folder: impl Into<PathBuf>) -> Self {
        self.folder = Some(folder.into());
        self
    }

    /// The same terminal, whose shell runs `script` after the owner's
    /// profile.
    #[must_use]
    pub fn with_setup(mut self, script: impl Into<String>) -> Self {
        self.setup = Some(script.into());
        self
    }

    /// The same terminal, opened only where the host's endpoint and identity
    /// are still those that an approval named.
    #[must_use]
    pub fn expecting(mut self, expect: ExpectedHost) -> Self {
        self.expect = Some(expect);
        self
    }
}

/// The endpoint and identity of a host as an approval named them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpectedHost {
    /// Where terminals run there.
    pub endpoint: String,
    /// What identifies the host beyond its address.
    pub identity: String,
}

/// The size of a terminal, in characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSize {
    /// The columns, from 1 to [`MAX_TERMINAL_COLUMNS`].
    pub columns: u16,
    /// The rows, from 1 to [`MAX_TERMINAL_ROWS`].
    pub rows: u16,
}

impl TerminalSize {
    /// A size, or `None` when it is outside the limits.
    pub fn new(columns: u16, rows: u16) -> Option<Self> {
        let fits = (1..=MAX_TERMINAL_COLUMNS).contains(&columns)
            && (1..=MAX_TERMINAL_ROWS).contains(&rows);
        fits.then_some(Self { columns, rows })
    }
}

/// A terminal that runs. Dropping it ends it, as `close` with no grace does.
pub trait Terminal: Send + fmt::Debug {
    /// The terminal's output as it arrives, then how the terminal ended. The
    /// plugin takes it once; a later call gets a stream that ends at once.
    fn output(&mut self) -> BoxStream<'static, TerminalOutput>;

    /// Writes bytes to the terminal's input, after the bytes of every earlier
    /// call, at most [`MAX_TERMINAL_WRITE_BYTES`] at a time.
    fn write(&self, bytes: Vec<u8>) -> BoxFuture<'static, Result<(), TerminalIoError>>;

    /// Changes the terminal's size.
    fn resize(&self, size: TerminalSize) -> BoxFuture<'static, Result<(), TerminalIoError>>;

    /// Marks whether a command runs in the terminal, which the warning before
    /// Mannie quits counts.
    fn set_running(&self, running: bool);

    /// Hangs up the terminal, waits at most `grace`, and at most
    /// [`MAX_TERMINAL_CLOSE_GRACE`], for its shell to end, then ends what is
    /// left. Resolves with how it ended.
    fn close(self: Box<Self>, grace: Duration) -> BoxFuture<'static, TerminalEnding>;
}

/// What a terminal's output carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum TerminalOutput {
    /// Bytes of the output, in order, as the pseudo console or the host
    /// wrote them. They may end in the middle of a character or of an escape
    /// sequence.
    Bytes {
        /// The bytes, as base64 in JSON.
        #[serde(with = "base64_bytes")]
        bytes: Vec<u8>,
    },
    /// How the terminal ended. Nothing follows.
    Ended {
        /// The end.
        ending: TerminalEnding,
    },
}

/// How a terminal ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum TerminalEnding {
    /// The shell exited with this code.
    Exited {
        /// The exit code.
        code: i32,
    },
    /// The plugin closed it, or the shell ended without an exit code.
    Closed,
    /// The core ended it: its conversation was deleted.
    ConversationDeleted,
    /// The core ended it: its plugin went off.
    PluginOff,
    /// The core ended it: Mannie quit.
    ShuttingDown,
    /// The connection to its host was lost, with the host's plugin's code.
    Lost {
        /// The plugin's code, such as `connection_lost`.
        code: String,
    },
}

/// Why a terminal did not open. The core adds reasons by addition, so a
/// plugin treats a reason that it does not know as a failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum TerminalOpenError {
    /// The conversation does not exist.
    #[error("the conversation does not exist")]
    ConversationNotFound,
    /// The shell is not one that the core lists.
    #[error("the shell is not one of this machine's shells")]
    ShellNotListed,
    /// No plugin offers the host, or its plugin is off.
    #[error("no plugin offers this host")]
    HostNotFound,
    /// The host's endpoint or identity is not the one that the approval
    /// named.
    #[error("the host changed since the approval")]
    HostChanged,
    /// The host's plugin does not open terminals there now, with its code.
    #[error("the host does not open terminals now: {reason}")]
    HostUnavailable {
        /// The plugin's code.
        reason: String,
    },
    /// The folder is not an absolute folder that exists.
    #[error("the folder is not a folder that exists")]
    FolderInvalid,
    /// The setup is larger than [`MAX_SETUP_BYTES`] or holds NUL.
    #[error("the setup cannot be given to the shell")]
    SetupRefused,
    /// The conversation or the machine holds as many terminals as it may.
    #[error("{limit} terminals run already, the most that are allowed")]
    TooMany {
        /// The limit that was reached.
        limit: u32,
    },
    /// This version of the system has no pseudo console, such as Windows
    /// before 10 1809.
    #[error("this system cannot run terminals")]
    Unsupported,
    /// The shell could not be started, or the host's plugin failed with its
    /// code. The log has the cause.
    #[error("the terminal could not be opened: {reason}")]
    Failed {
        /// The cause's code, such as `spawn_failed` or a plugin's code.
        reason: String,
    },
    /// Mannie is quitting.
    #[error("Mannie is quitting")]
    ShuttingDown,
}

/// Why a terminal took no bytes or no new size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum TerminalIoError {
    /// The terminal ended.
    #[error("the terminal ended")]
    Ended,
    /// The bytes are more than [`MAX_TERMINAL_WRITE_BYTES`], or the size is
    /// outside the limits.
    #[error("the input or the size is outside the limits")]
    OutOfLimits,
}

/// Bytes as base64 text in JSON.
mod base64_bytes {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        STANDARD.decode(text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::{TerminalEnding, TerminalHostRef, TerminalOpenError, TerminalOutput, TerminalSize};

    #[test]
    fn output_crosses_json_as_base64_and_an_ending_by_its_kind() {
        let bytes = TerminalOutput::Bytes {
            bytes: b"\x1b[32mok\x1b[0m\r\n".to_vec(),
        };
        let ended = TerminalOutput::Ended {
            ending: TerminalEnding::Exited { code: 130 },
        };

        assert_eq!(
            serde_json::from_value::<TerminalOutput>(serde_json::to_value(&bytes).unwrap())
                .unwrap(),
            bytes
        );
        assert_eq!(
            serde_json::to_value(&ended).unwrap(),
            json!({ "kind": "ended", "ending": { "kind": "exited", "code": 130 } })
        );
        assert_eq!(
            serde_json::to_value(TerminalOpenError::TooMany { limit: 8 }).unwrap(),
            json!({ "code": "too_many", "limit": 8 })
        );
    }

    #[test]
    fn a_host_reference_names_a_shell_or_a_plugins_key() {
        assert_eq!(
            serde_json::to_value(TerminalHostRef::ThisMachine {
                shell: PathBuf::from("/bin/bash")
            })
            .unwrap(),
            json!({ "kind": "this_machine", "shell": "/bin/bash" })
        );
        assert_eq!(
            serde_json::to_value(TerminalHostRef::Plugin {
                plugin: "ssh".to_owned(),
                key: "host-1".to_owned()
            })
            .unwrap(),
            json!({ "kind": "plugin", "plugin": "ssh", "key": "host-1" })
        );
    }

    #[test]
    fn a_size_stays_within_its_limits() {
        assert!(TerminalSize::new(120, 30).is_some());
        assert!(TerminalSize::new(0, 30).is_none());
        assert!(TerminalSize::new(120, 0).is_none());
        assert!(TerminalSize::new(1001, 30).is_none());
        assert!(TerminalSize::new(120, 501).is_none());
    }
}
