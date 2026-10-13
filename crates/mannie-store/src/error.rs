//! Errors of the store. Each variant has a stable code for logs.

use std::fmt;
use std::path::PathBuf;

/// An error from SQLite. The type is opaque, so that rusqlite stays inside
/// this crate (ADR-0009): other crates can format it and walk its sources,
/// but not depend on its type.
pub struct DatabaseError(pub(crate) rusqlite::Error);

impl fmt::Debug for DatabaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, formatter)
    }
}

impl fmt::Display for DatabaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

impl std::error::Error for DatabaseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

impl From<rusqlite::Error> for DatabaseError {
    fn from(error: rusqlite::Error) -> Self {
        Self(error)
    }
}

/// Why the store could not open, migrate, read or write.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The database was written by a newer version of Mannie. It was left
    /// untouched.
    #[error(
        "the database {path} has schema version {database}, and this version of Mannie supports up to version {supported}; the database was left untouched"
    )]
    SchemaTooNew {
        /// The database file.
        path: PathBuf,
        /// The schema version of the database.
        database: u32,
        /// The newest schema version this binary knows.
        supported: u32,
    },
    /// A folder or file of the store could not be created or changed.
    #[error("could not prepare {path}")]
    Io {
        /// The folder or file.
        path: PathBuf,
        /// The cause.
        #[source]
        source: std::io::Error,
    },
    /// The database could not be opened or configured.
    #[error("could not open the database {path}")]
    Open {
        /// The database file.
        path: PathBuf,
        /// The cause.
        #[source]
        source: DatabaseError,
    },
    /// SQLite could not switch the database to write-ahead logging, for
    /// example because it is on a network file system.
    #[error("the database {path} cannot use write-ahead logging (journal mode {journal_mode:?})")]
    WalUnavailable {
        /// The database file.
        path: PathBuf,
        /// The journal mode SQLite reported instead.
        journal_mode: String,
    },
    /// The copy of the database before a migration failed, so no migration
    /// ran.
    #[error("could not copy the database to {path} before migrating it; no migration ran")]
    Backup {
        /// The copy that was being written.
        path: PathBuf,
        /// The cause.
        #[source]
        source: DatabaseError,
    },
    /// A migration failed and was rolled back.
    #[error("migration {version} ({description}) failed and was rolled back")]
    Migration {
        /// The schema version the migration leads to.
        version: u32,
        /// What the migration does.
        description: &'static str,
        /// The cause.
        #[source]
        source: DatabaseError,
    },
    /// A query failed.
    #[error("a database query failed")]
    Query {
        /// The cause.
        #[source]
        source: DatabaseError,
    },
    /// The writer thread stopped, so the store takes no more writes.
    #[error("the store's writer has stopped")]
    WriterStopped,
    /// The reader pool is closed.
    #[error("the store's readers have stopped")]
    ReadersStopped,
}

impl StoreError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::SchemaTooNew { .. } => "store.schema_too_new",
            Self::Io { .. } => "store.io",
            Self::Open { .. } => "store.open",
            Self::WalUnavailable { .. } => "store.wal_unavailable",
            Self::Backup { .. } => "store.backup",
            Self::Migration { .. } => "store.migration",
            Self::Query { .. } => "store.query",
            Self::WriterStopped => "store.writer_stopped",
            Self::ReadersStopped => "store.readers_stopped",
        }
    }

    pub(crate) fn query(error: rusqlite::Error) -> Self {
        Self::Query {
            source: DatabaseError(error),
        }
    }
}
