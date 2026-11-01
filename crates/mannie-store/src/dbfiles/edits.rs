//! The owner's edits of rows in a plugin's view (ADR-0056, decision 4): a
//! change of one row's values, an insert, and a deletion of rows. The
//! engine builds each as statements with quoted names and parameters, which
//! run in one transaction under the authorizer as a write. An edit or a
//! deletion names its rows by their keys and checks the values that the
//! view showed, so a row that another program changed meanwhile is left as
//! it is and the transaction rolls back.

use std::path::Path;

use mannie_types::tools::StatementClass;

use super::browse::{MAX_PAGE_ROWS, key_of};
use super::engine::{self, Cancel, EngineError, Limits, Sql, quote};
use super::structure::TableInfo;
use super::values::Value;

/// A row as the view showed it: its key, in the order of the table's key,
/// and the values of the columns that the edit checks.
#[derive(Debug, Clone, PartialEq)]
pub struct RowMatch {
    /// The row's key.
    pub key: Vec<Value>,
    /// The columns whose values the row must still have, with those values.
    pub expected: Vec<(String, Value)>,
}

/// An edit of rows.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// Sets columns of one row.
    Update {
        /// The row.
        row: RowMatch,
        /// The columns and their new values.
        set: Vec<(String, Value)>,
    },
    /// Inserts one row with these values, and the defaults of the other
    /// columns.
    Insert {
        /// The columns and their values.
        values: Vec<(String, Value)>,
    },
    /// Deletes rows.
    Delete {
        /// The rows, at most [`MAX_PAGE_ROWS`].
        rows: Vec<RowMatch>,
    },
}

/// What an edit did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edited {
    /// The rows that it changed.
    pub changes: u64,
    /// The rowid of an inserted row, for a table with rowids.
    pub rowid: Option<i64>,
}

/// The condition that names a row: its key and the values that it must
/// still have.
fn row_condition(
    key: &[String],
    row: &RowMatch,
    parameters: &mut Vec<Value>,
) -> Result<String, EngineError> {
    if key.is_empty() || key.len() != row.key.len() {
        return Err(EngineError::NoKey);
    }
    let mut conditions = Vec::new();
    for (name, value) in key.iter().zip(&row.key) {
        parameters.push(value.clone());
        conditions.push(format!("{} = ?{}", quote(name), parameters.len()));
    }
    for (column, value) in &row.expected {
        parameters.push(value.clone());
        conditions.push(format!("{} IS ?{}", quote(column), parameters.len()));
    }
    Ok(conditions.join(" AND "))
}

/// The statements of an edit, each of which must change exactly one row,
/// except an insert, which changes the one row that it inserts.
fn statements_of(table: &TableInfo, edit: &Edit) -> Result<Vec<Sql>, EngineError> {
    let name = quote(&table.name);
    match edit {
        Edit::Update { row, set } => {
            if set.is_empty() {
                return Err(EngineError::BatchSize);
            }
            let key = key_of(table)?;
            let mut parameters = Vec::new();
            let mut assignments = Vec::new();
            for (column, value) in set {
                parameters.push(value.clone());
                assignments.push(format!("{} = ?{}", quote(column), parameters.len()));
            }
            let condition = row_condition(&key, row, &mut parameters)?;
            Ok(vec![Sql {
                text: format!(
                    "UPDATE {name} SET {} WHERE {condition}",
                    assignments.join(", ")
                ),
                parameters,
            }])
        }
        Edit::Insert { values } => {
            if values.is_empty() {
                return Ok(vec![Sql {
                    text: format!("INSERT INTO {name} DEFAULT VALUES"),
                    parameters: Vec::new(),
                }]);
            }
            let columns: Vec<String> = values.iter().map(|(column, _)| quote(column)).collect();
            let places: Vec<String> = (1..=values.len())
                .map(|place| format!("?{place}"))
                .collect();
            Ok(vec![Sql {
                text: format!(
                    "INSERT INTO {name} ({}) VALUES ({})",
                    columns.join(", "),
                    places.join(", ")
                ),
                parameters: values.iter().map(|(_, value)| value.clone()).collect(),
            }])
        }
        Edit::Delete { rows } => {
            if rows.is_empty() || rows.len() > MAX_PAGE_ROWS {
                return Err(EngineError::BatchSize);
            }
            let key = key_of(table)?;
            rows.iter()
                .map(|row| {
                    let mut parameters = Vec::new();
                    let condition = row_condition(&key, row, &mut parameters)?;
                    Ok(Sql {
                        text: format!("DELETE FROM {name} WHERE {condition}"),
                        parameters,
                    })
                })
                .collect()
        }
    }
}

/// The text of an edit's statements, which the journal of owner operations
/// keeps without their values.
pub fn edit_text(table: &TableInfo, edit: &Edit) -> Result<String, EngineError> {
    let statements = statements_of(table, edit)?;
    let mut texts: Vec<&str> = statements.iter().map(|sql| sql.text.as_str()).collect();
    texts.dedup();
    Ok(texts.join(";\n"))
}

/// Runs an edit on a registered database in one transaction.
pub fn edit(
    path: &Path,
    identity: &str,
    table: &TableInfo,
    edit: &Edit,
    limits: Limits,
    cancel: &Cancel,
) -> Result<Edited, EngineError> {
    let statements = statements_of(table, edit)?;
    let is_insert = matches!(edit, Edit::Insert { .. });
    engine::in_transaction(path, identity, limits.time, cancel, |opened| {
        let mut changes = 0;
        for sql in &statements {
            let done = engine::run_one(opened, sql, StatementClass::Write, limits)?;
            if done.changes != 1 {
                return Err(EngineError::RowChanged);
            }
            changes += done.changes;
        }
        let rowid =
            (is_insert && !table.without_rowid).then(|| opened.connection.last_insert_rowid());
        Ok(Edited { changes, rowid })
    })
}
