//! The SQLite authorizer of plugins' statements and migrations (ADR-0034,
//! decision 7, with the rules that spike S4 corrected): a plugin's SQL
//! reaches only the tables named `<prefix>_*` of the main schema, and a
//! migration may also create, alter and drop tables and indexes under the
//! prefix, with the statements that SQLite nests in that DDL and authorizes
//! like the plugin's own. Everything else is refused: the core's tables,
//! transactions and savepoints, `ATTACH`, `VACUUM`, pragmas, `ANALYZE`,
//! `REINDEX`, triggers, views, virtual tables, temporary objects,
//! `AUTOINCREMENT` and `load_extension`.
//!
//! The authorizer sees only the old name of `ALTER TABLE ... RENAME TO` and
//! never a `REFERENCES` clause, so after each plugin migration, inside its
//! transaction, [`check_schema`] compares the schema outside the prefix with
//! the schema before the migration, and refuses any change there and any
//! foreign key to a table outside the prefix.
//!
//! The store installs the authorizer before it prepares a plugin's statement
//! and removes it once the statement has finished stepping, because SQLite
//! prepares a statement again, under whatever authorizer is installed then,
//! when another connection changed the schema. Installing or removing it
//! makes SQLite prepare every cached statement of the connection again
//! before it next runs, so a core statement cached under no authorizer never
//! runs under a plugin's rules, nor a plugin's under none.

use std::collections::BTreeMap;

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::{Connection, ffi};
use sha2::{Digest, Sha256};

use crate::legacy::{EXEMPT_MIGRATIONS, ExemptMigration};

/// The prefix of the automatic indexes of PRIMARY KEY and UNIQUE
/// constraints, which SQLite names `sqlite_autoindex_<table>_<n>`.
const AUTOMATIC_INDEX: &str = "sqlite_autoindex_";

/// What a plugin's SQL may do while the authorizer is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// A listed statement: rows of the plugin's tables only.
    Statement,
    /// A migration: also the plugin's tables and indexes themselves.
    Migration,
}

/// The rules for one plugin's SQL.
#[derive(Debug, Clone)]
pub(crate) struct Policy {
    /// The plugin's tables' prefix with its underscore, such as `workers_`.
    prefix: String,
    mode: Mode,
    /// The one table outside the prefix that a frozen migration may insert
    /// into.
    may_insert_into: Option<&'static str>,
}

/// SQLite compares names without regard to ASCII case.
fn starts_with_ignoring_case(name: &str, prefix: &str) -> bool {
    name.len() > prefix.len()
        && name
            .get(..prefix.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
}

/// The SHA-256 of a text, in lower-case hexadecimal.
fn sha256_hex(text: &str) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(text.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push(char::from(DIGITS[usize::from(byte >> 4)]));
        hex.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    hex
}

/// The table outside the prefix that `records` let the migration `version`
/// of `module` insert into, when they name it with exactly the text `sql`:
/// another module, version or byte, comments and line endings included, gets
/// no exemption.
fn exemption(
    records: &[ExemptMigration],
    module: &str,
    version: u32,
    sql: &str,
) -> Option<&'static str> {
    let hash = sha256_hex(sql);
    records
        .iter()
        .find(|exempt| {
            exempt.module == module && exempt.version == version && exempt.sql_sha256 == hash
        })
        .map(|exempt| exempt.may_insert_into)
}

impl Policy {
    /// The rules for a listed statement of the module `module`.
    pub(crate) fn statement(module: &str) -> Self {
        Self {
            prefix: format!("{module}_"),
            mode: Mode::Statement,
            may_insert_into: None,
        }
    }

    /// The rules for the migration `version` of the module `module`, whose
    /// text is `sql`. Only a migration that the frozen record names by its
    /// module, version and exact text may insert into its one table outside
    /// the prefix.
    pub(crate) fn migration(module: &str, version: u32, sql: &str) -> Self {
        Self {
            prefix: format!("{module}_"),
            mode: Mode::Migration,
            may_insert_into: exemption(EXEMPT_MIGRATIONS, module, version, sql),
        }
    }

    /// Whether a table or an index belongs to the plugin by its name.
    fn owns(&self, name: &str) -> bool {
        starts_with_ignoring_case(name, &self.prefix)
    }

    /// Whether an index belongs to the plugin: by its name, or as the
    /// automatic index of one of the plugin's tables.
    fn owns_index(&self, name: &str) -> bool {
        self.owns(name)
            || name.get(AUTOMATIC_INDEX.len()..).is_some_and(|table| {
                starts_with_ignoring_case(name, AUTOMATIC_INDEX) && self.owns(table)
            })
    }

    /// Whether SQLite may go on with one action of the plugin's SQL.
    fn allows(&self, context: &AuthContext<'_>) -> bool {
        let migration = self.mode == Mode::Migration;
        let main = context.database_name == Some("main");
        let temp = context.database_name == Some("temp");
        match context.action {
            AuthAction::Select | AuthAction::Recursive => true,
            AuthAction::Function { function_name } => {
                !function_name.eq_ignore_ascii_case("load_extension")
            }
            // A read of no column, as in `count(*)` or `EXISTS`, comes with an
            // empty column and the schema name as the SQL writes it, which is
            // none when it is not qualified.
            AuthAction::Read {
                table_name,
                column_name: "",
            } if context.database_name.is_none() => self.owns(table_name),
            AuthAction::Read { table_name, .. } => {
                (main
                    && (self.owns(table_name)
                        || (migration && table_name == "sqlite_master")
                        // ADD COLUMN on a STRICT table or one with CHECK
                        // constraints runs quick_check on it.
                        || (migration && table_name == "pragma_quick_check")))
                    || (migration && temp && table_name == "sqlite_temp_master")
            }
            AuthAction::Update { table_name, .. } => {
                (main && (self.owns(table_name) || (migration && table_name == "sqlite_master")))
                    || (migration && temp && table_name == "sqlite_temp_master")
            }
            AuthAction::Delete { table_name } => {
                main && (self.owns(table_name) || (migration && table_name == "sqlite_master"))
            }
            AuthAction::Insert { table_name } => {
                main && (self.owns(table_name)
                    || (migration && table_name == "sqlite_master")
                    || (migration && self.may_insert_into == Some(table_name)))
            }
            AuthAction::CreateTable { table_name } | AuthAction::DropTable { table_name } => {
                migration && main && self.owns(table_name)
            }
            AuthAction::CreateIndex {
                index_name,
                table_name,
            }
            | AuthAction::DropIndex {
                index_name,
                table_name,
            } => migration && main && self.owns_index(index_name) && self.owns(table_name),
            // CREATE INDEX asks to reindex the new index.
            AuthAction::Reindex { index_name } => migration && main && self.owns_index(index_name),
            AuthAction::Pragma {
                pragma_name,
                pragma_value: Some(table),
            } => migration && main && pragma_name == "quick_check" && self.owns(table),
            // SQLite passes the schema's name as the first argument of an
            // ALTER TABLE, which rusqlite names `database_name`.
            AuthAction::AlterTable {
                database_name,
                table_name,
            } => migration && database_name == "main" && self.owns(table_name),
            _other => false,
        }
    }

    fn decide(&self, context: &AuthContext<'_>) -> Authorization {
        if self.allows(context) {
            Authorization::Allow
        } else {
            Authorization::Deny
        }
    }
}

/// Runs `work` on `connection` with the authorizer of `policy` installed,
/// and removes the authorizer afterwards, also when `work` failed.
pub(crate) fn confined<T>(
    connection: &Connection,
    policy: &Policy,
    work: impl FnOnce(&Connection) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    let rules = policy.clone();
    connection.authorizer(Some(move |context: AuthContext<'_>| rules.decide(&context)))?;
    let done = work(connection);
    let removed = connection.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let value = done?;
    removed?;
    Ok(value)
}

/// Runs the migration `version` of the plugin's module `module`, whose text
/// is `sql`, under the authorizer, then refuses what it changed outside the
/// prefix. The caller's transaction rolls a refused migration back.
pub(crate) fn run_plugin_migration(
    connection: &Connection,
    module: &str,
    version: u32,
    sql: &str,
) -> rusqlite::Result<()> {
    let policy = Policy::migration(module, version, sql);
    let before = read_schema(connection)?;
    confined(connection, &policy, |connection| {
        connection.execute_batch(sql)
    })?;
    check_schema(connection, &policy, &before)
}

/// The schema as the store reads it without the authorizer: each object's
/// type, table and SQL, by its name.
type Schema = BTreeMap<String, (String, String, Option<String>)>;

/// Reads the schema of the main database.
fn read_schema(connection: &Connection) -> rusqlite::Result<Schema> {
    let mut select = connection.prepare("SELECT name, type, tbl_name, sql FROM sqlite_schema")?;
    let rows = select.query_map([], |row| {
        Ok((row.get(0)?, (row.get(1)?, row.get(2)?, row.get(3)?)))
    })?;
    rows.collect()
}

/// Refuses what a plugin's migration changed outside its prefix, which the
/// authorizer cannot see: a table renamed out of the prefix, any other change
/// of an object outside it, and a foreign key from one of the plugin's tables
/// to a table outside it. It runs inside the migration's transaction and
/// without the authorizer, and refuses with SQLite's code of an unauthorized
/// statement, which rolls the migration back.
fn check_schema(connection: &Connection, policy: &Policy, before: &Schema) -> rusqlite::Result<()> {
    let after = read_schema(connection)?;
    let owned = |name: &str, table: &str| {
        policy.owns(table) && (policy.owns(name) || policy.owns_index(name))
    };
    for (name, object) in &after {
        let (kind, table, _sql) = object;
        let unchanged = before.get(name) == Some(object);
        let ours = owned(name, table) && (kind == "table" || kind == "index");
        if !unchanged && !ours {
            return Err(refused(format!(
                "the migration changed the {kind} {name} outside the plugin's prefix"
            )));
        }
    }
    for (name, (kind, table, _sql)) in before {
        if !after.contains_key(name) && !owned(name, table) {
            return Err(refused(format!(
                "the migration removed the {kind} {name} outside the plugin's prefix"
            )));
        }
    }
    let mut parents = connection.prepare("SELECT \"table\" FROM pragma_foreign_key_list(?1)")?;
    for (name, (kind, _table, _sql)) in &after {
        if kind != "table" || !policy.owns(name) {
            continue;
        }
        let referenced: Vec<String> = parents
            .query_map([name], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if let Some(parent) = referenced.iter().find(|parent| !policy.owns(parent)) {
            return Err(refused(format!(
                "the table {name} refers to the table {parent} outside the plugin's prefix"
            )));
        }
    }
    Ok(())
}

/// SQLite's error of an unauthorized statement, with what was refused.
fn refused(what: String) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(ffi::Error::new(ffi::SQLITE_AUTH), Some(what))
}

#[cfg(test)]
mod tests {
    use rusqlite::{Connection, ErrorCode};

    use super::{Policy, confined, exemption, run_plugin_migration};
    use crate::legacy::ExemptMigration;

    /// A database with a core table and the plugin `notes`'s table.
    fn database() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE runtime_conversations (id TEXT PRIMARY KEY, title TEXT) STRICT;
                 CREATE TABLE notes_items (id TEXT PRIMARY KEY, body TEXT NOT NULL) STRICT;
                 INSERT INTO runtime_conversations (id, title) VALUES ('c1', 'Secret plans');
                 INSERT INTO notes_items (id, body) VALUES ('n1', 'One');",
            )
            .unwrap();
        connection
    }

    /// Whether `sql` runs as a statement of `notes`.
    fn statement_runs(connection: &Connection, sql: &str) -> bool {
        confined(connection, &Policy::statement("notes"), |connection| {
            let mut statement = connection.prepare(sql)?;
            let mut rows = statement.raw_query();
            while rows.next()?.is_some() {}
            Ok(())
        })
        .is_ok()
    }

    fn is_unauthorized(error: &rusqlite::Error) -> bool {
        error.sqlite_error_code() == Some(ErrorCode::AuthorizationForStatementDenied)
    }

    #[test]
    fn a_statement_reaches_only_the_tables_of_its_prefix() {
        let connection = database();

        for allowed in [
            "SELECT id, body FROM notes_items",
            "SELECT count(*) FROM notes_items",
            "SELECT EXISTS (SELECT 1 FROM notes_items)",
            "INSERT INTO notes_items (id, body) VALUES ('n2', 'Two')",
            "UPDATE notes_items SET body = 'Uno' WHERE id = 'n1'",
            "DELETE FROM notes_items WHERE id = 'n2'",
            "SELECT upper(body) FROM notes_items",
        ] {
            assert!(statement_runs(&connection, allowed), "{allowed}");
        }
        for refused in [
            "SELECT title FROM runtime_conversations",
            "SELECT count(*) FROM runtime_conversations",
            "SELECT body FROM main.notes_items JOIN runtime_conversations ON 1",
            "INSERT INTO runtime_conversations (id, title) VALUES ('c2', 'x')",
            "DELETE FROM runtime_conversations",
            "PRAGMA user_version",
            "SELECT * FROM pragma_table_info('runtime_conversations')",
            "ATTACH DATABASE ':memory:' AS other",
            "BEGIN",
            "SAVEPOINT plugin",
            "SELECT load_extension('evil')",
            "CREATE TABLE notes_more (id TEXT) STRICT",
        ] {
            assert!(!statement_runs(&connection, refused), "{refused}");
        }
    }

    #[test]
    fn a_migration_changes_only_its_own_tables_and_indexes() {
        let connection = database();
        let migration = |sql: &str| run_plugin_migration(&connection, "notes", 2, sql);

        let allowed = migration(
            "CREATE TABLE notes_tags (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE) STRICT;
             CREATE INDEX notes_tags_name ON notes_tags (name);
             ALTER TABLE notes_items ADD COLUMN size INTEGER;
             CREATE TABLE notes_links (note TEXT NOT NULL REFERENCES notes_items (id)) STRICT;
             DROP INDEX notes_tags_name;",
        );

        assert!(allowed.is_ok(), "{allowed:?}");
        for refused in [
            "CREATE TABLE others (id TEXT) STRICT",
            "CREATE VIEW notes_view AS SELECT id FROM notes_items",
            "CREATE TRIGGER notes_trigger AFTER INSERT ON notes_items BEGIN SELECT 1; END",
            "CREATE TABLE notes_counter (id INTEGER PRIMARY KEY AUTOINCREMENT)",
            "CREATE TEMP TABLE notes_scratch (id TEXT)",
            "CREATE INDEX notes_titles ON runtime_conversations (title)",
            "DROP TABLE runtime_conversations",
            "ALTER TABLE runtime_conversations ADD COLUMN notes TEXT",
            "INSERT INTO runtime_conversations (id, title) VALUES ('c3', 'x')",
            "VACUUM",
            "PRAGMA foreign_keys = OFF",
        ] {
            let error = migration(refused).unwrap_err();
            assert!(is_unauthorized(&error), "{refused}: {error}");
        }
    }

    #[test]
    fn the_schema_check_refuses_a_rename_out_of_the_prefix_and_a_foreign_key_to_a_core_table() {
        let connection = database();

        let renamed = run_plugin_migration(
            &connection,
            "notes",
            2,
            "ALTER TABLE notes_items RENAME TO hidden_items",
        )
        .unwrap_err();
        let referring = run_plugin_migration(
            &connection,
            "notes",
            2,
            "CREATE TABLE notes_links (conversation TEXT REFERENCES runtime_conversations (id)) STRICT",
        )
        .unwrap_err();

        assert!(is_unauthorized(&renamed), "{renamed}");
        assert!(renamed.to_string().contains("hidden_items"), "{renamed}");
        assert!(is_unauthorized(&referring), "{referring}");
        assert!(
            referring.to_string().contains("runtime_conversations"),
            "{referring}"
        );
    }

    #[test]
    fn only_the_frozen_text_of_a_migration_may_insert_outside_its_prefix() {
        const SQL: &str = "INSERT INTO runtime_conversations (id, title) VALUES ('c9', 'moved');\n";
        let hash: &'static str = Box::leak(super::sha256_hex(SQL).into_boxed_str());
        let records = [ExemptMigration {
            module: "notes",
            version: 3,
            sql_sha256: hash,
            may_insert_into: "runtime_conversations",
        }];

        assert_eq!(
            exemption(&records, "notes", 3, SQL),
            Some("runtime_conversations")
        );
        assert_eq!(exemption(&records, "journal", 3, SQL), None);
        assert_eq!(exemption(&records, "notes", 2, SQL), None);
        assert_eq!(
            exemption(&records, "notes", 3, &SQL.replace('\n', "\r\n")),
            None
        );
        assert_eq!(
            exemption(&records, "notes", 3, &format!("-- moved\n{SQL}")),
            None
        );
    }

    #[test]
    fn the_authorizer_is_removed_after_a_plugin_statement_also_when_it_failed() {
        let connection = database();

        let failed = statement_runs(&connection, "SELECT title FROM runtime_conversations");
        let core: String = connection
            .query_row("SELECT title FROM runtime_conversations", [], |row| {
                row.get(0)
            })
            .unwrap();

        assert!(!failed);
        assert_eq!(core, "Secret plans");
    }

    #[test]
    fn the_hash_is_the_sha_256_of_the_text_in_lower_case_hexadecimal() {
        // The test vector of FIPS 180-2 for "abc".
        assert_eq!(
            super::sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
