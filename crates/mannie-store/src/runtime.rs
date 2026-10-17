//! The runtime's repository: conversations, turns, runs, parts, the event log,
//! checkpoints and projects (docs/features/chat/conversations.md, ADR-0012,
//! ADR-0023).
//!
//! The runtime changes a conversation through [`Store::apply`], which applies a
//! list of [`Change`]s in one transaction, appends an event with the
//! conversation's next sequence number for each change, and returns the events
//! to publish. The reads serve subscribers ([`Store::snapshot`] and
//! [`Store::events_after`]), the conversation list and the model's context.

mod apply;
mod archives;
mod images;
mod projects;
mod reads;
mod recovery;
mod rows;
mod tasks;

use mannie_types::clock::UtcMillis;
use mannie_types::ids::{
    ArchiveId, CheckpointId, ConversationId, PartId, ProfileId, ProjectId, RunId, TaskId, TurnId,
};
use mannie_types::images::{ImageBytes, ImageFormat, ImageView};
use mannie_types::mentions::MentionView;
use mannie_types::run::{RunActivity, RunFailure, RunOutcome, RunState};
use mannie_types::tasks::{TaskExecutorRef, TaskPartDocument, TaskState};
use mannie_types::tools::ToolCallDocument;
use mannie_types::transcript::{
    ContextUsage, ConversationEvent, ConversationSummary, PartKind, PartState,
};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

pub use archives::StoredArchive;
pub use images::StoredImage;
pub use projects::{AddedProject, ConversationList, ProjectDeletion};
pub use reads::{EventsAfter, KeptRun, ModelImage, ModelPart, ModelTurn, StoredCheckpoint};
pub use recovery::{Recovered, UncertainCall};
pub use tasks::{NewExecutorTask, OutboxEntry};

/// The version of the JSON documents this module writes.
const DOCUMENT_VERSION: u32 = 1;

/// A conversation to create, as the first change of its first batch.
#[derive(Debug, Clone)]
pub struct NewConversation {
    /// The conversation.
    pub id: ConversationId,
    /// Its title, from the first message.
    pub title: String,
    /// The provider profile the owner chose, or `None` for the default.
    pub profile_id: Option<ProfileId>,
    /// The project it starts in, whose folder becomes its workspace, or
    /// `None` for a general conversation.
    pub project_id: Option<ProjectId>,
    /// What makes it a task conversation, or `None` for the owner's.
    pub task: Option<TaskOrigin>,
}

/// The parent, number and agent of a task conversation
/// (docs/features/child-agents/tasks.md, task conversations).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskOrigin {
    /// The parent conversation, whose deletion deletes the task conversation.
    pub parent_id: ConversationId,
    /// The child's number among the parent's children, from 1.
    pub number: u32,
    /// The name of the child's agent definition.
    pub agent: String,
    /// The executor that serves the conversation, for the conversation of a
    /// task that a plugin's executor runs (ADR-0031, decision 6).
    pub executor: Option<TaskExecutorRef>,
}

/// A task to store with the turn that works on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTask {
    /// The task, as the agent that assigned it chose it.
    pub id: TaskId,
    /// The parent's part that assigned the task, or `None` for a task that
    /// another machine assigned.
    pub part_id: Option<PartId>,
    /// What another machine assigned, as a JSON document of the runtime's,
    /// for a task that a worker runs for the master (ADR-0028).
    pub assignment: Option<String>,
    /// Whether the task continues an earlier task of the same child.
    pub follow_up: bool,
    /// Whether the parent continues while the child works.
    pub background: bool,
}

/// How a task ended: the document that its row keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEnding {
    /// The final state.
    pub state: TaskState,
    /// The child's report, when it reported.
    #[serde(default)]
    pub report: Option<String>,
    /// Why the task failed.
    #[serde(default)]
    pub failure: Option<RunFailure>,
}

/// A task as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredTask {
    /// The task.
    pub id: TaskId,
    /// The child's conversation.
    pub conversation_id: ConversationId,
    /// The parent's part that assigned the task, or `None` for a task that
    /// another machine assigned.
    pub part_id: Option<PartId>,
    /// The turn that works on it, or `None` for a task that an executor
    /// runs.
    pub turn_id: Option<TurnId>,
    /// Whether it continues an earlier task of the same child.
    pub follow_up: bool,
    /// Whether the parent continues while the child works.
    pub background: bool,
    /// Where it is in its life.
    pub state: TaskState,
    /// How it ended, once it ended.
    pub ending: Option<TaskEnding>,
    /// When it was stored.
    pub created_at: UtcMillis,
    /// When it ended.
    pub ended_at: Option<UtcMillis>,
    /// For a task that a plugin's executor runs: the executor, the target,
    /// the version of the latest state that the executor published and its
    /// document.
    pub executor: Option<TaskExecution>,
    /// For a task that another machine assigned: what it assigned, as the
    /// runtime's JSON document.
    pub assignment: Option<String>,
    /// For a task that another machine assigned: the version of the task's
    /// latest stored state.
    pub state_version: u64,
}

/// The executor of a stored task (ADR-0031, decision 7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskExecution {
    /// The executor's kind: its plugin's ID.
    pub kind: String,
    /// The target that runs the task, as the executor names it.
    pub target: String,
    /// The version of the latest state that the executor published, which a
    /// state must exceed to change the task.
    pub version: u64,
    /// The executor's own document, which the core keeps as it is.
    pub document: Option<String>,
}

/// A child of a conversation: its task conversation and its latest task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredChild {
    /// The child's conversation.
    pub conversation_id: ConversationId,
    /// The child's number.
    pub number: u32,
    /// The name of the child's agent definition.
    pub agent: String,
    /// The title of the child's conversation: its first task's title.
    pub title: String,
    /// The executor that runs the child's tasks, or `None` for a child
    /// agent.
    pub executor: Option<TaskExecutorRef>,
    /// The child's latest task.
    pub latest: Option<StoredTask>,
}

/// A run to add.
#[derive(Debug, Clone)]
pub struct NewRun {
    /// The run.
    pub id: RunId,
    /// The name of the agent definition that runs it.
    pub agent: String,
    /// The provider profile it uses.
    pub profile_id: ProfileId,
    /// The profile and its configuration revision, which provider-private
    /// data is bound to.
    pub binding: String,
}

/// A part to add to a run.
#[derive(Debug, Clone)]
pub struct NewPart {
    /// The part.
    pub id: PartId,
    /// The turn.
    pub turn_id: TurnId,
    /// The run that writes it.
    pub run_id: RunId,
    /// The run's attempt that writes it, from 1.
    pub attempt: u32,
    /// What it is.
    pub kind: PartKind,
    /// The failure, for an error part.
    pub failure: Option<RunFailure>,
    /// The tool call, for a tool call part.
    pub tool_call: Option<ToolCallDocument>,
}

/// Something that the owner's message mentions, which the turn stores as an
/// owner part after the message (ADR-0032).
#[derive(Debug, Clone)]
pub struct NewMention {
    /// The part.
    pub part_id: PartId,
    /// What the model reads.
    pub text: String,
    /// What the owner sees.
    pub view: MentionView,
}

/// An image of the owner's message, which the turn stores as an owner part
/// after the message and its mentions, with its bytes beside it (ADR-0033).
#[derive(Debug, Clone)]
pub struct NewImage {
    /// The part.
    pub part_id: PartId,
    /// The source that gave the image, for the log and the owner.
    pub source: String,
    /// What the owner sees: the name, format, dimensions and size.
    pub view: ImageView,
    /// The SHA-256 digest of the image's bytes, in hexadecimal.
    pub digest: String,
    /// The image.
    pub data: ImageBytes,
    /// The format of the thumbnail.
    pub thumbnail_format: ImageFormat,
    /// The thumbnail.
    pub thumbnail: ImageBytes,
}

/// An output of a tool call to archive with the call's part.
#[derive(Debug, Clone)]
pub struct NewArchive {
    /// The archive.
    pub id: ArchiveId,
    /// What it holds, such as `stdout`.
    pub name: String,
    /// The output.
    pub content: String,
}

/// Provider-private data that belongs to a part, such as encrypted reasoning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderItem {
    /// The profile and configuration revision it may be replayed to.
    pub binding: String,
    /// The item, as the provider adapter wrote it: a JSON document.
    pub item: String,
}

/// A checkpoint to add.
#[derive(Debug, Clone)]
pub struct NewCheckpoint {
    /// The checkpoint.
    pub id: CheckpointId,
    /// The position of the last turn it covers.
    pub covers_through_turn: u32,
    /// The hash of the covered text.
    pub content_hash: String,
    /// The summary.
    pub summary: String,
    /// The context's size before the compaction, in tokens.
    pub tokens_before: u32,
    /// The context's expected size after it, in tokens.
    pub tokens_after: u32,
    /// What started the compaction, for the log and the owner.
    pub trigger: String,
}

/// One change to a conversation. Each change appends one event.
#[derive(Debug, Clone)]
pub enum Change {
    /// Creates the conversation. Only the first change of a batch can create
    /// it, and only when it does not exist.
    CreateConversation {
        /// The conversation.
        conversation: NewConversation,
    },
    /// Adds a turn with the owner's message and the queued run that answers
    /// it. Refused while the conversation has an active run.
    StartTurn {
        /// The turn.
        turn_id: TurnId,
        /// The part that holds the owner's message, or the instruction of a
        /// task.
        user_part_id: PartId,
        /// The message.
        text: String,
        /// The run.
        run: NewRun,
        /// The task that the turn works on, in a task conversation. Creating
        /// a task with an existing ID leaves that task as it is.
        task: Option<NewTask>,
        /// What the owner's message mentions, stored after it in the
        /// message's order (ADR-0032).
        mentions: Vec<NewMention>,
        /// The images of the owner's message, stored after its mentions in
        /// the order the owner added them (ADR-0033).
        images: Vec<NewImage>,
    },
    /// Adds a queued run that replaces the reply of the latest turn. Refused
    /// unless the turn's current run failed, stopped or was interrupted.
    RetryTurn {
        /// The latest turn.
        turn_id: TurnId,
        /// The new run.
        run: NewRun,
    },
    /// A queued run starts running.
    StartRun {
        /// The run.
        run_id: RunId,
    },
    /// What an active run does now.
    SetActivity {
        /// The run.
        run_id: RunId,
        /// What it does.
        activity: RunActivity,
    },
    /// An active run ends. In a task conversation, its task ends with it:
    /// as `task` says, or with the state that the run's end implies.
    EndRun {
        /// The run.
        run_id: RunId,
        /// Its final state.
        state: RunState,
        /// How it ended.
        outcome: RunOutcome,
        /// How the run's task ended, with the child's report.
        task: Option<TaskEnding>,
    },
    /// A part starts streaming.
    StartPart {
        /// The part.
        part: NewPart,
    },
    /// Text for a streaming part.
    Delta {
        /// The part.
        part_id: PartId,
        /// The text to append.
        text: String,
    },
    /// A streaming part finishes. Its deltas and `tail`, the text that was not
    /// stored yet, become its text, and its delta events are dropped.
    FinishPart {
        /// The part.
        part_id: PartId,
        /// Its final state.
        state: PartState,
        /// Text that arrived after the last stored delta.
        tail: String,
        /// Provider-private data that belongs to it.
        provider_item: Option<ProviderItem>,
    },
    /// Adds a part that is complete at once, such as an error or a
    /// continuation note.
    AddPart {
        /// The part.
        part: NewPart,
        /// Its text.
        text: String,
    },
    /// A tool call part that is not finished changes: its status, its
    /// request or its result. The archived outputs are stored with it. This
    /// is the journal entry of the call's operation (ADR-0013).
    UpdatePart {
        /// The part.
        part_id: PartId,
        /// The call as it is now.
        tool_call: ToolCallDocument,
        /// Outputs to archive with the part.
        archives: Vec<NewArchive>,
    },
    /// A task part that is not finished changes: its call record, the task it
    /// assigned or the tasks it waits for, and the result the model reads.
    /// The archived outputs, such as a long report, are stored with it.
    UpdateTaskPart {
        /// The part.
        part_id: PartId,
        /// The call as it is now.
        tool_call: ToolCallDocument,
        /// The task section as it is now.
        task: Box<TaskPartDocument>,
        /// Outputs to archive with the part.
        archives: Vec<NewArchive>,
    },
    /// The parts of a failed attempt are discarded before the run retries.
    DiscardAttempt {
        /// The run.
        run_id: RunId,
        /// The attempt.
        attempt: u32,
    },
    /// Adds a checkpoint.
    AddCheckpoint {
        /// The checkpoint.
        checkpoint: NewCheckpoint,
    },
    /// Stores the runtime's context ledger and the usage the view shows.
    UpdateContext {
        /// The ledger: a JSON document the runtime owns.
        ledger: String,
        /// The usage to show.
        usage: ContextUsage,
    },
    /// The owner renamed the conversation.
    Rename {
        /// The new title, already checked.
        title: String,
    },
    /// The owner chose another provider profile, or the default.
    SetProfile {
        /// The profile, or `None` for the default.
        profile_id: Option<ProfileId>,
    },
    /// The owner switched the conversation's full access (ADR-0029).
    SetFullAccess {
        /// Whether every operation runs without asking.
        enabled: bool,
    },
    /// A general conversation moves into a project, whose folder becomes
    /// its workspace. Refused when it already belongs to a project, which it
    /// never leaves.
    SetProject {
        /// The project.
        project_id: ProjectId,
    },
}

/// The result of applying changes: the events to publish, and the sequence
/// number of the last event stored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// The events for subscribers, with their sequence numbers. Events that
    /// only the model needs, such as a continuation note's, are stored but not
    /// listed.
    pub events: Vec<(u32, ConversationEvent)>,
    /// The sequence number of the last event the changes stored.
    pub last_seq: u32,
    /// The conversation as it is now, when the changes affected the
    /// conversation list: a new turn, a run that started or ended, a new
    /// title, profile or project.
    pub summary: Option<ConversationSummary>,
}

/// Why a change to a conversation was refused or failed.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeStoreError {
    /// The conversation does not exist.
    #[error("the conversation does not exist")]
    ConversationNotFound,
    /// The conversation already exists.
    #[error("the conversation already exists")]
    ConversationExists,
    /// The conversation already has an active run.
    #[error("the conversation already has an active run")]
    RunActive,
    /// The latest turn's reply cannot be retried.
    #[error("the latest turn's reply cannot be retried")]
    NothingToRetry,
    /// The run does not exist in the conversation, or is not in a state that
    /// allows the change.
    #[error("the run {run} cannot take this change")]
    RunNotActive {
        /// The run.
        run: RunId,
    },
    /// The part does not exist in the conversation, or is not streaming.
    #[error("the part {part} is not streaming")]
    PartNotStreaming {
        /// The part.
        part: PartId,
    },
    /// The part is not a tool call.
    #[error("the part {part} is not a tool call")]
    NotAToolCall {
        /// The part.
        part: PartId,
    },
    /// The part is not a task part.
    #[error("the part {part} is not a task part")]
    NotATaskPart {
        /// The part.
        part: PartId,
    },
    /// The task conversation already works on a task.
    #[error("the task conversation already works on a task")]
    TaskActive,
    /// The project does not exist.
    #[error("the project does not exist")]
    ProjectNotFound,
    /// The conversation already belongs to a project.
    #[error("the conversation already belongs to a project")]
    AlreadyInProject,
    /// Stored data does not have the shape this version expects.
    #[error("stored data is not valid: {what}")]
    InvalidData {
        /// What is wrong.
        what: String,
    },
    /// The sequence numbers of the conversation ran out.
    #[error("the conversation has no sequence numbers left")]
    SequenceExhausted,
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl RuntimeStoreError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::ConversationNotFound => "runtime_store.conversation_not_found",
            Self::ConversationExists => "runtime_store.conversation_exists",
            Self::RunActive => "runtime_store.run_active",
            Self::NothingToRetry => "runtime_store.nothing_to_retry",
            Self::RunNotActive { .. } => "runtime_store.run_not_active",
            Self::PartNotStreaming { .. } => "runtime_store.part_not_streaming",
            Self::NotAToolCall { .. } => "runtime_store.not_a_tool_call",
            Self::NotATaskPart { .. } => "runtime_store.not_a_task_part",
            Self::TaskActive => "runtime_store.task_active",
            Self::ProjectNotFound => "runtime_store.project_not_found",
            Self::AlreadyInProject => "runtime_store.already_in_project",
            Self::InvalidData { .. } => "runtime_store.invalid_data",
            Self::SequenceExhausted => "runtime_store.sequence_exhausted",
            Self::Store(error) => error.code(),
        }
    }

    pub(crate) fn invalid(what: impl Into<String>) -> Self {
        Self::InvalidData { what: what.into() }
    }
}

impl From<mannie_types::ids::InvalidId> for RuntimeStoreError {
    fn from(error: mannie_types::ids::InvalidId) -> Self {
        Self::invalid(error.to_string())
    }
}

impl From<rusqlite::Error> for RuntimeStoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Store(StoreError::query(error))
    }
}

/// Every SQL statement of this module, for the test that prepares them.
#[cfg(test)]
pub(crate) fn statements() -> impl Iterator<Item = &'static str> {
    apply::STATEMENTS
        .iter()
        .chain(archives::STATEMENTS)
        .chain(images::STATEMENTS)
        .chain(projects::STATEMENTS)
        .chain(reads::STATEMENTS)
        .chain(recovery::STATEMENTS)
        .chain(rows::STATEMENTS)
        .chain(tasks::STATEMENTS)
        .copied()
}

#[cfg(test)]
mod tests;
