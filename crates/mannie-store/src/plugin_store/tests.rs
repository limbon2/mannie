use std::path::PathBuf;
use std::sync::Arc;

use mannie_plugin_sdk::context::HostProfile;
use mannie_plugin_sdk::host::PluginBinding;
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::services::store::{
    Constraint, Durability, Expect, Migration, ModuleStoreApi, SqlValue, Statement, StoreError,
    StoreModule, WriteBatch,
};
use mannie_types::clock::{Clock, SystemClock};
use tempfile::TempDir;

use super::PluginModule;
use crate::error::{ModuleProblem, StoreError as OpenError};
use crate::{Store, StoreOptions};

const CREATE_ITEMS: &str = "CREATE TABLE notes_items (
     id TEXT PRIMARY KEY,
     slug TEXT NOT NULL UNIQUE,
     size INTEGER NOT NULL CONSTRAINT notes_size_positive CHECK (size > 0),
     weight REAL,
     data BLOB,
     parent TEXT REFERENCES notes_parents (id) DEFERRABLE INITIALLY DEFERRED
 ) STRICT;
 CREATE TABLE notes_parents (id TEXT PRIMARY KEY) STRICT;";

const MIGRATIONS: &[Migration] = &[Migration::new(1, "create the notes", CREATE_ITEMS)];

const INSERT: &str = "INSERT INTO notes_items (id, slug, size, weight, data, parent) VALUES (?1, ?2, ?3, ?4, ?5, ?6)";
const SELECT: &str = "SELECT id, slug, size, weight, data, parent FROM notes_items ORDER BY id";
const COUNT: &str = "SELECT count(*) FROM notes_items";
const DELETE: &str = "DELETE FROM notes_items WHERE id = ?1";
const STATEMENTS: &[&str] = &[INSERT, SELECT, COUNT, DELETE];

const NOTES: StoreModule = StoreModule::new("notes", MIGRATIONS, STATEMENTS);

struct Fixture {
    directory: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            directory: tempfile::tempdir().unwrap(),
        }
    }

    fn database(&self) -> PathBuf {
        self.directory.path().join("mannie.sqlite")
    }

    fn open(&self, modules: impl IntoIterator<Item = PluginModule>) -> Result<Store, OpenError> {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let options = StoreOptions::new(self.directory.path().join("backups"), clock)
            .with_plugin_modules(modules);
        Store::open(&self.database(), &options)
    }
}

fn plugin(id: &str) -> PluginId {
    PluginId::parse(id).unwrap()
}

fn notes_module() -> PluginModule {
    PluginModule::new(&plugin("notes"), NOTES)
}

/// The store of `notes`, opened with its module.
fn notes() -> (Fixture, Store, Arc<dyn ModuleStoreApi>) {
    let fixture = Fixture::new();
    let store = fixture.open([notes_module()]).unwrap();
    let notes = store.plugin_store(&plugin("notes")).unwrap();
    (fixture, store, notes)
}

fn insert(id: &str, slug: &str, size: i64) -> Statement {
    Statement::new(INSERT)
        .bind(id)
        .bind(slug)
        .bind(size)
        .bind(None::<f64>)
        .bind(None::<Vec<u8>>)
        .bind(None::<String>)
}

async fn count(notes: &Arc<dyn ModuleStoreApi>) -> i64 {
    let rows = notes.query(Statement::new(COUNT)).await.unwrap();
    rows[0].get::<i64>(0).unwrap()
}

#[tokio::test]
async fn every_listed_statement_prepares_when_the_module_registers() {
    let (_fixture, store, notes) = notes();

    assert_eq!(count(&notes).await, 0);
    store.plugin_store(&plugin("notes")).unwrap();
}

#[tokio::test]
async fn a_listed_statement_that_does_not_prepare_refuses_only_its_module() {
    const BROKEN: StoreModule = StoreModule::new(
        "notes",
        MIGRATIONS,
        &[SELECT, "SELECT title FROM notes_missing"],
    );
    let fixture = Fixture::new();
    let store = fixture
        .open([PluginModule::new(&plugin("notes"), BROKEN)])
        .unwrap();

    let refused = store.plugin_store(&plugin("notes")).unwrap_err();
    // The module stays migrated, and the core keeps working.
    let tables = store
        .query_unlisted("SELECT name FROM sqlite_schema WHERE name = 'notes_items'")
        .await
        .unwrap();

    assert!(
        matches!(&refused, OpenError::StatementsRefused { module } if module == "notes"),
        "{refused:?}"
    );
    assert_eq!(refused.code(), "store.statements_refused");
    assert_eq!(tables.len(), 1);
    store
        .load_settings::<mannie_types::plugins::PluginsSettings>()
        .await
        .unwrap();
}

#[tokio::test]
async fn a_module_that_the_store_did_not_open_with_is_not_registered() {
    let (_fixture, store, _notes) = notes();

    let refused = store.plugin_store(&plugin("other")).unwrap_err();

    assert!(
        matches!(&refused, OpenError::ModuleNotRegistered { module } if module == "other"),
        "{refused:?}"
    );
}

#[tokio::test]
async fn an_unlisted_statement_is_refused_before_anything_runs() {
    let (_fixture, _store, notes) = notes();
    // The same text with other white space is not a listed statement.
    let spaced = Box::leak(format!("{INSERT} ").into_boxed_str());

    let query = notes
        .query(Statement::new("SELECT id FROM notes_items"))
        .await;
    let batch = notes
        .write(
            WriteBatch::new()
                .push(insert("n1", "s1", 1), Expect::Changes(1))
                .push(
                    Statement::new(spaced)
                        .bind("n2")
                        .bind("s2")
                        .bind(1_i64)
                        .bind(None::<f64>)
                        .bind(None::<Vec<u8>>)
                        .bind(None::<String>),
                    Expect::Changes(1),
                ),
            Durability::Full,
        )
        .await;

    assert_eq!(query, Err(StoreError::UnknownStatement));
    assert_eq!(batch, Err(StoreError::UnknownStatement));
    assert_eq!(count(&notes).await, 0, "nothing of the batch ran");
}

#[tokio::test]
async fn a_query_refuses_a_listed_statement_that_writes() {
    let (_fixture, _store, notes) = notes();

    let refused = notes.query(Statement::new(DELETE).bind("n1")).await;

    assert_eq!(refused, Err(StoreError::NotPermitted));
}

#[tokio::test]
async fn a_batch_is_one_transaction() {
    let (_fixture, _store, notes) = notes();

    let failed = notes
        .write(
            WriteBatch::new()
                .push(insert("n1", "s1", 1), Expect::Changes(1))
                .push(insert("n2", "s2", 0), Expect::Changes(1)),
            Durability::Full,
        )
        .await;

    assert_eq!(failed, Err(StoreError::Failed), "an unmapped CHECK fails");
    assert_eq!(
        count(&notes).await,
        0,
        "the first statement was rolled back"
    );
}

#[tokio::test]
async fn a_different_change_count_rolls_the_batch_back() {
    let (_fixture, _store, notes) = notes();

    let refused = notes
        .write(
            WriteBatch::new()
                .push(insert("n1", "s1", 1), Expect::Changes(1))
                .push(Statement::new(DELETE).bind("missing"), Expect::Changes(1)),
            Durability::Full,
        )
        .await;
    let written = notes
        .write(
            WriteBatch::new()
                .push(insert("n1", "s1", 1), Expect::Changes(1))
                .push(Statement::new(DELETE).bind("missing"), Expect::Any),
            Durability::Normal,
        )
        .await;

    assert_eq!(refused, Err(StoreError::Expectation { index: 1 }));
    assert_eq!(written, Ok(vec![1, 0]));
    assert_eq!(count(&notes).await, 1);
}

#[tokio::test]
async fn a_failed_constraint_maps_to_the_plugins_code() {
    let (_fixture, _store, notes) = notes();
    notes
        .write(
            WriteBatch::new().push(insert("n1", "s1", 1), Expect::Changes(1)),
            Durability::Full,
        )
        .await
        .unwrap();
    let mapped = |statement: Statement| {
        WriteBatch::new()
            .push(statement, Expect::Changes(1))
            .on_constraint(
                Constraint::Key {
                    table: "notes_items",
                    columns: &["slug"],
                },
                "slug_taken",
            )
            .on_constraint(
                Constraint::Key {
                    table: "notes_items",
                    columns: &["id"],
                },
                "id_taken",
            )
            .on_constraint(
                Constraint::Check {
                    name: "notes_size_positive",
                },
                "too_small",
            )
    };

    let slug = notes
        .write(mapped(insert("n2", "s1", 1)), Durability::Full)
        .await;
    let id = notes
        .write(mapped(insert("n1", "s3", 1)), Durability::Full)
        .await;
    let check = notes
        .write(mapped(insert("n3", "s3", 0)), Durability::Full)
        .await;

    assert_eq!(slug, Err(StoreError::Constraint { code: "slug_taken" }));
    assert_eq!(id, Err(StoreError::Constraint { code: "id_taken" }));
    assert_eq!(check, Err(StoreError::Constraint { code: "too_small" }));
}

#[tokio::test]
async fn a_failed_foreign_key_maps_to_its_code_when_the_batch_commits() {
    let (_fixture, _store, notes) = notes();
    let orphan = Statement::new(INSERT)
        .bind("n1")
        .bind("s1")
        .bind(1_i64)
        .bind(None::<f64>)
        .bind(None::<Vec<u8>>)
        .bind("no-such-parent");

    // The foreign key is deferred, so it fails when the batch commits.
    let refused = notes
        .write(
            WriteBatch::new()
                .push(orphan, Expect::Changes(1))
                .on_constraint(Constraint::ForeignKey, "parent_missing"),
            Durability::Full,
        )
        .await;

    assert_eq!(
        refused,
        Err(StoreError::Constraint {
            code: "parent_missing"
        })
    );
    assert_eq!(count(&notes).await, 0);
}

#[tokio::test]
async fn rows_return_every_sql_value() {
    let (_fixture, _store, notes) = notes();
    notes
        .write(
            WriteBatch::new().push(
                Statement::new(INSERT)
                    .bind("n1")
                    .bind("s1")
                    .bind(3_i64)
                    .bind(1.5)
                    .bind(vec![0_u8, 255])
                    .bind(None::<String>),
                Expect::Changes(1),
            ),
            Durability::Full,
        )
        .await
        .unwrap();

    let rows = notes.query(Statement::new(SELECT)).await.unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].values(),
        [
            SqlValue::Text("n1".to_owned()),
            SqlValue::Text("s1".to_owned()),
            SqlValue::Integer(3),
            SqlValue::Real(1.5),
            SqlValue::Blob(vec![0, 255]),
            SqlValue::Null,
        ]
    );
    assert_eq!(rows[0].get::<u32>(2), Ok(3));
    assert_eq!(rows[0].get::<Option<String>>(5), Ok(None));
}

#[tokio::test]
async fn writes_of_both_durabilities_are_read_back() {
    let (_fixture, _store, notes) = notes();

    for (id, durability) in [("n1", Durability::Normal), ("n2", Durability::Full)] {
        notes
            .write(
                WriteBatch::new().push(insert(id, id, 1), Expect::Changes(1)),
                durability,
            )
            .await
            .unwrap();
    }
    let empty = notes.write(WriteBatch::new(), Durability::Full).await;

    assert_eq!(count(&notes).await, 2);
    assert_eq!(empty, Ok(Vec::new()));
}

#[tokio::test]
async fn the_store_provider_binds_a_plugin_to_its_own_module_only() {
    let (_fixture, store, _notes) = notes();
    let [provider] = crate::service_providers(&store).try_into().unwrap();
    let manifest = |id: &str| {
        Manifest::parse(&format!(
            "id = \"{id}\"\nsdk = 1\norder = 1\n\n[rust]\nentry = \"mannie-{id}\"\n\n[services]\nstore = 1\n"
        ))
        .unwrap()
    };
    let notes_manifest = manifest("notes");
    let other_manifest = manifest("other");

    let bound = provider.bind(&PluginBinding::new(
        &notes_manifest.id,
        &notes_manifest,
        HostProfile::Release,
    ));
    let refused = provider.bind(&PluginBinding::new(
        &other_manifest.id,
        &other_manifest,
        HostProfile::Release,
    ));

    assert_eq!((provider.name(), provider.major()), ("store", 1));
    let bound = bound.unwrap();
    let api = bound
        .get::<mannie_plugin_sdk::services::store::Store>()
        .unwrap();
    assert_eq!(count(&api).await, 0);
    let refused = refused.unwrap_err();
    assert_eq!(refused.service, "store");
}

#[tokio::test]
async fn a_module_whose_name_is_not_its_prefix_is_refused_and_the_file_is_not_made() {
    let fixture = Fixture::new();

    let refused = fixture
        .open([PluginModule::new(&plugin("journal"), NOTES)])
        .unwrap_err();

    assert!(
        matches!(
            &refused,
            OpenError::ModuleRefused { module, problem: ModuleProblem::NotThePrefix { prefix } }
                if module == "notes" && prefix == "journal"
        ),
        "{refused:?}"
    );
    assert_eq!(refused.code(), "store.module_refused");
    assert!(!fixture.database().exists());
}

#[tokio::test]
async fn module_versions_count_from_one_without_gaps_and_have_sql() {
    const GAP: &[Migration] = &[
        Migration::new(1, "first", "CREATE TABLE notes_a (id TEXT) STRICT"),
        Migration::new(3, "third", "CREATE TABLE notes_c (id TEXT) STRICT"),
    ];
    const EMPTY: &[Migration] = &[Migration::new(1, "nothing", "  ")];
    let fixture = Fixture::new();

    let gap = fixture
        .open([PluginModule::new(
            &plugin("notes"),
            StoreModule::new("notes", GAP, &[]),
        )])
        .unwrap_err();
    let empty = fixture
        .open([PluginModule::new(
            &plugin("notes"),
            StoreModule::new("notes", EMPTY, &[]),
        )])
        .unwrap_err();

    assert!(
        matches!(
            gap,
            OpenError::ModuleRefused {
                problem: ModuleProblem::VersionOutOfOrder {
                    position: 2,
                    version: 3
                },
                ..
            }
        ),
        "{gap:?}"
    );
    assert!(
        matches!(
            empty,
            OpenError::ModuleRefused {
                problem: ModuleProblem::EmptyMigration { version: 1 },
                ..
            }
        ),
        "{empty:?}"
    );
    assert!(!fixture.database().exists());
}

#[tokio::test]
async fn a_number_of_the_list_before_m5_is_accepted_only_as_the_record_gives_it() {
    const CLAIMS_19: &[Migration] = &[Migration::legacy(
        1,
        19,
        "claim a number of the old list",
        "CREATE TABLE notes_a (id TEXT) STRICT",
    )];
    const WORKERS_18: &[Migration] = &[
        Migration::legacy(
            1,
            18,
            "a number that is not Workers'",
            "CREATE TABLE workers_a (id TEXT) STRICT",
        ),
        Migration::legacy(
            2,
            20,
            "the second",
            "CREATE TABLE workers_b (id TEXT) STRICT",
        ),
        Migration::legacy(
            3,
            22,
            "the third",
            "CREATE TABLE workers_c (id TEXT) STRICT",
        ),
    ];
    const WORKERS_WITHOUT_22: &[Migration] = &[
        Migration::legacy(
            1,
            19,
            "the first",
            "CREATE TABLE workers_a (id TEXT) STRICT",
        ),
        Migration::legacy(
            2,
            20,
            "the second",
            "CREATE TABLE workers_b (id TEXT) STRICT",
        ),
        Migration::new(
            3,
            "the third, without its number",
            "CREATE TABLE workers_c (id TEXT) STRICT",
        ),
    ];
    let fixture = Fixture::new();
    let open = |id: &str, migrations: &'static [Migration]| {
        let name = if id == "notes" { "notes" } else { "workers" };
        fixture
            .open([PluginModule::new(
                &plugin(id),
                StoreModule::new(name, migrations, &[]),
            )])
            .unwrap_err()
    };

    let notes = open("notes", CLAIMS_19);
    let workers = open("workers", WORKERS_18);
    let missing = open("workers", WORKERS_WITHOUT_22);

    assert!(
        matches!(
            notes,
            OpenError::ModuleRefused {
                problem: ModuleProblem::LegacyVersionNotRecorded {
                    version: 1,
                    legacy_version: 19
                },
                ..
            }
        ),
        "{notes:?}"
    );
    assert!(
        matches!(
            workers,
            OpenError::ModuleRefused {
                problem: ModuleProblem::LegacyVersionNotRecorded {
                    version: 1,
                    legacy_version: 18
                },
                ..
            }
        ),
        "{workers:?}"
    );
    assert!(
        matches!(
            missing,
            OpenError::ModuleRefused {
                problem: ModuleProblem::LegacyVersionMissing {
                    version: 3,
                    legacy_version: 22
                },
                ..
            }
        ),
        "{missing:?}"
    );
    assert!(!fixture.database().exists());
}

#[tokio::test]
async fn a_listed_statement_that_reaches_a_core_table_is_refused_when_the_module_registers() {
    const SPYING: StoreModule = StoreModule::new(
        "notes",
        MIGRATIONS,
        &[SELECT, "SELECT document FROM settings_sections"],
    );
    let fixture = Fixture::new();
    let store = fixture
        .open([PluginModule::new(&plugin("notes"), SPYING)])
        .unwrap();

    let refused = store.plugin_store(&plugin("notes")).unwrap_err();

    assert!(
        matches!(&refused, OpenError::StatementsRefused { module } if module == "notes"),
        "{refused:?}"
    );
}

#[tokio::test]
async fn a_plugin_migration_that_reaches_a_core_table_stops_the_store_from_opening() {
    const REACHING: &[Migration] = &[Migration::new(
        1,
        "drop a table of the core",
        "DROP TABLE settings_sections",
    )];
    let fixture = Fixture::new();

    let refused = fixture
        .open([PluginModule::new(
            &plugin("notes"),
            StoreModule::new("notes", REACHING, &[]),
        )])
        .unwrap_err();
    // The core's tables stay, and Mannie opens its store without the module.
    let store = fixture.open([]).unwrap();
    let kept = store
        .query_unlisted("SELECT name FROM sqlite_schema WHERE name = 'settings_sections'")
        .await
        .unwrap();

    assert!(
        matches!(
            &refused,
            OpenError::Migration {
                module: "notes",
                version: 1,
                ..
            }
        ),
        "{refused:?}"
    );
    assert_eq!(kept.len(), 1);
}
