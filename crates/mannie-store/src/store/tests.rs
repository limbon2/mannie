use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use mannie_types::clock::{Clock, UtcMillis};
use mannie_types::ids::{ConversationId, PartId, ProfileId, TaskId, TurnId};
use mannie_types::interface::{InterfaceSettings, LocaleTag};
use mannie_types::settings::SettingsSection;
use mannie_types::tasks::TaskState;
use rusqlite::Connection;

use std::borrow::Cow;

use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::services::store as sdk;

use crate::PluginModule;
use crate::schema::{CORE_MODULES, MODULE_RECORDS, Migration, StoreModule};
use crate::{Durability, ExecutorTaskProfile, SettingsStoreError, Store, StoreError, StoreOptions};

/// A clock that tests move by hand.
#[derive(Debug)]
struct TestClock(AtomicI64);

impl TestClock {
    fn at(millis: i64) -> Arc<Self> {
        Arc::new(Self(AtomicI64::new(millis)))
    }

    fn advance(&self, millis: i64) {
        self.0.fetch_add(millis, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now(&self) -> UtcMillis {
        UtcMillis::from_millis(self.0.load(Ordering::SeqCst))
    }
}

/// 2026-10-13T12:00:00Z
const NOON: i64 = 1_791_892_800_000;

const FIRST: Migration = Migration {
    legacy_version: None,
    version: 1,
    description: "create the notes table",
    sql: "CREATE TABLE tests_notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL) STRICT;",
};

const SECOND: Migration = Migration {
    legacy_version: None,
    version: 2,
    description: "add a title to notes",
    sql: "ALTER TABLE tests_notes ADD COLUMN title TEXT NOT NULL DEFAULT '';",
};

const BROKEN: Migration = Migration {
    legacy_version: None,
    version: 2,
    description: "a migration that fails halfway",
    sql: "CREATE TABLE tests_created_before_failure (id INTEGER) STRICT; SELECT * FROM tests_missing_table;",
};

struct Fixture {
    directory: tempfile::TempDir,
    clock: Arc<TestClock>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            directory: tempfile::tempdir().unwrap(),
            clock: TestClock::at(NOON),
        }
    }

    fn database(&self) -> PathBuf {
        self.directory.path().join("mannie.sqlite")
    }

    fn backups(&self) -> PathBuf {
        self.directory.path().join("backups")
    }

    fn options(&self) -> StoreOptions {
        let clock: Arc<dyn Clock> = Arc::<TestClock>::clone(&self.clock);
        StoreOptions::new(self.backups(), clock)
    }

    /// Opens the database with one module, `tests`, of these migrations.
    fn open(&self, migrations: &[Migration]) -> Result<Store, StoreError> {
        Store::open_with_modules(
            &self.database(),
            &self.options(),
            &[tests_module(migrations)],
        )
    }

    /// Opens the database with every module of this binary.
    fn open_latest(&self) -> Result<Store, StoreError> {
        Store::open(&self.database(), &self.options())
    }

    /// Creates the database as a binary up to 0.2.0 left it at `version`.
    fn open_before_m5(&self, version: u32) -> Store {
        Store::open_as_before_m5(&self.database(), &self.options(), version).unwrap()
    }

    fn backup_files(&self) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(self.backups()) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries.map(|entry| entry.unwrap().path()).collect();
        files.sort();
        files
    }
}

/// The module `tests` with these migrations.
fn tests_module(migrations: &[Migration]) -> StoreModule {
    StoreModule {
        name: "tests",
        migrations: Cow::Owned(migrations.to_vec()),
        confined: false,
    }
}

/// The `user_version` of a database that has every migration of this binary.
fn latest_user_version() -> u32 {
    MODULE_RECORDS + CORE_MODULES.iter().map(StoreModule::version).sum::<u32>()
}

/// Each module's recorded version, in the order of their names.
fn records(path: &Path) -> Vec<(String, u32)> {
    let connection = Connection::open(path).unwrap();
    let mut select = connection
        .prepare("SELECT module, version FROM store_modules ORDER BY module")
        .unwrap();
    select
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn user_version(path: &Path) -> u32 {
    let connection = Connection::open(path).unwrap();
    connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

fn note_bodies(path: &Path) -> Vec<String> {
    let connection = Connection::open(path).unwrap();
    let mut select = connection
        .prepare("SELECT body FROM tests_notes ORDER BY id")
        .unwrap();
    select
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

async fn add_note(store: &Store, body: &'static str) {
    store
        .write(Durability::Full, move |transaction| {
            transaction
                .execute("INSERT INTO tests_notes (body) VALUES (?1)", [body])
                .map(drop)
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn a_new_database_gets_the_whole_schema_in_wal_mode_without_a_backup() {
    let fixture = Fixture::new();

    let store = Store::open(&fixture.database(), &fixture.options()).unwrap();
    let journal_mode: String = store
        .read(|connection| connection.pragma_query_value(None, "journal_mode", |row| row.get(0)))
        .await
        .unwrap();
    drop(store);

    assert_eq!(journal_mode, "wal");
    assert_eq!(user_version(&fixture.database()), latest_user_version());
    assert_eq!(
        records(&fixture.database()),
        [
            ("provider".to_owned(), 1),
            ("runtime".to_owned(), 20),
            ("settings".to_owned(), 1),
            ("tools".to_owned(), 3),
        ]
    );
    assert_eq!(fixture.backup_files(), Vec::<PathBuf>::new());
}

#[tokio::test]
async fn an_existing_database_is_copied_before_it_is_migrated() {
    let fixture = Fixture::new();
    let store = fixture.open(&[FIRST]).unwrap();
    add_note(&store, "written at schema version 1").await;
    drop(store);

    fixture.clock.advance(1_000);
    let store = fixture.open(&[FIRST, SECOND]).unwrap();
    drop(store);

    let backups = fixture.backup_files();
    assert_eq!(backups.len(), 1, "{backups:?}");
    let name = backups[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(name, "mannie.sqlite.schema-1001.20261013T120001.000Z.bak");
    assert_eq!(user_version(&backups[0]), 1001);
    assert_eq!(note_bodies(&backups[0]), ["written at schema version 1"]);
    assert_eq!(user_version(&fixture.database()), 1002);
    assert_eq!(records(&fixture.database()), [("tests".to_owned(), 2)]);
    assert_eq!(
        note_bodies(&fixture.database()),
        ["written at schema version 1"]
    );
}

#[tokio::test]
async fn reopening_an_up_to_date_database_makes_no_copy() {
    let fixture = Fixture::new();
    drop(fixture.open(&[FIRST]).unwrap());

    drop(fixture.open(&[FIRST]).unwrap());

    assert_eq!(fixture.backup_files(), Vec::<PathBuf>::new());
}

#[tokio::test]
async fn a_newer_schema_is_refused_and_left_untouched() {
    let fixture = Fixture::new();
    let store = fixture.open(&[FIRST, SECOND]).unwrap();
    add_note(&store, "written by a newer version").await;
    drop(store);
    let before = fs::read(fixture.database()).unwrap();

    let error = fixture.open(&[FIRST]).unwrap_err();

    match &error {
        StoreError::ModuleTooNew {
            module,
            database,
            supported,
            ..
        } => {
            assert_eq!((module.as_str(), *database, *supported), ("tests", 2, 1));
        }
        other => panic!("expected ModuleTooNew, got {other:?}"),
    }
    assert_eq!(error.code(), "store.schema_too_new");
    let message = error.to_string();
    assert!(
        message.contains("version 2 of the module tests") && message.contains("up to version 1"),
        "{message}"
    );
    assert_eq!(fs::read(fixture.database()).unwrap(), before);
    assert_eq!(fixture.backup_files(), Vec::<PathBuf>::new());
}

#[tokio::test]
async fn a_failed_migration_is_rolled_back_and_keeps_the_previous_version() {
    let fixture = Fixture::new();
    let store = fixture.open(&[FIRST]).unwrap();
    add_note(&store, "kept").await;
    drop(store);

    let error = fixture.open(&[FIRST, BROKEN]).unwrap_err();

    assert!(
        matches!(
            error,
            StoreError::Migration {
                module: "tests",
                version: 2,
                ..
            }
        ),
        "{error:?}"
    );
    assert_eq!(user_version(&fixture.database()), 1001);
    assert_eq!(records(&fixture.database()), [("tests".to_owned(), 1)]);
    assert_eq!(note_bodies(&fixture.database()), ["kept"]);
    let connection = Connection::open(fixture.database()).unwrap();
    let leftover: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name = 'tests_created_before_failure'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(leftover, 0);
    // The copy was made before the migration started.
    assert_eq!(fixture.backup_files().len(), 1);
}

#[tokio::test]
async fn only_the_three_newest_backups_are_kept() {
    let fixture = Fixture::new();
    let third = Migration {
        version: 3,
        sql: "CREATE TABLE tests_three (id INTEGER) STRICT;",
        ..SECOND
    };
    let fourth = Migration {
        version: 4,
        sql: "CREATE TABLE tests_four (id INTEGER) STRICT;",
        ..SECOND
    };
    let fifth = Migration {
        version: 5,
        sql: "CREATE TABLE tests_five (id INTEGER) STRICT;",
        ..SECOND
    };
    let all = [FIRST, SECOND, third, fourth, fifth];

    for count in 1..=all.len() {
        fixture.clock.advance(1_000);
        drop(fixture.open(&all[..count]).unwrap());
        // Backups are ordered by their modification time, which needs to differ.
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let names: Vec<String> = fixture
        .backup_files()
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 3, "{names:?}");
    assert!(
        names.iter().all(|name| !name.contains(".schema-1001.")),
        "{names:?}"
    );
}

/// A database of 0.2.0 has the whole single list of migrations, 22 of them.
/// The first open records each module's version from it, copies the
/// database first, and leaves a `user_version` that 0.2.0 refuses as newer.
#[tokio::test]
async fn a_database_of_0_2_0_gets_the_version_of_each_module() {
    let fixture = Fixture::new();
    drop(fixture.open_before_m5(22));
    assert_eq!(user_version(&fixture.database()), 22);

    fixture.clock.advance(1_000);
    drop(fixture.open_latest().unwrap());

    assert_eq!(
        records(&fixture.database()),
        [
            ("provider".to_owned(), 1),
            ("runtime".to_owned(), 20),
            ("settings".to_owned(), 1),
            ("tools".to_owned(), 3),
        ]
    );
    assert_eq!(user_version(&fixture.database()), latest_user_version());
    assert!(user_version(&fixture.database()) > 22);
    let backups = fixture.backup_files();
    assert_eq!(backups.len(), 1, "{backups:?}");
    assert_eq!(
        backups[0].file_name().unwrap().to_string_lossy(),
        "mannie.sqlite.schema-22.20261013T120001.000Z.bak"
    );
    assert_eq!(user_version(&backups[0]), 22);

    fixture.clock.advance(1_000);
    drop(fixture.open_latest().unwrap());
    assert_eq!(fixture.backup_files().len(), 1);
}

/// A binary that does not know a module leaves its tables and its record as
/// they are, and counts the record in `user_version`.
#[tokio::test]
async fn a_module_that_the_binary_does_not_know_keeps_its_record_and_tables() {
    let fixture = Fixture::new();
    let other = StoreModule {
        name: "other",
        migrations: Cow::Borrowed(&[Migration {
            version: 1,
            legacy_version: None,
            description: "create the other table",
            sql: "CREATE TABLE other_rows (id INTEGER PRIMARY KEY) STRICT;",
        }]),
        confined: false,
    };
    let store = Store::open_with_modules(
        &fixture.database(),
        &fixture.options(),
        &[tests_module(&[FIRST]), other],
    )
    .unwrap();
    store
        .write(Durability::Full, |transaction| {
            transaction
                .execute("INSERT INTO other_rows (id) VALUES (7)", [])
                .map(drop)
        })
        .await
        .unwrap();
    drop(store);

    let store = fixture.open(&[FIRST, SECOND]).unwrap();
    let kept: i64 = store
        .read(|connection| connection.query_row("SELECT id FROM other_rows", [], |row| row.get(0)))
        .await
        .unwrap();
    drop(store);

    assert_eq!(kept, 7);
    assert_eq!(
        records(&fixture.database()),
        [("other".to_owned(), 1), ("tests".to_owned(), 2)]
    );
    assert_eq!(user_version(&fixture.database()), MODULE_RECORDS + 3);
}

/// A database that one binary converted without a module, such as a worker's
/// database without the Workers plugin's module, keeps that module's tables
/// from before M5. A binary that registers the module later counts the
/// migrations that the old list applied and runs only the newer ones.
#[tokio::test]
async fn a_module_registered_after_the_conversion_carries_over_its_migrations() {
    const BEFORE: &[sdk::Migration] = &[
        sdk::Migration::legacy(
            1,
            19,
            "create a table of the old list",
            "CREATE TABLE workers_first (id INTEGER PRIMARY KEY) STRICT;",
        ),
        sdk::Migration::legacy(
            2,
            20,
            "create a second table of the old list",
            "CREATE TABLE workers_second (id INTEGER PRIMARY KEY) STRICT;",
        ),
        sdk::Migration::legacy(
            3,
            22,
            "create a third table of the old list",
            "CREATE TABLE workers_third (id INTEGER PRIMARY KEY) STRICT;",
        ),
    ];
    const LATER: &[sdk::Migration] = &[
        BEFORE[0],
        BEFORE[1],
        BEFORE[2],
        sdk::Migration::new(
            4,
            "create a table after the old list",
            "CREATE TABLE workers_newer (id INTEGER PRIMARY KEY) STRICT;",
        ),
    ];
    let workers = PluginId::parse("workers").unwrap();
    let module =
        |migrations| PluginModule::new(&workers, sdk::StoreModule::new("workers", migrations, &[]));
    let fixture = Fixture::new();
    let older_options = fixture.options().with_plugin_modules([module(BEFORE)]);
    drop(Store::open_as_before_m5(&fixture.database(), &older_options, 22).unwrap());
    drop(fixture.open_latest().unwrap());
    let converted = records(&fixture.database());

    let store = Store::open(
        &fixture.database(),
        &fixture.options().with_plugin_modules([module(LATER)]),
    )
    .unwrap();
    let tables: Vec<String> = store
        .read(|connection| {
            let mut select = connection.prepare(
                "SELECT name FROM sqlite_schema WHERE type = 'table' AND name LIKE 'workers_%' \
                 ORDER BY name",
            )?;
            let rows = select.query_map([], |row| row.get(0))?;
            rows.collect()
        })
        .await
        .unwrap();
    drop(store);

    assert!(
        converted
            .iter()
            .all(|(module, _version)| module != "workers"),
        "{converted:?}"
    );
    assert_eq!(
        tables,
        [
            "workers_first",
            "workers_newer",
            "workers_second",
            "workers_third"
        ]
    );
    assert!(records(&fixture.database()).contains(&("workers".to_owned(), 4)));
    assert_eq!(user_version(&fixture.database()), latest_user_version() + 4);
}

#[tokio::test]
async fn two_modules_with_one_name_are_refused() {
    let fixture = Fixture::new();

    let error = Store::open_with_modules(
        &fixture.database(),
        &fixture.options(),
        &[tests_module(&[FIRST]), tests_module(&[FIRST])],
    )
    .unwrap_err();

    assert!(
        matches!(error, StoreError::DuplicateModule { module: "tests" }),
        "{error:?}"
    );
    assert_eq!(error.code(), "store.duplicate_module");
    assert!(!fixture.database().exists());
}

/// A `user_version` past the single list of migrations without records was
/// written by no version of Mannie, so the database is left untouched.
#[tokio::test]
async fn a_user_version_past_the_list_before_m5_without_records_is_refused() {
    let fixture = Fixture::new();
    let connection = Connection::open(fixture.database()).unwrap();
    connection.pragma_update(None, "user_version", 50).unwrap();
    drop(connection);
    let before = fs::read(fixture.database()).unwrap();

    let error = fixture.open_latest().unwrap_err();

    assert!(
        matches!(
            error,
            StoreError::SchemaTooNew {
                database: 50,
                supported: 22,
                ..
            }
        ),
        "{error:?}"
    );
    assert_eq!(fs::read(fixture.database()).unwrap(), before);
}

#[tokio::test]
async fn the_store_of_a_module_reaches_only_the_modules_that_it_opened_with() {
    const MIGRATIONS: &[sdk::Migration] = &[sdk::Migration::new(
        1,
        "create the notes",
        "CREATE TABLE notes_items (body TEXT NOT NULL) STRICT;",
    )];
    const INSERT: &str = "INSERT INTO notes_items (body) VALUES (?1)";
    const SELECT: &str = "SELECT body FROM notes_items";
    const NOTES: sdk::StoreModule = sdk::StoreModule::new("notes", MIGRATIONS, &[INSERT, SELECT]);
    let notes = PluginId::parse("notes").unwrap();
    let fixture = Fixture::new();
    let store = Store::open(
        &fixture.database(),
        &fixture
            .options()
            .with_plugin_modules([PluginModule::new(&notes, NOTES)]),
    )
    .unwrap();

    let refused = store
        .plugin_store(&PluginId::parse("unknown").unwrap())
        .err()
        .unwrap();
    let module = store.plugin_store(&notes).unwrap();
    module
        .write(
            sdk::WriteBatch::new().push(
                sdk::Statement::new(INSERT).bind("through the module"),
                sdk::Expect::Changes(1),
            ),
            sdk::Durability::Full,
        )
        .await
        .unwrap();
    let bodies = module.query(sdk::Statement::new(SELECT)).await.unwrap();

    assert!(
        matches!(&refused, StoreError::ModuleNotRegistered { module } if module == "unknown"),
        "{refused:?}"
    );
    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0].get::<String>(0).unwrap(), "through the module");
}

#[tokio::test]
async fn a_failed_write_is_rolled_back() {
    let fixture = Fixture::new();
    let store = fixture.open(&[FIRST]).unwrap();

    let error = store
        .write(Durability::Full, |transaction| {
            transaction.execute("INSERT INTO tests_notes (body) VALUES ('rolled back')", [])?;
            transaction.execute("INSERT INTO tests_missing_table VALUES (1)", [])
        })
        .await
        .unwrap_err();
    add_note(&store, "committed").await;
    drop(store);

    assert_eq!(error.code(), "store.query");
    assert_eq!(note_bodies(&fixture.database()), ["committed"]);
}

#[tokio::test]
async fn readers_see_committed_writes_of_both_durabilities() {
    let fixture = Fixture::new();
    let store = fixture.open(&[FIRST]).unwrap();

    add_note(&store, "full").await;
    store
        .write(Durability::Normal, |transaction| {
            transaction
                .execute("INSERT INTO tests_notes (body) VALUES ('normal')", [])
                .map(drop)
        })
        .await
        .unwrap();
    let count: i64 = store
        .read(|connection| {
            connection.query_row("SELECT count(*) FROM tests_notes", [], |row| row.get(0))
        })
        .await
        .unwrap();

    assert_eq!(count, 2);
}

#[tokio::test]
async fn settings_default_until_saved_and_round_trip() {
    let fixture = Fixture::new();
    let store = Store::open(&fixture.database(), &fixture.options()).unwrap();

    let initial: InterfaceSettings = store.load_settings().await.unwrap();
    let chosen = InterfaceSettings {
        locale: Some(LocaleTag::parse("pt-BR").unwrap()),
    };
    store.save_settings(&chosen).await.unwrap();
    let saved: InterfaceSettings = store.load_settings().await.unwrap();

    assert_eq!(initial, InterfaceSettings::default());
    assert_eq!(saved, chosen);
    let stored: (u32, String, i64) = store
        .read(|connection| {
            connection.query_row(
                "SELECT version, document, updated_at FROM settings_sections WHERE key = 'interface'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        stored,
        (
            InterfaceSettings::VERSION,
            r#"{"locale":"pt-BR"}"#.to_owned(),
            NOON
        )
    );
}

#[tokio::test]
async fn settings_from_a_newer_version_are_neither_read_nor_overwritten() {
    let fixture = Fixture::new();
    let store = Store::open(&fixture.database(), &fixture.options()).unwrap();
    store
        .write(Durability::Full, |transaction| {
            transaction
                .execute(
                    "INSERT INTO settings_sections (key, version, document, updated_at) \
                     VALUES ('interface', 99, '{\"locale\":\"en\",\"future\":true}', 0)",
                    [],
                )
                .map(drop)
        })
        .await
        .unwrap();

    let read = store
        .load_settings::<InterfaceSettings>()
        .await
        .unwrap_err();
    let saved = store
        .save_settings(&InterfaceSettings::default())
        .await
        .unwrap_err();
    let document: String = store
        .read(|connection| {
            connection.query_row(
                "SELECT document FROM settings_sections WHERE key = 'interface'",
                [],
                |row| row.get(0),
            )
        })
        .await
        .unwrap();

    for error in [&read, &saved] {
        assert!(
            matches!(
                error,
                SettingsStoreError::NewerVersion {
                    stored: 99,
                    known: 1,
                    ..
                }
            ),
            "{error:?}"
        );
        assert_eq!(error.code(), "settings.newer_version");
    }
    assert_eq!(document, r#"{"locale":"en","future":true}"#);
}

#[tokio::test]
async fn an_invalid_stored_section_is_reported() {
    let fixture = Fixture::new();
    let store = Store::open(&fixture.database(), &fixture.options()).unwrap();
    store
        .write(Durability::Full, |transaction| {
            transaction
                .execute(
                    "INSERT INTO settings_sections (key, version, document, updated_at) \
                     VALUES ('interface', 1, '{\"locale\":\"not a tag\"}', 0)",
                    [],
                )
                .map(drop)
        })
        .await
        .unwrap();

    let error = store
        .load_settings::<InterfaceSettings>()
        .await
        .unwrap_err();

    assert!(
        matches!(
            error,
            SettingsStoreError::Invalid {
                key: "interface",
                ..
            }
        ),
        "{error:?}"
    );
}

/// The database of M0 has only the settings table. M1's migrations add the
/// runtime's and the profiles' tables, keep the settings and copy the database
/// first (engineering standards, data).
#[tokio::test]
async fn a_database_from_the_first_schema_is_upgraded_and_keeps_its_settings() {
    let fixture = Fixture::new();
    let older = fixture.open_before_m5(1);
    let settings = InterfaceSettings {
        locale: Some(LocaleTag::parse("de").unwrap()),
    };
    older.save_settings(&settings).await.unwrap();
    drop(older);
    assert_eq!(user_version(&fixture.database()), 1);

    let upgraded = fixture.open_latest().unwrap();

    assert_eq!(user_version(&fixture.database()), latest_user_version());
    assert_eq!(
        upgraded.load_settings::<InterfaceSettings>().await.unwrap(),
        settings
    );
    assert_eq!(upgraded.list_conversations().await.unwrap(), []);
    assert_eq!(upgraded.list_profiles().await.unwrap(), []);
    let backups = fixture.backup_files();
    assert_eq!(backups.len(), 1, "{backups:?}");
    assert_eq!(user_version(&backups[0]), 1);
}

/// The schema of M1 (version 6) holds conversations, runs, parts and
/// profiles. M2's migrations add kept runs, workspaces, archives, grants and
/// rules; the stored conversation and profile read as before, without a
/// workspace and with tools on (engineering standards, data). A fixture
/// database of v0.1.0 replaces this seed once that release exists.
#[tokio::test]
async fn a_database_from_the_chat_schema_is_upgraded_and_keeps_its_conversations() {
    let fixture = Fixture::new();
    let older = fixture.open_before_m5(6);
    older
        .write(Durability::Full, |transaction| {
            transaction.execute_batch(
                "INSERT INTO runtime_conversations (id, title, title_source, profile_id, last_seq, created_at, updated_at) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000001', 'Older chat', 'first_message', NULL, 0, 1, 1); \
                 INSERT INTO runtime_turns (id, conversation_id, position, current_run_id, created_at) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000002', '0199e0f2-7a1b-7c3d-8e4f-000000000001', 0, \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000003', 1); \
                 INSERT INTO runtime_runs (id, conversation_id, turn_id, agent, profile_id, binding, state, version, \
                 outcome, created_at, started_at, ended_at) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000003', '0199e0f2-7a1b-7c3d-8e4f-000000000001', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000002', 'conversation', '0199e0f2-0000-7000-8000-000000000001', \
                 '0199e0f2-0000-7000-8000-000000000001:1', 'completed', 1, '{}', 1, 1, 2); \
                 INSERT INTO runtime_parts (id, conversation_id, turn_id, run_id, attempt, position, kind, audience, \
                 state, text, version, document, created_at, finished_at) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000004', '0199e0f2-7a1b-7c3d-8e4f-000000000001', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000002', NULL, NULL, 0, 'user_text', 'both', 'complete', 'Hello', \
                 1, '{}', 1, 1), \
                 ('0199e0f2-7a1b-7c3d-8e4f-000000000005', '0199e0f2-7a1b-7c3d-8e4f-000000000001', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000002', '0199e0f2-7a1b-7c3d-8e4f-000000000003', 1, 1, 'text', \
                 'both', 'complete', 'Hi there', 1, '{}', 1, 2); \
                 INSERT INTO provider_profiles (id, name, interface, base_url, model, credential_ref, is_default, \
                 config_revision, version, document, created_at, updated_at) \
                 VALUES ('0199e0f2-0000-7000-8000-000000000001', 'Local', 'chat_completions', \
                 'http://127.0.0.1:8000/v1', 'qwen3', NULL, 1, 1, 1, \
                 '{\"contextWindow\":32768,\"outputLimit\":4096,\"reasoningEffort\":\"server_default\"}', 1, 1);",
            )
        })
        .await
        .unwrap();
    drop(older);
    assert_eq!(user_version(&fixture.database()), 6);

    let upgraded = fixture.open_latest().unwrap();

    assert_eq!(user_version(&fixture.database()), latest_user_version());
    let conversations = upgraded.list_conversations().await.unwrap();
    let [conversation] = conversations.as_slice() else {
        panic!("{conversations:?}");
    };
    assert_eq!(conversation.title, "Older chat");
    assert_eq!(conversation.workspace, None);
    assert!(!conversation.awaiting_approval);
    let turns = upgraded.model_turns(&conversation.id, None).await.unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].kept_runs, Vec::new());
    assert_eq!(turns[0].parts.len(), 2);
    let profiles = upgraded.list_profiles().await.unwrap();
    assert_eq!(profiles.len(), 1);
    assert!(profiles[0].use_tools);
    assert_eq!(upgraded.rules().await.unwrap(), Vec::new());
    assert_eq!(fixture.backup_files().len(), 1);
}

/// The schema before projects (version 11) keeps a workspace on each
/// conversation. Migration 13 turns each workspace into a project with the ID
/// and time of its earliest conversation, and moves the conversations into
/// it (docs/features/chat/projects.md, data).
#[tokio::test]
async fn a_database_with_workspaces_is_upgraded_into_projects() {
    let fixture = Fixture::new();
    let older = fixture.open_before_m5(11);
    older
        .write(Durability::Full, |transaction| {
            transaction.execute_batch(
                "INSERT INTO runtime_conversations (id, title, title_source, profile_id, last_seq, \
                 workspace, created_at, updated_at) VALUES \
                 ('0199e0f2-7a1b-7c3d-8e4f-000000000002', 'Later in A', 'first_message', NULL, 0, '/work/a', 20, 20), \
                 ('0199e0f2-7a1b-7c3d-8e4f-000000000001', 'First in A', 'first_message', NULL, 0, '/work/a', 10, 10), \
                 ('0199e0f2-7a1b-7c3d-8e4f-000000000003', 'Only in B', 'first_message', NULL, 0, '/work/b', 30, 30), \
                 ('0199e0f2-7a1b-7c3d-8e4f-000000000004', 'General', 'first_message', NULL, 0, NULL, 40, 40);",
            )
        })
        .await
        .unwrap();
    drop(older);
    assert_eq!(user_version(&fixture.database()), 11);

    let upgraded = fixture.open_latest().unwrap();

    assert_eq!(user_version(&fixture.database()), latest_user_version());
    let projects = upgraded.projects().await.unwrap();
    let summary = |project: &mannie_types::projects::ProjectView| {
        (
            project.id.to_string(),
            project.name.clone(),
            project.folder.clone(),
            project.created_at.as_millis(),
        )
    };
    assert_eq!(
        projects.iter().map(summary).collect::<Vec<_>>(),
        [
            (
                "0199e0f2-7a1b-7c3d-8e4f-000000000001".to_owned(),
                "a".to_owned(),
                "/work/a".to_owned(),
                10
            ),
            (
                "0199e0f2-7a1b-7c3d-8e4f-000000000003".to_owned(),
                "b".to_owned(),
                "/work/b".to_owned(),
                30
            ),
        ]
    );
    let conversations = upgraded.list_conversations().await.unwrap();
    let placed: Vec<(String, Option<String>, Option<String>)> = conversations
        .iter()
        .map(|conversation| {
            (
                conversation.title.clone(),
                conversation.project_id.as_ref().map(ToString::to_string),
                conversation.workspace.clone(),
            )
        })
        .collect();
    let in_a = Some("0199e0f2-7a1b-7c3d-8e4f-000000000001".to_owned());
    let in_b = Some("0199e0f2-7a1b-7c3d-8e4f-000000000003".to_owned());
    assert_eq!(
        placed,
        [
            ("General".to_owned(), None, None),
            ("Only in B".to_owned(), in_b, Some("/work/b".to_owned())),
            (
                "Later in A".to_owned(),
                in_a.clone(),
                Some("/work/a".to_owned())
            ),
            ("First in A".to_owned(), in_a, Some("/work/a".to_owned())),
        ]
    );
    let violations: Vec<String> = upgraded
        .read(|connection| {
            let mut check = connection.prepare("PRAGMA foreign_key_check")?;
            let rows = check.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect()
        })
        .await
        .unwrap();
    assert_eq!(violations, Vec::<String>::new());
}

/// The schema of system tools and projects (version 14) has no task
/// conversations. Migrations 15 and 16 add them; the stored conversation, its
/// project and its tool call read as before, as one of the owner's
/// conversations (docs/features/child-agents/tasks.md, data). No release
/// writes this schema, since M2, M3 and M4 ship together in v0.2.0
/// (docs/roadmap/milestones.md, D23), so the seed stays.
#[tokio::test]
async fn a_database_from_the_tools_schema_is_upgraded_for_task_conversations() {
    let fixture = Fixture::new();
    let older = fixture.open_before_m5(14);
    older
        .write(Durability::Full, |transaction| {
            transaction.execute_batch(
                "INSERT INTO runtime_projects (id, name, folder, created_at, updated_at) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000009', NULL, '/work/app', 1, 1); \
                 INSERT INTO runtime_conversations (id, title, title_source, profile_id, last_seq, \
                 workspace, project_id, created_at, updated_at) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000001', 'With tools', 'first_message', NULL, 0, \
                 '/work/app', '0199e0f2-7a1b-7c3d-8e4f-000000000009', 1, 1); \
                 INSERT INTO runtime_turns (id, conversation_id, position, current_run_id, created_at) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000002', '0199e0f2-7a1b-7c3d-8e4f-000000000001', 0, \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000003', 1); \
                 INSERT INTO runtime_runs (id, conversation_id, turn_id, agent, profile_id, binding, state, version, \
                 outcome, created_at, started_at, ended_at, kept) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000003', '0199e0f2-7a1b-7c3d-8e4f-000000000001', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000002', 'conversation', '0199e0f2-0000-7000-8000-000000000001', \
                 '0199e0f2-0000-7000-8000-000000000001:1', 'completed', 1, '{}', 1, 1, 2, 0); \
                 INSERT INTO runtime_parts (id, conversation_id, turn_id, run_id, attempt, position, kind, audience, \
                 state, text, version, document, created_at, finished_at) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000004', '0199e0f2-7a1b-7c3d-8e4f-000000000001', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000002', NULL, NULL, 0, 'user_text', 'both', 'complete', 'List it', \
                 1, '{}', 1, 1), \
                 ('0199e0f2-7a1b-7c3d-8e4f-000000000005', '0199e0f2-7a1b-7c3d-8e4f-000000000001', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000002', '0199e0f2-7a1b-7c3d-8e4f-000000000003', 1, 1, 'tool_call', \
                 'both', 'complete', '{\"path\":\".\"}', 1, \
                 '{\"toolCall\":{\"callId\":\"call_1\",\"tool\":\"list_directory\",\"status\":\"completed\"}}', 1, 2);",
            )
        })
        .await
        .unwrap();
    drop(older);
    assert_eq!(user_version(&fixture.database()), 14);

    let upgraded = fixture.open_latest().unwrap();

    assert_eq!(user_version(&fixture.database()), latest_user_version());
    let conversations = upgraded.list_conversations().await.unwrap();
    let [conversation] = conversations.as_slice() else {
        panic!("{conversations:?}");
    };
    assert_eq!(conversation.title, "With tools");
    assert_eq!(conversation.task, None);
    assert_eq!(conversation.workspace.as_deref(), Some("/work/app"));
    let turns = upgraded.model_turns(&conversation.id, None).await.unwrap();
    assert_eq!(turns[0].parts.len(), 2);
    assert_eq!(
        turns[0].parts[1]
            .tool_call
            .as_ref()
            .map(|call| call.tool.as_str()),
        Some("list_directory")
    );
    assert_eq!(turns[0].parts[1].task, None);
    assert_eq!(
        upgraded.task_conversations(&conversation.id).await.unwrap(),
        Vec::new()
    );
    assert_eq!(
        upgraded.next_task_number(&conversation.id).await.unwrap(),
        1
    );
    let violations: Vec<String> = upgraded
        .read(|connection| {
            let mut check = connection.prepare("PRAGMA foreign_key_check")?;
            let rows = check.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect()
        })
        .await
        .unwrap();
    assert_eq!(violations, Vec::<String>::new());
}

/// The schema of child agents (version 16) keeps every task with its part
/// and its turn. Migration 17 builds the tasks table again for worker tasks
/// and 18 adds the worker of a task conversation; a stored child and its
/// ended task read as before (plugins/workers/docs/delegation.md, data). No
/// release writes this schema, since M2, M3 and M4 ship together in v0.2.0
/// (docs/roadmap/milestones.md, D23), so the seed stays.
#[tokio::test]
async fn a_database_from_the_child_agents_schema_is_upgraded_for_worker_tasks() {
    let fixture = Fixture::new();
    let older = fixture.open_before_m5(16);
    seed_child_agents_schema(&older).await;
    drop(older);
    assert_eq!(user_version(&fixture.database()), 16);

    let upgraded = fixture.open_latest().unwrap();

    assert_eq!(user_version(&fixture.database()), latest_user_version());
    let parent = ConversationId::parse("0199e0f2-7a1b-7c3d-8e4f-000000000001").unwrap();
    let task_id = TaskId::parse("0199e0f2-7a1b-7c3d-8e4f-000000000021").unwrap();
    let task = upgraded.task(&task_id).await.unwrap().unwrap();
    assert_eq!(
        task.part_id.as_ref().map(PartId::as_str),
        Some("0199e0f2-7a1b-7c3d-8e4f-000000000005")
    );
    assert_eq!(
        task.turn_id.as_ref().map(TurnId::as_str),
        Some("0199e0f2-7a1b-7c3d-8e4f-000000000012")
    );
    assert_eq!(task.state, TaskState::Reported);
    assert_eq!(
        task.ending.and_then(|ending| ending.report).as_deref(),
        Some("In session.rs.")
    );
    assert_eq!(task.executor, None);
    assert_eq!(task.assignment, None);
    assert_eq!(task.state_version, 0);
    let child = upgraded.child_by_number(&parent, 1).await.unwrap().unwrap();
    assert_eq!(child.agent, "explore");
    assert_eq!(child.executor, None);
    assert_eq!(child.latest.map(|task| task.id), Some(task_id));
    let summary = upgraded
        .conversation_summary(&child.conversation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.task.and_then(|origin| origin.executor), None);
    let indexes = task_indexes(&upgraded).await;
    // A task needs the part or the assignment that started it.
    let refused = upgraded
        .write(Durability::Full, |transaction| {
            transaction.execute(
                "INSERT INTO runtime_tasks (id, conversation_id, part_id, turn_id, follow_up, \
                 background, state, version, created_at) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000022', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000011', NULL, \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000012', 0, 0, 'reported', 1, 3)",
                [],
            )
        })
        .await;
    assert_eq!(
        indexes,
        [
            "runtime_tasks_by_executor",
            "runtime_tasks_by_part",
            "runtime_tasks_by_state_version",
            "runtime_tasks_by_turn",
            "runtime_tasks_by_worker",
            "runtime_tasks_one_active",
        ]
    );
    refused.unwrap_err();
    let violations: Vec<String> = upgraded
        .read(|connection| {
            let mut check = connection.prepare("PRAGMA foreign_key_check")?;
            let rows = check.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect()
        })
        .await
        .unwrap();
    assert_eq!(violations, Vec::<String>::new());
}

/// A task that 0.2.0 delegated to a worker, which still runs there with a
/// cancel that waits for the worker, keeps its state. M5's migrations name the
/// Workers plugin's executor in its row, its conversation and its part, and
/// move the worker's columns into the executor's document (ADR-0031,
/// decision 7). The calls that 0.2.0 refused keep the text that the model
/// read, which the runtime no longer writes from the refusal.
#[tokio::test]
async fn a_database_of_0_2_0_names_the_executor_of_a_workers_task() {
    let fixture = Fixture::new();
    let older = fixture.open_before_m5(22);
    seed_worker_task(&older).await;
    drop(older);

    let upgraded = fixture.open_latest().unwrap();

    let task_id = TaskId::parse("0199e0f2-7a1b-7c3d-8e4f-000000000021").unwrap();
    let task = upgraded.task(&task_id).await.unwrap().unwrap();
    assert_eq!(task.state, TaskState::Running);
    assert_eq!(task.turn_id, None);
    let execution = task.executor.unwrap();
    assert_eq!(execution.kind, "workers");
    assert_eq!(execution.target, "0199e0f2-0000-7000-b000-000000000001");
    assert_eq!(execution.version, 4);
    let document: serde_json::Value =
        serde_json::from_str(execution.document.as_deref().unwrap()).unwrap();
    assert_eq!(
        document,
        serde_json::json!({
            "version": 1,
            "profileId": "0199e0f2-0000-7000-8000-000000000001",
            "submission": "accepted",
            "cancelRequestedAt": 3
        })
    );
    assert_eq!(
        upgraded
            .active_executor_tasks("workers")
            .await
            .unwrap()
            .iter()
            .map(|task| task.id.clone())
            .collect::<Vec<_>>(),
        std::slice::from_ref(&task_id)
    );
    let parent = ConversationId::parse("0199e0f2-7a1b-7c3d-8e4f-000000000001").unwrap();
    let child = upgraded.child_by_number(&parent, 1).await.unwrap().unwrap();
    let expected = mannie_types::tasks::TaskExecutorRef {
        kind: "workers".to_owned(),
        target_id: "0199e0f2-0000-7000-b000-000000000001".to_owned(),
        target_name: "build-01".to_owned(),
        noun: "the worker".to_owned(),
    };
    assert_eq!(child.executor.as_ref(), Some(&expected));
    let summary = upgraded
        .conversation_summary(&child.conversation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        summary.task.and_then(|origin| origin.executor),
        Some(expected.clone())
    );
    let turns = upgraded.model_turns(&parent, None).await.unwrap();
    let section = turns[0].parts[1].task.clone().unwrap();
    let document = section.task.unwrap();
    assert_eq!(document.executor, Some(expected));
    assert!(
        document.worker.is_some(),
        "the worker's field stays for 0.3.0"
    );
    assert_eq!(
        upgraded.executor_outbox("workers").await.unwrap(),
        Vec::new()
    );
    let refused: Vec<_> = turns[0].parts[2..]
        .iter()
        .map(|part| {
            let section = part.task.clone().unwrap();
            assert!(section.refusal.is_some(), "the refusal stays");
            section.output.unwrap()
        })
        .collect();
    assert_eq!(
        refused,
        [
            "Not run: the worker build-01 is offline. Choose another worker, or tell the owner.",
            "Not run: there is no worker named build-02.",
            "Not run: the worker build-03 runs a version of Mannie that this one does not work \
             with. Tell the owner to update it.",
            "Not run: this conversation's provider profile uses an address on this computer, \
             which the worker build-04 cannot reach. Tell the owner.",
        ]
    );
}

/// The worker tasks of [`seed_worker_tasks_in_every_state`]: each task's
/// number, its state, and its ending as 0.2.0 stored it.
const WORKER_TASKS_IN_EVERY_STATE: [(u32, TaskState, Option<&str>); 6] = [
    (1, TaskState::Accepted, None),
    (2, TaskState::Running, None),
    (
        3,
        TaskState::Reported,
        Some(r#"{"state":"reported","report":"Built."}"#),
    ),
    (
        4,
        TaskState::Failed,
        Some(r#"{"state":"failed","failure":{"code":"timed_out"}}"#),
    ),
    (5, TaskState::Canceled, Some(r#"{"state":"canceled"}"#)),
    (
        6,
        TaskState::Interrupted,
        Some(r#"{"state":"interrupted"}"#),
    ),
];

const SEEDED_WORKER: &str = "0199e0f2-0000-7000-b000-000000000001";

fn worker_task_id(number: u32) -> TaskId {
    TaskId::parse(&format!("0199e0f2-7a1b-7c3d-8e4f-0000000003{number:02}")).unwrap()
}

/// Worker tasks of 0.2.0 in every state keep their states and endings, the
/// worker's columns become the Workers plugin's executor's, and only the
/// tasks that are accepted or running stay active for the executor (ADR-0031,
/// decision 8).
#[tokio::test]
async fn a_database_of_0_2_0_keeps_its_workers_tasks_in_every_state() {
    let fixture = Fixture::new();
    let older = fixture.open_before_m5(22);
    seed_worker_tasks_in_every_state(&older).await;
    drop(older);

    let upgraded = fixture.open_latest().unwrap();

    let mut expected_active = Vec::new();
    for (number, state, ending) in WORKER_TASKS_IN_EVERY_STATE {
        let task = upgraded
            .task(&worker_task_id(number))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(task.state, state, "task {number}");
        let execution = task.executor.clone().unwrap();
        assert_eq!(execution.kind, "workers", "task {number}");
        assert_eq!(execution.target, SEEDED_WORKER, "task {number}");
        assert_eq!(execution.version, u64::from(number), "task {number}");
        let document: serde_json::Value =
            serde_json::from_str(execution.document.as_deref().unwrap()).unwrap();
        assert_eq!(
            document,
            serde_json::json!({
                "version": 1,
                "profileId": "0199e0f2-0000-7000-8000-000000000001",
                "submission": "accepted",
                "cancelRequestedAt": null
            }),
            "task {number}"
        );
        let stored_ending =
            ending.map(|text| serde_json::from_str::<crate::TaskEnding>(text).unwrap());
        assert_eq!(task.ending, stored_ending, "task {number}");
        if stored_ending.is_none() {
            expected_active.push(task.id);
        }
    }
    let mut active: Vec<_> = upgraded
        .active_executor_tasks("workers")
        .await
        .unwrap()
        .into_iter()
        .map(|task| task.id)
        .collect();
    active.sort_by(|left, right| left.as_str().cmp(right.as_str()));

    assert_eq!(active, expected_active);
}

/// Every task that 0.2.0 delegated to a worker names the part of the run
/// that delegated it, so after M5's migrations each names that run's profile
/// for the credentials that the Workers plugin borrows (ADR-0034, decision
/// 5), and is active while it was accepted or running.
#[tokio::test]
async fn a_task_that_0_2_0_delegated_names_the_profile_of_its_run() {
    let fixture = Fixture::new();
    let older = fixture.open_before_m5(22);
    seed_worker_task(&older).await;
    seed_worker_tasks_in_every_state(&older).await;
    drop(older);

    let upgraded = fixture.open_latest().unwrap();

    let profile = ProfileId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap();
    let running = TaskId::parse("0199e0f2-7a1b-7c3d-8e4f-000000000021").unwrap();
    let mut tasks = vec![(running, TaskState::Running)];
    tasks.extend(
        WORKER_TASKS_IN_EVERY_STATE
            .iter()
            .map(|(number, state, _ending)| (worker_task_id(*number), *state)),
    );
    for (task, state) in tasks {
        let named = upgraded
            .executor_task_profile(&task, "workers")
            .await
            .unwrap();
        assert_eq!(
            named,
            Some(ExecutorTaskProfile {
                active: state.is_active(),
                profile_id: Some(profile.clone()),
            }),
            "task {task}"
        );
        assert_eq!(
            upgraded
                .executor_task_profile(&task, "other")
                .await
                .unwrap(),
            None,
            "task {task}"
        );
    }
}

/// A parent whose run delegated six tasks to one worker, as 0.2.0 stored
/// them, each in a task conversation of its own and in another state.
async fn seed_worker_tasks_in_every_state(older: &Store) {
    let parent = "0199e0f2-7a1b-7c3d-8e4f-000000000101";
    let turn = "0199e0f2-7a1b-7c3d-8e4f-000000000102";
    let run = "0199e0f2-7a1b-7c3d-8e4f-000000000103";
    let mut sql = format!(
        "INSERT INTO runtime_conversations (id, title, title_source, profile_id, last_seq, \
         parent_conversation_id, task_number, task_agent, task_worker_id, task_worker_name, \
         created_at, updated_at) \
         VALUES ('{parent}', 'Build everything', 'first_message', NULL, 0, NULL, NULL, NULL, NULL, NULL, 1, 1); \
         INSERT INTO runtime_turns (id, conversation_id, position, current_run_id, created_at) \
         VALUES ('{turn}', '{parent}', 0, '{run}', 1); \
         INSERT INTO runtime_runs (id, conversation_id, turn_id, agent, profile_id, binding, state, version, \
         outcome, created_at, started_at, ended_at, kept) \
         VALUES ('{run}', '{parent}', '{turn}', 'conversation', '0199e0f2-0000-7000-8000-000000000001', \
         '0199e0f2-0000-7000-8000-000000000001:1', 'stopped', 1, '{{}}', 1, 1, 3, 0);"
    );
    for (number, state, ending) in WORKER_TASKS_IN_EVERY_STATE {
        let conversation = format!("0199e0f2-7a1b-7c3d-8e4f-0000000002{number:02}");
        let part = format!("0199e0f2-7a1b-7c3d-8e4f-0000000004{number:02}");
        let task = worker_task_id(number);
        let state = serde_json::to_value(state).unwrap();
        let state = state.as_str().unwrap();
        let ending = ending.map_or_else(|| "NULL".to_owned(), |text| format!("'{text}'"));
        let ended_at = if ending == "NULL" { "NULL" } else { "2" };
        write!(
            sql,
            " INSERT INTO runtime_conversations (id, title, title_source, profile_id, last_seq, \
             parent_conversation_id, task_number, task_agent, task_worker_id, task_worker_name, \
             created_at, updated_at) \
             VALUES ('{conversation}', 'Task {number}', 'first_message', NULL, 0, '{parent}', {number}, \
             'worker', '{SEEDED_WORKER}', 'build-01', 1, 1); \
             INSERT INTO runtime_parts (id, conversation_id, turn_id, run_id, attempt, position, kind, \
             audience, state, text, version, document, created_at, finished_at) \
             VALUES ('{part}', '{parent}', '{turn}', '{run}', 1, {number}, 'task', 'both', 'complete', \
             '{{}}', 1, '{{\"toolCall\":{{\"callId\":\"call_{number}\",\"tool\":\"delegate_task\",\
             \"status\":\"completed\"}},\"task\":{{\"task\":{{\"taskId\":\"{task}\",\"number\":{number},\
             \"conversationId\":\"{conversation}\",\"agent\":\"worker\",\"title\":\"Task {number}\",\
             \"state\":\"{state}\",\"worker\":{{\"id\":\"{SEEDED_WORKER}\",\"name\":\"build-01\"}},\
             \"detached\":false}}}}}}', 1, 3); \
             INSERT INTO runtime_tasks (id, conversation_id, part_id, turn_id, follow_up, background, \
             state, version, ending, created_at, ended_at, worker_id, profile_id, remote_version, \
             submission, cancel_requested_at) \
             VALUES ('{task}', '{conversation}', '{part}', NULL, 0, 0, '{state}', 1, {ending}, 1, \
             {ended_at}, '{SEEDED_WORKER}', '0199e0f2-0000-7000-8000-000000000001', {number}, \
             'accepted', NULL);"
        )
        .unwrap();
    }
    older
        .write(Durability::Full, move |transaction| {
            transaction.execute_batch(&sql)
        })
        .await
        .unwrap();
}

/// A parent whose run delegated a task to a worker, as 0.2.0 stored it: the
/// part with the worker, the task conversation with the worker, and the
/// task's row with the worker's columns. Four more calls of the run were
/// refused, one for each refusal of workers.
async fn seed_worker_task(older: &Store) {
    older
        .write(Durability::Full, |transaction| {
            transaction.execute_batch(
                "INSERT INTO runtime_conversations (id, title, title_source, profile_id, last_seq, \
                 parent_conversation_id, task_number, task_agent, task_worker_id, task_worker_name, \
                 created_at, updated_at) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000001', 'Build it on the server', \
                 'first_message', NULL, 0, NULL, NULL, NULL, NULL, NULL, 1, 1), \
                 ('0199e0f2-7a1b-7c3d-8e4f-000000000011', 'Build the backend', 'first_message', \
                 NULL, 0, '0199e0f2-7a1b-7c3d-8e4f-000000000001', 1, 'worker', \
                 '0199e0f2-0000-7000-b000-000000000001', 'build-01', 1, 1); \
                 INSERT INTO runtime_turns (id, conversation_id, position, current_run_id, created_at) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000002', '0199e0f2-7a1b-7c3d-8e4f-000000000001', 0, \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000003', 1); \
                 INSERT INTO runtime_runs (id, conversation_id, turn_id, agent, profile_id, binding, state, version, \
                 outcome, created_at, started_at, ended_at, kept) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000003', '0199e0f2-7a1b-7c3d-8e4f-000000000001', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000002', 'conversation', '0199e0f2-0000-7000-8000-000000000001', \
                 '0199e0f2-0000-7000-8000-000000000001:1', 'stopped', 1, '{}', 1, 1, 3, 0); \
                 INSERT INTO runtime_parts (id, conversation_id, turn_id, run_id, attempt, position, kind, audience, \
                 state, text, version, document, created_at, finished_at) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000004', '0199e0f2-7a1b-7c3d-8e4f-000000000001', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000002', NULL, NULL, 0, 'user_text', 'both', 'complete', \
                 'Build it on the server', 1, '{}', 1, 1), \
                 ('0199e0f2-7a1b-7c3d-8e4f-000000000005', '0199e0f2-7a1b-7c3d-8e4f-000000000001', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000002', '0199e0f2-7a1b-7c3d-8e4f-000000000003', 1, 1, 'task', \
                 'both', 'complete', '{}', 1, \
                 '{\"toolCall\":{\"callId\":\"call_1\",\"tool\":\"delegate_task\",\"status\":\"completed\"},\
                 \"task\":{\"task\":{\"taskId\":\"0199e0f2-7a1b-7c3d-8e4f-000000000021\",\"number\":1,\
                 \"conversationId\":\"0199e0f2-7a1b-7c3d-8e4f-000000000011\",\"agent\":\"worker\",\
                 \"title\":\"Build the backend\",\"state\":\"running\",\
                 \"worker\":{\"id\":\"0199e0f2-0000-7000-b000-000000000001\",\"name\":\"build-01\"},\
                 \"detached\":true}}}', 1, 3), \
                 ('0199e0f2-7a1b-7c3d-8e4f-000000000006', '0199e0f2-7a1b-7c3d-8e4f-000000000001', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000002', '0199e0f2-7a1b-7c3d-8e4f-000000000003', 1, 2, 'task', \
                 'both', 'complete', '{}', 1, \
                 '{\"toolCall\":{\"callId\":\"call_2\",\"tool\":\"delegate_task\",\"status\":\"completed\"},\
                 \"task\":{\"refusal\":{\"reason\":\"worker_offline\",\"worker\":\"build-01\"},\
                 \"output\":null}}', 1, 3), \
                 ('0199e0f2-7a1b-7c3d-8e4f-000000000007', '0199e0f2-7a1b-7c3d-8e4f-000000000001', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000002', '0199e0f2-7a1b-7c3d-8e4f-000000000003', 1, 3, 'task', \
                 'both', 'complete', '{}', 1, \
                 '{\"toolCall\":{\"callId\":\"call_3\",\"tool\":\"delegate_task\",\"status\":\"completed\"},\
                 \"task\":{\"refusal\":{\"reason\":\"worker_unknown\",\"worker\":\"build-02\"}}}', 1, 3), \
                 ('0199e0f2-7a1b-7c3d-8e4f-000000000008', '0199e0f2-7a1b-7c3d-8e4f-000000000001', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000002', '0199e0f2-7a1b-7c3d-8e4f-000000000003', 1, 4, 'task', \
                 'both', 'complete', '{}', 1, \
                 '{\"toolCall\":{\"callId\":\"call_4\",\"tool\":\"delegate_task\",\"status\":\"completed\"},\
                 \"task\":{\"refusal\":{\"reason\":\"worker_incompatible\",\"worker\":\"build-03\"}}}', 1, 3), \
                 ('0199e0f2-7a1b-7c3d-8e4f-000000000009', '0199e0f2-7a1b-7c3d-8e4f-000000000001', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000002', '0199e0f2-7a1b-7c3d-8e4f-000000000003', 1, 5, 'task', \
                 'both', 'complete', '{}', 1, \
                 '{\"toolCall\":{\"callId\":\"call_5\",\"tool\":\"delegate_task\",\"status\":\"completed\"},\
                 \"task\":{\"refusal\":{\"reason\":\"profile_not_reachable\",\"worker\":\"build-04\"}}}', 1, 3); \
                 INSERT INTO runtime_tasks (id, conversation_id, part_id, turn_id, follow_up, background, state, \
                 version, created_at, worker_id, profile_id, remote_version, submission, cancel_requested_at) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000021', '0199e0f2-7a1b-7c3d-8e4f-000000000011', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000005', NULL, 0, 0, 'running', 1, 1, \
                 '0199e0f2-0000-7000-b000-000000000001', '0199e0f2-0000-7000-8000-000000000001', 4, \
                 'accepted', 3);",
            )
        })
        .await
        .unwrap();
}

/// A parent with a child and its ended task, as the schema of child agents
/// stored them.
async fn seed_child_agents_schema(older: &Store) {
    older
        .write(Durability::Full, |transaction| {
            transaction.execute_batch(
                "INSERT INTO runtime_conversations (id, title, title_source, profile_id, last_seq, \
                 parent_conversation_id, task_number, task_agent, created_at, updated_at) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000001', 'Split the work', 'first_message', \
                 NULL, 0, NULL, NULL, NULL, 1, 1), \
                 ('0199e0f2-7a1b-7c3d-8e4f-000000000011', 'Find the sessions', 'first_message', \
                 NULL, 0, '0199e0f2-7a1b-7c3d-8e4f-000000000001', 1, 'explore', 1, 1); \
                 INSERT INTO runtime_turns (id, conversation_id, position, current_run_id, created_at) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000002', '0199e0f2-7a1b-7c3d-8e4f-000000000001', 0, \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000003', 1), \
                 ('0199e0f2-7a1b-7c3d-8e4f-000000000012', '0199e0f2-7a1b-7c3d-8e4f-000000000011', 0, \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000013', 1); \
                 INSERT INTO runtime_runs (id, conversation_id, turn_id, agent, profile_id, binding, state, version, \
                 outcome, created_at, started_at, ended_at, kept) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000003', '0199e0f2-7a1b-7c3d-8e4f-000000000001', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000002', 'conversation', '0199e0f2-0000-7000-8000-000000000001', \
                 '0199e0f2-0000-7000-8000-000000000001:1', 'completed', 1, '{}', 1, 1, 3, 0), \
                 ('0199e0f2-7a1b-7c3d-8e4f-000000000013', '0199e0f2-7a1b-7c3d-8e4f-000000000011', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000012', 'explore', '0199e0f2-0000-7000-8000-000000000001', \
                 '0199e0f2-0000-7000-8000-000000000001:1', 'completed', 1, '{}', 1, 1, 2, 0); \
                 INSERT INTO runtime_parts (id, conversation_id, turn_id, run_id, attempt, position, kind, audience, \
                 state, text, version, document, created_at, finished_at) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000004', '0199e0f2-7a1b-7c3d-8e4f-000000000001', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000002', NULL, NULL, 0, 'user_text', 'both', 'complete', \
                 'Split the work', 1, '{}', 1, 1), \
                 ('0199e0f2-7a1b-7c3d-8e4f-000000000005', '0199e0f2-7a1b-7c3d-8e4f-000000000001', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000002', '0199e0f2-7a1b-7c3d-8e4f-000000000003', 1, 1, 'task', \
                 'both', 'complete', '{}', 1, \
                 '{\"toolCall\":{\"callId\":\"call_1\",\"tool\":\"start_task\",\"status\":\"completed\"}}', 1, 3), \
                 ('0199e0f2-7a1b-7c3d-8e4f-000000000014', '0199e0f2-7a1b-7c3d-8e4f-000000000011', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000012', NULL, NULL, 0, 'user_text', 'both', 'complete', \
                 'Find where sessions expire.', 1, '{}', 1, 1); \
                 INSERT INTO runtime_tasks (id, conversation_id, part_id, turn_id, follow_up, background, state, \
                 version, ending, created_at, ended_at) \
                 VALUES ('0199e0f2-7a1b-7c3d-8e4f-000000000021', '0199e0f2-7a1b-7c3d-8e4f-000000000011', \
                 '0199e0f2-7a1b-7c3d-8e4f-000000000005', '0199e0f2-7a1b-7c3d-8e4f-000000000012', 0, 0, 'reported', \
                 1, '{\"state\":\"reported\",\"report\":\"In session.rs.\"}', 1, 2);",
            )
        })
        .await
        .unwrap();
}

/// The names of the indexes of the tasks table.
async fn task_indexes(store: &Store) -> Vec<String> {
    store
        .read(|connection| {
            let mut select = connection.prepare(
                "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'runtime_tasks' AND sql IS NOT NULL ORDER BY name",
            )?;
            let rows = select.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect()
        })
        .await
        .unwrap()
}
