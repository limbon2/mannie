//! The schema: the ordered list of migrations, the copy of the database
//! before migrating, and the migration runner (ADR-0009).
//!
//! Migrations are forward-only, one logical change each, and owned by the
//! module whose tables they change. Their SQL lives beside that module. The
//! schema version of a database is the number of migrations applied to it,
//! kept in SQLite's `user_version`, so a binary recognizes a database that a
//! newer binary migrated further.

use std::fs;
use std::path::{Path, PathBuf};

use mannie_types::clock::UtcMillis;
use rusqlite::{Connection, TransactionBehavior};
use time::OffsetDateTime;
use time::macros::format_description;

use crate::error::{DatabaseError, StoreError};

/// One schema change.
#[derive(Debug)]
pub(crate) struct Migration {
    /// The schema version after this migration: its position in
    /// [`MIGRATIONS`], counting from 1.
    pub(crate) version: u32,
    /// The module that owns the tables it changes.
    pub(crate) module: &'static str,
    /// What it does, for logs and errors.
    pub(crate) description: &'static str,
    /// The statements, run in one transaction.
    pub(crate) sql: &'static str,
}

/// Every migration, in order. New migrations are appended; an applied
/// migration is never edited or removed.
pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        module: "settings",
        description: "create the settings sections table",
        sql: include_str!("settings/migrations/0001_create_sections.sql"),
    },
    Migration {
        version: 2,
        module: "runtime",
        description: "create the conversations, turns and runs tables",
        sql: include_str!("runtime/migrations/0002_create_conversations.sql"),
    },
    Migration {
        version: 3,
        module: "runtime",
        description: "create the parts and provider items tables",
        sql: include_str!("runtime/migrations/0003_create_parts.sql"),
    },
    Migration {
        version: 4,
        module: "runtime",
        description: "create the events table",
        sql: include_str!("runtime/migrations/0004_create_events.sql"),
    },
    Migration {
        version: 5,
        module: "runtime",
        description: "create the checkpoints table",
        sql: include_str!("runtime/migrations/0005_create_checkpoints.sql"),
    },
    Migration {
        version: 6,
        module: "provider",
        description: "create the provider profiles table",
        sql: include_str!("provider/migrations/0006_create_profiles.sql"),
    },
    Migration {
        version: 7,
        module: "runtime",
        description: "mark the runs that a retry keeps",
        sql: include_str!("runtime/migrations/0007_add_kept_runs.sql"),
    },
    Migration {
        version: 8,
        module: "runtime",
        description: "add the workspace of a conversation",
        sql: include_str!("runtime/migrations/0008_add_conversation_workspaces.sql"),
    },
    Migration {
        version: 9,
        module: "runtime",
        description: "create the archives of tool outputs",
        sql: include_str!("runtime/migrations/0009_create_archives.sql"),
    },
    Migration {
        version: 10,
        module: "tools",
        description: "create the grants table",
        sql: include_str!("tools/migrations/0010_create_grants.sql"),
    },
    Migration {
        version: 11,
        module: "tools",
        description: "create the command rules table",
        sql: include_str!("tools/migrations/0011_create_rules.sql"),
    },
    Migration {
        version: 12,
        module: "runtime",
        description: "create the projects table",
        sql: include_str!("runtime/migrations/0012_create_projects.sql"),
    },
    Migration {
        version: 13,
        module: "runtime",
        description: "add the project of a conversation and turn workspaces into projects",
        sql: include_str!("runtime/migrations/0013_add_conversation_projects.sql"),
    },
    Migration {
        version: 14,
        module: "tools",
        description: "create the shared grants table",
        sql: include_str!("tools/migrations/0014_create_shared_grants.sql"),
    },
];

/// How many backups of earlier schema versions the store keeps.
const KEPT_BACKUPS: usize = 3;

/// The schema version that a list of migrations leads to.
pub(crate) fn supported_version(migrations: &[Migration]) -> u32 {
    u32::try_from(migrations.len()).expect("the number of migrations fits in u32")
}

/// Reads the schema version of an open database.
pub(crate) fn schema_version(connection: &Connection) -> rusqlite::Result<u32> {
    connection.pragma_query_value(None, "user_version", |row| row.get(0))
}

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

/// Applies every migration newer than the database's schema version, each in
/// its own transaction, which also records the new version.
pub(crate) fn migrate(
    connection: &mut Connection,
    migrations: &[Migration],
    from_version: u32,
) -> Result<(), StoreError> {
    for migration in migrations
        .iter()
        .filter(|migration| migration.version > from_version)
    {
        let _span = tracing::info_span!(
            "migration",
            version = migration.version,
            module = migration.module,
            description = migration.description
        )
        .entered();
        let failed = |source: rusqlite::Error| StoreError::Migration {
            version: migration.version,
            description: migration.description,
            source: DatabaseError(source),
        };
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(failed)?;
        transaction.execute_batch(migration.sql).map_err(failed)?;
        transaction
            .pragma_update(None, "user_version", migration.version)
            .map_err(failed)?;
        transaction.commit().map_err(failed)?;
        tracing::info!("applied the migration");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MIGRATIONS, backup_timestamp, supported_version};
    use mannie_types::clock::UtcMillis;

    #[test]
    fn migration_versions_count_from_one_without_gaps() {
        for (index, migration) in MIGRATIONS.iter().enumerate() {
            assert_eq!(
                migration.version as usize,
                index + 1,
                "{}",
                migration.description
            );
            assert_ne!(migration.module, "");
            assert_ne!(migration.sql.trim(), "");
        }
        assert_eq!(supported_version(MIGRATIONS) as usize, MIGRATIONS.len());
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
