//! Statements on the owner's SQLite files (ADR-0056): the class of a
//! statement, which the engine reads from the actions that SQLite reports
//! while it prepares the statement, and which a database access and its
//! policy name, and where a registered database lives. A write on a
//! worker's host carries what lets it run, which the worker checks against
//! its ceiling (ADR-0057).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ceiling::CeilingMode;
use crate::ids::{ConversationId, PartId, WorkerId};

use super::approval::GrantScope;

/// The most characters of one parameter's value as an access shows it.
pub const MAX_VIEW_PARAMETER_CHARS: usize = 200;

/// The most parameters that an access shows; the digest covers them all.
pub const MAX_VIEW_PARAMETERS: usize = 32;

/// Where a registered database lives (ADR-0056, decision 5).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DatabaseHome {
    /// This computer, whose registrations the core keeps.
    ThisComputer,
    /// A paired worker's host, whose worker runs each statement under its
    /// ceiling (ADR-0057, decision 1). The master's policy narrows its
    /// decision to the host's mode and to whether the host accepts the
    /// master's full access, as the worker reported them when it last
    /// connected (decision 6).
    Worker {
        /// The worker.
        worker: WorkerId,
        /// The worker's name, for display only.
        name: String,
        /// The mode of the host's ceiling.
        mode: CeilingMode,
        /// Whether the host accepts the master's full access (ADR-0029,
        /// decision 5).
        accepts_full_access: bool,
    },
}

impl DatabaseHome {
    /// Whether the master's full access runs a write here without asking:
    /// on this computer, and on a worker's host in full access or one that
    /// accepts the master's full access, unless it is read-only (ADR-0057,
    /// decision 6).
    pub fn takes_full_access(&self) -> bool {
        match self {
            Self::ThisComputer => true,
            Self::Worker {
                mode,
                accepts_full_access,
                ..
            } => match mode {
                CeilingMode::ReadOnly => false,
                CeilingMode::Ask | CeilingMode::Autonomous => *accepts_full_access,
                CeilingMode::FullAccess => true,
            },
        }
    }
}

/// What lets a write run on a database of a worker's host, as the master's
/// core states it in the write's evidence (ADR-0057, decision 7). The worker
/// checks it against its ceiling: a write that its ceiling does not accept
/// goes back to the owner or is refused, whatever the master sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WriteAuthority {
    /// The owner's own change from the view, an owner operation (ADR-0056,
    /// decision 8).
    OwnerAction,
    /// The owner allowed the call, with the digest of the write that the
    /// owner approved, which the worker computes again from its own
    /// registry and the write.
    AllowedOnce {
        /// The digest of the write.
        digest: String,
    },
    /// Grants of the master that cover the write's statements.
    Grants {
        /// The grants.
        scopes: Vec<GrantScope>,
    },
    /// The conversation's full access on the master (ADR-0029).
    FullAccess,
}

/// Where a write on a database of a worker's host comes from (ADR-0057,
/// decision 7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WriteOrigin {
    /// A call of a plugin's tool in a conversation of the master.
    Call {
        /// The conversation.
        conversation: ConversationId,
        /// The call's part.
        part: PartId,
    },
    /// An owner operation, which the master's journal holds under the
    /// write's operation ID.
    OwnerOperation,
}

/// Why a worker's ceiling refuses a request on a database of its host,
/// whatever the master sends (ADR-0057, decision 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum HostDatabaseRefusal {
    /// The host's configuration does not allow databases.
    NoDatabasesOnHost,
    /// The ceiling lets the host only read: every write and every change of
    /// a schema is refused.
    ReadOnlyHost,
    /// The file lies outside the folders that the ceiling allows, such as
    /// after the host's configuration changed.
    OutsideAllowedFolders,
    /// The file lies inside the worker's own folders.
    WorkerFolder,
}

/// What a statement on a database does (ADR-0056, decision 5), from the
/// weakest to the strongest, so the class of several statements is the
/// strongest of theirs.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "snake_case")]
pub enum StatementClass {
    /// Reads rows or the schema and changes nothing.
    Read,
    /// Inserts, updates or deletes rows.
    Write,
    /// Creates, changes or removes a table, an index, a view or a trigger,
    /// or runs `REINDEX` or `ANALYZE`.
    Schema,
}

#[cfg(test)]
mod tests {
    use super::{CeilingMode, DatabaseHome, StatementClass, WorkerId, WriteAuthority};

    #[test]
    fn a_class_serializes_as_its_name_and_orders_from_read_to_schema() {
        assert_eq!(
            serde_json::to_value(StatementClass::Schema).unwrap(),
            serde_json::json!("schema")
        );
        assert!(StatementClass::Read < StatementClass::Write);
        assert!(StatementClass::Write < StatementClass::Schema);
        assert_eq!(
            [StatementClass::Write, StatementClass::Read]
                .into_iter()
                .max(),
            Some(StatementClass::Write)
        );
    }

    fn worker(mode: CeilingMode, accepts_full_access: bool) -> DatabaseHome {
        DatabaseHome::Worker {
            worker: WorkerId::parse("0199e0f2-0000-7000-8000-0000000000aa").unwrap(),
            name: "build-box".to_owned(),
            mode,
            accepts_full_access,
        }
    }

    #[test]
    fn the_masters_full_access_counts_where_the_host_takes_it() {
        assert!(DatabaseHome::ThisComputer.takes_full_access());
        assert!(worker(CeilingMode::Ask, true).takes_full_access());
        assert!(worker(CeilingMode::Autonomous, true).takes_full_access());
        assert!(!worker(CeilingMode::Ask, false).takes_full_access());
        assert!(!worker(CeilingMode::Autonomous, false).takes_full_access());
        assert!(worker(CeilingMode::FullAccess, false).takes_full_access());
        assert!(!worker(CeilingMode::ReadOnly, true).takes_full_access());
    }

    #[test]
    fn a_worker_home_and_an_authority_name_their_kind() {
        assert_eq!(
            serde_json::to_value(worker(CeilingMode::Ask, false)).unwrap(),
            serde_json::json!({
                "kind": "worker",
                "worker": "0199e0f2-0000-7000-8000-0000000000aa",
                "name": "build-box",
                "mode": "ask",
                "acceptsFullAccess": false
            })
        );
        assert_eq!(
            serde_json::to_value(WriteAuthority::AllowedOnce {
                digest: "ab".to_owned()
            })
            .unwrap(),
            serde_json::json!({ "kind": "allowed_once", "digest": "ab" })
        );
    }
}
