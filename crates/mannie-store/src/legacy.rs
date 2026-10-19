//! The numbers that plugins' migrations carry from the store's single list
//! of migrations before M5 (ADR-0030, decision 5; ADR-0031, decision 8).
//!
//! A database from before M5 records how many migrations of that list it
//! applied, and a plugin's migration that was part of the list carries its
//! number, so a database of 0.2.0 keeps the tables that the migration made.
//! The record is frozen: only the migrations that 0.2.0 shipped are in it, a
//! plugin's module may claim a number only as the record lists it, and a
//! module that the record names must claim every number that it lists.
//! Otherwise a module that left one out would run its migration again on a
//! database of 0.2.0, and one that claimed another would skip a migration.

/// A migration of a plugin's module that carries a number of the list
/// before M5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RecordedMigration {
    /// The plugin's module.
    pub(crate) module: &'static str,
    /// The migration's version in the module.
    pub(crate) version: u32,
    /// Its number in the list before M5.
    pub(crate) legacy_version: u32,
}

/// The migrations of plugins' modules that 0.2.0 applied as part of its
/// single list: the Workers plugin's tables of paired workers, of the
/// deletions that wait for a worker, and of the names that the owner gives
/// workers.
pub(crate) const LEGACY_VERSIONS: &[RecordedMigration] = &[
    RecordedMigration {
        module: "workers",
        version: 1,
        legacy_version: 19,
    },
    RecordedMigration {
        module: "workers",
        version: 2,
        legacy_version: 20,
    },
    RecordedMigration {
        module: "workers",
        version: 3,
        legacy_version: 22,
    },
];

/// A migration of a plugin's module that may insert into one table outside
/// the module's prefix, which the authorizer refuses to every other
/// migration (ADR-0034, decision 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExemptMigration {
    /// The plugin's module.
    pub(crate) module: &'static str,
    /// The migration's version in the module.
    pub(crate) version: u32,
    /// The SHA-256 of the migration's text with LF line endings, which
    /// `.gitattributes` keeps on every checkout.
    pub(crate) sql_sha256: &'static str,
    /// The one table outside the prefix that the migration may insert into.
    pub(crate) may_insert_into: &'static str,
}

/// The migrations that may leave their prefix. The record is frozen: M5
/// added Workers' migration 4, which moves the deletions that a 0.2.0
/// database holds into the core's outbox and which no release has shipped
/// yet, and its hash pins it, because development databases of M5 applied
/// it already. No other migration can be added. Workers' tests run its
/// migrations under the authorizer, so a changed text fails there first.
pub(crate) const EXEMPT_MIGRATIONS: &[ExemptMigration] = &[ExemptMigration {
    module: "workers",
    version: 4,
    sql_sha256: "a13775a99bb852379bc6b6f32822e0811c4cbc184400a2e176426c084e5a8fab",
    may_insert_into: "runtime_executor_outbox",
}];
