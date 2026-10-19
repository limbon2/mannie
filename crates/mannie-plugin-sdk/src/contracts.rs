//! The contract items of `mannie-types` that plugins use (types, limits and
//! helpers), re-exported one by one (ADR-0034, decision 1). A plugin names
//! them through this module and cannot tell whether the SDK or
//! `mannie-types` defines them, so an item can move into the SDK later
//! without a change in any plugin. Each re-export has this one path.

// The most that a host's tools may do, which a host reports and its tasks
// run under (ADR-0028).
pub use mannie_types::ceiling::CeilingMode;
// The clock of every timestamp, which a plugin's start receives, and the
// time that it tells.
pub use mannie_types::clock::{Clock, UtcMillis};
// The reads of a task conversation that an executor serves (ADR-0031,
// decision 4): their arguments, their results and their errors, which reach
// the UI as codes (decision 6).
pub use mannie_types::chat::{ChatError, CheckpointRef, EarlierTurns, LoadEarlierTurns};
// The IDs of the conversations, profiles, projects and tasks that a context,
// a task or a lend names, of the workers that a task's target names
// (ADR-0031), and of the archives that a tool's execution reserves
// (ADR-0036).
pub use mannie_types::ids::{ArchiveId, ConversationId, ProfileId, ProjectId, TaskId, WorkerId};
// The envelopes of IPC (ADR-0007): an error as a command returns it, and the
// batches of a channel, which carry a plugin's channel messages (ADR-0034,
// decisions 8 and 9).
pub use mannie_types::ipc::{ChannelBatch, CommandError};
// Images (ADR-0033): the formats and limits of the images that a source
// gives, and the name that the core keeps.
pub use mannie_types::images::{
    ImageBytes, ImageFormat, MAX_IMAGE_BYTES, MAX_IMAGE_SIDE, MAX_THUMBNAIL_BYTES,
    MAX_THUMBNAIL_SIDE, clean_image_name,
};
// A short text that a model wrote for the owner, such as a task's title, as
// the owner reads it, cleaned as the core cleans its own.
pub use mannie_types::labels::clean_label;
// Mentions (ADR-0032): an entry of the "@" list, and the summary of what the
// model got of a mention, which the UI words.
pub use mannie_types::mentions::{MentionEntry, MentionSummary};
// A plugin's work in progress, which turning it off lets go, as its
// instance reports it (ADR-0034, decision 4).
pub use mannie_types::plugins::WorkItem;
// A secret value, such as a provider credential that the core lends, which
// never shows when formatted and never serializes (security architecture,
// rules for handling secrets).
pub use mannie_types::secret::Secret;
// Task executors (ADR-0031, decision 4): a task's target, the profile that
// comes with it, the assignment that an executor hands on with its limits,
// the states that the executor publishes, with a call that waits for the
// owner, and the texts of the tools that start tasks, which an executor's own
// tools share with the core's.
pub use mannie_types::tasks::{
    AssignedTaskState, ChildApproval, MAX_INSTRUCTION_BYTES, MAX_TITLE_CHARS, ProfileSnapshot,
    TASK_TITLE_ARGUMENT, TaskAssignment, TaskExecutorRef, TaskState, task_still_running,
};
// The owner's decisions and the archived outputs of a task conversation that
// an executor serves, and their errors.
pub use mannie_types::tools::{
    ApprovalChoice, DecideApproval, ReadToolOutput, ToolOutputPage, ToolsError,
};
// What a task conversation that an executor serves shows: its checkpoints and
// the updates of a subscription, and why the core ends a subscription, which
// also ends a plugin's channel (ADR-0034, decision 8), with the ID that the
// UI ends one with.
pub use mannie_types::transcript::{
    Checkpoint, ConversationUpdate, SubscriptionEnd, SubscriptionId,
};
