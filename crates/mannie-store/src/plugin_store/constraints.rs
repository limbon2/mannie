//! Which constraint of a plugin's tables failed, from SQLite's error, so a
//! batch can give it the plugin's code (ADR-0034, decision 7).
//!
//! SQLite tells the constraints apart by their extended codes and names them
//! in its message (spike S4 recorded both for SQLite 3.53.2): a UNIQUE or
//! PRIMARY KEY by its table and columns, whatever the constraint's name, a
//! unique index on an expression by its name, NOT NULL and a STRICT type
//! error by table and column, a CHECK by its name or its expression, and a
//! foreign key with no detail.

use mannie_plugin_sdk::services::store::Constraint;
use rusqlite::ErrorCode;
use rusqlite::ffi;

/// A constraint that SQLite reported as failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FailedConstraint {
    /// A UNIQUE or PRIMARY KEY, by its table and columns.
    Key { table: String, columns: Vec<String> },
    /// A unique index on an expression, by its name.
    UniqueIndex { name: String },
    /// A NOT NULL column.
    NotNull { table: String, column: String },
    /// A value of another type in a column of a STRICT table.
    Datatype { table: String, column: String },
    /// A CHECK, by its name, or by its expression when it has none.
    Check { name: String },
    /// A foreign key.
    ForeignKey,
}

/// The constraint whose failure `error` reports, or `None` for any other
/// error or a message that this version of SQLite words otherwise.
pub(crate) fn failed(error: &rusqlite::Error) -> Option<FailedConstraint> {
    let rusqlite::Error::SqliteFailure(failure, message) = error else {
        return None;
    };
    if failure.code != ErrorCode::ConstraintViolation {
        return None;
    }
    let message = message.as_deref().unwrap_or_default();
    match failure.extended_code {
        ffi::SQLITE_CONSTRAINT_PRIMARYKEY | ffi::SQLITE_CONSTRAINT_UNIQUE => {
            let failed = message.strip_prefix("UNIQUE constraint failed: ")?;
            if let Some(name) = failed
                .strip_prefix("index '")
                .and_then(|rest| rest.strip_suffix('\''))
            {
                return Some(FailedConstraint::UniqueIndex {
                    name: name.to_owned(),
                });
            }
            key(failed)
        }
        ffi::SQLITE_CONSTRAINT_NOTNULL => {
            let (table, column) = qualified(message.strip_prefix("NOT NULL constraint failed: ")?)?;
            Some(FailedConstraint::NotNull { table, column })
        }
        ffi::SQLITE_CONSTRAINT_DATATYPE => {
            // "cannot store TEXT value in INTEGER column notes_items.size"
            let (_value, column) = message.rsplit_once(" column ")?;
            let (table, column) = qualified(column)?;
            Some(FailedConstraint::Datatype { table, column })
        }
        ffi::SQLITE_CONSTRAINT_CHECK => Some(FailedConstraint::Check {
            name: message
                .strip_prefix("CHECK constraint failed: ")?
                .to_owned(),
        }),
        ffi::SQLITE_CONSTRAINT_FOREIGNKEY => Some(FailedConstraint::ForeignKey),
        _other => None,
    }
}

/// The table and columns of a failed key: `table.a, table.b`.
fn key(failed: &str) -> Option<FailedConstraint> {
    let mut table = None;
    let mut columns = Vec::new();
    for qualified_column in failed.split(", ") {
        let (column_table, column) = qualified(qualified_column)?;
        if table.get_or_insert_with(|| column_table.clone()) != &column_table {
            return None;
        }
        columns.push(column);
    }
    Some(FailedConstraint::Key {
        table: table?,
        columns,
    })
}

/// `table.column`, split at its last dot.
fn qualified(text: &str) -> Option<(String, String)> {
    let (table, column) = text.rsplit_once('.')?;
    if table.is_empty() || column.is_empty() {
        return None;
    }
    Some((table.to_owned(), column.to_owned()))
}

impl FailedConstraint {
    /// Whether this is the constraint that a batch named.
    pub(crate) fn is(&self, constraint: &Constraint) -> bool {
        match (self, constraint) {
            (
                Self::Key { table, columns },
                Constraint::Key {
                    table: named_table,
                    columns: named_columns,
                },
            ) => table == named_table && columns.iter().eq(named_columns.iter()),
            (Self::UniqueIndex { name }, Constraint::UniqueIndex { name: named })
            | (Self::Check { name }, Constraint::Check { name: named }) => name == named,
            (
                Self::NotNull { table, column },
                Constraint::NotNull {
                    table: named_table,
                    column: named_column,
                },
            )
            | (
                Self::Datatype { table, column },
                Constraint::Datatype {
                    table: named_table,
                    column: named_column,
                },
            ) => table == named_table && column == named_column,
            (Self::ForeignKey, Constraint::ForeignKey) => true,
            _other => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::services::store::Constraint;
    use rusqlite::Connection;

    use super::{FailedConstraint, failed};

    /// The error of `sql` on a database with tables of every kind of
    /// constraint, as SQLite reports it.
    fn error_of(sql: &str) -> rusqlite::Error {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE notes_parents (id TEXT PRIMARY KEY) STRICT;
                 CREATE TABLE notes_items (
                     id TEXT PRIMARY KEY,
                     slug TEXT NOT NULL UNIQUE,
                     size INTEGER NOT NULL CONSTRAINT notes_size_positive CHECK (size > 0),
                     label TEXT,
                     parent TEXT REFERENCES notes_parents (id)
                 ) STRICT;
                 CREATE TABLE notes_pairs (a TEXT, b TEXT, PRIMARY KEY (a, b)) WITHOUT ROWID, STRICT;
                 CREATE UNIQUE INDEX notes_label_lower ON notes_items (lower(label));
                 INSERT INTO notes_items (id, slug, size, label) VALUES ('n1', 's1', 1, 'One');
                 INSERT INTO notes_pairs (a, b) VALUES ('x', 'y');",
            )
            .unwrap();
        connection.execute_batch(sql).unwrap_err()
    }

    #[test]
    fn keys_fail_by_their_table_and_columns() {
        assert_eq!(
            failed(&error_of(
                "INSERT INTO notes_items (id, slug, size) VALUES ('n2', 's1', 1)"
            )),
            Some(FailedConstraint::Key {
                table: "notes_items".to_owned(),
                columns: vec!["slug".to_owned()],
            })
        );
        assert_eq!(
            failed(&error_of(
                "INSERT INTO notes_items (id, slug, size) VALUES ('n1', 's2', 1)"
            )),
            Some(FailedConstraint::Key {
                table: "notes_items".to_owned(),
                columns: vec!["id".to_owned()],
            })
        );
        assert_eq!(
            failed(&error_of(
                "INSERT INTO notes_pairs (a, b) VALUES ('x', 'y')"
            )),
            Some(FailedConstraint::Key {
                table: "notes_pairs".to_owned(),
                columns: vec!["a".to_owned(), "b".to_owned()],
            })
        );
    }

    #[test]
    fn an_expression_index_a_check_not_null_a_type_and_a_foreign_key_fail_as_sqlite_names_them() {
        assert_eq!(
            failed(&error_of(
                "INSERT INTO notes_items (id, slug, size, label) VALUES ('n2', 's2', 1, 'ONE')"
            )),
            Some(FailedConstraint::UniqueIndex {
                name: "notes_label_lower".to_owned(),
            })
        );
        assert_eq!(
            failed(&error_of(
                "INSERT INTO notes_items (id, slug, size) VALUES ('n2', 's2', 0)"
            )),
            Some(FailedConstraint::Check {
                name: "notes_size_positive".to_owned(),
            })
        );
        assert_eq!(
            failed(&error_of(
                "INSERT INTO notes_items (id, slug) VALUES ('n2', 's2')"
            )),
            Some(FailedConstraint::NotNull {
                table: "notes_items".to_owned(),
                column: "size".to_owned(),
            })
        );
        assert_eq!(
            failed(&error_of(
                "INSERT INTO notes_items (id, slug, size) VALUES ('n2', 's2', 'large')"
            )),
            Some(FailedConstraint::Datatype {
                table: "notes_items".to_owned(),
                column: "size".to_owned(),
            })
        );
        assert_eq!(
            failed(&error_of(
                "INSERT INTO notes_items (id, slug, size, parent) VALUES ('n2', 's2', 1, 'gone')"
            )),
            Some(FailedConstraint::ForeignKey)
        );
    }

    #[test]
    fn other_errors_are_no_failed_constraint() {
        assert_eq!(
            failed(&error_of("INSERT INTO notes_missing VALUES (1)")),
            None
        );
        assert_eq!(failed(&rusqlite::Error::QueryReturnedNoRows), None);
    }

    #[test]
    fn a_failed_constraint_is_the_one_that_names_it_and_no_other() {
        let key = FailedConstraint::Key {
            table: "notes_items".to_owned(),
            columns: vec!["slug".to_owned()],
        };

        assert!(key.is(&Constraint::Key {
            table: "notes_items",
            columns: &["slug"],
        }));
        assert!(!key.is(&Constraint::Key {
            table: "notes_items",
            columns: &["id"],
        }));
        assert!(!key.is(&Constraint::Key {
            table: "notes_parents",
            columns: &["slug"],
        }));
        assert!(!key.is(&Constraint::ForeignKey));
        assert!(FailedConstraint::ForeignKey.is(&Constraint::ForeignKey));
        assert!(
            FailedConstraint::Check {
                name: "notes_size_positive".to_owned()
            }
            .is(&Constraint::Check {
                name: "notes_size_positive"
            })
        );
    }
}
