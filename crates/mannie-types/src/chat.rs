//! The chat feature's contracts (docs/features/chat/conversations.md): its
//! settings section, the arguments and results of its commands, and its
//! errors.

use serde::{Deserialize, Deserializer, Serialize};
use specta::Type;

use crate::ids::{CheckpointId, ConversationId, ProfileId, RunId, TurnId};
use crate::run::RunFailure;
use crate::settings::SettingsSection;
use crate::transcript::{SubscriptionId, TurnView};

/// The longest message the owner can send, in bytes of UTF-8.
pub const MAX_MESSAGE_BYTES: usize = 256 * 1024;

/// The longest title the owner can give a conversation, in characters.
pub const MAX_TITLE_CHARACTERS: usize = 200;

/// The longest title taken from a first message, in characters.
pub const MAX_DERIVED_TITLE_CHARACTERS: usize = 80;

/// The longest global instructions the owner can write, in bytes of UTF-8.
pub const MAX_INSTRUCTIONS_BYTES: usize = 32 * 1024;

/// The most earlier turns one request loads.
pub const MAX_EARLIER_TURNS: u32 = 100;

/// The owner's instructions: plain text that applies to every conversation.
/// Deserializing checks the length, so a stored document is valid.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Type)]
#[serde(transparent)]
pub struct Instructions(String);

impl Instructions {
    /// Checks the length of the text.
    pub fn parse(text: String) -> Result<Self, ChatError> {
        if text.len() > MAX_INSTRUCTIONS_BYTES {
            return Err(ChatError::InstructionsTooLong {
                max_bytes: limit(MAX_INSTRUCTIONS_BYTES),
            });
        }
        Ok(Self(text))
    }

    /// The text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether there are no instructions, apart from white space.
    pub fn is_blank(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl<'de> Deserialize<'de> for Instructions {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(text).map_err(serde::de::Error::custom)
    }
}

/// The chat settings section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChatSettings {
    /// The owner's global instructions.
    pub instructions: Instructions,
}

impl SettingsSection for ChatSettings {
    const KEY: &'static str = "chat";
    const VERSION: u32 = 1;
}

/// The argument of `update_chat_settings`: the complete new section, checked by
/// the core.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChatSettings {
    /// The owner's global instructions.
    pub instructions: String,
}

impl UpdateChatSettings {
    /// Checks the request and turns it into the section to store.
    pub fn into_settings(self) -> Result<ChatSettings, ChatError> {
        Ok(ChatSettings {
            instructions: Instructions::parse(self.instructions)?,
        })
    }
}

/// Checks the text of a message: not blank, and not longer than
/// [`MAX_MESSAGE_BYTES`].
pub fn check_message(text: &str) -> Result<(), ChatError> {
    if text.trim().is_empty() {
        return Err(ChatError::MessageEmpty);
    }
    if text.len() > MAX_MESSAGE_BYTES {
        return Err(ChatError::MessageTooLong {
            max_bytes: limit(MAX_MESSAGE_BYTES),
        });
    }
    Ok(())
}

/// Checks a title the owner chose and returns it without surrounding white
/// space. Line breaks and other control characters become spaces.
pub fn check_title(title: &str) -> Result<String, ChatError> {
    let cleaned: String = title
        .trim()
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    if cleaned.trim().is_empty() {
        return Err(ChatError::TitleEmpty);
    }
    if cleaned.chars().count() > MAX_TITLE_CHARACTERS {
        return Err(ChatError::TitleTooLong {
            max_characters: limit(MAX_TITLE_CHARACTERS),
        });
    }
    Ok(cleaned)
}

/// The title of a new conversation: the first line of its first message that
/// is not blank, cut at [`MAX_DERIVED_TITLE_CHARACTERS`] characters.
pub fn title_from_message(text: &str) -> String {
    let first_line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let mut title: String = first_line
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_DERIVED_TITLE_CHARACTERS)
        .collect();
    if first_line.chars().count() > MAX_DERIVED_TITLE_CHARACTERS {
        title = title.trim_end().to_owned();
        title.push('…');
    }
    title
}

/// A limit as a number for the UI's message.
fn limit(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// The argument of `start_conversation`: the first message of a new
/// conversation.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StartConversation {
    /// The message.
    pub text: String,
    /// The provider profile to use, or `None` for the default profile.
    pub profile_id: Option<ProfileId>,
}

/// The result of `start_conversation`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
#[expect(
    clippy::struct_field_names,
    reason = "the fields are the IDs of three entities, and IPC contracts name every ID with an Id suffix"
)]
pub struct StartedConversation {
    /// The new conversation.
    pub conversation_id: ConversationId,
    /// Its first turn.
    pub turn_id: TurnId,
    /// The run that answers the message.
    pub run_id: RunId,
}

/// The argument of `send_message`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SendMessage {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The message.
    pub text: String,
}

/// The result of `send_message` and `retry_reply`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StartedReply {
    /// The turn.
    pub turn_id: TurnId,
    /// The run that writes the reply.
    pub run_id: RunId,
}

/// The argument of the commands that act on one conversation.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConversationRef {
    /// The conversation.
    pub conversation_id: ConversationId,
}

/// The result of `stop_reply`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum StopOutcome {
    /// The reply stopped.
    Stopped,
    /// No reply was being written.
    NotRunning,
}

/// The argument of `rename_conversation`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RenameConversation {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The new title.
    pub title: String,
}

/// The argument of `set_conversation_profile`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SetConversationProfile {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The profile, or `None` for the default profile.
    pub profile_id: Option<ProfileId>,
}

/// The argument of `get_checkpoint`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointRef {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The checkpoint.
    pub checkpoint_id: CheckpointId,
}

/// The argument of `load_earlier_turns`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LoadEarlierTurns {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The position of the oldest turn the view has.
    pub before_position: u32,
    /// How many turns to load, at most [`MAX_EARLIER_TURNS`].
    pub limit: u32,
}

/// The result of `load_earlier_turns`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EarlierTurns {
    /// The turns before the requested position, oldest first.
    pub turns: Vec<TurnView>,
    /// Whether even older turns exist.
    pub has_earlier_turns: bool,
}

/// The argument of `subscribe_conversation`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SubscribeConversation {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The sequence number of the last event the subscriber applied, or `None`
    /// to start from a snapshot.
    pub after_seq: Option<u32>,
}

/// The argument of `unsubscribe`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Unsubscribe {
    /// The subscription to end.
    pub subscription_id: SubscriptionId,
}

/// Why a chat command failed. The UI shows the message for the code from its
/// catalog (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ChatError {
    /// The message is empty or only white space.
    #[error("the message is empty")]
    MessageEmpty,
    /// The message is longer than Mannie accepts.
    #[error("the message is longer than {max_bytes} bytes")]
    MessageTooLong {
        /// The limit.
        max_bytes: u32,
    },
    /// The title is empty or only white space.
    #[error("the title is empty")]
    TitleEmpty,
    /// The title is longer than Mannie accepts.
    #[error("the title is longer than {max_characters} characters")]
    TitleTooLong {
        /// The limit.
        max_characters: u32,
    },
    /// The instructions are longer than Mannie accepts.
    #[error("the instructions are longer than {max_bytes} bytes")]
    InstructionsTooLong {
        /// The limit.
        max_bytes: u32,
    },
    /// The conversation does not exist, for example because it was deleted.
    #[error("the conversation does not exist")]
    ConversationNotFound,
    /// The checkpoint does not exist.
    #[error("the checkpoint does not exist")]
    CheckpointNotFound,
    /// A reply is already being written in this conversation.
    #[error("a reply is already being written in this conversation")]
    RunActive,
    /// The latest reply did not fail, stop or get interrupted, so there is
    /// nothing to retry.
    #[error("the latest reply cannot be retried")]
    NothingToRetry,
    /// No turn is old enough to be compacted.
    #[error("there is nothing to compact")]
    NothingToCompact,
    /// The conversation is being compacted. Replies wait until the checkpoint
    /// is written.
    #[error("the conversation is being compacted")]
    Compacting,
    /// The model call that writes the checkpoint failed.
    #[error("the checkpoint could not be written: {failure}")]
    CompactionFailed {
        /// Why the call failed.
        failure: RunFailure,
    },
    /// No provider profile exists.
    #[error("no provider profile exists")]
    NoProfile,
    /// The chosen provider profile does not exist.
    #[error("the provider profile does not exist")]
    ProfileNotFound,
    /// The settings were written by a newer version of Mannie, which this
    /// version does not overwrite.
    #[error("the chat settings were written by a newer version of Mannie")]
    WrittenByNewerVersion,
    /// The store failed. The log has the cause.
    #[error("the conversation could not be read or saved")]
    StorageFailed,
    /// The core started without its database.
    #[error("the database is not open")]
    StoreUnavailable,
    /// Mannie is quitting and starts no more replies.
    #[error("Mannie is quitting")]
    ShuttingDown,
}

impl ChatError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::MessageEmpty => "chat.message_empty",
            Self::MessageTooLong { .. } => "chat.message_too_long",
            Self::TitleEmpty => "chat.title_empty",
            Self::TitleTooLong { .. } => "chat.title_too_long",
            Self::InstructionsTooLong { .. } => "chat.instructions_too_long",
            Self::ConversationNotFound => "chat.conversation_not_found",
            Self::CheckpointNotFound => "chat.checkpoint_not_found",
            Self::RunActive => "chat.run_active",
            Self::NothingToRetry => "chat.nothing_to_retry",
            Self::NothingToCompact => "chat.nothing_to_compact",
            Self::Compacting => "chat.compacting",
            Self::CompactionFailed { .. } => "chat.compaction_failed",
            Self::NoProfile => "chat.no_profile",
            Self::ProfileNotFound => "chat.profile_not_found",
            Self::WrittenByNewerVersion => "chat.written_by_newer_version",
            Self::StorageFailed => "chat.storage_failed",
            Self::StoreUnavailable => "chat.store_unavailable",
            Self::ShuttingDown => "chat.shutting_down",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChatError, ChatSettings, MAX_INSTRUCTIONS_BYTES, MAX_MESSAGE_BYTES, UpdateChatSettings,
        check_message, check_title, title_from_message,
    };

    #[test]
    fn messages_must_have_text_within_the_limit() {
        assert_eq!(check_message("Hello"), Ok(()));
        assert_eq!(check_message(" \n\t"), Err(ChatError::MessageEmpty));
        assert_eq!(
            check_message(&"a".repeat(MAX_MESSAGE_BYTES + 1)),
            Err(ChatError::MessageTooLong { max_bytes: 262_144 })
        );
        assert_eq!(check_message(&"a".repeat(MAX_MESSAGE_BYTES)), Ok(()));
    }

    #[test]
    fn titles_are_trimmed_and_lose_their_control_characters() {
        assert_eq!(
            check_title("  Plans\nfor May "),
            Ok("Plans for May".to_owned())
        );
        assert_eq!(check_title("\n \t"), Err(ChatError::TitleEmpty));
        assert_eq!(
            check_title(&"é".repeat(201)),
            Err(ChatError::TitleTooLong {
                max_characters: 200
            })
        );
        assert_eq!(check_title(&"é".repeat(200)).unwrap().chars().count(), 200);
    }

    #[test]
    fn a_title_comes_from_the_first_line_with_text() {
        assert_eq!(
            title_from_message("\n\n  Hello there  \nsecond"),
            "Hello there"
        );
        let long = format!("{} tail", "word ".repeat(30));
        let title = title_from_message(&long);
        assert!(title.ends_with('…'), "{title}");
        assert!(title.chars().count() <= 81, "{title}");
    }

    #[test]
    fn instructions_are_checked_when_saved_and_when_read() {
        let valid = UpdateChatSettings {
            instructions: "Answer in English.".to_owned(),
        }
        .into_settings()
        .unwrap();
        let too_long = UpdateChatSettings {
            instructions: "a".repeat(MAX_INSTRUCTIONS_BYTES + 1),
        }
        .into_settings();
        let stored_too_long = serde_json::from_value::<ChatSettings>(serde_json::json!({
            "instructions": "a".repeat(MAX_INSTRUCTIONS_BYTES + 1)
        }));

        assert_eq!(valid.instructions.as_str(), "Answer in English.");
        assert_eq!(
            too_long,
            Err(ChatError::InstructionsTooLong { max_bytes: 32_768 })
        );
        stored_too_long.unwrap_err();
        assert_eq!(
            serde_json::to_value(&valid).unwrap(),
            serde_json::json!({ "instructions": "Answer in English." })
        );
    }

    #[test]
    fn errors_serialize_with_their_code() {
        assert_eq!(
            serde_json::to_value(ChatError::RunActive).unwrap(),
            serde_json::json!({ "code": "run_active" })
        );
        assert_eq!(ChatError::RunActive.code(), "chat.run_active");
    }
}
