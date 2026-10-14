//! The events of a conversation, as subscribers receive them.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::{PartId, RunId};
use crate::run::RunView;

use super::parts::PartView;
use super::views::{CheckpointView, ContextUsage, ConversationSummary, TurnView};

/// A change in a conversation. Applying the events after a cursor, in order,
/// to the state at that cursor gives the current state (ADR-0012).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ConversationEvent {
    /// The owner sent a message: a new turn with its message and the run that
    /// answers it.
    TurnStarted {
        /// The turn as it started.
        turn: TurnView,
    },
    /// A new run replaces the turn's reply, after the owner retried it. The
    /// parts of the previous run are no longer shown.
    RunStarted {
        /// The new run.
        run: RunView,
    },
    /// A run changed: it started, does something else, or ended.
    RunUpdated {
        /// The run as it is now.
        run: RunView,
    },
    /// A run added a part. A streaming part's text arrives in deltas.
    PartStarted {
        /// The part as it started.
        part: PartView,
    },
    /// Text for a streaming part.
    PartDelta {
        /// The part.
        part_id: PartId,
        /// The text to append.
        text: String,
    },
    /// A part is finished. Its content replaces everything the deltas wrote.
    PartFinished {
        /// The finished part.
        part: PartView,
    },
    /// An attempt failed and will be retried; its parts are no longer shown.
    AttemptDiscarded {
        /// The run.
        run_id: RunId,
        /// The parts the attempt wrote.
        part_ids: Vec<PartId>,
    },
    /// A checkpoint now stands in for older turns in the model's context.
    CheckpointCreated {
        /// The checkpoint.
        checkpoint: CheckpointView,
    },
    /// The conversation's use of the context changed.
    ContextUsageChanged {
        /// The new usage.
        usage: ContextUsage,
    },
    /// The conversation's title or provider profile changed.
    ConversationChanged {
        /// The conversation as it is now.
        conversation: ConversationSummary,
    },
    /// A kind of event that this version does not know.
    Unknown {
        /// The kind's name.
        name: String,
    },
}

impl ConversationEvent {
    /// The event kind's name as the store keeps it.
    pub fn kind(&self) -> &str {
        match self {
            Self::TurnStarted { .. } => "turn_started",
            Self::RunStarted { .. } => "run_started",
            Self::RunUpdated { .. } => "run_updated",
            Self::PartStarted { .. } => "part_started",
            Self::PartDelta { .. } => "part_delta",
            Self::PartFinished { .. } => "part_finished",
            Self::AttemptDiscarded { .. } => "attempt_discarded",
            Self::CheckpointCreated { .. } => "checkpoint_created",
            Self::ContextUsageChanged { .. } => "context_usage_changed",
            Self::ConversationChanged { .. } => "conversation_changed",
            Self::Unknown { name } => name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ConversationEvent;
    use crate::ids::PartId;

    #[test]
    fn events_serialize_with_their_kind() {
        let delta = ConversationEvent::PartDelta {
            part_id: PartId::parse("0199e0f2-7a1b-7c3d-8e4f-0123456789ab").unwrap(),
            text: "Hel".to_owned(),
        };

        assert_eq!(delta.kind(), "part_delta");
        assert_eq!(
            serde_json::to_value(&delta).unwrap(),
            serde_json::json!({
                "kind": "part_delta",
                "partId": "0199e0f2-7a1b-7c3d-8e4f-0123456789ab",
                "text": "Hel",
            })
        );
    }

    #[test]
    fn an_unknown_event_keeps_its_name() {
        let unknown = ConversationEvent::Unknown {
            name: "tool_progress".to_owned(),
        };

        assert_eq!(unknown.kind(), "tool_progress");
    }
}
