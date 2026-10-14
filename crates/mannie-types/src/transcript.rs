//! The transcript of a conversation and the events that change it
//! (docs/architecture/extension-points.md, transcript parts and run events).
//!
//! The transcript is a list of parts per turn. Parts are appended and never
//! edited: a part's text grows while it streams and is final once the part is
//! finished. Every change is an event with the conversation's next sequence
//! number, and subscribers replay the events after their cursor
//! (ADR-0012).
//!
//! Readers tolerate kinds they do not know: an unknown part becomes
//! [`PartContent::Unknown`], and an unknown event
//! [`ConversationEvent::Unknown`].

mod events;
mod parts;
mod updates;
mod views;

pub use events::ConversationEvent;
pub use parts::{Audience, PartContent, PartKind, PartState, PartView};
pub use updates::{ConversationListUpdate, ConversationUpdate, SubscriptionEnd, SubscriptionId};
pub use views::{
    Checkpoint, CheckpointView, ContextUsage, ConversationSnapshot, ConversationSummary,
    TitleSource, TurnView,
};
