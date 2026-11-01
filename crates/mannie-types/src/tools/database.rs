//! Statements on the owner's SQLite files (ADR-0056): the class of a
//! statement, which the engine reads from the actions that SQLite reports
//! while it prepares the statement, and which a database access and its
//! policy name, and where a registered database lives.

use serde::{Deserialize, Serialize};
use specta::Type;

/// The most characters of one parameter's value as an access shows it.
pub const MAX_VIEW_PARAMETER_CHARS: usize = 200;

/// The most parameters that an access shows; the digest covers them all.
pub const MAX_VIEW_PARAMETERS: usize = 32;

/// Where a registered database lives (ADR-0056, decision 5).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DatabaseHome {
    /// This computer, whose registrations the core keeps.
    ThisComputer,
}

/// What a statement on a database does (ADR-0056, decision 5), from the
/// weakest to the strongest, so the class of several statements is the
/// strongest of theirs.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "snake_case")]
pub enum StatementClass {
    /// Reads rows or the schema and changes nothing.
    Read,
    /// Inserts, updates or deletes rows.
    Write,
    /// Creates, changes or removes a table, an index, a view or a trigger,
    /// or runs `REINDEX` or `ANALYZE`.
    Schema,
}

#[cfg(test)]
mod tests {
    use super::StatementClass;

    #[test]
    fn a_class_serializes_as_its_name_and_orders_from_read_to_schema() {
        assert_eq!(
            serde_json::to_value(StatementClass::Schema).unwrap(),
            serde_json::json!("schema")
        );
        assert!(StatementClass::Read < StatementClass::Write);
        assert!(StatementClass::Write < StatementClass::Schema);
        assert_eq!(
            [StatementClass::Write, StatementClass::Read]
                .into_iter()
                .max(),
            Some(StatementClass::Write)
        );
    }
}
