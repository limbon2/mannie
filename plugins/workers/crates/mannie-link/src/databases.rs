//! The documents of the link's package `databases` (ADR-0057;
//! plugins/workers/docs/databases.md): what the master asks of the
//! databases that the owner registered on a worker's host, and what the
//! worker answers, on the plugin SDK's types of the service `sqlite-files`.
//! The worker writes its answers, so the master treats them as content.

use mannie_plugin_sdk::contracts::HostDatabaseRefusal;
use mannie_plugin_sdk::services::sqlite_files::{
    ChangeOutcome, DatabaseFile, SqliteFilesError, StatementOutcome,
};
use serde::{Deserialize, Serialize};

/// A file that the owner typed on the master, to register on the worker's
/// host (ADR-0057, decision 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterDatabase {
    /// The file's absolute path on the host.
    pub path: String,
    /// The owner's name for the database, or `None` for the file's name.
    #[serde(default)]
    pub name: Option<String>,
    /// Whether the owner marks it read-only.
    #[serde(default)]
    pub read_only: bool,
}

/// A database on the worker's host, as the worker lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostDatabase {
    /// The registration, with what the worker finds at its path.
    pub file: DatabaseFile,
    /// The file's identity, which the master's accesses and evidence name.
    pub identity: String,
}

/// A change of a registration on the worker's host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum UpdateDatabase {
    /// Gives the database another name.
    Rename {
        /// The owner's new name for it.
        name: String,
    },
    /// Marks the database read-only, or lifts the mark.
    Mark {
        /// Whether it is read-only.
        read_only: bool,
    },
    /// Accepts the file that is at the registration's path now.
    Accept,
}

/// What a write did on the worker's host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WriteDone {
    /// The statements of a call of the master's tool, each with its outcome.
    Statements {
        /// The outcomes, in the statements' order.
        outcomes: Vec<StatementOutcome>,
    },
    /// The owner's change.
    Change {
        /// What it did.
        outcome: ChangeOutcome,
    },
}

/// Why the worker did not do a request on one of its databases, as the
/// JSON of its refusal carries it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DatabaseRefusal {
    /// The host's ceiling refuses it, whatever the master sends.
    Refused {
        /// Why.
        refusal: HostDatabaseRefusal,
    },
    /// The owner decides first, through the master: the write runs with the
    /// owner's Allow once of this digest, which the worker computed.
    NeedsOwner {
        /// The digest of the write.
        digest: String,
    },
    /// The worker's engine, registry or journal refused it or failed.
    Files {
        /// The service's error.
        error: SqliteFilesError,
    },
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::HostDatabaseRefusal;
    use mannie_plugin_sdk::services::sqlite_files::SqliteFilesError;

    use super::{DatabaseRefusal, UpdateDatabase};

    #[test]
    fn a_refusal_names_its_code_and_carries_the_services_error_whole() {
        assert_eq!(
            serde_json::to_value(DatabaseRefusal::Files {
                error: SqliteFilesError::Busy
            })
            .unwrap(),
            serde_json::json!({ "code": "files", "error": { "code": "busy" } })
        );
        assert_eq!(
            serde_json::to_value(DatabaseRefusal::Refused {
                refusal: HostDatabaseRefusal::OutsideAllowedFolders
            })
            .unwrap(),
            serde_json::json!({ "code": "refused", "refusal": "outside_allowed_folders" })
        );
        assert_eq!(
            serde_json::to_value(UpdateDatabase::Mark { read_only: true }).unwrap(),
            serde_json::json!({ "kind": "mark", "readOnly": true })
        );
    }
}
