//! The values of the owner's databases as the engine reads and writes them:
//! SQLite's five storage classes, with integers as 64-bit integers. The
//! runtime turns them into the service's values, which carry integers as
//! text through IPC (ADR-0056, decision 4).

use rusqlite::ToSql;
use rusqlite::types::{ToSqlOutput, ValueRef};

/// A value of SQLite.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `NULL`.
    Null,
    /// A 64-bit integer.
    Integer(i64),
    /// A real number, which can be infinite. SQLite stores no NaN.
    Real(f64),
    /// Text in UTF-8. Text that is not UTF-8 reads with replacement
    /// characters.
    Text(String),
    /// Bytes.
    Blob(Vec<u8>),
}

impl Value {
    /// The bytes that the value counts against a result's limit: its text or
    /// bytes, or 8 for a number and 1 for `NULL`.
    pub fn size(&self) -> usize {
        match self {
            Self::Null => 1,
            Self::Integer(_) | Self::Real(_) => 8,
            Self::Text(text) => text.len(),
            Self::Blob(bytes) => bytes.len(),
        }
    }

    /// The value that SQLite returned for a column.
    pub(super) fn read(value: ValueRef<'_>) -> Self {
        match value {
            ValueRef::Null => Self::Null,
            ValueRef::Integer(integer) => Self::Integer(integer),
            ValueRef::Real(real) => Self::Real(real),
            ValueRef::Text(bytes) => Self::Text(String::from_utf8_lossy(bytes).into_owned()),
            ValueRef::Blob(bytes) => Self::Blob(bytes.to_vec()),
        }
    }
}

impl ToSql for Value {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(match self {
            Self::Null => ToSqlOutput::Borrowed(ValueRef::Null),
            Self::Integer(integer) => ToSqlOutput::Borrowed(ValueRef::Integer(*integer)),
            Self::Real(real) => ToSqlOutput::Borrowed(ValueRef::Real(*real)),
            Self::Text(text) => ToSqlOutput::Borrowed(ValueRef::Text(text.as_bytes())),
            Self::Blob(bytes) => ToSqlOutput::Borrowed(ValueRef::Blob(bytes)),
        })
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::Value;

    #[test]
    fn values_keep_their_storage_classes_through_a_round_trip() {
        let connection = Connection::open_in_memory().unwrap();
        let values = [
            Value::Null,
            Value::Integer(i64::MAX),
            Value::Integer(i64::MIN),
            Value::Real(1.5),
            Value::Real(f64::INFINITY),
            Value::Real(f64::NEG_INFINITY),
            Value::Text("Grüße".to_owned()),
            Value::Blob(vec![0, 159, 146, 150]),
        ];

        for value in values {
            let read = connection
                .query_row("SELECT ?1", [&value], |row| {
                    Ok(Value::read(row.get_ref(0)?))
                })
                .unwrap();
            assert_eq!(read, value);
        }
    }

    #[test]
    fn a_value_counts_its_bytes() {
        assert_eq!(Value::Null.size(), 1);
        assert_eq!(Value::Integer(7).size(), 8);
        assert_eq!(Value::Text("Grüße".to_owned()).size(), 7);
        assert_eq!(Value::Blob(vec![1, 2, 3]).size(), 3);
    }
}
