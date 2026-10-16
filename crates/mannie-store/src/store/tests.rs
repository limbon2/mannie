use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use mannie_types::clock::{Clock, UtcMillis};
use mannie_types::ids::{ConversationId, PartId, TaskId, TurnId};
use mannie_types::interface::{InterfaceSettings, LocaleTag};
use mannie_types::settings::SettingsSection;
use mannie_types::tasks::TaskState;
use rusqlite::Connection;

use crate::schema::{MIGRATIONS, Migration};
use crate::{Durability, SettingsStoreError, Store, StoreError, StoreOptions};

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
    version: 1,
    module: "tests",
    description: "create the notes table",
    sql: "CREATE TABLE tests_notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL) STRICT;",
};

const SECOND: Migration = Migration {
    version: 2,
    module: "tests",
    description: "add a title to notes",
    sql: "ALTER TABLE tests_notes ADD COLUMN title TEXT NOT NULL DEFAULT '';",
};

const BROKEN: Migration = Migration {
    version: 2,
    module: "tests",
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

    fn open(&self, migrations: &[Migration]) -> Result<Store, StoreError> {
        Store::open_with_migrations(&self.database(), &self.options(), migrations)
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
    assert_eq!(user_version(&fixture.database()), Store::schema_version());
    assert_eq!(Store::schema_version() as usize, MIGRATIONS.len());
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
    assert_eq!(name, "mannie.sqlite.schema-1.20261013T120001.000Z.bak");
    assert_eq!(user_version(&backups[0]), 1);
    assert_eq!(note_bodies(&backups[0]), ["written at schema version 1"]);
    assert_eq!(user_version(&fixture.database()), 2);
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
        StoreError::SchemaTooNew {
            database,
            supported,
            ..
        } => {
            assert_eq!((*database, *supported), (2, 1));
        }
        other => panic!("expected SchemaTooNew, got {other:?}"),
    }
    assert_eq!(error.code(), "store.schema_too_new");
    let message = error.to_string();
    assert!(
        message.contains("schema version 2") && message.contains("up to version 1"),
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
        matches!(error, StoreError::Migration { version: 2, .. }),
        "{error:?}"
    );
    assert_eq!(user_version(&fixture.database()), 1);
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
        names.iter().all(|name| !name.contains(".schema-1.")),
        "{names:?}"
    );
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
    let first_schema = &MIGRATIONS[..1];
    let older = fixture.open(first_schema).unwrap();
    let settings = InterfaceSettings {
        locale: Some(LocaleTag::parse("de").unwrap()),
    };
    older.save_settings(&settings).await.unwrap();
    drop(older);
    assert_eq!(user_version(&fixture.database()), 1);

    let upgraded = fixture.open(MIGRATIONS).unwrap();

    assert_eq!(user_version(&fixture.database()), Store::schema_version());
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
    let chat_schema = &MIGRATIONS[..6];
    let older = fixture.open(chat_schema).unwrap();
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

    let upgraded = fixture.open(MIGRATIONS).unwrap();

    assert_eq!(user_version(&fixture.database()), Store::schema_version());
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
    let workspace_schema = &MIGRATIONS[..11];
    let older = fixture.open(workspace_schema).unwrap();
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

    let upgraded = fixture.open(MIGRATIONS).unwrap();

    assert_eq!(user_version(&fixture.database()), Store::schema_version());
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
    let tools_schema = &MIGRATIONS[..14];
    let older = fixture.open(tools_schema).unwrap();
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

    let upgraded = fixture.open(MIGRATIONS).unwrap();

    assert_eq!(user_version(&fixture.database()), Store::schema_version());
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
/// ended task read as before (docs/features/workers/delegation.md, data). No
/// release writes this schema, since M2, M3 and M4 ship together in v0.2.0
/// (docs/roadmap/milestones.md, D23), so the seed stays.
#[tokio::test]
async fn a_database_from_the_child_agents_schema_is_upgraded_for_worker_tasks() {
    let fixture = Fixture::new();
    let child_agents_schema = &MIGRATIONS[..16];
    let older = fixture.open(child_agents_schema).unwrap();
    seed_child_agents_schema(&older).await;
    drop(older);
    assert_eq!(user_version(&fixture.database()), 16);

    let upgraded = fixture.open(MIGRATIONS).unwrap();

    assert_eq!(user_version(&fixture.database()), Store::schema_version());
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
    assert_eq!(task.worker_id, None);
    assert_eq!(task.submission, None);
    assert_eq!(task.assignment, None);
    assert_eq!(task.state_version, 0);
    let child = upgraded.child_by_number(&parent, 1).await.unwrap().unwrap();
    assert_eq!(child.agent, "explore");
    assert_eq!(child.worker, None);
    assert_eq!(child.latest.map(|task| task.id), Some(task_id));
    let summary = upgraded
        .conversation_summary(&child.conversation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.task.and_then(|origin| origin.worker), None);
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

/// The schema before the owner could name workers (version 21) keeps a
/// paired worker with the name in its configuration. Migration 22 adds the
/// owner's name, which such a worker does not have yet, so it keeps showing
/// its configured name (docs/features/workers/pairing-and-link.md, data).
#[tokio::test]
async fn a_database_with_paired_workers_is_upgraded_for_the_owners_names() {
    let fixture = Fixture::new();
    let before_names = &MIGRATIONS[..21];
    let older = fixture.open(before_names).unwrap();
    older
        .write(Durability::Full, |transaction| {
            transaction.execute(
                "INSERT INTO workers_paired (id, name, address, key_pin, version, details, \
                 paired_at, connected_at, updated_at) \
                 VALUES ('0199e0f2-0000-7000-b000-000000000001', 'build-01', 'build-01.lan:7445', \
                 ?1, 1, '{}', 1, NULL, 1)",
                ["d".repeat(64)],
            )
        })
        .await
        .unwrap();
    drop(older);
    assert_eq!(user_version(&fixture.database()), 21);

    let upgraded = fixture.open(MIGRATIONS).unwrap();

    assert_eq!(user_version(&fixture.database()), Store::schema_version());
    let workers = upgraded.paired_workers().await.unwrap();
    assert_eq!(workers.len(), 1);
    assert_eq!(workers[0].name, "build-01");
    assert_eq!(workers[0].chosen_name, None);
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
