//! The tool contract (docs/architecture/tools.md, the tool contract): what a
//! tool declares, how it prepares an operation without side effects, and how
//! it executes after authorization.

use std::fmt;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::Arc;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_store::Store;
use mannie_types::context::Attachment;
use mannie_types::ids::{ArchiveId, ConversationId, PartId};
use mannie_types::images::ImageBytes;
use mannie_types::tools::{Access, ProcessRecord, ToolFailure, ToolOutcome};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use tokio::sync::oneshot;

use crate::background::BackgroundCommands;
use crate::environment::CommandEnvironment;
use crate::lock::MutationLock;
use crate::projects::{Availability, ProjectFacts};
use crate::schema;
use crate::stop::StopSignal;
use crate::system::SystemFacts;
use crate::texts;

/// Whether a tool changes anything. Only tools with side effects perform
/// operations, which the journal records (ADR-0013, decision 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionClass {
    /// The tool reads and changes nothing.
    ReadOnly,
    /// The tool changes files or runs commands.
    SideEffects,
}

/// Everything a tool call needs from its surroundings.
#[derive(Clone)]
pub struct ToolContext {
    /// The conversation the call belongs to.
    pub conversation_id: ConversationId,
    /// The conversation's workspace, in canonical form.
    pub workspace: Option<PathBuf>,
    /// The store, which holds archived outputs.
    pub store: Store,
    /// The most bytes of a rendering for the model ([`crate::result_limit`]).
    pub result_limit: usize,
    /// The environment that commands receive.
    pub environment: Arc<CommandEnvironment>,
    /// The machine-wide lock that file changes take.
    pub mutations: MutationLock,
    /// Makes the ID of an archived output.
    pub archive_ids: Arc<dyn Fn() -> ArchiveId + Send + Sync>,
    /// What the tools know about the machine.
    pub system: Arc<SystemFacts>,
    /// The conversation's project, the owner's projects, and the move into
    /// one of them.
    pub projects: ProjectFacts,
    /// The machine's background commands, which the runtime keeps
    /// (ADR-0041).
    pub background: Arc<dyn BackgroundCommands>,
    /// The conversation's attachments, or those of the conversation that
    /// gave a child its task, whose keys a plugin's tool that is offered
    /// where they are reaches (ADR-0052).
    pub attachments: Arc<[Attachment]>,
}

impl fmt::Debug for ToolContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolContext")
            .field("conversation_id", &self.conversation_id)
            .field("workspace", &self.workspace)
            .field("result_limit", &self.result_limit)
            .field("projects", &self.projects)
            .finish_non_exhaustive()
    }
}

/// What preparing a call found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    /// What the operation touches, in canonical form. The policy decides on
    /// these, never on the arguments the model wrote.
    pub accesses: Vec<Access>,
}

/// Why a call could not be prepared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrepareError {
    /// The arguments are not valid for the tool, so the call never runs.
    InvalidArguments {
        /// What is wrong, as the parser said it.
        detail: String,
    },
    /// The call cannot run, for example because a path is refused or does
    /// not exist. The failure is the call's result.
    Failed(ToolFailure),
    /// The call ends at once with this result, without running, such as a
    /// plugin's refusal that carries a code for its card (ADR-0039). The
    /// result is boxed, because it is far larger than the other reasons.
    Ended(Box<Done>),
}

impl From<ToolFailure> for PrepareError {
    fn from(failure: ToolFailure) -> Self {
        Self::Failed(failure)
    }
}

impl PrepareError {
    /// Arguments that parse but break a rule of the tool, such as an empty
    /// list of edits.
    pub(crate) fn invalid(detail: &str) -> Self {
        Self::InvalidArguments {
            detail: detail.to_owned(),
        }
    }
}

/// An output to archive, under the ID that the rendering names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedOutput {
    /// The archive.
    pub id: ArchiveId,
    /// What it holds, such as `stdout`.
    pub name: String,
    /// The output.
    pub content: String,
}

/// What a tool produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    /// What the call did, for the UI.
    pub outcome: ToolOutcome,
    /// The result as the model reads it, within the result limit.
    pub output: String,
    /// The outputs that did not fit.
    pub archives: Vec<ArchivedOutput>,
    /// The files of the images that reach the model with the output, which
    /// the runtime checks and cleans before it keeps them (ADR-0044).
    pub images: Vec<ImageBytes>,
}

/// One execution of a call, after authorization.
#[derive(Debug)]
pub struct Execution {
    /// The call's surroundings.
    pub context: ToolContext,
    /// The arguments as the model wrote them.
    pub arguments: String,
    /// The accesses that were authorized, from the latest preparation.
    pub accesses: Vec<Access>,
    /// Stop: the tool ends its work as soon as it can.
    pub stop: StopSignal,
    /// Receives the process of a command once it started, so the journal can
    /// record it.
    pub process: Option<oneshot::Sender<ProcessRecord>>,
    /// The part of the call, which a command that runs in the background
    /// keeps as its start (ADR-0041).
    pub part_id: Option<PartId>,
    /// The call's activity, cleaned for display, which a command that runs
    /// in the background keeps as its name. The model wrote it, so nothing
    /// that authorizes the call reads it.
    pub activity: Option<String>,
}

/// A tool in the registry (docs/architecture/extension-points.md, tool
/// registry).
pub trait Tool: Send + Sync + fmt::Debug {
    /// The tool's name: snake case, unique, and never reused for another
    /// meaning. It borrows from the tool, so a plugin's tool can carry the
    /// name that its plugin gave it (ADR-0034, decision 6).
    fn name(&self) -> &str;

    /// What the tool does, for the model.
    fn description(&self) -> &str;

    /// The JSON Schema of the arguments.
    fn parameters(&self) -> &serde_json::Value;

    /// Whether the schema is strict, so a server that enforces strict tools
    /// may hold the model to it. The core's schemas are; a plugin's tool with
    /// a schema from elsewhere, such as a tool server's, is not (ADR-0043,
    /// decision 3).
    fn strict(&self) -> bool {
        true
    }

    /// Whether the tool changes anything.
    fn class(&self) -> PermissionClass;

    /// Whether a model call is offered the tool, which the runtime asks for
    /// every call. Most tools are always offered.
    fn offered(&self, _availability: &Availability<'_>) -> bool {
        true
    }

    /// Checks the arguments and computes the accesses, without side effects.
    fn prepare(
        &self,
        context: ToolContext,
        arguments: String,
    ) -> BoxFuture<'static, Result<Prepared, PrepareError>>;

    /// Performs the work. A failure is a result like any other.
    fn execute(&self, execution: Execution) -> BoxFuture<'static, Done>;
}

/// The checked arguments and the surroundings of a typed tool's execution.
#[derive(Debug)]
pub(crate) struct TypedExecution<I> {
    /// The call's surroundings.
    pub(crate) context: ToolContext,
    /// The checked arguments.
    pub(crate) input: I,
    /// The accesses that were authorized.
    pub(crate) accesses: Vec<Access>,
    /// Stop.
    pub(crate) stop: StopSignal,
    /// Receives a command's process once it started.
    #[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
    pub(crate) process: Option<oneshot::Sender<ProcessRecord>>,
    /// The part of the call.
    pub(crate) part_id: Option<PartId>,
    /// The call's activity.
    pub(crate) activity: Option<String>,
}

/// A tool with typed arguments, which [`Registered`] turns into a [`Tool`].
pub(crate) trait TypedTool: Send + Sync + 'static {
    /// The arguments. Unknown fields are refused, and the schema comes from
    /// the type.
    type Input: DeserializeOwned + JsonSchema + Send + Sync + 'static;

    /// The tool's name.
    const NAME: &'static str;

    /// Whether the tool changes anything.
    const CLASS: PermissionClass;

    /// What the tool does, for the model.
    fn description() -> &'static str;

    /// Whether a model call is offered the tool.
    fn offered(_availability: &Availability<'_>) -> bool {
        true
    }

    /// Computes the accesses. It may read the file system but changes
    /// nothing, and it runs on a blocking thread.
    fn prepare(context: &ToolContext, input: &Self::Input) -> Result<Vec<Access>, PrepareError>;

    /// Performs the work.
    fn execute(
        execution: TypedExecution<Self::Input>,
    ) -> BoxFuture<'static, Result<Done, ToolFailure>>;
}

/// A [`TypedTool`] as the registry holds it, with its schema generated once.
pub(crate) struct Registered<T> {
    parameters: serde_json::Value,
    tool: PhantomData<fn() -> T>,
}

impl<T: TypedTool> Registered<T> {
    /// The tool with its schema.
    pub(crate) fn new() -> Self {
        Self {
            parameters: schema::parameters::<T::Input>(),
            tool: PhantomData,
        }
    }
}

impl<T> fmt::Debug for Registered<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Registered").finish_non_exhaustive()
    }
}

/// Parses the arguments of a call. A model that calls a tool without
/// arguments may send nothing at all, or `null`, which count as an empty
/// object.
fn parse_arguments<I: DeserializeOwned>(arguments: &str) -> Result<I, String> {
    let trimmed = arguments.trim();
    let text = if trimmed.is_empty() || trimmed == "null" {
        "{}"
    } else {
        arguments
    };
    serde_json::from_str(text).map_err(|error| error.to_string())
}

impl<T: TypedTool> Tool for Registered<T> {
    fn name(&self) -> &str {
        T::NAME
    }

    fn description(&self) -> &str {
        T::description()
    }

    fn parameters(&self) -> &serde_json::Value {
        &self.parameters
    }

    fn class(&self) -> PermissionClass {
        T::CLASS
    }

    fn offered(&self, availability: &Availability<'_>) -> bool {
        T::offered(availability)
    }

    fn prepare(
        &self,
        context: ToolContext,
        arguments: String,
    ) -> BoxFuture<'static, Result<Prepared, PrepareError>> {
        async move {
            let input: T::Input = parse_arguments(&arguments)
                .map_err(|detail| PrepareError::InvalidArguments { detail })?;
            let accesses = tokio::task::spawn_blocking(move || T::prepare(&context, &input))
                .await
                .unwrap_or_else(|error| {
                    Err(PrepareError::Failed(ToolFailure::IoError {
                        kind: format!("the preparation stopped unexpectedly: {error}"),
                    }))
                })?;
            Ok(Prepared { accesses })
        }
        .boxed()
    }

    fn execute(&self, execution: Execution) -> BoxFuture<'static, Done> {
        async move {
            let input: T::Input = match parse_arguments(&execution.arguments) {
                Ok(input) => input,
                // The preparation parsed the same arguments, so they parse
                // again; should they not, the model still learns why.
                Err(detail) => return failed(ToolFailure::IoError { kind: detail }),
            };
            let typed = TypedExecution {
                context: execution.context,
                input,
                accesses: execution.accesses,
                stop: execution.stop,
                #[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
                process: execution.process,
                part_id: execution.part_id,
                activity: execution.activity,
            };
            T::execute(typed).await.unwrap_or_else(failed)
        }
        .boxed()
    }
}

/// Runs file system work on a blocking thread.
pub(crate) async fn blocking<T, F>(work: F) -> Result<T, ToolFailure>
where
    F: FnOnce() -> Result<T, ToolFailure> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|error| {
            Err(ToolFailure::IoError {
                kind: format!("the work stopped unexpectedly: {error}"),
            })
        })
}

/// The result of a call that failed.
pub(crate) fn failed(failure: ToolFailure) -> Done {
    Done::failed(failure)
}

impl Done {
    /// The result of a call that failed: its code and what the model can do
    /// about it.
    pub fn failed(failure: ToolFailure) -> Self {
        Self {
            output: texts::failure(&failure),
            outcome: ToolOutcome::Failed { failure },
            archives: Vec::new(),
            images: Vec::new(),
        }
    }
}
