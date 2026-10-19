//! Mannie's system tools (docs/features/system-tools/README.md): the tool
//! contract and registry, the file and command tools, the policy that
//! authorizes their operations, and the file safety and process containment
//! that they rely on.
//!
//! A tool prepares an operation first. Preparing has no side effects: it
//! checks the arguments, resolves paths and computes the operation's
//! accesses, on which the policy decides ([`decide`]). The runtime drives the
//! lifecycle around it: it stores each step in the tool call's part, asks the
//! owner when the policy says so, prepares again right before the execution,
//! and executes (ADR-0013).
//!
//! Only the platform modules of the process supervisor use `unsafe`
//! (ADR-0014).

mod activity;
mod ceiling;
mod digest;
mod environment;
mod identity;
mod lock;
mod mentions;
mod mutate;
mod output;
mod paths;
mod policy;
mod projects;
mod registry;
mod results;
mod rules;
mod schema;
mod stop;
mod supervisor;
mod system;
mod text;
mod texts;
mod tool;
mod tools;

pub use activity::{
    ACTIVITY_ARGUMENT, MAX_ACTIVITY_CHARS, SplitArguments, clean_label, split_activity,
    with_activity,
};
pub use ceiling::Ceiling;
pub use digest::digest;
pub use environment::CommandEnvironment;
pub use lock::MutationLock;
pub use mentions::{
    FileMention, FolderMention, IndexMatch, IndexMatches, MAX_FOLDER_ENTRIES, MAX_INDEX_ENTRIES,
    MentionIndex, MentionPage, MentionPathError, MentionReadFailure, MentionTarget, PageShows,
    may_hold_secrets, resolve_mention,
};
pub use output::output_page;
pub use paths::{FolderError, canonical_folder, workspace_folder};
pub use policy::{Grants, PolicyInputs, Verdict, decide, saved_rule};
pub use projects::{Availability, MoveFailure, ProjectFacts, ProjectMoves};
pub use registry::{RegisterError, ToolRegistry, takes_activity};
pub use results::{MIN_RESULT_LIMIT_BYTES, RESULT_LIMIT_BYTES, result_limit};
pub use rules::command_words;
pub use stop::{StopReason, StopSignal};
pub use supervisor::end_leftover_command;
pub use system::{ShellFacts, SystemFacts};
pub use tool::{
    ArchivedOutput, Done, Execution, PermissionClass, PrepareError, Prepared, Tool, ToolContext,
};
pub use tools::stopped;
