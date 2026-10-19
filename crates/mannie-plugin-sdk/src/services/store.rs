//! The service `store` (ADR-0034, decisions 5 and 7; ADR-0030, decision 9):
//! a plugin's own tables in Mannie's store, which it reaches as statements
//! that are data.
//!
//! A plugin declares its module with [`StoreModule::new`]: its name, which
//! equals the plugin's prefix, its migrations, and every statement that it
//! runs. The store prepares each listed statement when it registers the
//! module, and refuses any other text with [`StoreError::UnknownStatement`].
//! Queries return rows of [`SqlValue`]s. Writes run as a [`WriteBatch`], one
//! transaction on the store's writer, with an expected change count for each
//! statement and a mapping from [`Constraint`]s to the plugin's own codes. No
//! plugin code runs on the writer thread.
//!
//! `tests/protocol.rs` lists this module as a known exception: a module's
//! name, its migrations and its statements are `&'static str`, which keeps a
//! plugin's SQL as text in its source, where review sees it, and a
//! constraint's code is a `&'static str` that the plugin defines.

use std::fmt;

use crate::BoxFuture;
use crate::service::Service;

/// The service `store`, major 1.
#[derive(Debug)]
pub enum Store {}

impl Service for Store {
    const NAME: &'static str = "store";
    const MAJOR: u16 = 1;
    type Api = dyn ModuleStoreApi;
}

/// A plugin's module of the store: its tables, as its migrations make them,
/// and the statements that it runs on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct StoreModule {
    /// The module's name, which equals the plugin's prefix, such as
    /// `workers`; its tables are named `<prefix>_*`.
    pub name: &'static str,
    /// The module's migrations, numbered from 1 without gaps.
    pub migrations: &'static [Migration],
    /// Every statement that the plugin runs, as its text.
    pub statements: &'static [&'static str],
}

impl StoreModule {
    /// The module `name` with its migrations and its statements.
    pub const fn new(
        name: &'static str,
        migrations: &'static [Migration],
        statements: &'static [&'static str],
    ) -> Self {
        Self {
            name,
            migrations,
            statements,
        }
    }
}

/// One of a module's forward-only migrations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Migration {
    version: u32,
    legacy_version: Option<u32>,
    description: &'static str,
    sql: &'static str,
}

impl Migration {
    /// The module's migration `version`.
    pub const fn new(version: u32, description: &'static str, sql: &'static str) -> Self {
        Self {
            version,
            legacy_version: None,
            description,
            sql,
        }
    }

    /// A migration that carries a number of the store's single list from
    /// before M5, so a database of 0.2.0 that applied it keeps it. The store
    /// accepts it only for the versions that its frozen record of those
    /// numbers holds.
    pub const fn legacy(
        version: u32,
        legacy_version: u32,
        description: &'static str,
        sql: &'static str,
    ) -> Self {
        Self {
            version,
            legacy_version: Some(legacy_version),
            description,
            sql,
        }
    }

    /// The migration's version in its module.
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// Its number in the store's list from before M5, if it has one.
    pub const fn legacy_version(&self) -> Option<u32> {
        self.legacy_version
    }

    /// What the migration does.
    pub const fn description(&self) -> &'static str {
        self.description
    }

    /// Its SQL.
    pub const fn sql(&self) -> &'static str {
        self.sql
    }
}

/// A value that a statement binds or a row holds.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlValue {
    /// `NULL`.
    Null,
    /// An integer.
    Integer(i64),
    /// A floating-point number.
    Real(f64),
    /// Text.
    Text(String),
    /// Bytes.
    Blob(Vec<u8>),
}

impl From<i64> for SqlValue {
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}

impl From<u32> for SqlValue {
    fn from(value: u32) -> Self {
        Self::Integer(i64::from(value))
    }
}

impl From<bool> for SqlValue {
    fn from(value: bool) -> Self {
        Self::Integer(i64::from(value))
    }
}

impl From<f64> for SqlValue {
    fn from(value: f64) -> Self {
        Self::Real(value)
    }
}

impl From<String> for SqlValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for SqlValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<Vec<u8>> for SqlValue {
    fn from(value: Vec<u8>) -> Self {
        Self::Blob(value)
    }
}

impl<T: Into<Self>> From<Option<T>> for SqlValue {
    fn from(value: Option<T>) -> Self {
        value.map_or(Self::Null, Into::into)
    }
}

/// A Rust value that a column of a row reads as.
pub trait FromSqlValue: Sized {
    /// The value, or `None` when the column holds a value of another kind.
    fn from_sql_value(value: &SqlValue) -> Option<Self>;
}

impl FromSqlValue for i64 {
    fn from_sql_value(value: &SqlValue) -> Option<Self> {
        match value {
            SqlValue::Integer(integer) => Some(*integer),
            _other => None,
        }
    }
}

impl FromSqlValue for u32 {
    fn from_sql_value(value: &SqlValue) -> Option<Self> {
        i64::from_sql_value(value).and_then(|integer| Self::try_from(integer).ok())
    }
}

/// An integer column reads as `true` when it is not 0, as SQLite's truth
/// does.
impl FromSqlValue for bool {
    fn from_sql_value(value: &SqlValue) -> Option<Self> {
        i64::from_sql_value(value).map(|integer| integer != 0)
    }
}

/// A real column, or an integer one, which SQLite converts the same way.
impl FromSqlValue for f64 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "SQLite itself reads an integer column as a floating-point number this way"
    )]
    fn from_sql_value(value: &SqlValue) -> Option<Self> {
        match value {
            SqlValue::Real(real) => Some(*real),
            SqlValue::Integer(integer) => Some(*integer as Self),
            _other => None,
        }
    }
}

impl FromSqlValue for String {
    fn from_sql_value(value: &SqlValue) -> Option<Self> {
        match value {
            SqlValue::Text(text) => Some(text.clone()),
            _other => None,
        }
    }
}

impl FromSqlValue for Vec<u8> {
    fn from_sql_value(value: &SqlValue) -> Option<Self> {
        match value {
            SqlValue::Blob(bytes) => Some(bytes.clone()),
            _other => None,
        }
    }
}

/// `NULL` reads as `None`, and any other value as the inner type reads it.
impl<T: FromSqlValue> FromSqlValue for Option<T> {
    fn from_sql_value(value: &SqlValue) -> Option<Self> {
        match value {
            SqlValue::Null => Some(None),
            other => T::from_sql_value(other).map(Some),
        }
    }
}

impl FromSqlValue for SqlValue {
    fn from_sql_value(value: &SqlValue) -> Option<Self> {
        Some(value.clone())
    }
}

/// One row that a query returned.
#[derive(Debug, Clone, PartialEq)]
pub struct Row(Vec<SqlValue>);

impl Row {
    /// A row of these values, in the order of the query's columns.
    pub fn new(values: Vec<SqlValue>) -> Self {
        Self(values)
    }

    /// The value of a column, by its position from 0.
    pub fn get<T: FromSqlValue>(&self, column: usize) -> Result<T, StoreError> {
        self.0
            .get(column)
            .and_then(T::from_sql_value)
            .ok_or(StoreError::InvalidValue { column })
    }

    /// Every value of the row.
    pub fn values(&self) -> &[SqlValue] {
        &self.0
    }
}

/// A listed statement with its parameters, which bind `?1`, `?2` and on in
/// the order that [`Statement::bind`] adds them.
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    sql: &'static str,
    params: Vec<SqlValue>,
}

impl Statement {
    /// The statement `sql`, which must be one that the module lists.
    pub fn new(sql: &'static str) -> Self {
        Self {
            sql,
            params: Vec::new(),
        }
    }

    /// Binds the next parameter.
    #[must_use]
    pub fn bind(mut self, value: impl Into<SqlValue>) -> Self {
        self.params.push(value.into());
        self
    }

    /// The statement's text.
    pub fn sql(&self) -> &'static str {
        self.sql
    }

    /// Its parameters, in order.
    pub fn params(&self) -> &[SqlValue] {
        &self.params
    }
}

/// How many rows a statement of a batch must change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// Any number, such as 0 for a delete of a row that is gone.
    Any,
    /// Exactly this number; any other rolls the batch back with
    /// [`StoreError::Expectation`].
    Changes(u64),
}

/// A constraint of a plugin's tables, as SQLite reports its failure. SQLite
/// names a key by its table and columns whatever the constraint's name, so a
/// plugin names a CHECK that it maps, and a failed foreign key carries no
/// detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Constraint {
    /// A UNIQUE or PRIMARY KEY, by its table and its columns in the key's
    /// order.
    Key {
        /// The table.
        table: &'static str,
        /// The key's columns.
        columns: &'static [&'static str],
    },
    /// A unique index on an expression, by its name.
    UniqueIndex {
        /// The index's name.
        name: &'static str,
    },
    /// A NOT NULL column.
    NotNull {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
    },
    /// A value of another type in a column of a STRICT table.
    Datatype {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
    },
    /// A CHECK, by its name.
    Check {
        /// The constraint's name.
        name: &'static str,
    },
    /// Any foreign key, which SQLite reports with no detail.
    ForeignKey,
}

/// Statements that run in one transaction on the store's writer, with the
/// codes that the plugin gives the failures of its constraints.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WriteBatch {
    statements: Vec<(Statement, Expect)>,
    constraints: Vec<(Constraint, &'static str)>,
}

impl WriteBatch {
    /// An empty batch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a statement with the changes that it must make.
    #[must_use]
    pub fn push(mut self, statement: Statement, expect: Expect) -> Self {
        self.statements.push((statement, expect));
        self
    }

    /// Gives a failure of `constraint` the plugin's `code`, which the batch
    /// returns as [`StoreError::Constraint`].
    #[must_use]
    pub fn on_constraint(mut self, constraint: Constraint, code: &'static str) -> Self {
        self.constraints.push((constraint, code));
        self
    }

    /// The statements, in order.
    pub fn statements(&self) -> &[(Statement, Expect)] {
        &self.statements
    }

    /// The constraints with their codes.
    pub fn constraints(&self) -> &[(Constraint, &'static str)] {
        &self.constraints
    }

    /// Whether the batch holds no statement.
    pub fn is_empty(&self) -> bool {
        self.statements.is_empty()
    }
}

/// How durable a batch must be when it returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durability {
    /// Kept across a crash of Mannie, maybe not across a loss of power.
    Normal,
    /// Kept across a loss of power, for writes that the owner would miss.
    Full,
}

/// The queries of a plugin's module, which read its tables.
pub trait ModuleQueries: Send + Sync + fmt::Debug {
    /// Runs one of the module's listed statements that only reads.
    fn query(&self, statement: Statement) -> BoxFuture<'static, Result<Vec<Row>, StoreError>>;
}

/// A plugin's module of the store, bound to the plugin.
pub trait ModuleStoreApi: ModuleQueries {
    /// Runs a batch in one transaction on the store's writer. Returns the
    /// number of rows that each statement changed. A failure rolls the whole
    /// batch back.
    fn write(
        &self,
        batch: WriteBatch,
        durability: Durability,
    ) -> BoxFuture<'static, Result<Vec<u64>, StoreError>>;
}

/// Why the store did not do what a plugin asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// The store is closing or could not be used.
    #[error("the store is not available")]
    Unavailable,
    /// A constraint that the batch mapped failed, with the plugin's code.
    #[error("a constraint of the plugin failed: {code}")]
    Constraint {
        /// The plugin's code of the constraint.
        code: &'static str,
    },
    /// A statement of the batch changed another number of rows than it
    /// expected.
    #[error("statement {index} of the batch changed another number of rows")]
    Expectation {
        /// The statement's position in the batch, from 0.
        index: usize,
    },
    /// The text is not one of the module's listed statements.
    #[error("the statement is not one that the module lists")]
    UnknownStatement,
    /// A query that would write, or another use that the store refuses.
    #[error("the store does not permit the statement here")]
    NotPermitted,
    /// A column holds a value of another kind than the plugin read.
    #[error("column {column} holds a value of another kind")]
    InvalidValue {
        /// The column's position from 0.
        column: usize,
    },
    /// The store failed, such as an unmapped constraint or a full disk.
    #[error("the store failed")]
    Failed,
}

impl StoreError {
    /// The error's stable code, which a plugin logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unavailable => "store.unavailable",
            Self::Constraint { .. } => "store.constraint",
            Self::Expectation { .. } => "store.expectation",
            Self::UnknownStatement => "store.unknown_statement",
            Self::NotPermitted => "store.not_permitted",
            Self::InvalidValue { .. } => "store.invalid_value",
            Self::Failed => "store.failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Constraint, Expect, FromSqlValue, Migration, Row, SqlValue, Statement, StoreError,
        StoreModule, WriteBatch,
    };

    const MIGRATIONS: &[Migration] = &[
        Migration::legacy(
            1,
            19,
            "create the notes",
            "CREATE TABLE notes_items (id TEXT) STRICT",
        ),
        Migration::new(
            2,
            "add a title",
            "ALTER TABLE notes_items ADD COLUMN title TEXT",
        ),
    ];
    const STATEMENTS: &[&str] = &["SELECT id FROM notes_items"];
    // The module is a constant, as a plugin declares it.
    const NOTES: StoreModule = StoreModule::new("notes", MIGRATIONS, STATEMENTS);

    #[test]
    fn a_module_and_its_migrations_keep_what_the_plugin_declared() {
        assert_eq!(NOTES.name, "notes");
        assert_eq!(NOTES.statements, STATEMENTS);
        let [first, second] = NOTES.migrations else {
            panic!("two migrations");
        };
        assert_eq!(
            (
                first.version(),
                first.legacy_version(),
                first.description(),
                first.sql()
            ),
            (
                1,
                Some(19),
                "create the notes",
                "CREATE TABLE notes_items (id TEXT) STRICT"
            )
        );
        assert_eq!((second.version(), second.legacy_version()), (2, None));
    }

    #[test]
    fn values_read_as_the_types_that_hold_them_and_refuse_the_others() {
        assert_eq!(i64::from_sql_value(&SqlValue::Integer(-3)), Some(-3));
        assert_eq!(u32::from_sql_value(&SqlValue::Integer(7)), Some(7));
        assert_eq!(u32::from_sql_value(&SqlValue::Integer(-1)), None);
        assert_eq!(
            u32::from_sql_value(&SqlValue::Integer(i64::from(u32::MAX) + 1)),
            None
        );
        assert_eq!(bool::from_sql_value(&SqlValue::Integer(2)), Some(true));
        assert_eq!(bool::from_sql_value(&SqlValue::Integer(0)), Some(false));
        assert_eq!(f64::from_sql_value(&SqlValue::Integer(2)), Some(2.0));
        assert_eq!(
            String::from_sql_value(&SqlValue::Text("a".to_owned())),
            Some("a".to_owned())
        );
        assert_eq!(i64::from_sql_value(&SqlValue::Text("1".to_owned())), None);
        assert_eq!(String::from_sql_value(&SqlValue::Null), None);
        assert_eq!(
            Option::<String>::from_sql_value(&SqlValue::Null),
            Some(None)
        );
        assert_eq!(
            Option::<i64>::from_sql_value(&SqlValue::Integer(4)),
            Some(Some(4))
        );
        assert_eq!(Option::<i64>::from_sql_value(&SqlValue::Real(1.5)), None);
        assert_eq!(
            Vec::<u8>::from_sql_value(&SqlValue::Blob(vec![1, 2])),
            Some(vec![1, 2])
        );
    }

    #[test]
    fn a_row_names_the_column_that_holds_another_kind() {
        let row = Row::new(vec![SqlValue::Text("w1".to_owned()), SqlValue::Null]);

        assert_eq!(row.get::<String>(0), Ok("w1".to_owned()));
        assert_eq!(row.get::<Option<i64>>(1), Ok(None));
        assert_eq!(
            row.get::<i64>(0),
            Err(StoreError::InvalidValue { column: 0 })
        );
        assert_eq!(
            row.get::<i64>(2),
            Err(StoreError::InvalidValue { column: 2 })
        );
        assert_eq!(row.values().len(), 2);
    }

    #[test]
    fn a_statement_binds_its_parameters_in_order_and_a_batch_keeps_its_parts() {
        let statement = Statement::new("INSERT INTO notes_items (id, title) VALUES (?1, ?2)")
            .bind("n1")
            .bind(None::<String>);
        let batch = WriteBatch::new()
            .push(statement.clone(), Expect::Changes(1))
            .on_constraint(
                Constraint::Key {
                    table: "notes_items",
                    columns: &["id"],
                },
                "note_exists",
            );

        assert_eq!(
            statement.params(),
            [SqlValue::Text("n1".to_owned()), SqlValue::Null]
        );
        assert!(WriteBatch::new().is_empty());
        assert!(!batch.is_empty());
        assert_eq!(batch.statements(), [(statement, Expect::Changes(1))]);
        assert_eq!(batch.constraints().len(), 1);
    }

    #[test]
    fn each_error_has_a_code_of_its_own() {
        let codes = [
            StoreError::Unavailable,
            StoreError::Constraint { code: "x" },
            StoreError::Expectation { index: 0 },
            StoreError::UnknownStatement,
            StoreError::NotPermitted,
            StoreError::InvalidValue { column: 0 },
            StoreError::Failed,
        ]
        .map(|error| error.code());
        let mut unique = codes.to_vec();
        unique.sort_unstable();
        unique.dedup();

        assert_eq!(unique.len(), codes.len());
        assert!(codes.iter().all(|code| code.starts_with("store.")));
    }
}
