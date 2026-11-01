//! The engine's tests (ADR-0056, validation): a matrix of planted attacks,
//! each limit, interrupts, a busy file, a replaced file, the structure,
//! pages, edits, the registry and the recovery of an owner operation that a
//! crash cut off.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mannie_types::clock::SystemClock;
use mannie_types::ids::{DatabaseKey, DatabaseOperationId};
use mannie_types::tools::StatementClass;
use rusqlite::Connection;

use super::*;
use crate::{Store, StoreOptions};

/// Generous limits, for the tests that are not about limits.
const LIMITS: Limits = Limits {
    rows: 1_000,
    bytes: 1024 * 1024,
    time: Duration::from_secs(10),
};

/// A database file with tables, a child table, an index, a trigger, a view,
/// an FTS5 table and a table without rowid, and its checked identity.
struct Fixture {
    _folder: tempfile::TempDir,
    path: PathBuf,
    identity: String,
}

impl Fixture {
    fn new() -> Self {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("shop.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL, price REAL);
                 CREATE TABLE parts (item INTEGER NOT NULL REFERENCES items (id), part TEXT);
                 CREATE INDEX items_name ON items (name);
                 CREATE TABLE log (entry TEXT);
                 CREATE TRIGGER items_logged AFTER INSERT ON items BEGIN
                     INSERT INTO log (entry) VALUES ('added ' || new.name);
                 END;
                 CREATE VIEW names AS SELECT name FROM items;
                 CREATE VIRTUAL TABLE notes USING fts5 (body);
                 CREATE TABLE tags (name TEXT PRIMARY KEY, color TEXT) WITHOUT ROWID;
                 INSERT INTO items (name, price) VALUES ('bolt', 0.5), ('nut', NULL), ('gear', 12.0);
                 INSERT INTO parts (item, part) VALUES (1, 'head');
                 INSERT INTO tags (name, color) VALUES ('red', 'f00'), ('blue', '00f');",
            )
            .unwrap();
        drop(connection);
        let checked = check_new(&path, &[]).unwrap();
        Self {
            path: checked.path,
            identity: checked.identity,
            _folder: folder,
        }
    }

    fn read(&self, text: &str) -> Result<Returned, EngineError> {
        self.read_with(text, Vec::new(), LIMITS)
    }

    fn read_with(
        &self,
        text: &str,
        parameters: Vec<Value>,
        limits: Limits,
    ) -> Result<Returned, EngineError> {
        read(
            &self.path,
            &self.identity,
            &Sql {
                text: text.to_owned(),
                parameters,
            },
            limits,
            &Cancel::new(),
        )
    }

    fn write(&self, texts: &[&str], class: StatementClass) -> Result<Vec<Written>, EngineError> {
        let statements: Vec<Sql> = texts
            .iter()
            .map(|text| Sql {
                text: (*text).to_owned(),
                parameters: Vec::new(),
            })
            .collect();
        write(
            &self.path,
            &self.identity,
            &statements,
            class,
            LIMITS,
            &Cancel::new(),
        )
    }

    /// The rows of a statement, read with a connection of the test's own.
    fn plain(&self, text: &str) -> Vec<String> {
        let connection = Connection::open(&self.path).unwrap();
        let mut statement = connection.prepare(text).unwrap();
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap();
        rows.collect::<rusqlite::Result<_>>().unwrap()
    }

    fn table(&self, name: &str) -> TableInfo {
        structure(
            &self.path,
            &self.identity,
            Duration::from_secs(5),
            &Cancel::new(),
        )
        .unwrap()
        .tables
        .into_iter()
        .find(|table| table.name == name)
        .unwrap()
    }
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

#[test]
fn a_read_returns_its_columns_and_rows() {
    let fixture = Fixture::new();

    let rows = fixture
        .read_with(
            "SELECT id, name, price FROM items WHERE id <= ?1 ORDER BY id",
            vec![Value::Integer(2)],
            LIMITS,
        )
        .unwrap();

    assert_eq!(
        rows.columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["id", "name", "price"]
    );
    assert_eq!(rows.columns[1].declared_type.as_deref(), Some("TEXT"));
    assert_eq!(
        rows.rows,
        [
            vec![Value::Integer(1), text("bolt"), Value::Real(0.5)],
            vec![Value::Integer(2), text("nut"), Value::Null],
        ]
    );
    assert_eq!(rows.cut, None);
}

#[test]
fn a_read_stops_at_its_rows_and_bytes() {
    let fixture = Fixture::new();
    let few_rows = Limits { rows: 2, ..LIMITS };
    let few_bytes = Limits { bytes: 8, ..LIMITS };

    let by_rows = fixture
        .read_with("SELECT name FROM items", Vec::new(), few_rows)
        .unwrap();
    let by_bytes = fixture
        .read_with("SELECT name FROM items ORDER BY id", Vec::new(), few_bytes)
        .unwrap();

    assert_eq!(by_rows.rows.len(), 2);
    assert_eq!(by_rows.cut, Some(Cut::Rows));
    // "bolt" and "nut" fit in 8 bytes, "gear" does not.
    assert_eq!(by_bytes.rows.len(), 2);
    assert_eq!(by_bytes.cut, Some(Cut::Bytes));
}

#[test]
fn a_read_runs_only_one_statement_that_changes_nothing() {
    let fixture = Fixture::new();

    assert_eq!(
        fixture.read("DELETE FROM items"),
        Err(EngineError::NotARead {
            class: StatementClass::Write
        })
    );
    assert_eq!(
        fixture.read("DROP TABLE log"),
        Err(EngineError::NotARead {
            class: StatementClass::Schema
        })
    );
    assert_eq!(
        fixture.read("SELECT 1; DELETE FROM items"),
        Err(EngineError::NotOneStatement)
    );
    assert_eq!(
        fixture.read("  -- nothing "),
        Err(EngineError::NotOneStatement)
    );
    assert_eq!(
        fixture.read_with("SELECT ?1, ?2", vec![Value::Integer(1)], LIMITS),
        Err(EngineError::Parameters {
            expected: 2,
            given: 1
        })
    );
    assert!(matches!(
        fixture.read("SELECT nothing FROM nowhere"),
        Err(EngineError::Invalid { .. })
    ));
    // A trailing comment is no second statement.
    fixture.read("SELECT 1; -- done").unwrap();
    assert_eq!(fixture.plain("SELECT name FROM items ORDER BY id").len(), 3);
}

#[test]
fn the_engine_refuses_the_planted_attacks() {
    let fixture = Fixture::new();
    let target = fixture.path.with_file_name("copy.db");
    let into = format!("VACUUM INTO '{}'", target.display());

    for attack in [
        "ATTACH DATABASE 'other.db' AS other",
        "VACUUM",
        into.as_str(),
        "SELECT load_extension('evil')",
        "PRAGMA writable_schema = 1",
        "PRAGMA journal_mode = DELETE",
        "PRAGMA foreign_keys = OFF",
        "PRAGMA trusted_schema = ON",
        "BEGIN",
        "COMMIT",
        "SAVEPOINT inside",
        "CREATE TEMP TABLE scratch (id INTEGER)",
        "CREATE VIRTUAL TABLE more USING fts5 (body)",
    ] {
        let read = fixture.read(attack);
        assert!(read.is_err(), "a read ran {attack}");
        for class in [StatementClass::Write, StatementClass::Schema] {
            let written = fixture.write(&[attack], class);
            assert!(written.is_err(), "a write of class {class:?} ran {attack}");
        }
    }
    assert!(!target.exists(), "VACUUM INTO made a file");
}

#[test]
fn a_view_that_loads_an_extension_fails_under_trusted_schema_off() {
    let fixture = Fixture::new();
    Connection::open(&fixture.path)
        .unwrap()
        .execute_batch("CREATE VIEW evil AS SELECT load_extension('evil') AS loaded;")
        .unwrap();
    let identity = check_new(&fixture.path, &[]).unwrap().identity;

    let read = read(
        &fixture.path,
        &identity,
        &Sql {
            text: "SELECT loaded FROM evil".to_owned(),
            parameters: Vec::new(),
        },
        LIMITS,
        &Cancel::new(),
    );

    assert!(read.is_err(), "{read:?}");
}

#[test]
fn a_write_runs_its_statements_in_one_transaction_of_their_class() {
    let fixture = Fixture::new();

    let done = fixture
        .write(
            &[
                "INSERT INTO items (name) VALUES ('washer')",
                "UPDATE items SET price = 1 WHERE price IS NULL RETURNING id",
            ],
            StatementClass::Write,
        )
        .unwrap();
    let failed = fixture.write(
        &[
            "INSERT INTO items (name) VALUES ('spring')",
            "INSERT INTO items (name) VALUES (NULL)",
        ],
        StatementClass::Write,
    );

    assert_eq!(done[0].changes, 1);
    assert_eq!(done[0].rows, None);
    assert_eq!(done[1].changes, 2);
    assert_eq!(done[1].rows.as_ref().map(|rows| rows.rows.len()), Some(2));
    assert!(
        matches!(failed, Err(EngineError::Constraint { .. })),
        "{failed:?}"
    );
    // The failed batch left nothing, and the trigger of the first ran.
    assert_eq!(
        fixture.plain("SELECT name FROM items ORDER BY id"),
        ["bolt", "nut", "gear", "washer"]
    );
    assert_eq!(
        fixture.plain("SELECT entry FROM log"),
        ["added bolt", "added nut", "added gear", "added washer"]
    );
}

#[test]
fn a_statement_stronger_than_its_approval_goes_back_and_a_read_never_runs_as_a_write() {
    let fixture = Fixture::new();

    assert_eq!(
        fixture.write(&["CREATE TABLE more (id INTEGER)"], StatementClass::Write),
        Err(EngineError::ClassChanged {
            expected: StatementClass::Write,
            found: StatementClass::Schema
        })
    );
    assert_eq!(
        fixture.write(&["SELECT 1"], StatementClass::Write),
        Err(EngineError::ClassChanged {
            expected: StatementClass::Write,
            found: StatementClass::Read
        })
    );
    let schema = fixture
        .write(
            &[
                "CREATE TABLE more (id INTEGER)",
                "INSERT INTO more VALUES (1)",
            ],
            StatementClass::Schema,
        )
        .unwrap();
    assert_eq!(schema[0].changes, 0);
    assert_eq!(schema[1].changes, 1);
    assert_eq!(
        fixture.write(&[], StatementClass::Write),
        Err(EngineError::BatchSize)
    );
    let many = ["INSERT INTO log VALUES ('x')"; MAX_BATCH_STATEMENTS + 1];
    assert_eq!(
        fixture.write(&many, StatementClass::Write),
        Err(EngineError::BatchSize)
    );
}

#[test]
fn a_write_enforces_foreign_keys() {
    let fixture = Fixture::new();

    let orphan = fixture.write(
        &["INSERT INTO parts (item, part) VALUES (99, 'x')"],
        StatementClass::Write,
    );

    assert!(
        matches!(orphan, Err(EngineError::Constraint { .. })),
        "{orphan:?}"
    );
}

#[test]
fn a_write_fails_as_busy_while_another_program_writes() {
    let fixture = Fixture::new();
    let other = Connection::open(&fixture.path).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();

    let started = Instant::now();
    let written = fixture.write(&["INSERT INTO log VALUES ('x')"], StatementClass::Write);

    assert_eq!(written, Err(EngineError::Busy));
    assert!(started.elapsed() < Duration::from_secs(10));
    other.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn a_statement_stops_at_its_time_and_when_its_caller_stops() {
    let fixture = Fixture::new();
    let endless =
        "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n) SELECT max(x) FROM n";
    let short = Limits {
        time: Duration::from_millis(100),
        ..LIMITS
    };

    let timed_out = fixture.read_with(endless, Vec::new(), short);
    let cancel = Cancel::new();
    let canceller = cancel.clone();
    let stopping = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        canceller.stop();
    });
    let stopped = read(
        &fixture.path,
        &fixture.identity,
        &Sql {
            text: endless.to_owned(),
            parameters: Vec::new(),
        },
        LIMITS,
        &cancel,
    );
    stopping.join().unwrap();

    assert_eq!(timed_out, Err(EngineError::TimedOut));
    assert_eq!(stopped, Err(EngineError::Stopped));
}

#[tokio::test]
async fn dropping_a_call_interrupts_its_statement() {
    let fixture = Fixture::new();
    let (path, identity) = (fixture.path.clone(), fixture.identity.clone());
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let endless = Sql {
        text:
            "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n) SELECT max(x) FROM n"
                .to_owned(),
        parameters: Vec::new(),
    };

    let waited = tokio::time::timeout(
        Duration::from_millis(100),
        run(move |cancel| {
            let result = read(&path, &identity, &endless, LIMITS, cancel);
            // The test waits for this result after it stopped waiting for the call.
            let _sent = sender.send(result.clone());
            result
        }),
    )
    .await;
    let ended = tokio::time::timeout(Duration::from_secs(5), receiver)
        .await
        .unwrap()
        .unwrap();

    waited.unwrap_err();
    assert_eq!(ended, Err(EngineError::Stopped));
}

#[tokio::test]
async fn a_call_with_the_callers_handle_goes_on_when_dropped_and_stops_when_the_caller_stops_it() {
    let fixture = Fixture::new();
    let (path, identity) = (fixture.path.clone(), fixture.identity.clone());
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let counting = Sql {
        text: "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n WHERE x < 2000000) \
               SELECT max(x) FROM n"
            .to_owned(),
        parameters: Vec::new(),
    };
    let endless = Sql {
        text:
            "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n) SELECT max(x) FROM n"
                .to_owned(),
        parameters: Vec::new(),
    };

    let waited = tokio::time::timeout(
        Duration::ZERO,
        run_with(Cancel::new(), move |cancel| {
            let result = read(&path, &identity, &counting, LIMITS, cancel);
            // The test waits for this result after it stopped waiting for the call.
            let _sent = sender.send(result.clone());
            result
        }),
    )
    .await;
    let ended = tokio::time::timeout(Duration::from_secs(30), receiver)
        .await
        .unwrap()
        .unwrap();
    let cancel = Cancel::new();
    let canceller = cancel.clone();
    let stopping = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        canceller.stop();
    });
    let (path, identity) = (fixture.path.clone(), fixture.identity.clone());
    let stopped = run_with(cancel, move |cancel| {
        read(&path, &identity, &endless, LIMITS, cancel)
    })
    .await;
    stopping.await.unwrap();

    waited.unwrap_err();
    assert_eq!(ended.unwrap().rows, vec![vec![Value::Integer(2_000_000)]]);
    assert_eq!(stopped, Err(EngineError::Stopped));
}

#[test]
fn a_replaced_file_is_refused_until_the_owner_accepts_it() {
    let fixture = Fixture::new();
    let copy = fixture.path.with_file_name("copy.db");
    std::fs::copy(&fixture.path, &copy).unwrap();
    std::fs::rename(&copy, &fixture.path).unwrap();

    assert_eq!(
        fixture.read("SELECT 1"),
        Err(EngineError::File(FileRefusal::Changed))
    );
}

#[test]
fn the_structure_names_every_kind_of_object() {
    let fixture = Fixture::new();

    let read = structure(
        &fixture.path,
        &fixture.identity,
        Duration::from_secs(5),
        &Cancel::new(),
    )
    .unwrap();

    let kinds: Vec<(&str, TableKind)> = read
        .tables
        .iter()
        .map(|table| (table.name.as_str(), table.kind))
        .filter(|(name, _)| !name.starts_with("notes_"))
        .collect();
    assert_eq!(
        kinds,
        [
            ("items", TableKind::Table),
            ("log", TableKind::Table),
            ("names", TableKind::View),
            ("notes", TableKind::Virtual),
            ("parts", TableKind::Table),
            ("tags", TableKind::Table),
        ]
    );
    assert!(
        read.tables
            .iter()
            .any(|table| table.kind == TableKind::Shadow)
    );
    let items = read
        .tables
        .iter()
        .find(|table| table.name == "items")
        .unwrap();
    assert_eq!(
        items
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["id", "name", "price"]
    );
    assert_eq!(items.columns[0].primary_key, 1);
    assert!(items.columns[1].not_null);
    let parts = read
        .tables
        .iter()
        .find(|table| table.name == "parts")
        .unwrap();
    assert_eq!(parts.foreign_keys[0].parent, "items");
    assert_eq!(parts.foreign_keys[0].columns, ["item"]);
    let tags = read
        .tables
        .iter()
        .find(|table| table.name == "tags")
        .unwrap();
    assert!(tags.without_rowid);
    let index = read
        .indexes
        .iter()
        .find(|index| index.name == "items_name")
        .unwrap();
    assert_eq!(index.columns, [Some("name".to_owned())]);
    assert!(!index.automatic);
    assert_eq!(read.triggers[0].name, "items_logged");
}

#[test]
fn a_copy_of_the_schema_classifies_statements_as_the_file_does() {
    let fixture = Fixture::new();
    let read = structure(
        &fixture.path,
        &fixture.identity,
        Duration::from_secs(5),
        &Cancel::new(),
    )
    .unwrap();
    let schema = read.create_statements();

    for statement in [
        "SELECT name FROM names",
        "INSERT INTO items (name) VALUES ('x')",
        "DELETE FROM parts",
        "INSERT INTO notes (body) VALUES ('text')",
        "UPDATE tags SET color = 'fff'",
        "CREATE INDEX parts_part ON parts (part)",
        "DROP VIEW names",
    ] {
        assert_eq!(
            classify_on_schema(&schema, statement).unwrap(),
            classify(&fixture.path, &fixture.identity, statement).unwrap(),
            "{statement}"
        );
    }
    classify_on_schema(&schema, "ATTACH DATABASE 'x' AS x").unwrap_err();
}

#[test]
fn a_forged_schema_runs_nothing_but_its_create_statements_in_the_copy() {
    let folder = tempfile::tempdir().unwrap();
    let attached = folder.path().join("attached.db");
    let vacuumed = folder.path().join("vacuumed.db");
    let forged = vec![
        format!("ATTACH DATABASE '{}' AS other", attached.display()),
        format!(
            "CREATE TABLE main.t (x); ATTACH DATABASE '{}' AS other",
            attached.display()
        ),
        format!("VACUUM INTO '{}'", vacuumed.display()),
        "PRAGMA hard_heap_limit = 1".to_owned(),
        "CREATE TEMP TABLE scratch (x)".to_owned(),
        "CREATE TABLE loaded AS SELECT load_extension('evil') AS x".to_owned(),
        "CREATE TABLE kept (id INTEGER PRIMARY KEY, name TEXT)".to_owned(),
    ];

    let class = classify_on_schema(&forged, "INSERT INTO kept (name) VALUES ('x')");
    let missing = classify_on_schema(&forged, "SELECT x FROM t");
    let loaded = classify_on_schema(&forged, "SELECT x FROM loaded");

    assert_eq!(class, Ok(StatementClass::Write));
    assert!(
        matches!(missing, Err(EngineError::Invalid { .. })),
        "{missing:?}"
    );
    assert!(
        matches!(loaded, Err(EngineError::Invalid { .. })),
        "{loaded:?}"
    );
    assert!(!attached.exists(), "the copy attached a file");
    assert!(!vacuumed.exists(), "the copy vacuumed into a file");
    // The pragma never ran, so the process keeps its heap.
    let connection = Connection::open_in_memory().unwrap();
    let limit: i64 = connection
        .query_row("PRAGMA hard_heap_limit", [], |row| row.get(0))
        .unwrap();
    assert_eq!(limit, 0);
}

/// Every row of a table, page by page, with the request that each page
/// continues.
fn all_pages(fixture: &Fixture, table: &str, order: Option<Order>, rows: usize) -> Vec<Vec<Value>> {
    let info = fixture.table(table);
    let mut request = PageRequest {
        table: table.to_owned(),
        order,
        filters: Vec::new(),
        after: None,
        rows,
    };
    let mut seen = Vec::new();
    loop {
        let read = page(
            &fixture.path,
            &fixture.identity,
            &info,
            &request,
            LIMITS,
            &Cancel::new(),
        )
        .unwrap();
        seen.extend(read.rows.into_iter().map(|row| row.values));
        match read.next {
            Some(next) => request.after = Some(next),
            None => return seen,
        }
    }
}

#[test]
fn pages_follow_keys_through_every_row_once_in_either_order() {
    let fixture = Fixture::new();
    fixture
        .write(
            &["INSERT INTO items (name, price) VALUES ('pin', NULL), ('cam', 3.0), ('axle', 3.0)"],
            StatementClass::Write,
        )
        .unwrap();
    let names =
        |rows: &[Vec<Value>]| -> Vec<Value> { rows.iter().map(|row| row[1].clone()).collect() };

    let by_key = all_pages(&fixture, "items", None, 2);
    let ascending = all_pages(
        &fixture,
        "items",
        Some(Order {
            column: "price".to_owned(),
            descending: false,
        }),
        2,
    );
    let descending = all_pages(
        &fixture,
        "items",
        Some(Order {
            column: "price".to_owned(),
            descending: true,
        }),
        2,
    );

    assert_eq!(
        names(&by_key),
        ["bolt", "nut", "gear", "pin", "cam", "axle"].map(text)
    );
    // NULL first in ascending order, ties by rowid.
    assert_eq!(
        names(&ascending),
        ["nut", "pin", "bolt", "cam", "axle", "gear"].map(text)
    );
    assert_eq!(
        names(&descending),
        ["gear", "cam", "axle", "bolt", "nut", "pin"].map(text)
    );
}

#[test]
fn a_page_filters_rows_and_follows_a_primary_key_or_an_offset() {
    let fixture = Fixture::new();
    let items = fixture.table("items");
    let filtered = page(
        &fixture.path,
        &fixture.identity,
        &items,
        &PageRequest {
            table: "items".to_owned(),
            order: None,
            filters: vec![
                Filter {
                    column: "price".to_owned(),
                    operator: FilterOperator::IsNotNull,
                    value: Value::Null,
                },
                Filter {
                    column: "name".to_owned(),
                    operator: FilterOperator::Like,
                    value: text("%o%"),
                },
            ],
            after: None,
            rows: 10,
        },
        LIMITS,
        &Cancel::new(),
    )
    .unwrap();
    let tags = all_pages(&fixture, "tags", None, 1);
    let names = all_pages(&fixture, "names", None, 2);
    let tags_page = page(
        &fixture.path,
        &fixture.identity,
        &fixture.table("tags"),
        &PageRequest {
            table: "tags".to_owned(),
            order: None,
            filters: Vec::new(),
            after: None,
            rows: 10,
        },
        LIMITS,
        &Cancel::new(),
    )
    .unwrap();

    assert_eq!(filtered.key, ["rowid"]);
    assert_eq!(filtered.rows.len(), 1);
    assert_eq!(filtered.rows[0].key, [Value::Integer(1)]);
    assert_eq!(tags_page.key, ["name"]);
    assert_eq!(
        tags,
        [
            vec![text("blue"), text("00f")],
            vec![text("red"), text("f00")]
        ]
    );
    assert_eq!(names.len(), 3);
}

#[test]
fn an_edit_checks_the_values_that_the_view_showed() {
    let fixture = Fixture::new();
    let items = fixture.table("items");
    let edit_of = |edit: &Edit| {
        super::edit(
            &fixture.path,
            &fixture.identity,
            &items,
            edit,
            LIMITS,
            &Cancel::new(),
        )
    };
    let bolt = |name: &str| RowMatch {
        key: vec![Value::Integer(1)],
        expected: vec![("name".to_owned(), text(name))],
    };

    let updated = edit_of(&Edit::Update {
        row: bolt("bolt"),
        set: vec![("name".to_owned(), text("screw"))],
    });
    let stale = edit_of(&Edit::Update {
        row: bolt("bolt"),
        set: vec![("name".to_owned(), text("rivet"))],
    });
    let inserted = edit_of(&Edit::Insert {
        values: vec![("name".to_owned(), text("clip"))],
    })
    .unwrap();
    let clip = |name: &str| RowMatch {
        key: vec![Value::Integer(4)],
        expected: vec![("name".to_owned(), text(name))],
    };
    let nut = RowMatch {
        key: vec![Value::Integer(2)],
        expected: Vec::new(),
    };
    let partly_stale = edit_of(&Edit::Delete {
        rows: vec![nut.clone(), clip("staple")],
    });
    let deleted = edit_of(&Edit::Delete {
        rows: vec![nut, clip("clip")],
    });
    // A part refers to the bolt, and foreign keys are enforced.
    let referred = edit_of(&Edit::Delete {
        rows: vec![bolt("screw")],
    });

    assert_eq!(
        updated,
        Ok(Edited {
            changes: 1,
            rowid: None
        })
    );
    assert_eq!(stale, Err(EngineError::RowChanged));
    assert_eq!(inserted.rowid, Some(4));
    assert_eq!(partly_stale, Err(EngineError::RowChanged));
    assert_eq!(
        deleted,
        Ok(Edited {
            changes: 2,
            rowid: None
        })
    );
    assert!(
        matches!(referred, Err(EngineError::Constraint { .. })),
        "{referred:?}"
    );
    assert_eq!(
        fixture.plain("SELECT name FROM items ORDER BY id"),
        ["screw", "gear"]
    );
    assert_eq!(
        edit_text(
            &items,
            &Edit::Update {
                row: bolt("x"),
                set: vec![("price".to_owned(), Value::Real(2.0))],
            }
        )
        .unwrap(),
        "UPDATE \"items\" SET \"price\" = ?1 WHERE \"rowid\" = ?2 AND \"name\" IS ?3"
    );
}

#[test]
fn a_view_counts_rows_on_request() {
    let fixture = Fixture::new();

    assert_eq!(
        count(
            &fixture.path,
            &fixture.identity,
            "items",
            LIMITS,
            &Cancel::new()
        ),
        Ok(3)
    );
    count(
        &fixture.path,
        &fixture.identity,
        "missing",
        LIMITS,
        &Cancel::new(),
    )
    .unwrap_err();
}

/// A store in a folder of its own.
fn store() -> (tempfile::TempDir, Store) {
    let folder = tempfile::tempdir().unwrap();
    let store = Store::open(
        &folder.path().join("mannie.sqlite"),
        &StoreOptions::new(folder.path().join("backups"), Arc::new(SystemClock)),
    )
    .unwrap();
    (folder, store)
}

fn key(last: u8) -> DatabaseKey {
    DatabaseKey::parse(&format!("0199e0f2-0000-7000-8000-0000000000{last:02x}")).unwrap()
}

fn registration(last: u8, path: &Path) -> NewRegistration {
    NewRegistration {
        key: key(last),
        name: format!("Database {last}"),
        path: path.to_owned(),
        identity: "1:2".to_owned(),
        read_only: false,
    }
}

#[tokio::test]
async fn the_registry_keeps_each_plugins_files_by_key() {
    let (_folder, store) = store();
    let path = PathBuf::from("/data/shop.db");

    let added = store
        .add_db_file("workers", registration(1, &path))
        .await
        .unwrap();
    let again = store.add_db_file("workers", registration(2, &path)).await;
    let other_plugin = store.add_db_file("notes", registration(3, &path)).await;
    store
        .rename_db_file("workers", &key(1), "Shop")
        .await
        .unwrap();
    store.mark_db_file("workers", &key(1), true).await.unwrap();
    store
        .accept_db_file("workers", &key(1), "3:4")
        .await
        .unwrap();
    let read = store.db_file("workers", &key(1)).await.unwrap();

    assert_eq!(added.key, key(1));
    assert!(
        matches!(again, Err(RegistryError::AlreadyRegistered { key: found }) if found == key(1))
    );
    other_plugin.unwrap();
    assert_eq!(read.name, "Shop");
    assert!(read.read_only);
    assert_eq!(read.identity, "3:4");
    assert_eq!(store.db_files("workers").await.unwrap().len(), 1);
    assert!(matches!(
        store.db_file("notes", &key(1)).await,
        Err(RegistryError::NotFound)
    ));
    store.forget_db_file("workers", &key(1)).await.unwrap();
    assert!(matches!(
        store.forget_db_file("workers", &key(1)).await,
        Err(RegistryError::NotFound)
    ));
}

#[tokio::test]
async fn a_plugin_registers_at_most_its_share_of_files() {
    let (_folder, store) = store();
    for index in 0..MAX_REGISTRATIONS {
        let last = u8::try_from(index % 256).unwrap();
        let mut new = registration(last, &PathBuf::from(format!("/data/{index}.db")));
        new.key = DatabaseKey::parse(&format!("0199e0f2-0000-7000-8000-{index:012x}")).unwrap();
        store.add_db_file("workers", new).await.unwrap();
    }

    let over = store
        .add_db_file(
            "workers",
            registration(255, &PathBuf::from("/data/over.db")),
        )
        .await;

    assert!(matches!(over, Err(RegistryError::TooMany)), "{over:?}");
}

fn operation(last: u8, class: StatementClass) -> NewOperation {
    NewOperation {
        id: DatabaseOperationId::parse(&format!("0199e0f2-0000-7000-8000-0000000001{last:02x}"))
            .unwrap(),
        plugin: "workers".to_owned(),
        key: key(1),
        statement: "UPDATE \"items\" SET \"name\" = ?1 WHERE \"rowid\" = ?2".to_owned(),
        parameters_digest: "digest".to_owned(),
        class,
    }
}

#[tokio::test]
async fn the_journal_records_an_owner_operation_before_it_runs_and_never_twice() {
    let (_folder, store) = store();
    let completed = operation(1, StatementClass::Write);
    let failed = operation(2, StatementClass::Schema);
    let cut_off = operation(3, StatementClass::Write);

    store.begin_db_operation(&completed).await.unwrap();
    let twice = store.begin_db_operation(&completed).await;
    store.complete_db_operation(&completed.id, 1).await.unwrap();
    store.begin_db_operation(&failed).await.unwrap();
    store
        .fail_db_operation(&failed.id, "dbfiles.busy")
        .await
        .unwrap();
    store.begin_db_operation(&cut_off).await.unwrap();
    let recovered = store.recover_db_operations().await.unwrap();

    assert!(matches!(twice, Err(OperationError::AlreadyJournaled)));
    assert_eq!(recovered, 1);
    assert_eq!(
        store.db_operation_state(&completed.id).await.unwrap(),
        Some(OperationState::Completed)
    );
    assert_eq!(
        store.db_operation_state(&failed.id).await.unwrap(),
        Some(OperationState::Failed)
    );
    assert_eq!(
        store.db_operation_state(&cut_off.id).await.unwrap(),
        Some(OperationState::Uncertain)
    );
    assert!(matches!(
        store.begin_db_operation(&cut_off).await,
        Err(OperationError::AlreadyJournaled)
    ));
    assert!(matches!(
        store
            .begin_db_operation(&operation(4, StatementClass::Read))
            .await,
        Err(OperationError::NotAChange)
    ));
}

#[tokio::test]
async fn a_long_statement_is_cut_in_the_journal() {
    let (_folder, store) = store();
    let mut long = operation(5, StatementClass::Write);
    long.statement = "x".repeat(MAX_STATEMENT_CHARS + 10);

    store.begin_db_operation(&long).await.unwrap();

    let (kept, cut): (i64, i64) = store
        .read(move |connection| {
            connection.query_row(
                "SELECT length(statement), statement_cut FROM dbfiles_operations",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .await
        .unwrap();
    assert_eq!(usize::try_from(kept).unwrap(), MAX_STATEMENT_CHARS);
    assert_eq!(cut, 1);
}
