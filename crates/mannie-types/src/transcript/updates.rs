//! What the conversation channels carry (docs/architecture/desktop-app.md,
//! streaming to the UI).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::ConversationId;

use super::events::ConversationEvent;
use super::views::{ConversationSnapshot, ConversationSummary};

/// Identifies one subscription of the webview, so that it can end it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct SubscriptionId(u32);

impl SubscriptionId {
    /// The subscription with this number.
    pub const fn new(number: u32) -> Self {
        Self(number)
    }

    /// The subscription's number.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Why the core ended a subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionEnd {
    /// The subscriber fell too far behind. It subscribes again from its
    /// cursor and misses nothing.
    Lagged,
    /// The conversation was deleted.
    Deleted,
    /// Mannie is quitting.
    Shutdown,
}

/// A message of a conversation's channel. The first one is a snapshot or the
/// first event after the subscriber's cursor; events follow in the order of
/// their sequence numbers, without gaps and without repeats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ConversationUpdate {
    /// The conversation as of `snapshot.through_seq`.
    Snapshot {
        /// The snapshot.
        snapshot: ConversationSnapshot,
    },
    /// One event. `seq` is the cursor to subscribe with after a reload.
    Event {
        /// The event's sequence number.
        seq: u32,
        /// The event.
        event: ConversationEvent,
    },
    /// The core ended the subscription.
    Ended {
        /// Why.
        reason: SubscriptionEnd,
    },
}

/// A message of the conversation list's channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ConversationListUpdate {
    /// Every conversation, the most recent first.
    Snapshot {
        /// The conversations.
        conversations: Vec<ConversationSummary>,
    },
    /// A conversation was created or changed.
    Upserted {
        /// The conversation as it is now.
        conversation: ConversationSummary,
    },
    /// A conversation was deleted.
    Removed {
        /// The deleted conversation.
        id: ConversationId,
    },
    /// The core ended the subscription.
    Ended {
        /// Why.
        reason: SubscriptionEnd,
    },
}

#[cfg(test)]
mod tests {
    use super::{ConversationUpdate, SubscriptionEnd, SubscriptionId};

    #[test]
    fn updates_serialize_with_their_type() {
        let ended = ConversationUpdate::Ended {
            reason: SubscriptionEnd::Lagged,
        };

        assert_eq!(
            serde_json::to_value(&ended).unwrap(),
            serde_json::json!({ "type": "ended", "reason": "lagged" })
        );
    }

    #[test]
    fn subscription_ids_are_plain_numbers() {
        assert_eq!(serde_json::to_string(&SubscriptionId::new(7)).unwrap(), "7");
        assert_eq!(SubscriptionId::new(7).get(), 7);
    }
}
