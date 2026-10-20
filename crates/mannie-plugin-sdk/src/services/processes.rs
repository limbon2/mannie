//! The service `processes` (ADR-0038, decision 7): runs the installed
//! programs that a plugin's manifest declares under `[processes]`, such as
//! `git`, without a shell, under the core's process supervisor.
//!
//! The core finds a program on Mannie's `PATH`, outside the owner's projects
//! and Mannie's own folders, and runs it with the arguments that the manifest
//! fixes before the plugin's own, exactly as given. The program receives the
//! environment of commands, the names that the manifest inherits when
//! Mannie's environment has them, and the variables that the plugin sets.
//! Its input and output travel as bytes, and dropping a run's future ends its
//! whole process tree.
//!
//! A tool's run acts within the accesses that the policy allowed for the
//! call, which the plugin declared in its preparation. A run for a plugin's
//! command or mention provider may only read, and names the folders that it
//! reads, which the core checks as it checks a model's reads there before
//! anything starts (decision 10).

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::BoxFuture;
use crate::contracts::{ConversationId, ProjectId};
use crate::service::Service;

/// The most bytes of input that a run gives its program.
pub const MAX_PROGRAM_INPUT_BYTES: usize = 16 * 1024 * 1024;

/// The largest limit of a run's output, in bytes.
pub const MAX_PROGRAM_OUTPUT_BYTES: u32 = 16 * 1024 * 1024;

/// The bytes of a program's error output that a run keeps.
pub const PROGRAM_ERROR_BYTES: usize = 256 * 1024;

/// The longest that a run may last, in seconds, as a command's.
pub const MAX_PROGRAM_TIMEOUT_SECONDS: u32 = 600;

/// The timeout of a run that names none, in seconds, as a command's.
const DEFAULT_TIMEOUT_SECONDS: u32 = 60;

/// The output limit of a run that names none.
const DEFAULT_OUTPUT_LIMIT: u32 = 1024 * 1024;

/// The service `processes`, major 1. A program runs with the owner's rights,
/// so the service is sensitive.
#[derive(Debug)]
pub enum Processes {}

impl Service for Processes {
    const NAME: &'static str = "processes";
    const MAJOR: u16 = 1;
    const SENSITIVE: bool = true;
    type Api = dyn ProcessesApi;
}

/// The installed programs that the plugin's manifest declares.
pub trait ProcessesApi: Send + Sync + fmt::Debug {
    /// Where the core found the program on the `PATH`, with its links
    /// resolved, such as for the plugin to tell the model which git it runs.
    fn locate(&self, program: &str) -> BoxFuture<'static, Result<PathBuf, ProcessError>>;

    /// Runs a program and returns what it produced. Dropping the future ends
    /// the program and every process that it started.
    fn run(&self, run: ProgramRun) -> BoxFuture<'static, Result<ProgramOutput, ProcessError>>;

    /// The canonical folder of one of the owner's projects, for a command
    /// that names the project by its ID and never by a path.
    fn project_folder(
        &self,
        project: ProjectId,
    ) -> BoxFuture<'static, Result<PathBuf, ProcessError>>;
}

/// A run of an installed program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ProgramRun {
    /// The program's name, as the manifest declares it under `[processes]`.
    pub program: String,
    /// The arguments after the ones that the manifest fixes, exactly as the
    /// program receives them.
    pub arguments: Vec<String>,
    /// The absolute folder that the program runs in, which must exist.
    pub working_directory: PathBuf,
    /// The variables that the plugin sets, besides the environment of
    /// commands and the names that the manifest inherits. A name that looks
    /// like a secret's, loads libraries or belongs to the environment of
    /// every command is refused.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub environment: Vec<ProgramVariable>,
    /// The program's standard input, at most [`MAX_PROGRAM_INPUT_BYTES`]. It
    /// closes once written, and at once when it is empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<u8>,
    /// How long the run may last, at most [`MAX_PROGRAM_TIMEOUT_SECONDS`].
    pub timeout_seconds: u32,
    /// The bytes of standard output that the run keeps, at most
    /// [`MAX_PROGRAM_OUTPUT_BYTES`]. Output past it ends the run.
    pub output_limit: u32,
    /// For a run of a plugin's command or mention provider, the folders that
    /// it reads and the place whose policy decides on them. A tool's run
    /// gives none: the policy decided on its call already.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reads: Option<ReadPlace>,
}

impl ProgramRun {
    /// A run of `program` with `arguments` in `working_directory`, with no
    /// variables of its own, no input, a timeout of 60 seconds and an output
    /// limit of 1 MiB.
    pub fn new(
        program: impl Into<String>,
        working_directory: impl Into<PathBuf>,
        arguments: Vec<String>,
    ) -> Self {
        Self {
            program: program.into(),
            arguments,
            working_directory: working_directory.into(),
            environment: Vec::new(),
            input: Vec::new(),
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            output_limit: DEFAULT_OUTPUT_LIMIT,
            reads: None,
        }
    }

    /// The same run, with a variable that the plugin sets.
    #[must_use]
    pub fn with_variable(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.environment.push(ProgramVariable {
            name: name.into(),
            value: value.into(),
        });
        self
    }

    /// The same run, with the program's standard input.
    #[must_use]
    pub fn with_input(mut self, input: Vec<u8>) -> Self {
        self.input = input;
        self
    }

    /// The same run, with its timeout in seconds.
    #[must_use]
    pub fn with_timeout_seconds(mut self, timeout_seconds: u32) -> Self {
        self.timeout_seconds = timeout_seconds;
        self
    }

    /// The same run, with its output limit in bytes.
    #[must_use]
    pub fn with_output_limit(mut self, output_limit: u32) -> Self {
        self.output_limit = output_limit;
        self
    }

    /// The same run, checked as reads of `reads` before it starts.
    #[must_use]
    pub fn checked_as(mut self, reads: ReadPlace) -> Self {
        self.reads = Some(reads);
        self
    }
}

/// A variable that a plugin sets for a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ProgramVariable {
    /// The variable's name.
    pub name: String,
    /// Its value.
    pub value: String,
}

/// The folders that a run of a plugin's command or mention provider reads,
/// and the place whose policy decides on them, as on a model's reads there
/// (ADR-0038, decision 10). A read that would ask refuses the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "place",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ReadPlace {
    /// As a model's reads in this conversation: its workspace, its grants and
    /// its full access. A mention provider's runs.
    Conversation {
        /// The conversation.
        conversation: ConversationId,
        /// The absolute folders that the run reads.
        folders: Vec<PathBuf>,
    },
    /// As a model's reads in a conversation of this project: the project's
    /// folder, and the grants of the project and of every conversation. A
    /// command's runs, which the owner's view of a project makes.
    Project {
        /// The project.
        project: ProjectId,
        /// The absolute folders that the run reads.
        folders: Vec<PathBuf>,
    },
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramEnding {
    /// The program exited on its own.
    Exited,
    /// Its timeout passed, and its tree was ended.
    TimedOut,
    /// Its output went past the run's limit, and its tree was ended.
    OutputFull,
}

/// What a run produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ProgramOutput {
    /// How the run ended.
    pub ending: ProgramEnding,
    /// The program's exit code, when it exited on its own with one.
    pub exit_code: Option<i32>,
    /// How long the run lasted, in milliseconds.
    pub duration_ms: u64,
    /// The standard output, at most the run's limit.
    pub output: Vec<u8>,
    /// The bytes that the program wrote to standard output in all.
    pub output_total: u64,
    /// The error output, at most [`PROGRAM_ERROR_BYTES`].
    pub errors: Vec<u8>,
    /// The bytes that the program wrote to error output in all.
    pub errors_total: u64,
}

impl ProgramOutput {
    /// What a run produced. Only the core builds one.
    #[cfg(feature = "host")]
    pub fn new(ending: ProgramEnding, exit_code: Option<i32>, duration_ms: u64) -> Self {
        Self {
            ending,
            exit_code,
            duration_ms,
            output: Vec::new(),
            output_total: 0,
            errors: Vec::new(),
            errors_total: 0,
        }
    }

    /// The same output, with the program's standard output and its size.
    #[cfg(feature = "host")]
    #[must_use]
    pub fn with_output(mut self, output: Vec<u8>, total: u64) -> Self {
        self.output = output;
        self.output_total = total;
        self
    }

    /// The same output, with the program's error output and its size.
    #[cfg(feature = "host")]
    #[must_use]
    pub fn with_errors(mut self, errors: Vec<u8>, total: u64) -> Self {
        self.errors = errors;
        self.errors_total = total;
        self
    }

    /// Whether the program exited on its own with the exit code 0.
    pub fn succeeded(&self) -> bool {
        self.ending == ProgramEnding::Exited && self.exit_code == Some(0)
    }

    /// Whether the run kept less standard output than the program wrote.
    pub fn output_cut(&self) -> bool {
        u64::try_from(self.output.len()).unwrap_or(u64::MAX) < self.output_total
    }
}

/// Why a run did not start, or failed to. The core adds reasons by
/// addition, so a plugin treats a reason that it does not know as a failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum ProcessError {
    /// The manifest does not declare the program under `[processes]`.
    #[error("the manifest does not declare the program {program}")]
    NotDeclared {
        /// The program's name.
        program: String,
    },
    /// No entry of the `PATH` holds the program, outside the owner's
    /// projects and Mannie's own folders.
    #[error("{program} was not found on the PATH")]
    NotFound {
        /// The program's name.
        program: String,
    },
    /// The run breaks a rule: a limit, or a working directory that is not an
    /// absolute folder that exists, or one inside Mannie's own folders.
    #[error("the run is not valid: {detail}")]
    InvalidRun {
        /// What is wrong, for the log and the plugin.
        detail: String,
    },
    /// A variable of the run is refused.
    #[error("the variable {name} is refused")]
    VariableRefused {
        /// The variable's name.
        name: String,
    },
    /// The run reads a folder that the place's policy would ask the owner
    /// about, so it did not start.
    #[error("reading {path} is not allowed there")]
    ReadNotAllowed {
        /// The folder.
        path: String,
    },
    /// The project does not exist.
    #[error("the project does not exist")]
    ProjectNotFound,
    /// The program could not be started.
    #[error("the program could not be started: {detail}")]
    StartFailed {
        /// What the operating system reported.
        detail: String,
    },
}
