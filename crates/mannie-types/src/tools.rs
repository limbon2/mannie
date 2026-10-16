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
mod call;
mod commands;
mod outcome;
mod settings;

pub use access::{Access, AccessMode, EntryCount};
pub use approval::{
    ApprovalChoice, ApprovalReason, ApprovalRequest, DecideApproval, Decision, GrantScope,
    RuleProposal,
};
pub use call::{
    MAX_ARGUMENTS_BYTES, MAX_VIEW_ARGUMENTS_BYTES, NotRunReason, ProcessRecord, Refusal,
    ToolCallDocument, ToolCallStatus, ToolCallView,
};
pub use commands::{
    CommandRule, ConversationGrant, GrantRef, MAX_OUTPUT_PAGE_LINES, ReadToolOutput, RuleRef,
    SetFullAccess, SharedGrant, SharedGrantRef, ToolOutputPage, ToolsError,
};
pub use outcome::{
    ArchiveRef, CommandEnding, EntryKind, PathRefusal, ToolFailure, ToolOutcome, ToolResult,
};
pub use settings::{EnvName, MAX_ENVIRONMENT_NAMES, ToolsSettings, UpdateToolsSettings};
