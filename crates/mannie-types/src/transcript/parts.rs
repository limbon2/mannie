//! Parts: the typed elements of a turn.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::clock::UtcMillis;
use crate::ids::{PartId, RunId, TurnId};
use crate::run::RunFailure;
use crate::tools::ToolCallView;

/// What a part is. Features add kinds; a reader that meets a kind it does not
/// know shows a generic placeholder instead of failing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PartKind {
    /// The owner's message.
    UserText,
    /// The text of a reply.
    Text,
    /// A reasoning summary or the reasoning text of a reply.
    Reasoning,
    /// Why a reply failed, for the owner.
    Error,
    /// A note that asks the model to continue, for the model only.
    ContinuationNote,
    /// A tool call and everything Mannie did with it
    /// (docs/features/system-tools/tool-calls.md).
    ToolCall,
}

impl PartKind {
    /// Every kind this version knows.
    pub const ALL: [Self; 6] = [
        Self::UserText,
        Self::Text,
        Self::Reasoning,
        Self::Error,
        Self::ContinuationNote,
        Self::ToolCall,
    ];

    /// The kind's name as the store and the UI know it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserText => "user_text",
            Self::Text => "text",
            Self::Reasoning => "reasoning",
            Self::Error => "error",
            Self::ContinuationNote => "continuation_note",
            Self::ToolCall => "tool_call",
        }
    }

    /// The kind with this name, or `None` for a kind this version does not
    /// know.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }

    /// Who the part is for.
    pub fn audience(self) -> Audience {
        match self {
            Self::UserText | Self::Text | Self::Reasoning | Self::ToolCall => Audience::Both,
            Self::Error => Audience::Owner,
            Self::ContinuationNote => Audience::Model,
        }
    }
}

/// Who a part is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    /// The owner sees it, and the model reads it.
    Both,
    /// Only the model reads it.
    Model,
    /// Only the owner sees it.
    Owner,
}

impl Audience {
    /// The audience's name as the store keeps it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Both => "both",
            Self::Model => "model",
            Self::Owner => "owner",
        }
    }

    /// The audience with this name.
    pub fn parse(name: &str) -> Option<Self> {
        [Self::Both, Self::Model, Self::Owner]
            .into_iter()
            .find(|audience| audience.as_str() == name)
    }

    /// Whether the owner sees parts for this audience.
    pub fn includes_owner(self) -> bool {
        matches!(self, Self::Both | Self::Owner)
    }

    /// Whether the model reads parts for this audience.
    pub fn includes_model(self) -> bool {
        matches!(self, Self::Both | Self::Model)
    }
}

/// Where a part is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PartState {
    /// Text is still arriving.
    Streaming,
    /// The part is finished.
    Complete,
    /// The owner stopped the reply; the text so far stays.
    Stopped,
    /// The process ended while the part streamed; the stored text stays.
    Interrupted,
    /// The reply failed while the part streamed; the text so far stays.
    Incomplete,
    /// The attempt that wrote the part failed and was retried; nobody sees
    /// the part.
    Discarded,
}

impl PartState {
    /// Every state.
    pub const ALL: [Self; 6] = [
        Self::Streaming,
        Self::Complete,
        Self::Stopped,
        Self::Interrupted,
        Self::Incomplete,
        Self::Discarded,
    ];

    /// The state's name as the store keeps it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Streaming => "streaming",
            Self::Complete => "complete",
            Self::Stopped => "stopped",
            Self::Interrupted => "interrupted",
            Self::Incomplete => "incomplete",
            Self::Discarded => "discarded",
        }
    }

    /// The state with this name.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == name)
    }
}

/// What a part holds, as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum PartContent {
    /// The owner's message, shown as the owner wrote it.
    UserText {
        /// The message.
        text: String,
    },
    /// Reply text, rendered as Markdown.
    Text {
        /// The text so far.
        text: String,
    },
    /// Reasoning, shown collapsed above the reply.
    Reasoning {
        /// The reasoning so far.
        text: String,
    },
    /// Why the reply failed.
    Error {
        /// The cause.
        failure: RunFailure,
    },
    /// A tool call.
    ToolCall {
        /// The call, its status and its result.
        call: Box<ToolCallView>,
    },
    /// A kind of part that this version does not know.
    Unknown {
        /// The kind's name.
        name: String,
    },
}

/// A part as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PartView {
    /// The part.
    pub id: PartId,
    /// The turn it belongs to.
    pub turn_id: TurnId,
    /// The run that wrote it, or `None` for the owner's message.
    pub run_id: Option<RunId>,
    /// Its position within the turn, from 0.
    pub position: u32,
    /// Where it is in its life.
    pub state: PartState,
    /// What it holds.
    pub content: PartContent,
    /// When it was created.
    pub created_at: UtcMillis,
}

#[cfg(test)]
mod tests {
    use super::{Audience, PartContent, PartKind, PartState};

    #[test]
    fn kinds_round_trip_through_their_names() {
        for kind in PartKind::ALL {
            assert_eq!(PartKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(PartKind::parse("task_report"), None);
    }

    #[test]
    fn states_and_audiences_round_trip_through_their_names() {
        for state in PartState::ALL {
            assert_eq!(PartState::parse(state.as_str()), Some(state));
        }
        for audience in [Audience::Both, Audience::Model, Audience::Owner] {
            assert_eq!(Audience::parse(audience.as_str()), Some(audience));
        }
    }

    #[test]
    fn continuation_notes_are_for_the_model_and_errors_for_the_owner() {
        assert!(PartKind::ContinuationNote.audience().includes_model());
        assert!(!PartKind::ContinuationNote.audience().includes_owner());
        assert!(PartKind::Error.audience().includes_owner());
        assert!(!PartKind::Error.audience().includes_model());
        assert_eq!(PartKind::Text.audience(), Audience::Both);
    }

    #[test]
    fn content_serializes_with_its_kind() {
        let text = PartContent::Text {
            text: "Hello".to_owned(),
        };
        let unknown = PartContent::Unknown {
            name: "task_report".to_owned(),
        };

        assert_eq!(
            serde_json::to_value(&text).unwrap(),
            serde_json::json!({ "kind": "text", "text": "Hello" })
        );
        assert_eq!(
            serde_json::to_value(&unknown).unwrap(),
            serde_json::json!({ "kind": "unknown", "name": "task_report" })
        );
    }
}
