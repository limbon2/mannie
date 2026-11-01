//! The authorizer of the statements that the engine prepares on the owner's
//! SQLite files (ADR-0056, decision 3), with the rules that spike S1 of M15
//! settled (docs/roadmap/m15-validation.md, section 2), and the class of a
//! statement, which comes from the actions that SQLite reports while it
//! prepares the statement (decision 6).
//!
//! The authorizer refuses `ATTACH` and `DETACH`, transactions and
//! savepoints, since the engine owns its transactions, the creation of the
//! temporary database's objects and writes to it, the creation and removal
//! of virtual tables,
//! `load_extension`, and every pragma outside a list of pragmas that read.
//! `VACUUM` reports no action when it is prepared and attaches a database
//! when it runs, which the attach limit of 0 and the refusal of `ATTACH` both
//! stop. SQLite prepares a statement again, under the authorizer installed
//! then, when the schema changed, so the engine keeps its authorizer
//! installed for the whole life of each of its connections.

use std::sync::{Arc, Mutex, PoisonError};

use mannie_types::tools::StatementClass;
use rusqlite::Connection;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

/// The pragmas that only read and name a table or an index as their
/// argument, such as `PRAGMA table_info(items)`.
const PRAGMAS_WITH_TARGET: &[&str] = &[
    "table_info",
    "table_xinfo",
    "index_list",
    "index_info",
    "index_xinfo",
    "foreign_key_list",
];

/// The pragmas that only read and take no value. The table-valued form of
/// `table_list`, which reads the structure, reports this action when it
/// runs, as the other table-valued pragmas do not.
const PRAGMAS_WITHOUT_VALUE: &[&str] = &[
    "table_list",
    "foreign_key_check",
    "collation_list",
    "page_count",
    "page_size",
    "freelist_count",
    "encoding",
    "user_version",
    "application_id",
    "schema_version",
    "data_version",
    "quick_check",
    "integrity_check",
];

/// The tables in which SQLite keeps the schema, which a schema change
/// writes and a write never does.
const SCHEMA_TABLES: &[&str] = &["sqlite_master", "sqlite_schema", "sqlite_temp_master"];

/// What the actions of a statement showed so far.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Seen {
    /// The statement creates, changes or removes a schema object, or runs
    /// `REINDEX` or `ANALYZE`.
    schema: bool,
    /// The statement inserts, updates or deletes rows of a table.
    write: bool,
}

impl Seen {
    fn class(self) -> StatementClass {
        if self.schema {
            StatementClass::Schema
        } else if self.write {
            StatementClass::Write
        } else {
            StatementClass::Read
        }
    }
}

/// Whether a pragma only reads, as its name and value show.
fn pragma_reads(name: &str, value: Option<&str>) -> bool {
    let name = name.to_ascii_lowercase();
    let names_target = PRAGMAS_WITH_TARGET.contains(&name.as_str());
    let takes_nothing = PRAGMAS_WITHOUT_VALUE.contains(&name.as_str()) && value.is_none();
    names_target || takes_nothing
}

/// Whether SQLite may go on with one action, and what the action shows of
/// the statement's class. A refused action makes the statement fail to
/// prepare, or to run when SQLite authorizes it then.
fn decide(context: &AuthContext<'_>, seen: &mut Seen) -> Authorization {
    let in_temp = context.database_name == Some("temp");
    let allowed = match context.action {
        // Reads pass in the temporary database too, whose schema table
        // `pragma_table_list` reads: no statement can create an object there.
        AuthAction::Select | AuthAction::Recursive | AuthAction::Read { .. } => true,
        AuthAction::Function { function_name } => {
            !function_name.eq_ignore_ascii_case("load_extension")
        }
        AuthAction::Pragma {
            pragma_name,
            pragma_value,
        } => pragma_reads(pragma_name, pragma_value),
        AuthAction::Insert { table_name }
        | AuthAction::Delete { table_name }
        | AuthAction::Update { table_name, .. } => {
            let is_schema_table = SCHEMA_TABLES
                .iter()
                .any(|schema| table_name.eq_ignore_ascii_case(schema));
            if is_schema_table {
                seen.schema = true;
            } else {
                seen.write = true;
            }
            !in_temp
        }
        AuthAction::CreateIndex { .. }
        | AuthAction::CreateTable { .. }
        | AuthAction::CreateTrigger { .. }
        | AuthAction::CreateView { .. }
        | AuthAction::DropIndex { .. }
        | AuthAction::DropTable { .. }
        | AuthAction::DropTrigger { .. }
        | AuthAction::DropView { .. }
        | AuthAction::AlterTable { .. }
        | AuthAction::Reindex { .. }
        | AuthAction::Analyze { .. } => {
            seen.schema = true;
            !in_temp
        }
        // ATTACH and DETACH, transactions and savepoints, the temporary
        // database's objects, virtual tables and any action that this
        // version of SQLite's binding does not name.
        _other => false,
    };
    if allowed {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}

/// The actions that the authorizer of a connection saw since the engine
/// last cleared them, shared with the authorizer's closure.
#[derive(Debug, Clone, Default)]
pub(super) struct Actions(Arc<Mutex<Seen>>);

impl Actions {
    /// Forgets what the actions showed, before the engine prepares the next
    /// statement.
    pub(super) fn clear(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Seen::default();
    }

    /// The class that the actions since the last clearing show.
    pub(super) fn class(&self) -> StatementClass {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .class()
    }
}

/// Installs the authorizer on a connection for its whole life, and returns
/// what its actions show.
pub(super) fn install(connection: &Connection) -> rusqlite::Result<Actions> {
    let actions = Actions::default();
    let seen = Arc::clone(&actions.0);
    connection.authorizer(Some(move |context: AuthContext<'_>| {
        let mut seen = seen.lock().unwrap_or_else(PoisonError::into_inner);
        decide(&context, &mut seen)
    }))?;
    Ok(actions)
}

#[cfg(test)]
mod tests {
    use mannie_types::tools::StatementClass;
    use rusqlite::{Connection, ErrorCode};

    use super::install;

    /// A database with a table, a child table, an index, a trigger, a view
    /// and a virtual table, under the authorizer.
    fn database() -> (Connection, super::Actions) {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
                 CREATE TABLE parts (item INTEGER REFERENCES items (id), part TEXT);
                 CREATE INDEX items_name ON items (name);
                 CREATE TRIGGER items_named AFTER INSERT ON items BEGIN
                     INSERT INTO parts (item, part) VALUES (new.id, 'first');
                 END;
                 CREATE VIEW names AS SELECT name FROM items;
                 CREATE VIRTUAL TABLE notes USING fts5 (body);
                 INSERT INTO items (name) VALUES ('one');",
            )
            .unwrap();
        let actions = install(&connection).unwrap();
        (connection, actions)
    }

    /// The class of a statement that prepares under the authorizer, or the
    /// error of one that it refuses.
    fn class_of(
        connection: &Connection,
        actions: &super::Actions,
        sql: &str,
    ) -> rusqlite::Result<StatementClass> {
        actions.clear();
        connection.prepare(sql)?;
        Ok(actions.class())
    }

    #[test]
    fn reads_writes_and_schema_changes_have_their_classes() {
        let (connection, actions) = database();
        let cases = [
            ("SELECT name FROM items", StatementClass::Read),
            ("SELECT name FROM names", StatementClass::Read),
            (
                "SELECT body FROM notes WHERE notes MATCH 'x'",
                StatementClass::Read,
            ),
            ("SELECT name, sql FROM sqlite_schema", StatementClass::Read),
            (
                "SELECT * FROM pragma_table_info('items')",
                StatementClass::Read,
            ),
            ("PRAGMA table_info(items)", StatementClass::Read),
            ("PRAGMA user_version", StatementClass::Read),
            (
                "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n WHERE x < 3) SELECT x FROM n",
                StatementClass::Read,
            ),
            (
                "INSERT INTO items (name) VALUES ('two')",
                StatementClass::Write,
            ),
            (
                "UPDATE items SET name = 'uno' WHERE id = 1",
                StatementClass::Write,
            ),
            ("DELETE FROM parts", StatementClass::Write),
            (
                "INSERT INTO notes (body) VALUES ('text')",
                StatementClass::Write,
            ),
            (
                "INSERT INTO items (name) VALUES ('three') RETURNING id",
                StatementClass::Write,
            ),
            ("CREATE TABLE more (id INTEGER)", StatementClass::Schema),
            (
                "CREATE INDEX parts_part ON parts (part)",
                StatementClass::Schema,
            ),
            (
                "ALTER TABLE items ADD COLUMN size INTEGER",
                StatementClass::Schema,
            ),
            ("DROP VIEW names", StatementClass::Schema),
            ("DROP TRIGGER items_named", StatementClass::Schema),
            ("REINDEX items_name", StatementClass::Schema),
            ("ANALYZE", StatementClass::Schema),
        ];

        for (sql, class) in cases {
            assert_eq!(
                class_of(&connection, &actions, sql).unwrap(),
                class,
                "{sql}"
            );
        }
    }

    #[test]
    fn the_authorizer_refuses_what_the_engine_never_runs() {
        let (connection, actions) = database();

        for sql in [
            "ATTACH DATABASE 'other.db' AS other",
            "DETACH DATABASE main",
            "BEGIN",
            "BEGIN IMMEDIATE",
            "COMMIT",
            "SAVEPOINT engine",
            "RELEASE engine",
            "SELECT load_extension('evil')",
            "SELECT LOAD_EXTENSION('evil')",
            "CREATE TEMP TABLE scratch (id INTEGER)",
            "CREATE TEMP VIEW scratch AS SELECT 1",
            "CREATE VIRTUAL TABLE more USING fts5 (body)",
            "DROP TABLE notes",
            "PRAGMA writable_schema = 1",
            "PRAGMA journal_mode = DELETE",
            "PRAGMA user_version = 5",
            "PRAGMA foreign_keys = OFF",
            "PRAGMA trusted_schema = ON",
            "PRAGMA quick_check(items)",
        ] {
            let error = class_of(&connection, &actions, sql).unwrap_err();
            // SQLite reports a denied function as an error of the statement
            // with the authorizer's message, and every other denial with the
            // code of an unauthorized statement.
            let denied = error.sqlite_error_code()
                == Some(ErrorCode::AuthorizationForStatementDenied)
                || error.to_string().contains("not authorized");
            assert!(denied, "{sql}: {error}");
        }
    }
}
