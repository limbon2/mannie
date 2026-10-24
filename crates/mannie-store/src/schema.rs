//! The schema: the modules of the database and their migrations, the records
//! of what each module applied, the copy of the database before migrating,
//! and the migration runner (ADR-0009, ADR-0030).
//!
//! A module owns the tables that its name prefixes and the forward-only
//! migrations that create and change them, one logical change each, with
//! their SQL beside the module. The core's modules are built in, and a
//! composition root adds the modules of its plugins. The table
//! `store_modules` records how many migrations of each module a database has,
//! so a binary recognizes a module that a newer binary migrated further.
//!
//! Before M5 the migrations were one ordered list, and SQLite's
//! `user_version` held how many of them a database had. Each migration of
//! that list keeps its number as its legacy version, and the first open of
//! such a database turns `user_version` into records. It also keeps the old
//! `user_version` in `store_legacy`, so a module that a binary registers
//! later, such as a plugin's module on a database that the worker converted,
//! still counts the migrations that the old list applied. From then on
//! `user_version` is [`MODULE_RECORDS`] plus the sum of the recorded
//! versions, which binaries up to 0.2.0 refuse as newer than theirs.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use mannie_types::clock::UtcMillis;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use time::OffsetDateTime;
use time::macros::format_description;

use crate::error::{DatabaseError, StoreError};
use crate::plugin_store::run_plugin_migration;

/// A module of the database: the owner of the tables that its name
/// prefixes, with its migrations in order. The core's modules are built in;
/// a plugin's module comes from the SDK's declaration, whose migrations the
/// module owns (`plugin_store`).
#[derive(Debug, Clone)]
pub(crate) struct StoreModule {
    /// The module's name, which prefixes its tables, such as `runtime`.
    pub(crate) name: &'static str,
    /// The module's migrations in order. A new migration is appended, and an
    /// applied migration is never edited or removed.
    pub(crate) migrations: Cow<'static, [Migration]>,
    /// Whether the module is a plugin's, whose migrations run under the
    /// authorizer and the schema check (ADR-0034, decision 7).
    pub(crate) confined: bool,
}

impl StoreModule {
    /// The version that the module's migrations lead to.
    pub(crate) fn version(&self) -> u32 {
        // No module has more migrations than u32 counts.
        u32::try_from(self.migrations.len()).unwrap_or(u32::MAX)
    }
}

/// One schema change of a module.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Migration {
    /// The module's version after this migration: its position in the
    /// module's list, counting from 1.
    pub(crate) version: u32,
    /// Its number in the single list of migrations before M5, which carries
    /// over the `user_version` of an older database. Later migrations have
    /// none.
    pub(crate) legacy_version: Option<u32>,
    /// What it does, for logs and errors.
    pub(crate) description: &'static str,
    /// The statements, run in one transaction.
    pub(crate) sql: &'static str,
}

/// The last version of the single list of migrations before M5.
pub(crate) const LAST_LEGACY_VERSION: u32 = 22;

/// The `user_version` of a database whose migrations are recorded for each
/// module, before the sum of the recorded versions is added.
pub(crate) const MODULE_RECORDS: u32 = 1000;

const SETTINGS: StoreModule = StoreModule {
    name: "settings",
    migrations: Cow::Borrowed(&[Migration {
        version: 1,
        legacy_version: Some(1),
        description: "create the settings sections table",
        sql: include_str!("settings/migrations/0001_create_sections.sql"),
    }]),
    confined: false,
};

const RUNTIME: StoreModule = StoreModule {
    name: "runtime",
    migrations: Cow::Borrowed(&[
        Migration {
            version: 1,
            legacy_version: Some(2),
            description: "create the conversations, turns and runs tables",
            sql: include_str!("runtime/migrations/0002_create_conversations.sql"),
        },
        Migration {
            version: 2,
            legacy_version: Some(3),
            description: "create the parts and provider items tables",
            sql: include_str!("runtime/migrations/0003_create_parts.sql"),
        },
        Migration {
            version: 3,
            legacy_version: Some(4),
            description: "create the events table",
            sql: include_str!("runtime/migrations/0004_create_events.sql"),
        },
        Migration {
            version: 4,
            legacy_version: Some(5),
            description: "create the checkpoints table",
            sql: include_str!("runtime/migrations/0005_create_checkpoints.sql"),
        },
        Migration {
            version: 5,
            legacy_version: Some(7),
            description: "mark the runs that a retry keeps",
            sql: include_str!("runtime/migrations/0007_add_kept_runs.sql"),
        },
        Migration {
            version: 6,
            legacy_version: Some(8),
            description: "add the workspace of a conversation",
            sql: include_str!("runtime/migrations/0008_add_conversation_workspaces.sql"),
        },
        Migration {
            version: 7,
            legacy_version: Some(9),
            description: "create the archives of tool outputs",
            sql: include_str!("runtime/migrations/0009_create_archives.sql"),
        },
        Migration {
            version: 8,
            legacy_version: Some(12),
            description: "create the projects table",
            sql: include_str!("runtime/migrations/0012_create_projects.sql"),
        },
        Migration {
            version: 9,
            legacy_version: Some(13),
            description: "add the project of a conversation and turn workspaces into projects",
            sql: include_str!("runtime/migrations/0013_add_conversation_projects.sql"),
        },
        Migration {
            version: 10,
            legacy_version: Some(15),
            description: "add the parent, number and agent of task conversations",
            sql: include_str!("runtime/migrations/0015_add_task_conversations.sql"),
        },
        Migration {
            version: 11,
            legacy_version: Some(16),
            description: "create the tasks table",
            sql: include_str!("runtime/migrations/0016_create_tasks.sql"),
        },
        Migration {
            version: 12,
            legacy_version: Some(17),
            description: "rebuild the tasks table for tasks that run on workers",
            sql: include_str!("runtime/migrations/0017_rebuild_tasks_for_workers.sql"),
        },
        Migration {
            version: 13,
            legacy_version: Some(18),
            description: "add the worker of a task conversation",
            sql: include_str!("runtime/migrations/0018_add_task_workers.sql"),
        },
        Migration {
            version: 14,
            legacy_version: Some(21),
            description: "add the full access of a conversation",
            sql: include_str!("runtime/migrations/0021_add_conversation_full_access.sql"),
        },
        Migration {
            version: 15,
            legacy_version: None,
            description: "add the executor of a task that a plugin runs",
            sql: include_str!("runtime/migrations/0023_add_task_executors.sql"),
        },
        Migration {
            version: 16,
            legacy_version: None,
            description: "add the executor of a task conversation",
            sql: include_str!("runtime/migrations/0024_add_conversation_executors.sql"),
        },
        Migration {
            version: 17,
            legacy_version: None,
            description: "name the executor in the task parts of workers' tasks",
            sql: include_str!("runtime/migrations/0025_name_executors_in_task_parts.sql"),
        },
        Migration {
            version: 18,
            legacy_version: None,
            description: "create the outbox of the executors' deletions",
            sql: include_str!("runtime/migrations/0026_create_executor_outbox.sql"),
        },
        Migration {
            version: 19,
            legacy_version: None,
            description: "keep the texts of workers' refusals in their task parts",
            sql: include_str!("runtime/migrations/0027_keep_worker_refusal_texts.sql"),
        },
        Migration {
            version: 20,
            legacy_version: None,
            description: "create the table of the images that the owner's messages hold",
            sql: include_str!("runtime/migrations/0029_create_images.sql"),
        },
        Migration {
            version: 21,
            legacy_version: None,
            description: "create the table of background commands",
            sql: include_str!("runtime/migrations/0032_create_background_commands.sql"),
        },
        Migration {
            version: 22,
            legacy_version: None,
            description: "create the table of the images of tool results",
            sql: include_str!("runtime/migrations/0034_create_tool_images.sql"),
        },
    ]),
    confined: false,
};

const PROVIDER: StoreModule = StoreModule {
    name: "provider",
    migrations: Cow::Borrowed(&[Migration {
        version: 1,
        legacy_version: Some(6),
        description: "create the provider profiles table",
        sql: include_str!("provider/migrations/0006_create_profiles.sql"),
    }]),
    confined: false,
};

const TOOLS: StoreModule = StoreModule {
    name: "tools",
    migrations: Cow::Borrowed(&[
        Migration {
            version: 1,
            legacy_version: Some(10),
            description: "create the grants table",
            sql: include_str!("tools/migrations/0010_create_grants.sql"),
        },
        Migration {
            version: 2,
            legacy_version: Some(11),
            description: "create the command rules table",
            sql: include_str!("tools/migrations/0011_create_rules.sql"),
        },
        Migration {
            version: 3,
            legacy_version: Some(14),
            description: "create the shared grants table",
            sql: include_str!("tools/migrations/0014_create_shared_grants.sql"),
        },
        Migration {
            version: 4,
            legacy_version: None,
            description: "create the rules for commands on remote hosts",
            sql: include_str!("tools/migrations/0030_create_remote_rules.sql"),
        },
        Migration {
            version: 5,
            legacy_version: None,
            description: "create the rules for runs of installed programs",
            sql: include_str!("tools/migrations/0031_create_program_rules.sql"),
        },
        Migration {
            version: 6,
            legacy_version: None,
            description: "let rules cover commands in the background",
            sql: include_str!("tools/migrations/0033_add_background_to_rules.sql"),
        },
    ]),
    confined: false,
};

/// The built-in modules, in the order that a new database applies them: the
/// tools' tables refer to the runtime's. The modules of plugins follow them,
/// so a plugin's migration finds the core's tables up to date. Migrations 19,
/// 20 and 22 of the single list before M5 belong to the Workers plugin's
/// module now (ADR-0031, decision 8).
const EXPORTS: StoreModule = StoreModule {
    name: "exports",
    migrations: Cow::Borrowed(&[Migration {
        version: 1,
        legacy_version: None,
        description: "create the owner-initiated export journal",
        sql: include_str!("exports/0001_create_operations.sql"),
    }]),
    confined: false,
};

pub(crate) const CORE_MODULES: [StoreModule; 5] = [SETTINGS, RUNTIME, PROVIDER, TOOLS, EXPORTS];

/// The records of a database: each module's name and how many of its
/// migrations the database has. Modules that this binary does not know keep
/// their records, and their tables stay untouched.
pub(crate) type Records = BTreeMap<String, u32>;

const CREATE_RECORDS: &str = "CREATE TABLE store_modules (\
     module TEXT PRIMARY KEY, \
     version INTEGER NOT NULL CHECK (version >= 0), \
     migrated_at INTEGER NOT NULL\
     ) STRICT, WITHOUT ROWID; \
     CREATE TABLE store_legacy (\
     id INTEGER PRIMARY KEY CHECK (id = 1), \
     user_version INTEGER NOT NULL CHECK (user_version >= 0)\
     ) STRICT";
const HAS_RECORDS: &str =
    "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'store_modules'";
const SELECT_RECORDS: &str = "SELECT module, version FROM store_modules";
const INSERT_LEGACY: &str = "INSERT INTO store_legacy (id, user_version) VALUES (1, ?1)";
const SELECT_LEGACY: &str = "SELECT user_version FROM store_legacy WHERE id = 1";
const UPSERT_RECORD: &str = "INSERT INTO store_modules (module, version, migrated_at) \
     VALUES (?1, ?2, ?3) \
     ON CONFLICT (module) DO UPDATE SET version = excluded.version, \
     migrated_at = excluded.migrated_at";

/// What a database holds about its schema when it is opened.
#[derive(Debug)]
pub(crate) struct Schema {
    /// SQLite's `user_version`.
    pub(crate) user_version: u32,
    /// The records, for a database that has them.
    pub(crate) records: Option<Records>,
    /// The `user_version` that a database with records had before M5, or 0
    /// for one that M5 created.
    pub(crate) legacy_version: u32,
}

impl Schema {
    /// Reads `user_version` and the records of an open database.
    pub(crate) fn read(connection: &Connection) -> rusqlite::Result<Self> {
        let user_version: u32 =
            connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let has_records = connection
            .query_row(HAS_RECORDS, [], |_| Ok(()))
            .optional()?
            .is_some();
        if user_version < MODULE_RECORDS || !has_records {
            return Ok(Self {
                user_version,
                records: None,
                legacy_version: user_version,
            });
        }
        let mut select = connection.prepare(SELECT_RECORDS)?;
        let rows = select.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let records = rows.collect::<rusqlite::Result<Records>>()?;
        let legacy_version = connection
            .query_row(SELECT_LEGACY, [], |row| row.get(0))
            .optional()?
            .unwrap_or(0);
        Ok(Self {
            user_version,
            records: Some(records),
            legacy_version,
        })
    }

    /// Refuses a database that a newer binary migrated further than this one
    /// can: a known module with a newer version, a single list of migrations
    /// longer than the one before M5, or a `user_version` without records.
    pub(crate) fn refuse_newer(
        &self,
        path: &Path,
        modules: &[StoreModule],
    ) -> Result<(), StoreError> {
        let too_new = |database: u32, supported: u32| StoreError::SchemaTooNew {
            path: path.to_path_buf(),
            database,
            supported,
        };
        match &self.records {
            Some(records) => {
                for module in modules {
                    let recorded = records.get(module.name).copied().unwrap_or(0);
                    if recorded > module.version() {
                        tracing::error!(
                            module = module.name,
                            database = recorded,
                            supported = module.version(),
                            "refused a database written by a newer version of Mannie"
                        );
                        return Err(StoreError::ModuleTooNew {
                            path: path.to_path_buf(),
                            module: module.name.to_owned(),
                            database: recorded,
                            supported: module.version(),
                        });
                    }
                }
                Ok(())
            }
            None if self.user_version > LAST_LEGACY_VERSION => {
                tracing::error!(
                    database = self.user_version,
                    "refused a database written by a newer version of Mannie"
                );
                Err(too_new(self.user_version, LAST_LEGACY_VERSION))
            }
            None => Ok(()),
        }
    }

    /// The records as they stand: the stored ones, and for each module
    /// without one, the migrations whose legacy version the database's
    /// `user_version` before M5 covers.
    pub(crate) fn current_records(&self, modules: &[StoreModule]) -> Records {
        let mut records = self.records.clone().unwrap_or_default();
        for module in modules {
            if !records.contains_key(module.name) {
                records.insert(module.name.to_owned(), self.carried_over(module));
            }
        }
        records
    }

    /// How many of a module's migrations the single list before M5 applied.
    fn carried_over(&self, module: &StoreModule) -> u32 {
        let applied = module
            .migrations
            .iter()
            .take_while(|migration| {
                migration
                    .legacy_version
                    .is_some_and(|legacy| legacy <= self.legacy_version)
            })
            .count();
        u32::try_from(applied).unwrap_or(u32::MAX)
    }

    /// The modules that have no record yet.
    fn unrecorded<'a>(&self, modules: &'a [StoreModule]) -> Vec<&'a StoreModule> {
        modules
            .iter()
            .filter(|module| {
                self.records
                    .as_ref()
                    .is_none_or(|records| !records.contains_key(module.name))
            })
            .collect()
    }

    /// Whether opening changes the database: it has no records yet, a module
    /// has no record, or a module has migrations that it lacks.
    pub(crate) fn needs_changes(&self, modules: &[StoreModule]) -> bool {
        let records = self.current_records(modules);
        !self.unrecorded(modules).is_empty()
            || modules
                .iter()
                .any(|module| records.get(module.name).copied().unwrap_or(0) < module.version())
    }
}

/// The `user_version` that records with these versions stand for.
fn records_user_version(records: &Records) -> u32 {
    records
        .values()
        .fold(MODULE_RECORDS, |sum, version| sum.saturating_add(*version))
}

/// How many backups of earlier schema versions the store keeps.
const KEPT_BACKUPS: usize = 3;

/// Copies the database before its migrations run, so the owner can return to
/// the earlier version. `VACUUM INTO` writes a consistent snapshot even while
/// the database is in WAL mode. Only the newest [`KEPT_BACKUPS`] copies of each
/// database are kept.
pub(crate) fn back_up(
    connection: &Connection,
    database: &Path,
    backup_directory: &Path,
    from_version: u32,
    now: UtcMillis,
) -> Result<PathBuf, StoreError> {
    fs::create_dir_all(backup_directory).map_err(|source| StoreError::Io {
        path: backup_directory.to_path_buf(),
        source,
    })?;
    let file_name = database
        .file_name()
        .map_or_else(|| "database".into(), |name| name.to_string_lossy());
    let prefix = format!("{file_name}.schema-{from_version}.");
    let backup = backup_directory.join(format!("{prefix}{}.bak", backup_timestamp(now)));
    let target = backup.to_string_lossy().into_owned();
    connection
        .execute("VACUUM INTO ?1", [&target])
        .map_err(|source| StoreError::Backup {
            path: backup.clone(),
            source: DatabaseError(source),
        })?;
    remove_old_backups(backup_directory, &file_name);
    Ok(backup)
}

/// A timestamp for a backup file name: UTC, sortable, without characters
/// that Windows forbids in file names.
fn backup_timestamp(now: UtcMillis) -> String {
    let nanos = i128::from(now.as_millis()) * 1_000_000;
    let format =
        format_description!("[year][month][day]T[hour][minute][second].[subsecond digits:3]Z");
    OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .ok()
        .and_then(|moment| moment.format(&format).ok())
        .unwrap_or_else(|| now.as_millis().to_string())
}

/// Deletes all but the newest backups of one database. Failing to delete an
/// old backup is logged and never stops the store from opening.
fn remove_old_backups(backup_directory: &Path, file_name: &str) {
    let entries = match fs::read_dir(backup_directory) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(directory = %backup_directory.display(), %error, "could not list the database backups");
            return;
        }
    };
    let prefix = format!("{file_name}.schema-");
    let mut backups: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            let path = entry.path();
            let is_backup = path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("bak"));
            let is_this_database = entry.file_name().to_string_lossy().starts_with(&prefix);
            is_backup && is_this_database
        })
        .filter_map(|entry| {
            let modified = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()?;
            Some((modified, entry.path()))
        })
        .collect();
    backups.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    for (_, old) in backups.into_iter().skip(KEPT_BACKUPS) {
        if let Err(error) = fs::remove_file(&old) {
            tracing::warn!(backup = %old.display(), %error, "could not delete an old database backup");
        }
    }
}

/// Brings the database up to date. A database without records first gets
/// them, in one transaction, from its `user_version`, and so does a module
/// without a record from the `user_version` before M5. Then each module, in
/// the given order, applies the migrations that it lacks, each in its own
/// transaction, which also records the module's new version and the new
/// `user_version`.
pub(crate) fn migrate(
    connection: &mut Connection,
    schema: &Schema,
    modules: &[StoreModule],
    now: UtcMillis,
) -> Result<(), StoreError> {
    let mut records = schema.current_records(modules);
    let unrecorded = schema.unrecorded(modules);
    if !unrecorded.is_empty() {
        let _span = tracing::info_span!("record_modules", from = schema.user_version).entered();
        let failed = |source: rusqlite::Error| StoreError::Records {
            source: DatabaseError(source),
        };
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(failed)?;
        if schema.records.is_none() {
            transaction.execute_batch(CREATE_RECORDS).map_err(failed)?;
            transaction
                .execute(INSERT_LEGACY, params![schema.legacy_version])
                .map_err(failed)?;
        }
        for module in &unrecorded {
            let version = records.get(module.name).copied().unwrap_or(0);
            transaction
                .execute(
                    UPSERT_RECORD,
                    params![module.name, version, now.as_millis()],
                )
                .map_err(failed)?;
        }
        transaction
            .pragma_update(None, "user_version", records_user_version(&records))
            .map_err(failed)?;
        transaction.commit().map_err(failed)?;
        tracing::info!("recorded the version of each module");
    }
    for module in modules {
        let applied = records.get(module.name).copied().unwrap_or(0);
        for migration in module
            .migrations
            .iter()
            .filter(|migration| migration.version > applied)
        {
            let _span = tracing::info_span!(
                "migration",
                module = module.name,
                version = migration.version,
                description = migration.description
            )
            .entered();
            let failed = |source: rusqlite::Error| StoreError::Migration {
                module: module.name,
                version: migration.version,
                description: migration.description,
                source: DatabaseError(source),
            };
            records.insert(module.name.to_owned(), migration.version);
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(failed)?;
            if module.confined {
                run_plugin_migration(&transaction, module.name, migration.version, migration.sql)
                    .map_err(failed)?;
            } else {
                transaction.execute_batch(migration.sql).map_err(failed)?;
            }
            transaction
                .execute(
                    UPSERT_RECORD,
                    params![module.name, migration.version, now.as_millis()],
                )
                .map_err(failed)?;
            transaction
                .pragma_update(None, "user_version", records_user_version(&records))
                .map_err(failed)?;
            transaction.commit().map_err(failed)?;
            tracing::info!("applied the migration");
        }
    }
    Ok(())
}

/// Applies the migrations of `modules` that have a legacy version up to
/// `version`, in the order of the single list before M5, and sets
/// `user_version` to `version`, as a binary up to 0.2.0 left it. Tests use
/// it to stand in for such a binary; without the modules of plugins, the
/// database lacks their tables.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn migrate_as_before_m5(
    connection: &mut Connection,
    modules: &[StoreModule],
    version: u32,
) -> rusqlite::Result<()> {
    let mut migrations: Vec<(u32, &Migration)> = modules
        .iter()
        .flat_map(|module| module.migrations.iter())
        .filter_map(|migration| {
            migration
                .legacy_version
                .filter(|legacy| *legacy <= version)
                .map(|legacy| (legacy, migration))
        })
        .collect();
    migrations.sort_by_key(|(legacy, _)| *legacy);
    for (legacy, migration) in migrations {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(migration.sql)?;
        transaction.pragma_update(None, "user_version", legacy)?;
        transaction.commit()?;
    }
    connection.pragma_update(None, "user_version", version)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{CORE_MODULES, LAST_LEGACY_VERSION, StoreModule, backup_timestamp};
    use mannie_types::clock::UtcMillis;

    fn every_module() -> Vec<StoreModule> {
        CORE_MODULES.to_vec()
    }

    #[test]
    fn each_module_counts_its_versions_from_one_without_gaps() {
        for module in every_module() {
            assert_ne!(module.name, "");
            for (index, migration) in module.migrations.iter().enumerate() {
                assert_eq!(
                    migration.version as usize,
                    index + 1,
                    "{}: {}",
                    module.name,
                    migration.description
                );
                assert_ne!(migration.sql.trim(), "");
            }
            assert_eq!(module.version() as usize, module.migrations.len());
        }
    }

    #[test]
    fn the_core_covers_its_part_of_the_list_before_m5_once_each() {
        let mut seen = BTreeSet::new();
        for module in every_module() {
            let legacy: Vec<u32> = module
                .migrations
                .iter()
                .map_while(|migration| migration.legacy_version)
                .collect();
            assert!(
                legacy.windows(2).all(|pair| pair[0] < pair[1]),
                "{}: legacy versions grow",
                module.name
            );
            let without_legacy = module.migrations.len() - legacy.len();
            assert!(
                module.migrations[legacy.len()..]
                    .iter()
                    .all(|migration| migration.legacy_version.is_none()),
                "{}: only the first migrations have legacy versions ({without_legacy} later ones)",
                module.name
            );
            for version in legacy {
                assert!(
                    seen.insert(version),
                    "legacy version {version} appears twice"
                );
            }
        }
        // The plugins' modules hold the rest of the list, as the store's
        // frozen record of their numbers says (ADR-0031, decision 8).
        let plugins: BTreeSet<u32> = crate::legacy::LEGACY_VERSIONS
            .iter()
            .map(|recorded| recorded.legacy_version)
            .collect();
        let list: BTreeSet<u32> = (1..=LAST_LEGACY_VERSION).collect();
        assert_eq!(seen, &list - &plugins);
    }

    #[test]
    fn module_names_are_unique() {
        let names: BTreeSet<&str> = every_module().iter().map(|module| module.name).collect();
        assert_eq!(names.len(), every_module().len());
    }

    #[test]
    fn backup_timestamps_sort_and_avoid_forbidden_characters() {
        let first = backup_timestamp(UtcMillis::from_millis(1_760_389_684_123));
        let second = backup_timestamp(UtcMillis::from_millis(1_760_389_684_124));

        assert_eq!(first, "20251013T210804.123Z");
        assert!(first < second);
        assert!(!first.contains([':', '/', '\\']));
    }
}
