//! The service `local-servers` (ADR-0043, decision 5): starts a program that
//! the owner named in a plugin's settings as a long-running server, such as
//! an MCP server, which the plugin talks to over the server's standard input
//! and output.
//!
//! The core finds the program: an absolute path as the owner gave it, or a
//! name on Mannie's `PATH`, never inside the owner's projects or Mannie's own
//! folders. On Windows that is an `.exe` file, or a `.cmd` or `.bat` file,
//! which `cmd.exe` starts with arguments that it cannot read as code. The
//! program receives its arguments exactly, the environment of commands, and
//! the variables that the plugin gives it, among them the server's secrets,
//! which reach no other process (decision 6). The core contains the server's
//! process tree as it contains a command's, without a timeout, and ends the
//! tree when the plugin stops the server, when the plugin goes off and when
//! Mannie quits.
//!
//! A server comes back as a trait object (ADR-0034, decision 3): bytes go in
//! through `send`, and its output and the lines of its error output come out
//! as streams. Dropping a server ends its tree too.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::contracts::Secret;
use crate::service::Service;
use crate::{BoxFuture, BoxStream};

/// The most servers that run at once for one plugin.
pub const MAX_SERVERS_PER_PLUGIN: u32 = 32;

/// The most arguments of a server's program.
pub const MAX_SERVER_ARGUMENTS: usize = 256;

/// The most bytes of a server's arguments together.
pub const MAX_SERVER_ARGUMENT_BYTES: usize = 64 * 1024;

/// The most variables that a plugin gives a server, its secrets among them.
pub const MAX_SERVER_VARIABLES: usize = 64;

/// The most bytes of one message to a server's input.
pub const MAX_SERVER_SEND_BYTES: usize = 16 * 1024 * 1024;

/// The most bytes of a line of a server's error output. A longer line is
/// cut, and its rest is dropped.
pub const MAX_SERVER_ERROR_LINE_BYTES: usize = 4096;

/// The longest that a stop waits for a server to end on its own after its
/// input closed, before it ends the server's tree.
pub const MAX_SERVER_STOP_GRACE: Duration = Duration::from_secs(10);

/// The service `local-servers`, major 1. A server runs a program of the
/// owner's choice with the owner's rights, so the service is sensitive.
#[derive(Debug)]
pub enum LocalServers {}

impl Service for LocalServers {
    const NAME: &'static str = "local-servers";
    const MAJOR: u16 = 1;
    const SENSITIVE: bool = true;
    type Api = dyn LocalServersApi;
}

/// The servers of a plugin.
pub trait LocalServersApi: Send + Sync + fmt::Debug {
    /// Where the core would find a program that the owner named, for the
    /// plugin's settings to show before the server starts.
    fn locate(&self, program: &str)
    -> BoxFuture<'static, Result<LocatedProgram, LocalServerError>>;

    /// Starts a server, and returns it once its program runs.
    fn start(
        &self,
        spec: LocalServerSpec,
    ) -> BoxFuture<'static, Result<Box<dyn LocalServer>, LocalServerError>>;
}

/// A program as the core found it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct LocatedProgram {
    /// The program's canonical path, with its links resolved.
    pub path: PathBuf,
    /// Whether the program lies inside one of the owner's projects, where a
    /// model's changes could replace it (T-63). The settings warn of it.
    pub in_project: bool,
}

impl LocatedProgram {
    /// A program that the core found. Only the core makes one.
    #[cfg(feature = "host")]
    pub fn new(path: PathBuf, in_project: bool) -> Self {
        Self { path, in_project }
    }
}

/// A server to start.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LocalServerSpec {
    /// The program, as the owner named it: an absolute path, or a name that
    /// the core finds on Mannie's `PATH`, such as `npx`.
    pub program: String,
    /// The program's arguments, exactly as it receives them.
    pub arguments: Vec<String>,
    /// The absolute folder that the program runs in, which must exist, or
    /// `None` for the owner's home folder.
    pub working_directory: Option<PathBuf>,
    /// Variables that the server receives besides the environment of
    /// commands, which replace a variable of the same name. A name that
    /// loads libraries into programs, or one of the environment of every
    /// command, is refused.
    pub variables: Vec<ServerVariable>,
    /// The server's secrets, as variables of its environment, with the same
    /// rules for their names. They reach no other process.
    pub secrets: Vec<ServerSecret>,
}

impl LocalServerSpec {
    /// A server that runs `program` with `arguments` in the owner's home
    /// folder, with no variables of its own.
    pub fn new(program: impl Into<String>, arguments: Vec<String>) -> Self {
        Self {
            program: program.into(),
            arguments,
            working_directory: None,
            variables: Vec::new(),
            secrets: Vec::new(),
        }
    }

    /// The same server, run in this folder.
    #[must_use]
    pub fn in_folder(mut self, folder: impl Into<PathBuf>) -> Self {
        self.working_directory = Some(folder.into());
        self
    }

    /// The same server, with a variable of its own.
    #[must_use]
    pub fn with_variable(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.variables.push(ServerVariable {
            name: name.into(),
            value: value.into(),
        });
        self
    }

    /// The same server, with a secret as a variable of its environment.
    #[must_use]
    pub fn with_secret(mut self, name: impl Into<String>, value: Secret) -> Self {
        self.secrets.push(ServerSecret {
            name: name.into(),
            value,
        });
        self
    }
}

/// A variable of a server's environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ServerVariable {
    /// Its name.
    pub name: String,
    /// Its value.
    pub value: String,
}

/// A secret of a server, as a variable of its environment. Its value never
/// serializes, so it reaches no log or response by accident.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ServerSecret {
    /// The variable's name.
    pub name: String,
    /// Its value.
    pub value: Secret,
}

/// A server that runs. Dropping it ends its tree, as `stop` with no grace
/// does.
pub trait LocalServer: Send + fmt::Debug {
    /// Writes bytes to the server's standard input, after the bytes of every
    /// earlier call, and resolves once the core took them. A server that
    /// ended takes nothing.
    fn send(&self, bytes: Vec<u8>) -> BoxFuture<'static, Result<(), LocalServerError>>;

    /// The server's standard output as it arrives, then how the server
    /// ended. The plugin takes it once; a later call gets a stream that ends
    /// at once.
    fn output(&mut self) -> BoxStream<'static, ServerOutput>;

    /// The lines of the server's error output, each cut to
    /// [`MAX_SERVER_ERROR_LINE_BYTES`], until the server ends. The plugin
    /// takes it once; a later call gets a stream that ends at once.
    fn errors(&mut self) -> BoxStream<'static, String>;

    /// Closes the server's input, waits at most `grace`, and at most
    /// [`MAX_SERVER_STOP_GRACE`], for it to end on its own, then ends its
    /// whole tree. Resolves with how it ended.
    fn stop(self: Box<Self>, grace: Duration) -> BoxFuture<'static, ServerEnding>;
}

/// What a server's output carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum ServerOutput {
    /// Bytes of the output, in order. They may end in the middle of a line
    /// or of a character.
    Bytes {
        /// The bytes, as base64 in JSON.
        #[serde(with = "base64_bytes")]
        bytes: Vec<u8>,
    },
    /// How the server ended. Nothing follows.
    Ended {
        /// The end.
        ending: ServerEnding,
    },
}

/// How a server ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum ServerEnding {
    /// The program exited with this code.
    Exited {
        /// The exit code.
        code: i32,
    },
    /// The core ended the server's tree: the plugin stopped it, the plugin
    /// went off, Mannie quit, or the program ended without an exit code.
    Ended,
}

/// Why a server did not start or take bytes. The core adds reasons by
/// addition, so a plugin treats a reason that it does not know as a failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum LocalServerError {
    /// The program is neither an absolute path nor a name that the core
    /// finds on Mannie's `PATH`, outside the owner's projects and Mannie's
    /// own folders.
    #[error("{program} was not found")]
    ProgramNotFound {
        /// The program, as it was given.
        program: String,
    },
    /// The program is a file that the core does not start: a relative path,
    /// a path on a network share, a script of another kind, or a program in
    /// Mannie's own folders.
    #[error("{program} is not a program that a server may run")]
    ProgramRefused {
        /// The program, as it was given.
        program: String,
    },
    /// The working folder is not an absolute folder that exists.
    #[error("the working folder is not a folder that exists")]
    WorkingDirectoryInvalid,
    /// An argument holds NUL, the arguments are past their limits, or on
    /// Windows an argument of a `.cmd` or `.bat` file holds a character that
    /// `cmd.exe` would read as code.
    #[error("an argument cannot be given to the program")]
    ArgumentRefused,
    /// A variable's name is not valid, loads libraries into programs or
    /// belongs to the environment of every command, its value holds NUL, or
    /// there are more than [`MAX_SERVER_VARIABLES`].
    #[error("the variable {name:?} cannot be given to the server")]
    VariableRefused {
        /// The variable's name, as it was given.
        name: String,
    },
    /// The plugin runs as many servers as it may.
    #[error("{limit} servers run already, the most for a plugin")]
    TooMany {
        /// The limit.
        limit: u32,
    },
    /// The program could not be started. The log has the cause.
    #[error("the program could not be started")]
    SpawnFailed,
    /// A message is larger than [`MAX_SERVER_SEND_BYTES`].
    #[error("the message is larger than a server takes")]
    TooLarge,
    /// The server ended, so it takes nothing.
    #[error("the server ended")]
    Ended,
    /// Mannie is quitting.
    #[error("Mannie is quitting")]
    ShuttingDown,
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
    use serde_json::json;

    use super::{LocalServerError, ServerEnding, ServerOutput};

    #[test]
    fn output_crosses_json_as_base64_and_an_ending_by_its_kind() {
        let bytes = ServerOutput::Bytes {
            bytes: b"{\"jsonrpc\":\"2.0\"}\n".to_vec(),
        };
        let ended = ServerOutput::Ended {
            ending: ServerEnding::Exited { code: 1 },
        };

        assert_eq!(
            serde_json::to_value(&bytes).unwrap(),
            json!({ "kind": "bytes", "bytes": "eyJqc29ucnBjIjoiMi4wIn0K" })
        );
        assert_eq!(
            serde_json::to_value(&ended).unwrap(),
            json!({ "kind": "ended", "ending": { "kind": "exited", "code": 1 } })
        );
        assert_eq!(
            serde_json::from_value::<ServerOutput>(serde_json::to_value(&bytes).unwrap()).unwrap(),
            bytes
        );
        assert_eq!(
            serde_json::to_value(LocalServerError::TooMany { limit: 32 }).unwrap(),
            json!({ "code": "too_many", "limit": 32 })
        );
    }
}
