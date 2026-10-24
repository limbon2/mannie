//! The chat feature's contracts (docs/features/chat/conversations.md): its
//! settings section, the arguments and results of its commands, and its
//! errors.

use serde::{Deserialize, Deserializer, Serialize};
use specta::Type;

use crate::context::AttachmentRef;
use crate::ids::{CheckpointId, ConversationId, ProfileId, ProjectId, RunId, TurnId};
use crate::images::ImageRef;
use crate::mentions::MentionRef;
use crate::run::RunFailure;
use crate::settings::SettingsSection;
use crate::tools::PathRefusal;
use crate::transcript::{SubscriptionId, TurnView};

/// The longest message the owner can send, in bytes of UTF-8.
pub const MAX_MESSAGE_BYTES: usize = 256 * 1024;

/// The longest title the owner can give a conversation, in characters.
pub const MAX_TITLE_CHARACTERS: usize = 200;

/// The longest title taken from a first message, in characters.
pub const MAX_DERIVED_TITLE_CHARACTERS: usize = 80;

/// The longest global instructions the owner can write, in characters. It
/// was 32 KiB of UTF-8 before, so every stored text stays within it.
pub const MAX_INSTRUCTIONS_CHARACTERS: usize = 32_768;

/// The most earlier turns one request loads.
pub const MAX_EARLIER_TURNS: u32 = 100;

/// The owner's instructions: plain text that applies to every conversation.
/// Deserializing checks the length, so a stored document is valid.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Type)]
#[serde(transparent)]
pub struct Instructions(String);

impl Instructions {
    /// Checks the length of the text, in characters, as the owner counts it.
    pub fn parse(text: String) -> Result<Self, ChatError> {
        if text.chars().count() > MAX_INSTRUCTIONS_CHARACTERS {
            return Err(ChatError::InstructionsTooManyCharacters {
                max_characters: limit(MAX_INSTRUCTIONS_CHARACTERS),
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

/// Checks the text of a message: not longer than [`MAX_MESSAGE_BYTES`], and
/// not blank unless the message has images (ADR-0033, decision 1).
pub fn check_message(text: &str, has_images: bool) -> Result<(), ChatError> {
    if text.trim().is_empty() && !has_images {
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
// Other crates build it with `new` and the `with_` methods, so a new field
// does not change every caller (ADR-0034, decision 16). A plain comment keeps
// this out of the TypeScript bindings.
#[non_exhaustive]
pub struct StartConversation {
    /// The message.
    pub text: String,
    /// The provider profile to use, or `None` for the default profile.
    pub profile_id: Option<ProfileId>,
    /// The project the conversation starts in, or `None` for a general
    /// conversation.
    pub project_id: Option<ProjectId>,
    /// What the message mentions, in the order of its text (ADR-0032).
    #[serde(default)]
    pub mentions: Vec<MentionRef>,
    /// The images of the message, in the order the owner added them
    /// (ADR-0033).
    #[serde(default)]
    pub images: Vec<ImageRef>,
    /// What the conversation starts with attached, such as a notebook, in
    /// the order the owner attached them (ADR-0046).
    #[serde(default)]
    pub attachments: Vec<AttachmentRef>,
    /// The attachment that the conversation belongs to, such as the notebook
    /// of a notebook's chat, which it starts with attached first (ADR-0051).
    /// A conversation with a home has no project.
    #[serde(default)]
    pub home: Option<AttachmentRef>,
}

impl StartConversation {
    /// The first message of a general conversation with the default profile,
    /// with no mentions, no images, nothing attached and no home.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            profile_id: None,
            project_id: None,
            mentions: Vec::new(),
            images: Vec::new(),
            attachments: Vec::new(),
            home: None,
        }
    }

    /// Sets the attachment that the conversation belongs to.
    #[must_use]
    pub fn with_home(mut self, home: Option<AttachmentRef>) -> Self {
        self.home = home;
        self
    }

    /// Sets the project the conversation starts in, or `None` for a general
    /// conversation.
    #[must_use]
    pub fn with_project(mut self, project_id: Option<ProjectId>) -> Self {
        self.project_id = project_id;
        self
    }

    /// Sets what the message mentions, in the order of its text.
    #[must_use]
    pub fn with_mentions(mut self, mentions: Vec<MentionRef>) -> Self {
        self.mentions = mentions;
        self
    }

    /// Sets the images of the message, in the order the owner added them.
    #[must_use]
    pub fn with_images(mut self, images: Vec<ImageRef>) -> Self {
        self.images = images;
        self
    }

    /// Sets what the conversation starts with attached, in the order the
    /// owner attached them.
    #[must_use]
    pub fn with_attachments(mut self, attachments: Vec<AttachmentRef>) -> Self {
        self.attachments = attachments;
        self
    }
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
// Other crates build it with `new` and the `with_` methods, so a new field
// does not change every caller (ADR-0034, decision 16).
#[non_exhaustive]
pub struct SendMessage {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The message.
    pub text: String,
    /// What the message mentions, in the order of its text (ADR-0032).
    #[serde(default)]
    pub mentions: Vec<MentionRef>,
    /// The images of the message, in the order the owner added them
    /// (ADR-0033).
    #[serde(default)]
    pub images: Vec<ImageRef>,
}

impl SendMessage {
    /// A message in a conversation, with no mentions and no images.
    pub fn new(conversation_id: ConversationId, text: impl Into<String>) -> Self {
        Self {
            conversation_id,
            text: text.into(),
            mentions: Vec::new(),
            images: Vec::new(),
        }
    }

    /// Sets what the message mentions, in the order of its text.
    #[must_use]
    pub fn with_mentions(mut self, mentions: Vec<MentionRef>) -> Self {
        self.mentions = mentions;
        self
    }

    /// Sets the images of the message, in the order the owner added them.
    #[must_use]
    pub fn with_images(mut self, images: Vec<ImageRef>) -> Self {
        self.images = images;
        self
    }
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
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointRef {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The checkpoint.
    pub checkpoint_id: CheckpointId,
}

/// The argument of `load_earlier_turns`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
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
    /// The instructions were longer than Mannie accepted, in bytes.
    /// Deprecated: the limit counts characters now, so the core no longer
    /// returns this; it stays until a later minor release removes it
    /// (ADR-0002).
    #[error("the instructions are longer than {max_bytes} bytes")]
    InstructionsTooLong {
        /// The limit.
        max_bytes: u32,
    },
    /// The instructions are longer than Mannie accepts.
    #[error("the instructions are longer than {max_characters} characters")]
    InstructionsTooManyCharacters {
        /// The limit.
        max_characters: u32,
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
    /// The project does not exist, for example because it was removed.
    #[error("the project does not exist")]
    ProjectNotFound,
    /// The project's name is empty or longer than Mannie accepts.
    #[error("a project's name has 1 to {max_characters} characters")]
    ProjectNameInvalid {
        /// The limit.
        max_characters: u32,
    },
    /// A conversation of the project is starting a reply or being compacted,
    /// so the project cannot be removed until that ends.
    #[error("a conversation of the project is busy")]
    ProjectBusy,
    /// The project has a different number of conversations than the owner
    /// confirmed to delete, so nothing was removed.
    #[error("the project has {conversations} conversations now")]
    ProjectChanged {
        /// The number of its conversations now.
        conversations: u32,
    },
    /// The folder's path was refused before any access to the file system.
    #[error("the path is not allowed: {reason:?}")]
    PathRefused {
        /// Why.
        reason: PathRefusal,
    },
    /// The folder does not exist.
    #[error("the folder does not exist")]
    FolderNotFound,
    /// The path is not a folder.
    #[error("the path is not a folder")]
    NotAFolder,
    /// The folder could not be opened or resolved.
    #[error("the folder could not be read: {kind}")]
    FolderUnreadable {
        /// What failed, in English, for logs.
        kind: String,
    },
    /// The system's folder dialog failed.
    #[error("the folder dialog failed")]
    FolderDialogFailed,
    /// The conversation is a child agent's, which its parent conversation
    /// directs: the owner can stop it, and nothing else (docs/features/
    /// child-agents/tasks.md, task conversations).
    #[error("the conversation belongs to a child agent")]
    TaskConversation,
    /// The conversation belongs to a task on a worker that cannot be reached
    /// now (plugins/workers/docs/delegation.md).
    #[error("the worker of this conversation cannot be reached")]
    WorkerUnreachable,
    /// The conversation belongs to a task whose target, such as a worker,
    /// cannot be reached now (ADR-0031, decision 6).
    #[error("the target of this task conversation cannot be reached")]
    TaskTargetUnreachable {
        /// The executor's kind: its plugin's ID.
        kind: String,
        /// The target's name.
        target: String,
    },
    /// The conversation belongs to a task of a plugin that is off (ADR-0030,
    /// decision 12).
    #[error("the plugin of this conversation is off")]
    PluginOff {
        /// The plugin's ID.
        plugin: String,
    },
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
    /// The message mentions more than Mannie sends with one message.
    #[error("a message mentions at most {max} things")]
    TooManyMentions {
        /// The limit.
        max: u32,
    },
    /// A mention's provider is not there, for example because its plugin is
    /// off, or it is not offered in this conversation (ADR-0032).
    #[error("the mentions of {provider} are not available here")]
    MentionUnavailable {
        /// The provider's ID.
        provider: String,
    },
    /// A mention's provider refused what the mention names.
    #[error("{provider} refused a mention: {reason}")]
    MentionRefused {
        /// The provider's ID.
        provider: String,
        /// The provider's code for why, such as `outside_workspace`.
        reason: String,
    },
    /// The message has more images than Mannie sends with one message.
    #[error("a message has at most {max} images")]
    TooManyImages {
        /// The limit.
        max: u32,
    },
    /// An image's source is not there, for example because its plugin is off
    /// (ADR-0033).
    #[error("the images of {image_source} are not available")]
    ImageUnavailable {
        /// The source's ID.
        image_source: String,
    },
    /// An image's source refused its key, or returned an image outside the
    /// core's limits.
    #[error("{image_source} refused an image: {reason}")]
    ImageRefused {
        /// The source's ID.
        image_source: String,
        /// The source's code for why, such as `not_found`, or
        /// `invalid_image` from the core's checks.
        reason: String,
    },
    /// The image does not exist in the conversation, for example because the
    /// conversation was deleted.
    #[error("the image does not exist")]
    ImageNotFound,
    /// The conversation holds as many attachments as Mannie keeps (ADR-0046).
    #[error("a conversation has at most {max} attachments")]
    TooManyAttachments {
        /// The limit.
        max: u32,
    },
    /// An attachment's context provider is not there, for example because
    /// its plugin is off (ADR-0046).
    #[error("the context of {provider} is not available")]
    ContextUnavailable {
        /// The provider's ID.
        provider: String,
    },
    /// An attachment's context provider refused its key.
    #[error("{provider} refused an attachment: {reason}")]
    ContextRefused {
        /// The provider's ID.
        provider: String,
        /// The provider's code for why, such as `notebook_not_found`, or
        /// `invalid_key` from the core's checks.
        reason: String,
    },
    /// The conversation belongs to an attachment, such as a notebook, so it
    /// has no project and cannot move into one (ADR-0051).
    #[error("a conversation that belongs to an attachment has no project")]
    HomeConversation,
    /// The attachment is the conversation's home, which stays attached.
    #[error("the attachment that the conversation belongs to cannot be detached")]
    HomeAttachment,
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
            Self::InstructionsTooManyCharacters { .. } => "chat.instructions_too_many_characters",
            Self::ConversationNotFound => "chat.conversation_not_found",
            Self::CheckpointNotFound => "chat.checkpoint_not_found",
            Self::RunActive => "chat.run_active",
            Self::NothingToRetry => "chat.nothing_to_retry",
            Self::NothingToCompact => "chat.nothing_to_compact",
            Self::Compacting => "chat.compacting",
            Self::CompactionFailed { .. } => "chat.compaction_failed",
            Self::NoProfile => "chat.no_profile",
            Self::ProfileNotFound => "chat.profile_not_found",
            Self::ProjectNotFound => "chat.project_not_found",
            Self::ProjectNameInvalid { .. } => "chat.project_name_invalid",
            Self::ProjectBusy => "chat.project_busy",
            Self::ProjectChanged { .. } => "chat.project_changed",
            Self::PathRefused { .. } => "chat.path_refused",
            Self::FolderNotFound => "chat.folder_not_found",
            Self::NotAFolder => "chat.not_a_folder",
            Self::FolderUnreadable { .. } => "chat.folder_unreadable",
            Self::FolderDialogFailed => "chat.folder_dialog_failed",
            Self::TaskConversation => "chat.task_conversation",
            Self::WorkerUnreachable => "chat.worker_unreachable",
            Self::TaskTargetUnreachable { .. } => "chat.task_target_unreachable",
            Self::PluginOff { .. } => "chat.plugin_off",
            Self::WrittenByNewerVersion => "chat.written_by_newer_version",
            Self::StorageFailed => "chat.storage_failed",
            Self::StoreUnavailable => "chat.store_unavailable",
            Self::ShuttingDown => "chat.shutting_down",
            Self::TooManyMentions { .. } => "chat.too_many_mentions",
            Self::MentionUnavailable { .. } => "chat.mention_unavailable",
            Self::MentionRefused { .. } => "chat.mention_refused",
            Self::TooManyImages { .. } => "chat.too_many_images",
            Self::ImageUnavailable { .. } => "chat.image_unavailable",
            Self::ImageRefused { .. } => "chat.image_refused",
            Self::ImageNotFound => "chat.image_not_found",
            Self::TooManyAttachments { .. } => "chat.too_many_attachments",
            Self::ContextUnavailable { .. } => "chat.context_unavailable",
            Self::ContextRefused { .. } => "chat.context_refused",
            Self::HomeConversation => "chat.home_conversation",
            Self::HomeAttachment => "chat.home_attachment",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChatError, ChatSettings, MAX_INSTRUCTIONS_CHARACTERS, MAX_MESSAGE_BYTES, SendMessage,
        StartConversation, UpdateChatSettings, check_message, check_title, title_from_message,
    };
    use crate::context::AttachmentRef;
    use crate::ids::{ConversationId, ProjectId};
    use crate::images::ImageRef;
    use crate::mentions::MentionRef;

    const CONVERSATION: &str = "0199e0f2-0000-7000-a000-000000000001";
    const PROJECT: &str = "0199e0f2-0000-7000-a000-000000000002";

    fn conversation() -> ConversationId {
        ConversationId::parse(CONVERSATION).unwrap()
    }

    fn project() -> ProjectId {
        ProjectId::parse(PROJECT).unwrap()
    }

    fn file_mention(key: &str) -> MentionRef {
        MentionRef {
            provider: "files".to_owned(),
            key: key.to_owned(),
        }
    }

    fn shelf_image(key: &str) -> ImageRef {
        ImageRef {
            source: "images".to_owned(),
            key: key.to_owned(),
        }
    }

    #[test]
    fn a_first_message_starts_a_general_conversation_with_nothing_attached() {
        assert_eq!(
            StartConversation::new("Hi"),
            StartConversation {
                text: "Hi".to_owned(),
                profile_id: None,
                project_id: None,
                mentions: Vec::new(),
                images: Vec::new(),
                attachments: Vec::new(),
                home: None,
            }
        );
        assert_eq!(
            SendMessage::new(conversation(), String::from("Again")),
            SendMessage {
                conversation_id: conversation(),
                text: "Again".to_owned(),
                mentions: Vec::new(),
                images: Vec::new(),
            }
        );
    }

    #[test]
    fn the_with_methods_set_the_project_mentions_and_images() {
        let start = StartConversation::new("Look at @notes.txt")
            .with_project(Some(project()))
            .with_mentions(vec![file_mention("notes.txt")])
            .with_images(vec![shelf_image("one"), shelf_image("two")]);
        assert_eq!(start.project_id, Some(project()));
        assert_eq!(start.mentions, vec![file_mention("notes.txt")]);
        assert_eq!(start.images, vec![shelf_image("one"), shelf_image("two")]);
        assert_eq!(start.profile_id, None);

        // A later call replaces what an earlier one set, so `None` makes the
        // conversation general again.
        let general = start.with_project(None).with_images(Vec::new());
        assert_eq!(
            general,
            StartConversation::new("Look at @notes.txt")
                .with_mentions(vec![file_mention("notes.txt")])
        );

        let message = SendMessage::new(conversation(), "And @plan.md")
            .with_mentions(vec![file_mention("plan.md")])
            .with_images(vec![shelf_image("three")]);
        assert_eq!(message.mentions, vec![file_mention("plan.md")]);
        assert_eq!(message.images, vec![shelf_image("three")]);
    }

    #[test]
    fn the_inputs_keep_their_ipc_shape() {
        let start: StartConversation = serde_json::from_value(serde_json::json!({
            "text": "Hi",
            "profileId": null,
            "projectId": null
        }))
        .unwrap();
        assert_eq!(start, StartConversation::new("Hi"));

        let attached: StartConversation = serde_json::from_value(serde_json::json!({
            "text": "Look at @notes.txt",
            "profileId": null,
            "projectId": PROJECT,
            "mentions": [{ "provider": "files", "key": "notes.txt" }],
            "images": [{ "source": "images", "key": "one" }],
            "attachments": [{ "provider": "notebooks", "key": "n1" }]
        }))
        .unwrap();
        assert_eq!(
            attached,
            StartConversation::new("Look at @notes.txt")
                .with_project(Some(project()))
                .with_mentions(vec![file_mention("notes.txt")])
                .with_images(vec![shelf_image("one")])
                .with_attachments(vec![AttachmentRef {
                    provider: "notebooks".to_owned(),
                    key: "n1".to_owned(),
                }])
        );

        let message: SendMessage = serde_json::from_value(serde_json::json!({
            "conversationId": CONVERSATION,
            "text": "Hi"
        }))
        .unwrap();
        assert_eq!(message, SendMessage::new(conversation(), "Hi"));

        let message_with_images: SendMessage = serde_json::from_value(serde_json::json!({
            "conversationId": CONVERSATION,
            "text": "",
            "mentions": [],
            "images": [{ "source": "images", "key": "two" }]
        }))
        .unwrap();
        assert_eq!(
            message_with_images,
            SendMessage::new(conversation(), "").with_images(vec![shelf_image("two")])
        );
    }

    #[test]
    fn inputs_without_their_text_or_with_an_invalid_id_are_refused() {
        serde_json::from_value::<StartConversation>(serde_json::json!({
            "profileId": null,
            "projectId": null
        }))
        .unwrap_err();
        serde_json::from_value::<StartConversation>(serde_json::json!({
            "text": "Hi",
            "profileId": null,
            "projectId": "not-a-project"
        }))
        .unwrap_err();
        serde_json::from_value::<SendMessage>(serde_json::json!({ "text": "Hi" })).unwrap_err();
        serde_json::from_value::<SendMessage>(serde_json::json!({
            "conversationId": "not-a-conversation",
            "text": "Hi"
        }))
        .unwrap_err();
    }

    #[test]
    fn messages_must_have_text_within_the_limit() {
        assert_eq!(check_message("Hello", false), Ok(()));
        assert_eq!(check_message(" \n\t", false), Err(ChatError::MessageEmpty));
        assert_eq!(
            check_message(&"a".repeat(MAX_MESSAGE_BYTES + 1), false),
            Err(ChatError::MessageTooLong { max_bytes: 262_144 })
        );
        assert_eq!(check_message(&"a".repeat(MAX_MESSAGE_BYTES), false), Ok(()));
    }

    #[test]
    fn a_message_with_images_may_have_no_text_but_not_too_much() {
        assert_eq!(check_message("", true), Ok(()));
        assert_eq!(check_message(" \n", true), Ok(()));
        assert_eq!(
            check_message(&"a".repeat(MAX_MESSAGE_BYTES + 1), true),
            Err(ChatError::MessageTooLong { max_bytes: 262_144 })
        );
    }

    #[test]
    fn image_errors_name_their_source_for_the_ui() {
        let refused = ChatError::ImageRefused {
            image_source: "images".to_owned(),
            reason: "not_found".to_owned(),
        };

        assert_eq!(
            serde_json::to_value(&refused).unwrap(),
            serde_json::json!({
                "code": "image_refused",
                "imageSource": "images",
                "reason": "not_found"
            })
        );
        assert_eq!(refused.code(), "chat.image_refused");
        assert_eq!(ChatError::ImageNotFound.code(), "chat.image_not_found");
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
            instructions: "a".repeat(MAX_INSTRUCTIONS_CHARACTERS + 1),
        }
        .into_settings();
        let stored_too_long = serde_json::from_value::<ChatSettings>(serde_json::json!({
            "instructions": "a".repeat(MAX_INSTRUCTIONS_CHARACTERS + 1)
        }));

        assert_eq!(valid.instructions.as_str(), "Answer in English.");
        assert_eq!(
            too_long,
            Err(ChatError::InstructionsTooManyCharacters {
                max_characters: 32_768
            })
        );
        stored_too_long.unwrap_err();
        assert_eq!(
            serde_json::to_value(&valid).unwrap(),
            serde_json::json!({ "instructions": "Answer in English." })
        );
    }

    #[test]
    fn instructions_are_counted_in_characters() {
        let at_the_limit = |text: &str| UpdateChatSettings {
            instructions: text.repeat(MAX_INSTRUCTIONS_CHARACTERS),
        };
        // 32,768 characters of two, three and four bytes each: all longer
        // than the 32 KiB that the limit was before.
        for text in ["é", "語", "🙂"] {
            let settings = at_the_limit(text).into_settings().unwrap();
            assert!(settings.instructions.as_str().len() > 32 * 1024, "{text}");
        }
        let one_more = UpdateChatSettings {
            instructions: format!("{}a", "🙂".repeat(MAX_INSTRUCTIONS_CHARACTERS)),
        };
        assert_eq!(
            one_more.into_settings(),
            Err(ChatError::InstructionsTooManyCharacters {
                max_characters: 32_768
            })
        );
        assert_eq!(
            serde_json::to_value(ChatError::InstructionsTooManyCharacters {
                max_characters: 32_768
            })
            .unwrap(),
            serde_json::json!({ "code": "instructions_too_many_characters", "maxCharacters": 32_768 })
        );
    }

    #[test]
    fn errors_serialize_with_their_code() {
        assert_eq!(
            serde_json::to_value(ChatError::RunActive).unwrap(),
            serde_json::json!({ "code": "run_active" })
        );
        assert_eq!(ChatError::RunActive.code(), "chat.run_active");
        assert_eq!(
            serde_json::to_value(ChatError::PathRefused {
                reason: crate::tools::PathRefusal::NetworkPath
            })
            .unwrap(),
            serde_json::json!({ "code": "path_refused", "reason": "network_path" })
        );
    }
}
