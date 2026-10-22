//! The window and tray feature's contracts (docs/features/window-and-tray.md):
//! the settings section, the questions the core asks the window, and the
//! labels of the tray's menu.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::settings::SettingsSection;

/// The longest label of the tray, in characters.
pub const MAX_TRAY_LABEL_CHARACTERS: usize = 100;

/// What closing the main window does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CloseAction {
    /// Ask on the next close.
    #[default]
    Ask,
    /// Hide the window and keep running in the system tray.
    Tray,
    /// Quit, after a warning when runs are active.
    Quit,
}

/// The window settings section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WindowSettings {
    /// What closing the main window does.
    pub close_action: CloseAction,
}

impl SettingsSection for WindowSettings {
    const KEY: &'static str = "window";
    const VERSION: u32 = 1;
}

/// Where a request to quit came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum QuitSource {
    /// The window's close button, in quit mode.
    Window,
    /// The tray menu's Quit.
    Tray,
}

/// A question the core asks through the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WindowRequest {
    /// The owner closed the window for the first time: keep running in the
    /// tray, or quit?
    AskCloseAction {
        /// How many runs quitting would interrupt.
        active_runs: u32,
        /// How many background commands quitting would end (ADR-0041).
        background_commands: u32,
    },
    /// Quitting would interrupt active runs or end background commands: quit
    /// anyway, or cancel?
    ConfirmQuit {
        /// How many runs quitting would interrupt.
        active_runs: u32,
        /// How many background commands quitting would end (ADR-0041).
        background_commands: u32,
        /// What asked to quit.
        source: QuitSource,
    },
}

/// A message of the window requests channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WindowRequestUpdate {
    /// A question waits for the owner's answer.
    Pending {
        /// Identifies the question in the answer.
        request_id: u32,
        /// The question.
        request: WindowRequest,
    },
    /// The question no longer waits, for example because it was answered.
    Cleared {
        /// The question.
        request_id: u32,
    },
}

/// The owner's answer to the first-close question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CloseChoice {
    /// Keep running in the system tray.
    Tray,
    /// Quit when the window closes.
    Quit,
}

impl From<CloseChoice> for CloseAction {
    fn from(choice: CloseChoice) -> Self {
        match choice {
            CloseChoice::Tray => Self::Tray,
            CloseChoice::Quit => Self::Quit,
        }
    }
}

/// The argument of `answer_close_question`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AnswerCloseQuestion {
    /// The question being answered.
    pub request_id: u32,
    /// The owner's choice.
    pub choice: CloseChoice,
}

/// The argument of `confirm_quit` and `dismiss_window_request`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WindowRequestRef {
    /// The question.
    pub request_id: u32,
}

/// The argument of `update_window_settings`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWindowSettings {
    /// What closing the main window does.
    pub close_action: CloseAction,
}

/// The labels of the tray, from the window's message catalog (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TrayLabels {
    /// The menu item that shows the window.
    pub open: String,
    /// The menu item that quits.
    pub quit: String,
    /// The tooltip of the icon.
    pub tooltip: String,
}

impl TrayLabels {
    /// Checks each label: 1 to 100 characters without control characters.
    pub fn check(&self) -> Result<(), WindowError> {
        for label in [&self.open, &self.quit, &self.tooltip] {
            let is_valid = !label.trim().is_empty()
                && label.chars().count() <= MAX_TRAY_LABEL_CHARACTERS
                && !label.chars().any(char::is_control);
            if !is_valid {
                return Err(WindowError::InvalidLabel);
            }
        }
        Ok(())
    }
}

/// Why a window command failed. The UI shows the message for the code from
/// its catalog (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WindowError {
    /// A tray label is empty, too long or has control characters.
    #[error("a tray label is not valid")]
    InvalidLabel,
    /// The question was already answered or withdrawn.
    #[error("the question no longer waits for an answer")]
    RequestNotPending,
    /// The settings were written by a newer version of Mannie, which this
    /// version does not overwrite.
    #[error("the window settings were written by a newer version of Mannie")]
    WrittenByNewerVersion,
    /// The store failed. The log has the cause.
    #[error("the window settings could not be read or saved")]
    StorageFailed,
    /// The core started without its database.
    #[error("the database is not open")]
    StoreUnavailable,
}

impl WindowError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidLabel => "window.invalid_label",
            Self::RequestNotPending => "window.request_not_pending",
            Self::WrittenByNewerVersion => "window.written_by_newer_version",
            Self::StorageFailed => "window.storage_failed",
            Self::StoreUnavailable => "window.store_unavailable",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CloseAction, CloseChoice, TrayLabels, WindowError, WindowRequest, WindowRequestUpdate,
        WindowSettings,
    };

    fn labels() -> TrayLabels {
        TrayLabels {
            open: "Open Mannie".to_owned(),
            quit: "Quit".to_owned(),
            tooltip: "Mannie".to_owned(),
        }
    }

    #[test]
    fn the_default_asks_on_the_first_close() {
        assert_eq!(WindowSettings::default().close_action, CloseAction::Ask);
        assert_eq!(
            serde_json::to_value(WindowSettings::default()).unwrap(),
            serde_json::json!({ "closeAction": "ask" })
        );
        assert_eq!(CloseAction::from(CloseChoice::Tray), CloseAction::Tray);
    }

    #[test]
    fn labels_are_checked() {
        let blank = TrayLabels {
            quit: " ".to_owned(),
            ..labels()
        };
        let long = TrayLabels {
            open: "o".repeat(101),
            ..labels()
        };
        let control = TrayLabels {
            tooltip: "Man\nnie".to_owned(),
            ..labels()
        };

        assert_eq!(labels().check(), Ok(()));
        for invalid in [blank, long, control] {
            assert_eq!(invalid.check(), Err(WindowError::InvalidLabel));
        }
    }

    #[test]
    fn requests_serialize_with_their_kind() {
        let update = WindowRequestUpdate::Pending {
            request_id: 3,
            request: WindowRequest::AskCloseAction {
                active_runs: 2,
                background_commands: 1,
            },
        };

        assert_eq!(
            serde_json::to_value(update).unwrap(),
            serde_json::json!({
                "type": "pending",
                "requestId": 3,
                "request": {
                    "kind": "ask_close_action",
                    "activeRuns": 2,
                    "backgroundCommands": 1,
                },
            })
        );
    }
}
