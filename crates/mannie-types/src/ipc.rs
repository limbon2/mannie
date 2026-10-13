//! Envelopes shared by every IPC contract (docs/architecture/desktop-app.md,
//! IPC contract).
//!
//! A command returns `Result<T, CommandError<E>>`, where `E` is the feature's
//! error enum, tagged by a stable `code`. A channel carries
//! [`ChannelBatch`]es, whose items have consecutive sequence numbers.

use serde::Serialize;
use specta::Type;

/// An error as a command returns it (ADR-0007): the feature's error, flattened
/// so that its `code` and arguments sit at the top level, and an English
/// description for logs only. The UI shows the message for the code from its
/// catalog and never shows the description (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CommandError<E> {
    /// The typed error, with its `code` and arguments.
    #[serde(flatten)]
    pub error: E,
    /// What went wrong, in English, for logs.
    pub description: String,
}

impl<E: std::error::Error> CommandError<E> {
    /// Wraps an error and records its description.
    pub fn new(error: E) -> Self {
        let description = error.to_string();
        Self { error, description }
    }
}

impl<E: std::error::Error> From<E> for CommandError<E> {
    fn from(error: E) -> Self {
        Self::new(error)
    }
}

/// Messages that a channel delivers together (docs/architecture/desktop-app.md,
/// streaming to the UI).
///
/// The core sends a batch at most every 16 to 33 ms, so the frame rate of the
/// webview sets the pace. Each item has a sequence number: the first one is
/// `first_sequence`, and each following item has the next number. A receiver
/// that sees a batch start anywhere else than right after the previous one
/// knows that it missed messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelBatch<T> {
    /// The sequence number of the first item.
    pub first_sequence: u32,
    /// The messages, in order.
    pub items: Vec<T>,
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use super::{ChannelBatch, CommandError};

    #[derive(Debug, Serialize, thiserror::Error)]
    #[serde(tag = "code", rename_all = "snake_case")]
    enum ExampleError {
        #[error("the item {name:?} does not exist")]
        Missing { name: String },
    }

    #[test]
    fn command_error_flattens_the_code_and_adds_the_description() {
        let error = CommandError::from(ExampleError::Missing {
            name: "draft".to_owned(),
        });

        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            serde_json::json!({
                "code": "missing",
                "name": "draft",
                "description": "the item \"draft\" does not exist",
            })
        );
    }

    #[test]
    fn channel_batch_serializes_its_sequence_and_items() {
        let batch = ChannelBatch {
            first_sequence: 7,
            items: vec!["a", "b"],
        };

        assert_eq!(
            serde_json::to_value(&batch).unwrap(),
            serde_json::json!({ "firstSequence": 7, "items": ["a", "b"] })
        );
    }
}
