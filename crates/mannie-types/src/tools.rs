//! The system tools feature's contracts (docs/features/system-tools/): the
//! document of a tool call part and its view, the accesses an operation
//! needs, approval requests and decisions, results and their failure codes,
//! the `tools` settings section, and the arguments, results and errors of the
//! feature's commands.
//!
//! A tool call is one part of the transcript. Its document is the journal
//! record of the operation it performs (ADR-0013): each change of its status
//! is stored in the same transaction as the event that publishes it.

mod access;
mod approval;
mod background;
mod call;
mod commands;
mod external;
mod outcome;
mod program;
mod settings;

pub use access::{Access, AccessMode, EntryCount};
pub use approval::{
    ApprovalChoice, ApprovalReason, ApprovalRequest, DecideApproval, Decision, GrantScope,
    RemoteTarget, RuleProposal,
};
pub use background::{
    BackgroundCommandDocument, BackgroundCommandView, BackgroundEnding, BackgroundLimit,
    BackgroundSource, CommandEndView, CommandNoteView, MAX_COMMANDS_PER_CONVERSATION,
    MAX_COMMANDS_PER_MACHINE, MAX_VIEW_COMMAND_CHARS,
};
pub use call::{
    Interruption, MAX_ARGUMENTS_BYTES, MAX_VIEW_ARGUMENTS_BYTES, NotRunReason, ProcessRecord,
    Refusal, ToolCallDocument, ToolCallStatus, ToolCallView,
};
pub use commands::{
    CommandOutputUpdate, CommandRule, ConversationGrant, GrantRef, MAX_OUTPUT_PAGE_LINES,
    MAX_OUTPUT_VIEW_LINES, ProgramRule, ProgramRuleRef, ReadToolImage, ReadToolOutput,
    RemoteCommandRule, RemoteRuleRef, RuleRef, SetFullAccess, SharedGrant, SharedGrantRef,
    StopBackgroundCommand, SubscribeCommandOutput, ToolOutputPage, ToolsError,
};
pub use external::{ToolHint, ToolServerEndpoint};
pub use outcome::{
    ArchiveRef, CommandEnding, EntryKind, MAX_TOOL_IMAGE_BYTES, MAX_TOOL_IMAGE_SIDE,
    MAX_TOOL_IMAGES, PathRefusal, ToolFailure, ToolImageRef, ToolOutcome, ToolResult,
};
pub use program::{
    DiscardedWork, MAX_EFFECT_ITEMS, ProgramEffect, ProgramEffectKey, ProgramTarget, effect_keys,
};
pub use settings::{
    EnvName, LOADER_NAME_PREFIXES, MAX_ENVIRONMENT_NAMES, SECRET_NAME_PARTS, ToolsSettings,
    UpdateToolsSettings, is_loader_name, looks_like_secret,
};
