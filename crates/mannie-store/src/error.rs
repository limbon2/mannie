//! Errors of the store. Each variant has a stable code for logs.

use std::fmt;
use std::path::PathBuf;

/// An error from SQLite. The type is opaque, so that the core's other crates
/// never depend on rusqlite's types (ADR-0009): they can format it and walk
/// its sources.
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
    /// A newer version of Mannie migrated one module of the database further
    /// than this one knows. The database was left untouched.
    #[error(
        "the database {path} has version {database} of the module {module}, and this version of Mannie supports up to version {supported}; the database was left untouched"
    )]
    ModuleTooNew {
        /// The database file.
        path: PathBuf,
        /// The module.
        module: String,
        /// The module's version in the database.
        database: u32,
        /// The newest version of the module that this binary knows.
        supported: u32,
    },
    /// Two modules of the store have the same name, which is a fault of the
    /// composition root.
    #[error("two modules of the store are named {module}")]
    DuplicateModule {
        /// The name.
        module: &'static str,
    },
    /// Code asked for the tables of a module that the store did not open
    /// with, which is a fault of the composition root.
    #[error("the store did not open with the module {module}")]
    ModuleNotRegistered {
        /// The module, by the plugin's prefix.
        module: String,
    },
    /// The store refused a plugin's module before it touched the database,
    /// which is a fault of the plugin.
    #[error("the store refused the module {module}: {problem}")]
    ModuleRefused {
        /// The module.
        module: String,
        /// What is wrong with it.
        problem: ModuleProblem,
    },
    /// A statement that a plugin's module lists does not prepare against the
    /// migrated schema. The module stays migrated, and its plugin cannot
    /// reach it.
    #[error("a statement that the module {module} lists does not prepare")]
    StatementsRefused {
        /// The module.
        module: String,
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
    /// Recording the version of each module of a database from before M5
    /// failed and was rolled back, so no migration ran.
    #[error("could not record the version of each module; no migration ran")]
    Records {
        /// The cause.
        #[source]
        source: DatabaseError,
    },
    /// A migration failed and was rolled back.
    #[error(
        "migration {version} of the module {module} ({description}) failed and was rolled back"
    )]
    Migration {
        /// The module whose migration failed.
        module: &'static str,
        /// The module's version that the migration leads to.
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
            Self::SchemaTooNew { .. } | Self::ModuleTooNew { .. } => "store.schema_too_new",
            Self::DuplicateModule { .. } => "store.duplicate_module",
            Self::ModuleNotRegistered { .. } => "store.module_not_registered",
            Self::ModuleRefused { .. } => "store.module_refused",
            Self::StatementsRefused { .. } => "store.statements_refused",
            Self::Records { .. } => "store.records",
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

/// What is wrong with a plugin's module of the store.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModuleProblem {
    /// The module's name is not the plugin's prefix.
    #[error("its name is not the plugin's prefix {prefix}")]
    NotThePrefix {
        /// The plugin's prefix.
        prefix: String,
    },
    /// The migrations do not count their versions from 1 without gaps.
    #[error("migration {position} has the version {version}")]
    VersionOutOfOrder {
        /// The migration's position, counting from 1.
        position: u32,
        /// The version that it has.
        version: u32,
    },
    /// A migration has no SQL.
    #[error("migration {version} has no SQL")]
    EmptyMigration {
        /// The migration's version.
        version: u32,
    },
    /// A migration claims a number of the list before M5 that the store's
    /// frozen record does not give it.
    #[error("migration {version} claims the number {legacy_version} of the list before M5")]
    LegacyVersionNotRecorded {
        /// The migration's version.
        version: u32,
        /// The number that it claims.
        legacy_version: u32,
    },
    /// The module lacks a migration that the store's frozen record gives a
    /// number of the list before M5.
    #[error("migration {version} does not carry the number {legacy_version} of the list before M5")]
    LegacyVersionMissing {
        /// The migration's version.
        version: u32,
        /// The number that the record gives it.
        legacy_version: u32,
    },
}
