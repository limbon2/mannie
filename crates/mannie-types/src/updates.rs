//! The contracts of desktop updates (docs/features/updates.md, ADR-0063): the
//! settings section, the status that the core sends to the window, the
//! arguments and answers of the commands, and their errors.
//!
//! The core sends codes and versions only. The window takes every text from
//! its message catalog (ADR-0019).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::clock::UtcMillis;
use crate::settings::SettingsSection;
use crate::transcript::SubscriptionEnd;

/// The updates settings section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdatesSettings {
    /// Whether Mannie checks for updates on its own: one minute after it
    /// starts, then once a day. On by default (D55), also in a section that
    /// was stored without the field.
    #[serde(default = "check_by_default")]
    pub check_automatically: bool,
}

/// Automatic checks are on unless the owner turns them off.
const fn check_by_default() -> bool {
    true
}

impl Default for UpdatesSettings {
    fn default() -> Self {
        Self {
            check_automatically: check_by_default(),
        }
    }
}

impl SettingsSection for UpdatesSettings {
    const KEY: &'static str = "updates";
    const VERSION: u32 = 1;
}

/// The argument of `update_updates_settings`: the complete new section.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUpdatesSettings {
    /// Whether Mannie checks for updates on its own.
    pub check_automatically: bool,
}

/// Why this build does not update itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum UpdatesOffReason {
    /// A development build, which the developer rebuilds.
    DevelopmentBuild,
    /// A release build that no installer or package put in place, so there is
    /// no installation that an installer could replace.
    NotInstalledFromPackage,
}

/// Why a check or a download failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum UpdateFailureCode {
    /// The releases could not be reached.
    Unreachable,
    /// The latest release has no update information, or information that this
    /// version cannot read.
    NoUpdateInformation,
    /// The latest release has no installer for the way this Mannie was
    /// installed.
    NoPackageForThisInstallation,
    /// The installer could not be downloaded.
    DownloadFailed,
    /// The installer was not signed by Mannie's release key for the version
    /// that the release names, so Mannie dropped it.
    SignatureInvalid,
}

impl UpdateFailureCode {
    /// The stable code of the failure, for logs.
    pub fn code(self) -> &'static str {
        match self {
            Self::Unreachable => "unreachable",
            Self::NoUpdateInformation => "no_update_information",
            Self::NoPackageForThisInstallation => "no_package_for_this_installation",
            Self::DownloadFailed => "download_failed",
            Self::SignatureInvalid => "signature_invalid",
        }
    }
}

/// What the updates of this Mannie are doing.
///
/// The byte counts fit `u32`, because the core stops a download at its size
/// limit, far below 4 GiB.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum UpdateState {
    /// This build does not update itself.
    Off {
        /// Why.
        reason: UpdatesOffReason,
    },
    /// Nothing is running and nothing is ready.
    Idle {
        /// When the last check ended that found this version to be the
        /// latest, or nothing when no check ended since Mannie started.
        checked_at: Option<UtcMillis>,
    },
    /// A check is reading the latest release.
    Checking,
    /// The installer of a newer version is arriving.
    Downloading {
        /// The version that the installer installs.
        version: String,
        /// How many bytes arrived.
        received_bytes: u32,
        /// The installer's size, when the server names it.
        total_bytes: Option<u32>,
    },
    /// A verified installer waits for the owner's "Restart and update".
    Ready {
        /// The version that the installer installs.
        version: String,
    },
    /// The last check or download failed, and no installer is ready.
    Failed {
        /// Why.
        code: UpdateFailureCode,
        /// When it failed.
        checked_at: UtcMillis,
    },
}

/// How an install ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum UpdateResult {
    /// The version that started after the install is the one that was to be
    /// installed.
    Completed,
    /// Another version started after the install.
    Failed,
}

/// The result of the last install, which Mannie tells the owner once
/// (ADR-0063, decision 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOutcome {
    /// The version that ran when the owner chose "Restart and update".
    pub from_version: String,
    /// The version that was to be installed.
    pub to_version: String,
    /// How the install ended.
    pub result: UpdateResult,
}

/// Everything the window shows about updates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// The version that is running.
    pub current_version: String,
    /// What the updates are doing.
    pub state: UpdateState,
    /// The result of the last install, until the owner closes its notice.
    pub last_outcome: Option<UpdateOutcome>,
    /// Whether the system asks for the owner's password during an install,
    /// as it does for a package on Linux.
    pub needs_system_password: bool,
}

/// A message of the updates channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum UpdateStatusUpdate {
    /// The status: as it is first, then after each change.
    Status {
        /// The status.
        status: UpdateStatus,
    },
    /// The subscription ended, and no message follows.
    Ended {
        /// Why.
        reason: SubscriptionEnd,
    },
}

/// The argument of `restart_to_update`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RestartToUpdate {
    /// The version that the owner saw as ready. The core installs nothing
    /// when another version is ready by now.
    pub version: String,
    /// Whether the owner already confirmed that the restart may end what is
    /// running.
    pub confirmed: bool,
}

/// The answer of `restart_to_update`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RestartToUpdateAnswer {
    /// The restart would end work that is running, so the owner confirms it
    /// first. Nothing changed.
    ConfirmFirst {
        /// How many runs the restart would interrupt.
        active_runs: u32,
        /// How many background commands the restart would end.
        background_commands: u32,
        /// How many terminals run a command that the restart would end.
        terminals: u32,
    },
    /// Mannie quits, installs the version and starts again.
    Restarting,
}

/// A page of Mannie's releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ReleasePage {
    /// The release of the version that is ready to install.
    Ready,
    /// The release of the version that is running.
    Running,
    /// The latest release.
    Latest,
}

/// The argument of `open_release_page`. The window names a page and never an
/// address: the core builds the address from a version that it holds.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenReleasePage {
    /// The page to open in the system browser.
    pub page: ReleasePage,
}

/// Why an updates command failed. The UI shows the message for the code from
/// its catalog (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum UpdatesError {
    /// This build does not update itself.
    #[error("this build does not update itself")]
    NotAvailableInThisBuild,
    /// The named version is not the one that is ready to install.
    #[error("the named version is not ready to install")]
    VersionNotReady,
    /// The settings were written by a newer version of Mannie, which this
    /// version does not overwrite.
    #[error("the updates settings were written by a newer version of Mannie")]
    WrittenByNewerVersion,
    /// The store failed. The log has the cause.
    #[error("the updates settings or the record of the install could not be read or saved")]
    StorageFailed,
    /// The core started without its database.
    #[error("the database is not open")]
    StoreUnavailable,
    /// The system browser did not open the release page.
    #[error("the system browser could not be opened")]
    BrowserUnavailable,
}

impl UpdatesError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotAvailableInThisBuild => "updates.not_available_in_this_build",
            Self::VersionNotReady => "updates.version_not_ready",
            Self::WrittenByNewerVersion => "updates.written_by_newer_version",
            Self::StorageFailed => "updates.storage_failed",
            Self::StoreUnavailable => "updates.store_unavailable",
            Self::BrowserUnavailable => "updates.browser_unavailable",
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        RestartToUpdateAnswer, UpdateFailureCode, UpdateOutcome, UpdateResult, UpdateState,
        UpdateStatus, UpdateStatusUpdate, UpdatesError, UpdatesOffReason, UpdatesSettings,
    };
    use crate::clock::UtcMillis;
    use crate::transcript::SubscriptionEnd;

    #[test]
    fn automatic_checks_are_on_by_default_and_in_a_section_without_the_field() {
        let stored_empty: UpdatesSettings = serde_json::from_value(json!({})).unwrap();
        let turned_off: UpdatesSettings =
            serde_json::from_value(json!({ "checkAutomatically": false })).unwrap();

        assert!(UpdatesSettings::default().check_automatically);
        assert!(stored_empty.check_automatically);
        assert!(!turned_off.check_automatically);
        assert_eq!(
            serde_json::to_value(UpdatesSettings::default()).unwrap(),
            json!({ "checkAutomatically": true })
        );
    }

    #[test]
    fn the_status_serializes_with_the_kind_of_its_state() {
        let status = UpdateStatus {
            current_version: "0.6.1".to_owned(),
            state: UpdateState::Downloading {
                version: "0.7.0".to_owned(),
                received_bytes: 1_048_576,
                total_bytes: None,
            },
            last_outcome: Some(UpdateOutcome {
                from_version: "0.6.0".to_owned(),
                to_version: "0.6.1".to_owned(),
                result: UpdateResult::Completed,
            }),
            needs_system_password: true,
        };

        assert_eq!(
            serde_json::to_value(UpdateStatusUpdate::Status { status }).unwrap(),
            json!({
                "type": "status",
                "status": {
                    "currentVersion": "0.6.1",
                    "state": {
                        "kind": "downloading",
                        "version": "0.7.0",
                        "receivedBytes": 1_048_576,
                        "totalBytes": null,
                    },
                    "lastOutcome": {
                        "fromVersion": "0.6.0",
                        "toVersion": "0.6.1",
                        "result": "completed",
                    },
                    "needsSystemPassword": true,
                },
            })
        );
        assert_eq!(
            serde_json::to_value(UpdateStatusUpdate::Ended {
                reason: SubscriptionEnd::Shutdown,
            })
            .unwrap(),
            json!({ "type": "ended", "reason": "shutdown" })
        );
    }

    #[test]
    fn states_carry_reasons_codes_and_times() {
        assert_eq!(
            serde_json::to_value(UpdateState::Off {
                reason: UpdatesOffReason::NotInstalledFromPackage,
            })
            .unwrap(),
            json!({ "kind": "off", "reason": "not_installed_from_package" })
        );
        assert_eq!(
            serde_json::to_value(UpdateState::Idle { checked_at: None }).unwrap(),
            json!({ "kind": "idle", "checkedAt": null })
        );
        assert_eq!(
            serde_json::to_value(UpdateState::Failed {
                code: UpdateFailureCode::NoPackageForThisInstallation,
                checked_at: UtcMillis::from_millis(1_794_000_000_000),
            })
            .unwrap(),
            json!({
                "kind": "failed",
                "code": "no_package_for_this_installation",
                "checkedAt": 1_794_000_000_000_i64,
            })
        );
    }

    #[test]
    fn a_failure_code_logs_as_it_serializes() {
        for code in [
            UpdateFailureCode::Unreachable,
            UpdateFailureCode::NoUpdateInformation,
            UpdateFailureCode::NoPackageForThisInstallation,
            UpdateFailureCode::DownloadFailed,
            UpdateFailureCode::SignatureInvalid,
        ] {
            assert_eq!(serde_json::to_value(code).unwrap(), json!(code.code()));
        }
    }

    #[test]
    fn the_answer_to_a_restart_names_what_runs() {
        assert_eq!(
            serde_json::to_value(RestartToUpdateAnswer::ConfirmFirst {
                active_runs: 1,
                background_commands: 2,
                terminals: 0,
            })
            .unwrap(),
            json!({
                "kind": "confirm_first",
                "activeRuns": 1,
                "backgroundCommands": 2,
                "terminals": 0,
            })
        );
        assert_eq!(
            serde_json::to_value(RestartToUpdateAnswer::Restarting).unwrap(),
            json!({ "kind": "restarting" })
        );
    }

    #[test]
    fn an_error_serializes_with_the_code_that_its_log_code_ends_with() {
        for error in [
            UpdatesError::NotAvailableInThisBuild,
            UpdatesError::VersionNotReady,
            UpdatesError::WrittenByNewerVersion,
            UpdatesError::StorageFailed,
            UpdatesError::StoreUnavailable,
            UpdatesError::BrowserUnavailable,
        ] {
            let serialized = serde_json::to_value(&error).unwrap();
            let code = serialized["code"].as_str().unwrap();

            assert_eq!(error.code(), format!("updates.{code}"));
        }
    }
}
