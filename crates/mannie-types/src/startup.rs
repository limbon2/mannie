//! How the desktop app's start went: the shell asks once, before it renders
//! anything, and shows a problem instead of the views when the core could not
//! start.

use serde::Serialize;
use specta::Type;

/// The result of starting the core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StartupStatus {
    /// What stopped the core from starting, or `None` when it started.
    pub problem: Option<StartupProblem>,
}

/// Why the core could not start. The UI shows the message for the code from
/// its catalog, with what the owner can do about it (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum StartupProblem {
    /// A newer version of Mannie wrote the database. It was left untouched.
    DatabaseTooNew {
        /// The database file.
        path: String,
        /// The schema version of the database.
        database_version: u32,
        /// The newest schema version this version of Mannie supports.
        supported_version: u32,
    },
    /// A migration failed and was rolled back. A copy of the database from
    /// before the migration is in the backup folder.
    MigrationFailed {
        /// The database file.
        path: String,
        /// The schema version the failed migration leads to.
        version: u32,
        /// The folder with the copies made before migrations.
        backup_directory: String,
    },
    /// The database could not be opened, for example because of its
    /// permissions, a full disk or damage.
    DatabaseUnavailable {
        /// The database file.
        path: String,
    },
    /// The folder for Mannie's data could not be determined.
    DataDirectoryUnavailable,
    /// The HTTP client for model providers could not be set up, for example
    /// because the system's certificates could not be loaded.
    HttpClientUnavailable,
}
