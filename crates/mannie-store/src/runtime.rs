//! The runtime's repository: conversations, turns, runs, parts, the event log
//! and checkpoints (docs/features/chat/conversations.md, ADR-0012).
//!
//! The runtime changes a conversation through [`Store::apply`], which applies a
//! list of [`Change`]s in one transaction, appends an event with the
//! conversation's next sequence number for each change, and returns the events
//! to publish. The reads serve subscribers ([`Store::snapshot`] and
//! [`Store::events_after`]), the conversation list and the model's context.

mod apply;
mod reads;
mod recovery;
mod rows;

use mannie_types::ids::{CheckpointId, ConversationId, PartId, ProfileId, RunId, TurnId};
use mannie_types::run::{RunActivity, RunFailure, RunOutcome, RunState};
use mannie_types::transcript::{
    ContextUsage, ConversationEvent, ConversationSummary, PartKind, PartState,
};

use crate::error::StoreError;

pub use reads::{EventsAfter, ModelPart, ModelTurn, StoredCheckpoint};
pub use recovery::Recovered;

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
        /// The part that holds the owner's message.
        user_part_id: PartId,
        /// The message.
        text: String,
        /// The run.
        run: NewRun,
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
    /// An active run ends.
    EndRun {
        /// The run.
        run_id: RunId,
        /// Its final state.
        state: RunState,
        /// How it ended.
        outcome: RunOutcome,
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
    /// title or profile.
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
        .chain(reads::STATEMENTS)
        .chain(recovery::STATEMENTS)
        .chain(rows::STATEMENTS)
        .copied()
}

#[cfg(test)]
mod tests;
