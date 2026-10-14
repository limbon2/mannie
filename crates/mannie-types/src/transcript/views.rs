//! Conversations, turns and checkpoints as the UI shows them.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::clock::UtcMillis;
use crate::ids::{CheckpointId, ConversationId, ProfileId, TurnId};
use crate::run::RunView;

use super::parts::PartView;

/// Where a conversation's title comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TitleSource {
    /// The first line of the first message.
    FirstMessage,
    /// The owner renamed the conversation.
    Owner,
}

impl TitleSource {
    /// The source's name as the store keeps it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FirstMessage => "first_message",
            Self::Owner => "owner",
        }
    }

    /// The source with this name.
    pub fn parse(name: &str) -> Option<Self> {
        [Self::FirstMessage, Self::Owner]
            .into_iter()
            .find(|source| source.as_str() == name)
    }
}

/// A conversation as the conversation list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    /// The conversation.
    pub id: ConversationId,
    /// Its title.
    pub title: String,
    /// Where the title comes from.
    pub title_source: TitleSource,
    /// The provider profile the owner chose for it, or `None` for the
    /// default profile.
    pub profile_id: Option<ProfileId>,
    /// Whether a reply is being written.
    pub has_active_run: bool,
    /// When it was created.
    pub created_at: UtcMillis,
    /// When it last changed. The list shows the most recent first.
    pub updated_at: UtcMillis,
}

/// A turn: the owner's message and the reply of its current run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TurnView {
    /// The turn.
    pub id: TurnId,
    /// Its position in the conversation, from 0.
    pub position: u32,
    /// The run whose reply the turn shows: the latest one.
    pub run: Option<RunView>,
    /// The owner's message and the visible parts of the current run, in order.
    pub parts: Vec<PartView>,
    /// When the owner sent the message.
    pub created_at: UtcMillis,
}

/// How much of the context budget a conversation uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    /// The tokens the next request is expected to use.
    pub used_tokens: u32,
    /// The tokens one request may use: the context window minus the output
    /// limit and a margin.
    pub budget_tokens: u32,
    /// Whether `used_tokens` is an estimate rather than built on the counts the
    /// provider reported.
    pub estimated: bool,
}

/// A checkpoint as the conversation view marks it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointView {
    /// The checkpoint.
    pub id: CheckpointId,
    /// The position of the last turn it covers.
    pub covers_through_turn: u32,
    /// The context's size before the compaction, in tokens.
    pub tokens_before: u32,
    /// The context's expected size after the compaction, in tokens.
    pub tokens_after: u32,
    /// When it was written.
    pub created_at: UtcMillis,
}

/// A checkpoint with its summary, for the owner to read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Checkpoint {
    /// What the conversation view knows about it.
    pub view: CheckpointView,
    /// The summary the model wrote.
    pub summary: String,
}

/// A conversation as of one event: the latest turns, and what the view shows
/// around them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSnapshot {
    /// The conversation.
    pub conversation: ConversationSummary,
    /// The sequence number of the last event the snapshot includes.
    pub through_seq: u32,
    /// The latest turns, oldest first.
    pub turns: Vec<TurnView>,
    /// Whether older turns exist before the first one in `turns`.
    pub has_earlier_turns: bool,
    /// The latest checkpoint, if the conversation has one.
    pub checkpoint: Option<CheckpointView>,
    /// How much of the context the conversation uses, once it is known.
    pub context: Option<ContextUsage>,
}

#[cfg(test)]
mod tests {
    use super::TitleSource;

    #[test]
    fn title_sources_round_trip_through_their_names() {
        for source in [TitleSource::FirstMessage, TitleSource::Owner] {
            assert_eq!(TitleSource::parse(source.as_str()), Some(source));
        }
        assert_eq!(TitleSource::parse("model"), None);
    }
}
