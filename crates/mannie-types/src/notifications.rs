//! The notifications and tray badge feature's contracts
//! (docs/features/notifications.md; ADR-0074): settings, localized labels from
//! the window's message catalog, channel updates, and error codes.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::ConversationId;
use crate::settings::SettingsSection;

/// The longest localized label accepted for notifications or the attention
/// tray tooltip, in characters.
pub const MAX_NOTIFICATION_LABEL_CHARACTERS: usize = 100;

/// The longest conversation title shown in a desktop notification body, in
/// characters.
pub const MAX_NOTIFICATION_BODY_CHARACTERS: usize = 120;

const fn default_true() -> bool {
    true
}

/// Which kind of conversation event triggered a desktop notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    /// An agent asked the owner a question through `ask_owner`.
    Question,
    /// A tool call waits for the owner's approval.
    Approval,
    /// A run completed its reply.
    Completed,
    /// A run ended with a failure.
    Failed,
}

impl NotificationKind {
    /// The stable name of this kind, for logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Question => "question",
            Self::Approval => "approval",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// The notifications settings section (ADR-0074).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each switch is an independent persisted toggle of the notifications settings section"
)]
pub struct NotificationSettings {
    /// Master switch for desktop notifications.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Send a desktop notification when an agent asks a question.
    #[serde(default = "default_true")]
    pub on_question: bool,
    /// Send a desktop notification when a tool call asks for approval.
    #[serde(default = "default_true")]
    pub on_approval: bool,
    /// Send a desktop notification when a run completes.
    #[serde(default = "default_true")]
    pub on_completion: bool,
    /// Send a desktop notification when a run fails.
    #[serde(default = "default_true")]
    pub on_failure: bool,
    /// Show the attention count on the taskbar or dock icon while unread
    /// finished runs or pending questions or approvals exist.
    #[serde(default = "default_true")]
    pub taskbar_badge: bool,
    /// Show the badge on the system tray icon while unread finished runs or
    /// pending questions or approvals exist.
    #[serde(default = "default_true")]
    pub tray_badge: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            on_question: true,
            on_approval: true,
            on_completion: true,
            on_failure: true,
            taskbar_badge: true,
            tray_badge: true,
        }
    }
}

impl NotificationSettings {
    /// Whether these settings allow sending a desktop notification of `kind`.
    pub fn allows(self, kind: NotificationKind) -> bool {
        if !self.enabled {
            return false;
        }
        match kind {
            NotificationKind::Question => self.on_question,
            NotificationKind::Approval => self.on_approval,
            NotificationKind::Completed => self.on_completion,
            NotificationKind::Failed => self.on_failure,
        }
    }
}

impl SettingsSection for NotificationSettings {
    const KEY: &'static str = "notifications";
    const VERSION: u32 = 1;
}

/// The argument of `update_notification_settings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "mirrors the independent switches of NotificationSettings"
)]
pub struct UpdateNotificationSettings {
    /// Master switch for desktop notifications.
    pub enabled: bool,
    /// Send a desktop notification when an agent asks a question.
    pub on_question: bool,
    /// Send a desktop notification when a tool call asks for approval.
    pub on_approval: bool,
    /// Send a desktop notification when a run completes.
    pub on_completion: bool,
    /// Send a desktop notification when a run fails.
    pub on_failure: bool,
    /// Show the attention count on the taskbar or dock icon.
    pub taskbar_badge: bool,
    /// Show the badge on the system tray icon.
    pub tray_badge: bool,
}

impl From<UpdateNotificationSettings> for NotificationSettings {
    fn from(update: UpdateNotificationSettings) -> Self {
        Self {
            enabled: update.enabled,
            on_question: update.on_question,
            on_approval: update.on_approval,
            on_completion: update.on_completion,
            on_failure: update.on_failure,
            taskbar_badge: update.taskbar_badge,
            tray_badge: update.tray_badge,
        }
    }
}

/// Localized titles for desktop notifications and the badged tray tooltip,
/// sent by the window from its message catalog (ADR-0019, ADR-0074).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NotificationLabels {
    /// Notification title when an agent asks a question.
    pub question_title: String,
    /// Notification title when a tool call asks for approval.
    pub approval_title: String,
    /// Notification title when a run completes.
    pub completed_title: String,
    /// Notification title when a run fails.
    pub failed_title: String,
    /// Tray icon tooltip while the tray badge is active.
    pub tray_attention_tooltip: String,
}

impl NotificationLabels {
    /// Checks every label: 1 to 100 non-blank characters without control
    /// characters.
    pub fn check(&self) -> Result<(), NotificationError> {
        for label in [
            &self.question_title,
            &self.approval_title,
            &self.completed_title,
            &self.failed_title,
            &self.tray_attention_tooltip,
        ] {
            let is_valid = !label.trim().is_empty()
                && label.chars().count() <= MAX_NOTIFICATION_LABEL_CHARACTERS
                && !label.chars().any(char::is_control);

            if !is_valid {
                return Err(NotificationError::InvalidLabel);
            }
        }
        Ok(())
    }

    /// The localized notification title for `kind`.
    pub fn title_for(&self, kind: NotificationKind) -> &str {
        match kind {
            NotificationKind::Question => &self.question_title,
            NotificationKind::Approval => &self.approval_title,
            NotificationKind::Completed => &self.completed_title,
            NotificationKind::Failed => &self.failed_title,
        }
    }
}

/// The argument of `set_viewed_conversation`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SetViewedConversation {
    /// The conversation currently open in the main window, or `None` when
    /// another view or settings page is shown.
    pub conversation_id: Option<ConversationId>,
}

/// A message of the notifications channel from the core to the window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum NotificationUpdate {
    /// The complete set of unread finished conversations, sorted by ID.
    UnreadChanged {
        /// Conversations whose last completed or failed run has not been
        /// viewed yet.
        conversation_ids: Vec<ConversationId>,
    },
    /// The owner clicked a desktop notification for this conversation.
    OpenConversation {
        /// The conversation to open in the main view.
        conversation_id: ConversationId,
    },
}

/// Why a notifications command failed. The UI shows the message for the code
/// from its catalog (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum NotificationError {
    /// A notification label is empty, too long or has control characters.
    #[error("a notification label is not valid")]
    InvalidLabel,
    /// The settings were written by a newer version of Mannie, which this
    /// version does not overwrite.
    #[error("the notification settings were written by a newer version of Mannie")]
    WrittenByNewerVersion,
    /// The store failed. The log has the cause.
    #[error("the notification settings could not be read or saved")]
    StorageFailed,
    /// The core started without its database.
    #[error("the database is not open")]
    StoreUnavailable,
}

impl NotificationError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidLabel => "notifications.invalid_label",
            Self::WrittenByNewerVersion => "notifications.written_by_newer_version",
            Self::StorageFailed => "notifications.storage_failed",
            Self::StoreUnavailable => "notifications.store_unavailable",
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ids::ConversationId;

    use super::{
        NotificationError, NotificationKind, NotificationLabels, NotificationSettings,
        NotificationUpdate,
    };

    fn labels() -> NotificationLabels {
        NotificationLabels {
            question_title: "Waiting for your answer".to_owned(),
            approval_title: "Approval needed".to_owned(),
            completed_title: "Reply finished".to_owned(),
            failed_title: "Reply failed".to_owned(),
            tray_attention_tooltip: "Mannie (attention needed)".to_owned(),
        }
    }

    #[test]
    fn settings_default_to_all_enabled_and_fill_missing_fields() {
        let defaults = NotificationSettings::default();

        for kind in [
            NotificationKind::Question,
            NotificationKind::Approval,
            NotificationKind::Completed,
            NotificationKind::Failed,
        ] {
            assert!(defaults.allows(kind));
        }
        assert!(defaults.taskbar_badge);
        assert!(defaults.tray_badge);

        let partial: NotificationSettings =
            serde_json::from_value(serde_json::json!({ "onFailure": false })).unwrap();
        assert!(partial.enabled);
        assert!(partial.on_question);
        assert!(partial.on_approval);
        assert!(partial.on_completion);
        assert!(!partial.on_failure);
        assert!(partial.taskbar_badge);
        assert!(partial.tray_badge);
    }

    #[test]
    fn master_switch_overrides_per_event_switches() {
        let settings = NotificationSettings {
            enabled: false,
            ..NotificationSettings::default()
        };

        for kind in [
            NotificationKind::Question,
            NotificationKind::Approval,
            NotificationKind::Completed,
            NotificationKind::Failed,
        ] {
            assert!(!settings.allows(kind));
        }
    }

    #[test]
    fn labels_are_checked_and_mapped_by_kind() {
        let valid = labels();
        assert_eq!(valid.check(), Ok(()));
        assert_eq!(
            valid.title_for(NotificationKind::Question),
            "Waiting for your answer"
        );
        assert_eq!(
            valid.title_for(NotificationKind::Approval),
            "Approval needed"
        );
        assert_eq!(
            valid.title_for(NotificationKind::Completed),
            "Reply finished"
        );
        assert_eq!(valid.title_for(NotificationKind::Failed), "Reply failed");

        let blank = NotificationLabels {
            question_title: "  ".to_owned(),
            ..valid.clone()
        };
        let long = NotificationLabels {
            completed_title: "a".repeat(101),
            ..valid.clone()
        };
        let control = NotificationLabels {
            tray_attention_tooltip: "Mannie\nready".to_owned(),
            ..valid
        };

        for invalid in [blank, long, control] {
            assert_eq!(invalid.check(), Err(NotificationError::InvalidLabel));
        }
    }

    #[test]
    fn updates_serialize_with_their_type() {
        let id = ConversationId::parse("0199e0f2-7a1b-7c3d-8e4f-0123456789ab").unwrap();
        let unread = NotificationUpdate::UnreadChanged {
            conversation_ids: vec![id.clone()],
        };
        let open = NotificationUpdate::OpenConversation {
            conversation_id: id,
        };

        assert_eq!(
            serde_json::to_value(&unread).unwrap(),
            serde_json::json!({
                "type": "unread_changed",
                "conversationIds": ["0199e0f2-7a1b-7c3d-8e4f-0123456789ab"],
            })
        );
        assert_eq!(
            serde_json::to_value(&open).unwrap(),
            serde_json::json!({
                "type": "open_conversation",
                "conversationId": "0199e0f2-7a1b-7c3d-8e4f-0123456789ab",
            })
        );
    }
}
