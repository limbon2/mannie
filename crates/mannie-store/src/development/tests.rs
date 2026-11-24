use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use mannie_types::clock::{Clock, UtcMillis};
use rusqlite::{Connection, OpenFlags};

use super::{
    DATABASE_FILE, DEV_DIRECTORY, MANIFEST_FILE, MAX_DEV_DATABASES, STAGING_PREFIX, SchemaIdentity,
    read_manifest, wal_path,
};
use crate::schema::{Migration, StoreModule};
use crate::{Durability, Store, StoreOptions};

/// 2026-11-22T12:00:00Z
const BASE_TIME: i64 = 1_795_348_800_000;

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

const NOTES_V1: Migration = Migration {
    version: 1,
    legacy_version: None,
    description: "create notes table",
    sql: "CREATE TABLE notes_items (id INTEGER PRIMARY KEY, body TEXT NOT NULL) STRICT;",
};

const NOTES_V2_ALPHA: Migration = Migration {
    version: 2,
    legacy_version: None,
    description: "add alpha table",
    sql: "CREATE TABLE notes_alpha (id INTEGER PRIMARY KEY, tag TEXT NOT NULL) STRICT;",
};

const NOTES_V2_BETA: Migration = Migration {
    version: 2,
    legacy_version: None,
    description: "add beta table",
    sql: "CREATE TABLE notes_beta (id INTEGER PRIMARY KEY, score INTEGER NOT NULL) STRICT;",
};

const NOTES_V2_ALPHA_EDITED: Migration = Migration {
    version: 2,
    legacy_version: None,
    description: "add alpha table with extra column",
    sql: "CREATE TABLE notes_alpha (id INTEGER PRIMARY KEY, tag TEXT NOT NULL, extra TEXT NOT NULL DEFAULT '') STRICT;",
};

const EXTRA_V1: Migration = Migration {
    version: 1,
    legacy_version: None,
    description: "create extra plugin table",
    sql: "CREATE TABLE extra_records (id INTEGER PRIMARY KEY, label TEXT NOT NULL) STRICT;",
};

const EXTRA_V1_EDITED: Migration = Migration {
    version: 1,
    legacy_version: None,
    description: "create extra plugin table with kind",
    sql: "CREATE TABLE extra_records (id INTEGER PRIMARY KEY, label TEXT NOT NULL, kind TEXT NOT NULL DEFAULT '') STRICT;",
};

struct DevFixture {
    directory: tempfile::TempDir,
    clock: Arc<TestClock>,
}

impl DevFixture {
    fn new() -> Self {
        Self {
            directory: tempfile::tempdir().unwrap(),
            clock: TestClock::at(BASE_TIME),
        }
    }

    fn data_directory(&self) -> &Path {
        self.directory.path()
    }

    fn legacy_database(&self) -> PathBuf {
        self.directory.path().join(DATABASE_FILE)
    }

    fn backups_directory(&self) -> PathBuf {
        self.directory.path().join("backups")
    }

    fn dev_root(&self) -> PathBuf {
        self.directory.path().join(DEV_DIRECTORY)
    }

    fn options(&self) -> StoreOptions {
        let clock: Arc<dyn Clock> = Arc::<TestClock>::clone(&self.clock);
        StoreOptions::new(self.backups_directory(), clock)
    }

    fn open_dev(&self, modules: &[StoreModule]) -> Store {
        Store::open_development_with_modules(self.data_directory(), &self.options(), modules)
            .unwrap()
    }
}

fn module(name: &'static str, migrations: &[Migration]) -> StoreModule {
    StoreModule {
        name,
        migrations: Cow::Owned(migrations.to_vec()),
        confined: false,
    }
}

async fn insert_note(store: &Store, id: i64, body: &str) {
    let body = body.to_owned();
    store
        .write(Durability::Full, move |tx| {
            tx.execute(
                "INSERT INTO notes_items (id, body) VALUES (?1, ?2)",
                (id, body),
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

async fn read_notes(store: &Store) -> Vec<String> {
    store
        .read(|connection| {
            let mut stmt = connection.prepare("SELECT body FROM notes_items ORDER BY id")?;
            let rows = stmt.query_map([], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<Vec<String>>>()
        })
        .await
        .unwrap()
}

async fn has_table(store: &Store, table: &str) -> bool {
    let table = table.to_owned();
    store
        .read(move |connection| {
            let found: Option<i64> = connection
                .query_row(
                    "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                    [&table],
                    |row| row.get(0),
                )
                .ok();
            Ok(found.is_some())
        })
        .await
        .unwrap()
}

async fn has_column(store: &Store, table: &str, column: &str) -> bool {
    let table = table.to_owned();
    let column = column.to_owned();
    store
        .read(move |connection| {
            let mut stmt = connection.prepare(&format!("PRAGMA table_info({table})"))?;
            let names = stmt
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(names.iter().any(|name| name == &column))
        })
        .await
        .unwrap()
}

fn recorded_version(path: &Path, module_name: &str) -> u32 {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    connection
        .query_row(
            "SELECT version FROM store_modules WHERE module = ?1",
            [module_name],
            |row| row.get(0),
        )
        .unwrap()
}

fn schema_folder_count(dev_root: &Path) -> usize {
    fs::read_dir(dev_root)
        .unwrap()
        .flatten()
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            entry.file_type().is_ok_and(|kind| kind.is_dir()) && !name.starts_with(STAGING_PREFIX)
        })
        .count()
}

#[tokio::test]
async fn fresh_schema_creates_directory_manifest_and_reopens_directly() {
    let fixture = DevFixture::new();
    let modules = [module("notes", &[NOTES_V1])];

    let store = fixture.open_dev(&modules);
    let path = store.path().to_path_buf();
    assert!(path.starts_with(fixture.dev_root()));
    insert_note(&store, 1, "first").await;
    drop(store);

    let manifest_path = path.with_file_name(MANIFEST_FILE);
    let first_manifest = read_manifest(&manifest_path).unwrap();
    assert_eq!(first_manifest.last_used_at, BASE_TIME);

    fixture.clock.advance(5_000);
    let reopened = fixture.open_dev(&modules);
    assert_eq!(reopened.path(), path);
    assert_eq!(read_notes(&reopened).await, ["first"]);
    drop(reopened);

    let updated_manifest = read_manifest(&manifest_path).unwrap();
    assert_eq!(updated_manifest.last_used_at, BASE_TIME + 5_000);
}

#[tokio::test]
async fn forward_upgrade_seeds_from_previous_schema_and_downgrade_reopens_original() {
    let fixture = DevFixture::new();
    let v1 = [module("notes", &[NOTES_V1])];
    let v2 = [module("notes", &[NOTES_V1, NOTES_V2_ALPHA])];

    let store_v1 = fixture.open_dev(&v1);
    let path_v1 = store_v1.path().to_path_buf();
    insert_note(&store_v1, 1, "from main").await;
    drop(store_v1);

    fixture.clock.advance(1_000);
    let store_v2 = fixture.open_dev(&v2);
    let path_v2 = store_v2.path().to_path_buf();
    assert_ne!(path_v1, path_v2);
    assert!(has_table(&store_v2, "notes_alpha").await);
    assert_eq!(read_notes(&store_v2).await, ["from main"]);
    insert_note(&store_v2, 2, "from branch").await;
    drop(store_v2);

    // Switching back to the older schema opens v1 cleanly without error and
    // keeps v1 isolated from v2's writes.
    assert_eq!(recorded_version(&path_v1, "notes"), 1);
    assert_eq!(recorded_version(&path_v2, "notes"), 2);

    fixture.clock.advance(1_000);
    let back_to_v1 = fixture.open_dev(&v1);
    assert_eq!(back_to_v1.path(), path_v1);
    assert_eq!(read_notes(&back_to_v1).await, ["from main"]);
    assert!(!has_table(&back_to_v1, "notes_alpha").await);
}

#[tokio::test]
async fn recovers_from_pre_migration_backup_when_legacy_database_is_newer() {
    let fixture = DevFixture::new();
    let v1 = [module("notes", &[NOTES_V1])];
    let v2 = [module("notes", &[NOTES_V1, NOTES_V2_ALPHA])];

    // Simulate pre-fix dev usage: main wrote v1 at the legacy root, then a
    // feature branch migrated the same file to v2 (creating a v1 backup).
    let legacy_v1 =
        Store::open_with_modules(&fixture.legacy_database(), &fixture.options(), &v1).unwrap();
    insert_note(&legacy_v1, 1, "kept before branch upgrade").await;
    drop(legacy_v1);

    fixture.clock.advance(1_000);
    let legacy_v2 =
        Store::open_with_modules(&fixture.legacy_database(), &fixture.options(), &v2).unwrap();
    insert_note(&legacy_v2, 2, "written on branch").await;
    drop(legacy_v2);

    assert_eq!(recorded_version(&fixture.legacy_database(), "notes"), 2);

    // Opening dev mode on main (v1) skips the newer legacy file, leaves it
    // untouched, and seeds from the compatible v1 pre-migration backup.
    fixture.clock.advance(1_000);
    let recovered = fixture.open_dev(&v1);
    assert_eq!(read_notes(&recovered).await, ["kept before branch upgrade"]);
    assert!(!has_table(&recovered, "notes_alpha").await);
    drop(recovered);

    assert_eq!(recorded_version(&fixture.legacy_database(), "notes"), 2);
}

#[tokio::test]
async fn starts_fresh_when_only_newer_databases_exist_and_leaves_them_untouched() {
    let fixture = DevFixture::new();
    let v1 = [module("notes", &[NOTES_V1])];
    let v2 = [module("notes", &[NOTES_V1, NOTES_V2_ALPHA])];

    let store_v2 = fixture.open_dev(&v2);
    let path_v2 = store_v2.path().to_path_buf();
    insert_note(&store_v2, 1, "only in v2").await;
    drop(store_v2);

    fixture.clock.advance(1_000);
    let store_v1 = fixture.open_dev(&v1);
    assert_eq!(read_notes(&store_v1).await, Vec::<String>::new());
    assert!(!has_table(&store_v1, "notes_alpha").await);
    drop(store_v1);

    assert_eq!(recorded_version(&path_v2, "notes"), 2);
}

#[tokio::test]
async fn divergent_same_version_branches_do_not_collide() {
    let fixture = DevFixture::new();
    let base = [module("notes", &[NOTES_V1])];
    let branch_a = [module("notes", &[NOTES_V1, NOTES_V2_ALPHA])];
    let branch_b = [module("notes", &[NOTES_V1, NOTES_V2_BETA])];

    let store_base = fixture.open_dev(&base);
    insert_note(&store_base, 1, "shared base").await;
    drop(store_base);

    fixture.clock.advance(1_000);
    let store_a = fixture.open_dev(&branch_a);
    assert!(has_table(&store_a, "notes_alpha").await);
    insert_note(&store_a, 2, "only on branch a").await;
    drop(store_a);

    // Branch B also has notes version 2, with different SQL. It must reject
    // Branch A's dev DB by digest mismatch and seed from the compatible base.
    fixture.clock.advance(1_000);
    let store_b = fixture.open_dev(&branch_b);
    assert!(has_table(&store_b, "notes_beta").await);
    assert!(!has_table(&store_b, "notes_alpha").await);
    assert_eq!(read_notes(&store_b).await, ["shared base"]);
}

#[tokio::test]
async fn editing_in_progress_migration_sql_reseeds_and_applies_new_sql() {
    let fixture = DevFixture::new();
    let base = [module("notes", &[NOTES_V1])];
    let first_draft = [module("notes", &[NOTES_V1, NOTES_V2_ALPHA])];
    let edited_draft = [module("notes", &[NOTES_V1, NOTES_V2_ALPHA_EDITED])];

    let store_base = fixture.open_dev(&base);
    insert_note(&store_base, 1, "base note").await;
    drop(store_base);

    fixture.clock.advance(1_000);
    let store_draft = fixture.open_dev(&first_draft);
    assert!(!has_column(&store_draft, "notes_alpha", "extra").await);
    drop(store_draft);

    fixture.clock.advance(1_000);
    let store_edited = fixture.open_dev(&edited_draft);
    assert!(has_column(&store_edited, "notes_alpha", "extra").await);
    assert_eq!(read_notes(&store_edited).await, ["base note"]);
}

#[tokio::test]
async fn uncheckpointed_wal_rows_survive_seeding_even_without_shm() {
    let fixture = DevFixture::new();
    let v1 = [module("notes", &[NOTES_V1])];
    let v2 = [module("notes", &[NOTES_V1, NOTES_V2_ALPHA])];

    // Create a legacy v1 database and leave a committed write in its -wal file
    // without checkpointing, then remove -shm as if a crash left only -wal.
    let legacy =
        Store::open_with_modules(&fixture.legacy_database(), &fixture.options(), &v1).unwrap();
    insert_note(&legacy, 1, "committed in base").await;
    drop(legacy);

    // Open directly in WAL mode with autocheckpoint disabled and keep a second
    // read connection open so closing does not auto-checkpoint the WAL, then
    // simulate an uncheckpointed WAL file with no -shm.
    let legacy_path = fixture.legacy_database();
    let conn = Connection::open(&legacy_path).unwrap();
    conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    conn.execute(
        "INSERT INTO notes_items (id, body) VALUES (2, 'sitting in wal')",
        [],
    )
    .unwrap();
    let wal_bytes = fs::read(wal_path(&legacy_path)).unwrap();
    assert_ne!(wal_bytes.len(), 0);

    // Copy base + wal into a second fixture without -shm to simulate a hard crash.
    let crashed = DevFixture::new();
    let crashed_db = crashed.legacy_database();
    fs::copy(&legacy_path, &crashed_db).unwrap();
    fs::write(wal_path(&crashed_db), &wal_bytes).unwrap();
    drop(conn);

    let seeded = crashed.open_dev(&v2);
    assert_eq!(
        read_notes(&seeded).await,
        ["committed in base", "sitting in wal"]
    );
    assert!(has_table(&seeded, "notes_alpha").await);
}

#[tokio::test]
async fn carried_over_unknown_plugin_module_prevents_silent_stale_reuse() {
    let fixture = DevFixture::new();
    let core_only = [module("notes", &[NOTES_V1])];
    let with_plugin = [module("notes", &[NOTES_V1]), module("extra", &[EXTRA_V1])];
    let with_edited_plugin = [
        module("notes", &[NOTES_V1]),
        module("extra", &[EXTRA_V1_EDITED]),
    ];

    // First run the branch that includes the plugin, so it is the only seed.
    let store_plugin = fixture.open_dev(&with_plugin);
    insert_note(&store_plugin, 1, "note from plugin branch").await;
    drop(store_plugin);

    // Switching to core-only seeds from the plugin branch (which carries over
    // extra's tables and its verified digest in manifest.json).
    fixture.clock.advance(1_000);
    let store_core = fixture.open_dev(&core_only);
    assert_eq!(read_notes(&store_core).await, ["note from plugin branch"]);
    let core_manifest = read_manifest(&store_core.path().with_file_name(MANIFEST_FILE)).unwrap();
    assert!(core_manifest.modules.contains_key("extra"));
    drop(store_core);

    // Editing the plugin's migration 1 must NOT silently seed from either
    // database (both already have the old extra v1 SQL in store_modules).
    fixture.clock.advance(1_000);
    let store_edited = fixture.open_dev(&with_edited_plugin);
    assert!(has_column(&store_edited, "extra_records", "kind").await);
    assert_eq!(read_notes(&store_edited).await, Vec::<String>::new());
}

#[tokio::test]
async fn seeds_from_legacy_root_database_when_compatible_and_leaves_it_untouched() {
    let fixture = DevFixture::new();
    let v1 = [module("notes", &[NOTES_V1])];

    let legacy =
        Store::open_with_modules(&fixture.legacy_database(), &fixture.options(), &v1).unwrap();
    insert_note(&legacy, 1, "existing dev data").await;
    drop(legacy);

    fixture.clock.advance(1_000);
    let dev = fixture.open_dev(&v1);
    assert_eq!(read_notes(&dev).await, ["existing dev data"]);
    insert_note(&dev, 2, "new dev write").await;
    drop(dev);

    // Legacy root file stays untouched with only its original row.
    let legacy_again =
        Store::open_with_modules(&fixture.legacy_database(), &fixture.options(), &v1).unwrap();
    assert_eq!(read_notes(&legacy_again).await, ["existing dev data"]);
}

#[tokio::test]
async fn skips_corrupted_backup_and_falls_through_to_next_compatible_seed() {
    let fixture = DevFixture::new();
    let v1 = [module("notes", &[NOTES_V1])];
    let v2 = [module("notes", &[NOTES_V1, NOTES_V2_ALPHA])];

    let legacy_v1 =
        Store::open_with_modules(&fixture.legacy_database(), &fixture.options(), &v1).unwrap();
    insert_note(&legacy_v1, 1, "from good backup").await;
    drop(legacy_v1);

    fixture.clock.advance(1_000);
    let legacy_v2 =
        Store::open_with_modules(&fixture.legacy_database(), &fixture.options(), &v2).unwrap();
    drop(legacy_v2);

    // Plant a newer-named corrupted backup beside the real one.
    fs::write(
        fixture
            .backups_directory()
            .join("mannie.sqlite.schema-9999.99999999T000000.000Z.bak"),
        b"not a sqlite database",
    )
    .unwrap();

    fixture.clock.advance(1_000);
    let recovered = fixture.open_dev(&v1);
    assert_eq!(read_notes(&recovered).await, ["from good backup"]);
}

#[tokio::test]
async fn cleans_up_abandoned_staging_directories_on_open() {
    let fixture = DevFixture::new();
    let modules = [module("notes", &[NOTES_V1])];

    let abandoned = fixture.dev_root().join(format!("{STAGING_PREFIX}99999-1"));
    fs::create_dir_all(&abandoned).unwrap();
    fs::write(abandoned.join("partial"), b"leftover").unwrap();

    let store = fixture.open_dev(&modules);
    drop(store);

    assert!(!abandoned.exists());
}

#[tokio::test]
async fn prunes_least_recently_used_schemas_beyond_limit_keeping_active() {
    let fixture = DevFixture::new();
    let total = MAX_DEV_DATABASES + 3;
    let mut oldest_directory = String::new();
    let mut newest_directory = String::new();

    for index in 0..total {
        fixture.clock.advance(1_000);
        let sql: &'static str = Box::leak(
            format!("CREATE TABLE notes_v{index} (id INTEGER PRIMARY KEY) STRICT;")
                .into_boxed_str(),
        );
        let migration = Migration {
            version: 1,
            legacy_version: None,
            description: "distinct schema",
            sql,
        };
        let modules = [module("notes", &[migration])];
        let identity = SchemaIdentity::from_modules(&modules);

        if index == 0 {
            oldest_directory = identity.directory.clone();
        }
        newest_directory = identity.directory;

        let store = fixture.open_dev(&modules);
        drop(store);
    }

    assert_eq!(schema_folder_count(&fixture.dev_root()), MAX_DEV_DATABASES);
    assert!(!fixture.dev_root().join(&oldest_directory).exists());
    assert!(fixture.dev_root().join(&newest_directory).exists());
}
