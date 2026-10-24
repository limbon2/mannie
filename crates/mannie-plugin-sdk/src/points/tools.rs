//! Tools (docs/architecture/extension-points.md, tool registry): a plugin's
//! tools, which the model calls, each offered to the agents that its
//! selector names, so a plugin does not depend on the core's agent names
//! (ADR-0034, decision 6).
//!
//! The core runs a plugin's tool through its own tool loop (ADR-0013): it
//! prepares the call, which checks the arguments and returns what the call
//! touches, the policy decides on that, the call is journaled, and only then
//! does the tool execute.
//!
//! A tool declares its effect ([`ToolEffect`]). A tool that reads only its
//! own plugin's data returns no access, and the owner is never asked about
//! it. A tool that reads the owner's files returns its reads, which the
//! policy decides as it decides `read_file`'s (ADR-0038). A tool with side
//! effects returns the accesses on which the policy decides, and each of its
//! calls is an operation with a journal record: a command on a remote host
//! (ADR-0036), a path that it reads or changes, or a run of an installed
//! program with an effect beyond the workspace (ADR-0038). Any other access
//! needs an SDK change with an ADR that maps it onto the core's accesses and
//! policy.
//!
//! A call learns the conversation's workspace, and a tool says where it is
//! offered ([`ToolOffering`]). A result may carry data and a failure code for
//! the call's card in the transcript, which the model never reads
//! (ADR-0039).
//!
//! A tool's name follows the rule `Global`: it is unique among the core's
//! tools, the task tools and every plugin's tools (decision 6).

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::BoxFuture;
use crate::contracts::{
    AccessMode, ArchiveId, ConversationId, ImageBytes, ProgramEffect, ToolHint, ToolServerEndpoint,
};
use crate::contributions::{ExtensionPoint, IdRule};
use crate::services::background_commands::BackgroundTicket;

/// The point of plugins' tools, whose entries are [`PluginTool`]s named by
/// their tools' names.
#[derive(Debug)]
pub enum Tools {}

impl ExtensionPoint for Tools {
    const NAME: &'static str = "tools";
    const MAJOR: u16 = 1;
    const IDS: IdRule = IdRule::Global;
    type Entry = PluginTool;

    fn id(entry: &PluginTool) -> &str {
        &entry.spec.name
    }
}

/// A tool that a plugin adds while it is on.
///
/// Both futures own their data, and the core cancels a call by dropping its
/// future (decision 3), as it does when the owner stops the reply.
pub trait Tool: Send + Sync + fmt::Debug + 'static {
    /// What the model is told of the tool. The core reads it once, when the
    /// plugin builds its [`PluginTool`].
    fn spec(&self) -> ToolSpec;

    /// Checks a call's arguments without side effects and returns what the
    /// call touches. A refusal says what is wrong with the arguments, the
    /// model reads it, and the call never runs.
    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>>;

    /// Performs a call after the core authorized it. A failure is a result
    /// like any other, whose output tells the model what went wrong.
    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult>;
}

/// What the model is told of a tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ToolSpec {
    /// The tool's name: lower-case snake case of at most 64 characters,
    /// unique among the core's and every plugin's tools, and never reused for
    /// another meaning.
    pub name: String,
    /// What the tool does, for the model.
    pub description: String,
    /// The JSON Schema of the arguments. It has no argument named
    /// `activity`, which the core adds to every call for the owner.
    pub parameters: serde_json::Value,
    /// Whether the schema is strict: every property required and no other
    /// allowed, so a server that enforces strict tools may hold the model to
    /// it. A schema that the plugin did not write, such as a tool server's,
    /// is not, and goes as it is (ADR-0043, decision 3).
    #[serde(default = "strict_by_default", skip_serializing_if = "is_strict")]
    pub strict: bool,
}

impl ToolSpec {
    /// The spec of a tool, with the JSON Schema of its arguments, which the
    /// plugin made strict.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
            strict: true,
        }
    }

    /// The same spec, whose schema is not strict.
    #[must_use]
    pub fn with_loose_schema(mut self) -> Self {
        self.strict = false;
        self
    }
}

/// The most characters of a plugin tool's name. The providers' interfaces
/// accept names of lower-case letters, digits and underscores up to this
/// length.
pub const MAX_TOOL_NAME_CHARS: usize = 64;

/// Whether `name` can name a plugin's tool: a lower-case letter, then
/// lower-case letters, digits and underscores, up to
/// [`MAX_TOOL_NAME_CHARS`] characters.
pub fn is_tool_name(name: &str) -> bool {
    let mut characters = name.chars();
    let starts_with_letter = characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase());
    starts_with_letter
        && name.len() <= MAX_TOOL_NAME_CHARS
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}

const fn strict_by_default() -> bool {
    true
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes the field by reference"
)]
const fn is_strict(strict: &bool) -> bool {
    *strict
}

/// What a call of a plugin's tool touches, on which the core's policy
/// decides.
///
/// SDK 1 began without accesses (decision 6). ADR-0036 adds the first, a
/// command on a remote host, ADR-0038 a path and a run of an installed
/// program, and ADR-0043 a call of a tool server's tool; an access to other
/// remote services comes with the SDK change that maps it onto the core's
/// accesses. The enum stays exhaustive, because plugins produce its values
/// and the core matches them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolAccess {
    /// A command on a remote host that the plugin reaches itself, such as a
    /// host that the owner registered with the SSH plugin (ADR-0036).
    RemoteCommand(RemoteCommandAccess),
    /// A path that the call reads or changes on the owner's machine, such as
    /// a repository's folder (ADR-0038, decision 4).
    Path(PathAccess),
    /// A run of an installed program with an effect beyond the workspace,
    /// such as git running a repository's hooks (ADR-0038, decision 5).
    Program(ProgramAccess),
    /// A call of a tool that a tool server offers, such as a tool of an MCP
    /// server that the owner registered with the plugin (ADR-0043,
    /// decision 4).
    ExternalTool(ExternalToolAccess),
}

/// A call of a tool server's tool, as a plugin's tool prepares it. The core
/// adds the plugin, so a plugin never speaks for another, and decides on
/// every field: a grant covers the same tool of the same server at the same
/// endpoint with any arguments, and a server whose endpoint changed between
/// the approval and the execution sends the call back to the policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ExternalToolAccess {
    /// The plugin's key for the server, which stays while the server exists
    /// and is never reused for another.
    pub target: String,
    /// The server's name, as the owner gave it.
    pub label: String,
    /// Where the server runs, without any credential.
    pub endpoint: ToolServerEndpoint,
    /// The tool, as the server names it, or `read_resource` for a read of
    /// one of its resources.
    pub tool: String,
    /// The tool's title, as the server gives it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The call's arguments, as JSON with sorted keys, or the URI of the
    /// resource that a read reads.
    pub arguments: String,
    /// What the server says of the tool, which the owner sees and nothing
    /// decides on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<ToolHint>,
}

impl ExternalToolAccess {
    /// A call of `tool` with `arguments` on a server. The five values are
    /// easy to swap, so callers pass named locals.
    pub fn new(
        target: String,
        label: String,
        endpoint: ToolServerEndpoint,
        tool: String,
        arguments: String,
    ) -> Self {
        Self {
            target,
            label,
            endpoint,
            tool,
            title: None,
            arguments,
            hints: Vec::new(),
        }
    }

    /// The same call, with the tool's title.
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// The same call, with what the server says of the tool.
    #[must_use]
    pub fn with_hints(mut self, hints: Vec<ToolHint>) -> Self {
        self.hints = hints;
        self
    }
}

/// A run of an installed program that the plugin's manifest declares under
/// `[processes]`, with an effect beyond the workspace that the owner decides
/// on (ADR-0038, decision 5). A call returns it beside the path accesses of
/// the place that it changes, when one of the effects applies.
///
/// The core adds the plugin, the program's path and the arguments that the
/// manifest fixes, resolves the working directory to its canonical place and
/// decides on every field: an approval covers only this run, a grant or a
/// rule covers only the same program with no other effect, and a changed
/// state sends the call back to the policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ProgramAccess {
    /// The program's name under `[processes]` in the plugin's manifest, such
    /// as `git`.
    pub name: String,
    /// The plugin's arguments, exactly as the program receives them after
    /// the ones that the manifest fixes.
    pub arguments: Vec<String>,
    /// The absolute folder that the program runs in. A call's authorized
    /// accesses name its canonical form.
    pub working_directory: PathBuf,
    /// What the run does beyond the workspace, each shown to the owner.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<ProgramEffect>,
    /// What the plugin read of the place when it prepared, such as a
    /// repository's HEAD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

impl ProgramAccess {
    /// A run of the program `name` with `arguments` in `working_directory`,
    /// without an effect or a state.
    pub fn new(
        name: impl Into<String>,
        working_directory: impl Into<PathBuf>,
        arguments: Vec<String>,
    ) -> Self {
        Self {
            name: name.into(),
            arguments,
            working_directory: working_directory.into(),
            effects: Vec::new(),
            state: None,
        }
    }

    /// The same run, with one more effect.
    #[must_use]
    pub fn with_effect(mut self, effect: ProgramEffect) -> Self {
        self.effects.push(effect);
        self
    }

    /// The same run, with the state that the plugin read of the place.
    #[must_use]
    pub fn with_state(mut self, state: impl Into<String>) -> Self {
        self.state = Some(state.into());
        self
    }
}

/// A path that a call of a plugin's tool reads or changes. The core resolves
/// it to its canonical place, following links, and decides on it as it
/// decides on the paths of its own tools: reads inside the workspace are
/// allowed, and changes inside it follow the owner's setting. A call's
/// authorized accesses name the canonical path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct PathAccess {
    /// The absolute path. A path that could reach a network host, such as a
    /// UNC path, is refused before any access to the file system.
    pub path: PathBuf,
    /// How the call uses it.
    pub mode: AccessMode,
    /// What the plugin read of the place when it prepared, such as a
    /// repository's HEAD. The core shows nothing of it, and an approval
    /// covers only the same state, so a tool that finds another state when
    /// it executes refuses to act.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

impl PathAccess {
    /// A path that a call uses as `mode` says, without a state.
    pub fn new(path: impl Into<PathBuf>, mode: AccessMode) -> Self {
        Self {
            path: path.into(),
            mode,
            state: None,
        }
    }

    /// The same access, with the state that the plugin read of the place.
    #[must_use]
    pub fn with_state(mut self, state: impl Into<String>) -> Self {
        self.state = Some(state.into());
        self
    }
}

/// A command on a remote host, as a plugin's tool prepares it. The core adds
/// the plugin, so a plugin never speaks for another, and decides on every
/// field: an approval, a grant or a rule never covers another host, endpoint
/// or command, and a changed identity sends the call back to the policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct RemoteCommandAccess {
    /// The plugin's key for the host, which stays while the host exists and
    /// is never reused for another.
    pub target: String,
    /// The host's name, as the owner gave it.
    pub label: String,
    /// Where the command runs, such as `deploy@build-box.lan:22`.
    pub endpoint: String,
    /// What identifies the host beyond its address, such as the fingerprint
    /// of its pinned key.
    pub identity: String,
    /// The command's text, exactly as the host receives it.
    pub command: String,
    /// Whether the command runs in the background, past its call, until it
    /// ends or is stopped (ADR-0041). A grant or a rule from the foreground
    /// never covers it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub background: bool,
}

impl RemoteCommandAccess {
    /// A command on a remote host. The five texts are easy to swap, so
    /// callers pass named locals.
    pub fn new(
        target: String,
        label: String,
        endpoint: String,
        identity: String,
        command: String,
    ) -> Self {
        Self {
            target,
            label,
            endpoint,
            identity,
            command,
            background: false,
        }
    }

    /// The same command, run in the background: its call hands it to the
    /// core through the service `background-commands` (ADR-0041,
    /// decision 11).
    #[must_use]
    pub fn in_background(mut self) -> Self {
        self.background = true;
        self
    }
}

/// What a plugin's tool does beyond reading its plugin's own data, which
/// decides how the core runs its calls (ADR-0036). The enum stays
/// exhaustive, because plugins choose its values and the core matches them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolEffect {
    /// It reads only its own plugin's data. Its preparation returns no
    /// access, the owner is never asked, and its calls are not journaled.
    #[default]
    ReadsOwnData,
    /// It reads the owner's files. Its preparation returns 1 to 16 path
    /// accesses that only read, which the policy decides as it decides the
    /// reads of `read_file`; its calls are not journaled, and the explore
    /// agent may be offered it (ADR-0038, decision 3).
    ReadsFiles,
    /// It acts outside Mannie, such as on a remote host. Its preparation
    /// returns at least one access, the policy decides on them, and each call
    /// is an operation whose executing status is journaled before it starts
    /// (ADR-0013). The core never offers such a tool to the explore agent,
    /// which only reads.
    SideEffects,
}

/// Where a plugin's tool is offered, which the core decides for each model
/// call from the conversation, without running the plugin's code (ADR-0038,
/// decision 2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOffering {
    /// In every conversation.
    #[default]
    Always,
    /// Only in a conversation with a workspace, a project's folder.
    InWorkspace,
    /// Only where the workspace, or a folder above it, holds an entry of
    /// this name, such as `.git` for a repository. The name is one entry,
    /// without a separator.
    WhereWorkspaceHolds(String),
}

/// The agents whose runs are offered a plugin's tool, when their profiles
/// use tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSelector {
    /// The agents that answer to the owner: those of the owner's
    /// conversations.
    Conversations,
    /// The agents that answer to another agent: child agents, the core's and
    /// the plugins'.
    ChildAgents,
    /// Every agent.
    Every,
    /// The agents with these names, the core's or the plugins'.
    Agents(Vec<String>),
}

/// A tool of a plugin, with the agents that it is offered to.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PluginTool {
    /// The tool.
    pub tool: Arc<dyn Tool>,
    /// The agents whose runs are offered the tool.
    pub offered_to: AgentSelector,
    /// What the model is told of the tool, as the tool gave it when the
    /// plugin built this entry.
    pub spec: ToolSpec,
    /// What the tool does beyond reading its plugin's own data.
    pub effect: ToolEffect,
    /// Where the tool is offered.
    pub offering: ToolOffering,
}

impl PluginTool {
    /// A tool for the agents that `offered_to` names, which reads only its
    /// plugin's own data and is offered in every conversation. Reads the
    /// tool's spec once, so the model is told the same of the tool while the
    /// plugin is on.
    pub fn new(tool: Arc<dyn Tool>, offered_to: AgentSelector) -> Self {
        let spec = tool.spec();
        Self {
            tool,
            offered_to,
            spec,
            effect: ToolEffect::ReadsOwnData,
            offering: ToolOffering::Always,
        }
    }

    /// The same tool, with side effects: its calls are operations that the
    /// policy decides on and the journal records (ADR-0036).
    #[must_use]
    pub fn with_side_effects(mut self) -> Self {
        self.effect = ToolEffect::SideEffects;
        self
    }

    /// The same tool, reading the owner's files: its calls return their
    /// reads, which the policy decides (ADR-0038).
    #[must_use]
    pub fn with_file_reads(mut self) -> Self {
        self.effect = ToolEffect::ReadsFiles;
        self
    }

    /// The same tool, offered only where `offering` says.
    #[must_use]
    pub fn offered_where(mut self, offering: ToolOffering) -> Self {
        self.offering = offering;
        self
    }
}

/// A call of a plugin's tool, as the core prepares it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ToolCall {
    /// The arguments as the model wrote them, as JSON text, without the
    /// call's activity. A model that calls a tool without arguments may send
    /// an empty text or `null`.
    pub arguments: String,
    /// The conversation of the agent that made the call.
    pub conversation_id: ConversationId,
    /// The conversation's workspace, the canonical folder of its project, or
    /// none for a general conversation (ADR-0038, decision 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathBuf>,
}

impl ToolCall {
    /// A call with its arguments, without a workspace. Only the core builds
    /// one.
    #[cfg(feature = "host")]
    pub fn new(arguments: String, conversation_id: ConversationId) -> Self {
        Self {
            arguments,
            conversation_id,
            workspace: None,
        }
    }

    /// The same call, in the conversation's workspace.
    #[cfg(feature = "host")]
    #[must_use]
    pub fn with_workspace(mut self, workspace: Option<PathBuf>) -> Self {
        self.workspace = workspace;
        self
    }
}

/// The most archives that one result of a plugin's tool may carry.
pub const MAX_TOOL_ARCHIVES: usize = 4;

/// The most bytes of one archive of a plugin's tool.
pub const MAX_TOOL_ARCHIVE_BYTES: usize = 2 * 1024 * 1024;

/// The most bytes of a result's data for its card, as JSON text.
pub const MAX_TOOL_DATA_BYTES: usize = 64 * 1024;

/// The most characters of a result's or a refusal's failure code.
pub const MAX_FAILURE_CODE_CHARS: usize = 64;

/// The result limit of an execution that names none: the core's largest.
const DEFAULT_RESULT_LIMIT: u32 = 64 * 1024;

const fn default_result_limit() -> u32 {
    DEFAULT_RESULT_LIMIT
}

/// A call of a plugin's tool that the core authorized and now executes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ToolExecution {
    /// The arguments as the model wrote them, as JSON text, without the
    /// call's activity: the same arguments that the call was prepared with.
    pub arguments: String,
    /// The conversation of the agent that made the call.
    pub conversation_id: ConversationId,
    /// The accesses that the policy or the owner allowed, as the tool's
    /// last preparation returned them. A tool with side effects acts on
    /// exactly these and refuses to act when what they name has changed
    /// since, such as a host whose address changed (ADR-0036).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authorized: Vec<ToolAccess>,
    /// The most bytes of output that the model reads of this call. The core
    /// cuts a longer output, so a tool that shows the start and the end of
    /// what it captured fits them in this room itself.
    #[serde(default = "default_result_limit")]
    pub result_limit: u32,
    /// The IDs that the core reserved for this call's archives: a tool gives
    /// an output that does not fit one of them, names it in its output, and
    /// the model reads it with `read_tool_output`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archive_ids: Vec<ArchiveId>,
    /// The conversation's workspace, as the call's preparation had it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathBuf>,
    /// The ticket that lets the call hand the core one command that it
    /// starts on a remote host in the background, when the authorized
    /// accesses hold one (ADR-0041, decision 11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<BackgroundTicket>,
}

impl ToolExecution {
    /// An execution of a call with its arguments, with nothing authorized,
    /// the largest result limit, no archives and no workspace. Only the core
    /// builds one.
    #[cfg(feature = "host")]
    pub fn new(arguments: String, conversation_id: ConversationId) -> Self {
        Self {
            arguments,
            conversation_id,
            authorized: Vec::new(),
            result_limit: DEFAULT_RESULT_LIMIT,
            archive_ids: Vec::new(),
            workspace: None,
            background: None,
        }
    }

    /// The same execution, in the conversation's workspace.
    #[cfg(feature = "host")]
    #[must_use]
    pub fn with_workspace(mut self, workspace: Option<PathBuf>) -> Self {
        self.workspace = workspace;
        self
    }

    /// The same execution, with the accesses that were authorized.
    #[cfg(feature = "host")]
    #[must_use]
    pub fn with_authorized(mut self, authorized: Vec<ToolAccess>) -> Self {
        self.authorized = authorized;
        self
    }

    /// The same execution, with the run's result limit.
    #[cfg(feature = "host")]
    #[must_use]
    pub fn with_result_limit(mut self, result_limit: u32) -> Self {
        self.result_limit = result_limit;
        self
    }

    /// The same execution, with the IDs that the core reserved for its
    /// archives.
    #[cfg(feature = "host")]
    #[must_use]
    pub fn with_archive_ids(mut self, archive_ids: Vec<ArchiveId>) -> Self {
        self.archive_ids = archive_ids;
        self
    }

    /// The same execution, with the ticket of a command in the background.
    #[cfg(feature = "host")]
    #[must_use]
    pub fn with_background(mut self, ticket: BackgroundTicket) -> Self {
        self.background = Some(ticket);
        self
    }
}

/// Why a plugin's tool refuses a call's arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ToolRefusal {
    /// What is wrong with the arguments, for the model.
    pub detail: String,
    /// The plugin's code of the refusal, such as `not_a_repository`, which
    /// the call's card words (ADR-0039). A refusal with a code ends the call
    /// as completed with that failure, and one without stays a call that did
    /// not run for its arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl ToolRefusal {
    /// A refusal that tells the model what is wrong with the arguments.
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
            code: None,
        }
    }

    /// The same refusal, with the plugin's code for the call's card: 1 to 64
    /// lower-case letters and underscores.
    #[must_use]
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }
}

/// What a call of a plugin's tool gives the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ToolResult {
    /// The result as the model reads it. The core cuts a longer output to the
    /// result limit of the run.
    pub output: String,
    /// The outputs that did not fit, at most [`MAX_TOOL_ARCHIVES`], each
    /// under one of the IDs that the execution reserved. The core stores them
    /// with the call, and the model reads them with `read_tool_output`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archives: Vec<ToolArchive>,
    /// The data of the call's card, a JSON object of at most
    /// [`MAX_TOOL_DATA_BYTES`] as text, which the plugin's card reads and the
    /// model never does (ADR-0039). Data that breaks these rules is dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    /// The plugin's code of how the call failed, such as `merge_conflicts`,
    /// which the call's card words: 1 to 64 lower-case letters and
    /// underscores.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    /// The images that reach the model with the output, such as a
    /// screenshot (ADR-0044). The core keeps the first
    /// [`MAX_TOOL_IMAGES`](crate::contracts::MAX_TOOL_IMAGES) of them, and
    /// the output ends with a line for each one that it leaves out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ToolImage>,
}

impl ToolResult {
    /// The result of a call, as the model reads it.
    pub fn new(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            archives: Vec::new(),
            data: None,
            failure: None,
            images: Vec::new(),
        }
    }

    /// The same result, with the images that reach the model.
    #[must_use]
    pub fn with_images(mut self, images: Vec<ToolImage>) -> Self {
        self.images = images;
        self
    }

    /// The same result, with the outputs that did not fit.
    #[must_use]
    pub fn with_archives(mut self, archives: Vec<ToolArchive>) -> Self {
        self.archives = archives;
        self
    }

    /// The same result, with the data of the call's card.
    #[must_use]
    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }

    /// The same result, failed with the plugin's code.
    #[must_use]
    pub fn with_failure(mut self, code: impl Into<String>) -> Self {
        self.failure = Some(code.into());
        self
    }
}

/// An image of a call's result: the bytes of a PNG or JPEG file, which cross
/// JSON as base64 (ADR-0044). The core reads the image's format and size from
/// the file's header, removes the file's metadata, and leaves out an image
/// of another format, one past
/// [`MAX_TOOL_IMAGE_BYTES`](crate::contracts::MAX_TOOL_IMAGE_BYTES) or
/// [`MAX_TOOL_IMAGE_SIDE`](crate::contracts::MAX_TOOL_IMAGE_SIDE), and a
/// damaged one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ToolImage {
    /// The file.
    #[serde(with = "base64_bytes")]
    pub data: ImageBytes,
}

impl ToolImage {
    /// An image from the bytes of its file.
    pub fn new(data: impl Into<ImageBytes>) -> Self {
        Self { data: data.into() }
    }
}

/// Bytes as base64 text in JSON, as MCP and `data:` URLs carry images.
mod base64_bytes {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::contracts::ImageBytes;

    pub(super) fn serialize<S: Serializer>(
        bytes: &ImageBytes,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes.as_slice()))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<ImageBytes, D::Error> {
        let text = String::deserialize(deserializer)?;
        STANDARD
            .decode(text)
            .map(ImageBytes::from)
            .map_err(serde::de::Error::custom)
    }
}

/// An output of a call that did not fit its result, kept whole for the model
/// to read on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ToolArchive {
    /// One of the IDs that the execution reserved.
    pub id: ArchiveId,
    /// What it holds, such as `stdout` or `stderr`, which the transcript
    /// shows.
    pub name: String,
    /// The output, at most [`MAX_TOOL_ARCHIVE_BYTES`].
    pub content: String,
}

impl ToolArchive {
    /// An archive under a reserved ID.
    pub fn new(id: ArchiveId, name: impl Into<String>, content: String) -> Self {
        Self {
            id,
            name: name.into(),
            content,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures_util::FutureExt;
    use serde_json::json;

    use super::{
        AgentSelector, MAX_TOOL_NAME_CHARS, PluginTool, Tool, ToolAccess, ToolCall, ToolEffect,
        ToolExecution, ToolRefusal, ToolResult, ToolSpec, is_tool_name,
    };
    use crate::BoxFuture;

    /// A tool that counts how often the core asked for its spec.
    #[derive(Debug, Default)]
    struct CountedSpec {
        specs: AtomicUsize,
    }

    impl Tool for CountedSpec {
        fn spec(&self) -> ToolSpec {
            self.specs.fetch_add(1, Ordering::SeqCst);
            ToolSpec::new(
                "notes_find",
                "Finds the owner's notes.",
                json!({ "type": "object", "properties": {} }),
            )
        }

        fn prepare(
            &self,
            _call: ToolCall,
        ) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
            async { Err(ToolRefusal::new("no notes yet")) }.boxed()
        }

        fn execute(&self, _execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
            async { ToolResult::new("No notes.") }.boxed()
        }
    }

    #[test]
    fn a_plugin_tool_reads_its_spec_once_and_keeps_its_selector() {
        let tool = Arc::new(CountedSpec::default());

        let entry = PluginTool::new(
            Arc::<CountedSpec>::clone(&tool),
            AgentSelector::Agents(vec!["explore".to_owned()]),
        );
        let copy = entry.clone();

        // A copy of the entry keeps the spec and asks the tool for none.
        assert_eq!(tool.specs.load(Ordering::SeqCst), 1);
        assert_eq!(copy.spec, entry.spec);
        assert_eq!(entry.spec.name, "notes_find");
        assert_eq!(entry.spec.description, "Finds the owner's notes.");
        assert_eq!(
            entry.spec.parameters,
            json!({ "type": "object", "properties": {} })
        );
        assert_eq!(
            entry.offered_to,
            AgentSelector::Agents(vec!["explore".to_owned()])
        );
    }

    #[test]
    fn a_tool_reads_its_own_data_unless_it_says_it_has_side_effects() {
        let reads = PluginTool::new(
            Arc::new(CountedSpec::default()),
            AgentSelector::Conversations,
        );
        let acts = reads.clone().with_side_effects();

        assert_eq!(reads.effect, ToolEffect::ReadsOwnData);
        assert_eq!(acts.effect, ToolEffect::SideEffects);
        assert_eq!(acts.spec, reads.spec);
        assert_eq!(acts.offered_to, reads.offered_to);
    }

    #[test]
    fn a_spec_is_strict_unless_its_plugin_says_that_its_schema_is_loose() {
        let strict = ToolSpec::new("notes_find", "Finds notes.", json!({ "type": "object" }));
        let loose = strict.clone().with_loose_schema();

        assert!(strict.strict);
        assert!(!loose.strict);
        // A strict spec writes the JSON of SDK 1 as it was, and a spec
        // without the field reads as strict.
        assert!(
            serde_json::to_value(&strict)
                .unwrap()
                .get("strict")
                .is_none()
        );
        assert_eq!(
            serde_json::to_value(&loose).unwrap()["strict"],
            json!(false)
        );
        let read: ToolSpec = serde_json::from_value(json!({
            "name": "notes_find",
            "description": "Finds notes.",
            "parameters": { "type": "object" },
        }))
        .unwrap();
        assert_eq!(read, strict);
    }

    #[test]
    fn a_name_is_lower_case_snake_case_of_at_most_64_characters() {
        let longest = format!("n{}", "o".repeat(MAX_TOOL_NAME_CHARS - 1));
        for valid in ["notes", "notes_find", "n2", "a_b_c_", &longest] {
            assert!(is_tool_name(valid), "{valid}");
        }
        let too_long = format!("{longest}o");
        for invalid in [
            "",
            "_notes",
            "2notes",
            "Notes",
            "notes-find",
            "notes.find",
            "notes find",
            "nötes",
            &too_long,
        ] {
            assert!(!is_tool_name(invalid), "{invalid}");
        }
    }

    #[test]
    fn the_constructors_keep_each_argument_in_its_field() {
        assert_eq!(
            ToolRefusal::new("the text is empty").detail,
            "the text is empty"
        );
        assert_eq!(ToolResult::new("2 notes.").output, "2 notes.");
    }

    #[cfg(feature = "host")]
    #[test]
    fn the_core_s_constructors_keep_the_arguments_and_the_conversation() {
        use crate::contracts::ConversationId;

        let conversation = ConversationId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap();

        let call = ToolCall::new(r#"{"text":"hi"}"#.to_owned(), conversation.clone());
        let execution = ToolExecution::new(r#"{"text":"hi"}"#.to_owned(), conversation.clone());

        assert_eq!(call.arguments, r#"{"text":"hi"}"#);
        assert_eq!(call.conversation_id, conversation);
        assert_eq!(execution.arguments, r#"{"text":"hi"}"#);
        assert_eq!(execution.conversation_id, conversation);
    }
}
