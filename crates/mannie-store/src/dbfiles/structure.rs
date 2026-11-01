//! The structure of a registered database: its tables, views and virtual
//! tables with their columns and foreign keys, its indexes and its triggers
//! (ADR-0056, decision 4). The engine reads each in one statement under the
//! authorizer, through the pragmas' table-valued forms, which read only, so
//! a schema of thousands of tables reads in tens of milliseconds (spike S9).

use std::collections::BTreeMap;
use std::path::Path;

use super::engine::{Cancel, EngineError, Mode, Opened};

/// What kind of table an object of the structure is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableKind {
    /// An ordinary table.
    Table,
    /// A view.
    View,
    /// A virtual table, such as an FTS5 index.
    Virtual,
    /// A table that a virtual table keeps its data in.
    Shadow,
}

/// A column of a table or a view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnInfo {
    /// The column's name.
    pub name: String,
    /// Its declared type, or an empty text for none.
    pub declared_type: String,
    /// Whether it is `NOT NULL`.
    pub not_null: bool,
    /// Its default as SQL, if it has one.
    pub default: Option<String>,
    /// Its place in the primary key from 1, or 0 for a column outside it.
    pub primary_key: u32,
    /// Whether it is hidden or generated, which an insert leaves out.
    pub hidden: bool,
}

/// A foreign key of a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignKeyInfo {
    /// The table's columns, in order.
    pub columns: Vec<String>,
    /// The table that it refers to.
    pub parent: String,
    /// The parent's columns, in order, or `None` for the parent's primary
    /// key.
    pub parent_columns: Vec<Option<String>>,
    /// What an update of the parent does, such as `CASCADE`.
    pub on_update: String,
    /// What a deletion of the parent does.
    pub on_delete: String,
}

/// A table, a view or a virtual table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableInfo {
    /// Its name.
    pub name: String,
    /// What kind of table it is.
    pub kind: TableKind,
    /// The statement that created it.
    pub sql: Option<String>,
    /// Its columns, in order.
    pub columns: Vec<ColumnInfo>,
    /// Whether it is a table `WITHOUT ROWID`.
    pub without_rowid: bool,
    /// Whether it is a `STRICT` table.
    pub strict: bool,
    /// Its foreign keys.
    pub foreign_keys: Vec<ForeignKeyInfo>,
}

/// An index of a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexInfo {
    /// Its name.
    pub name: String,
    /// Its table.
    pub table: String,
    /// Whether it is unique.
    pub unique: bool,
    /// Whether it came from a `PRIMARY KEY` or `UNIQUE` constraint, which
    /// has no statement of its own.
    pub automatic: bool,
    /// Whether it is partial, with a `WHERE` clause.
    pub partial: bool,
    /// Its columns, in order, or `None` for an expression.
    pub columns: Vec<Option<String>>,
    /// The statement that created it.
    pub sql: Option<String>,
}

/// A trigger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerInfo {
    /// Its name.
    pub name: String,
    /// The table or view that it belongs to.
    pub table: String,
    /// The statement that created it.
    pub sql: Option<String>,
}

/// The structure of a database.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Structure {
    /// The tables, views and virtual tables, by name.
    pub tables: Vec<TableInfo>,
    /// The indexes, by table and name.
    pub indexes: Vec<IndexInfo>,
    /// The triggers, by name.
    pub triggers: Vec<TriggerInfo>,
}

impl Structure {
    /// The `CREATE` statements of the structure, in an order that recreates
    /// it: tables and views by name, then indexes, then triggers.
    pub fn create_statements(&self) -> Vec<String> {
        let tables = self.tables.iter().filter_map(|table| table.sql.clone());
        let indexes = self.indexes.iter().filter_map(|index| index.sql.clone());
        let triggers = self
            .triggers
            .iter()
            .filter_map(|trigger| trigger.sql.clone());
        tables.chain(indexes).chain(triggers).collect()
    }
}

/// The tables of the main schema, without SQLite's own `sqlite_*` tables.
const TABLES: &str = "SELECT l.name, l.type, l.wr, l.strict, s.sql \
     FROM pragma_table_list AS l LEFT JOIN sqlite_schema AS s ON s.name = l.name \
     WHERE l.schema = 'main' AND l.name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
     ORDER BY l.name";

/// The columns of every table and view.
const COLUMNS: &str = "SELECT s.name, c.name, c.type, c.\"notnull\", c.dflt_value, c.pk, c.hidden \
     FROM sqlite_schema AS s JOIN pragma_table_xinfo(s.name) AS c \
     WHERE s.type IN ('table', 'view') AND s.name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
     ORDER BY s.name, c.cid";

/// The foreign keys of every table.
const FOREIGN_KEYS: &str = "SELECT s.name, f.id, f.\"table\", f.\"from\", f.\"to\", f.on_update, f.on_delete \
     FROM sqlite_schema AS s JOIN pragma_foreign_key_list(s.name) AS f \
     WHERE s.type = 'table' AND s.name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
     ORDER BY s.name, f.id, f.seq";

/// The indexes of every table, with their columns.
const INDEXES: &str = "SELECT s.name, l.name, l.\"unique\", l.origin, l.partial, i.name, x.sql \
     FROM sqlite_schema AS s JOIN pragma_index_list(s.name) AS l \
     JOIN pragma_index_info(l.name) AS i \
     LEFT JOIN sqlite_schema AS x ON x.type = 'index' AND x.name = l.name \
     WHERE s.type = 'table' AND s.name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
     ORDER BY s.name, l.name, i.seqno";

/// The triggers.
const TRIGGERS: &str =
    "SELECT name, tbl_name, sql FROM sqlite_schema WHERE type = 'trigger' ORDER BY name";

/// Reads every row of one statement of the structure.
fn rows<T>(
    opened: &Opened,
    text: &str,
    mut row: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>, EngineError> {
    let (mut statement, _class) = opened.prepare(text)?;
    let mut stepped = statement.raw_query();
    let mut read = Vec::new();
    while let Some(next) = stepped.next().map_err(|error| opened.error(error))? {
        read.push(row(next).map_err(|error| opened.error(error))?);
    }
    Ok(read)
}

/// Reads the tables, views and virtual tables, without their columns and
/// foreign keys.
fn read_tables(opened: &Opened) -> Result<Vec<TableInfo>, EngineError> {
    rows(opened, TABLES, |row| {
        let kind = match row.get::<_, String>(1)?.as_str() {
            "view" => TableKind::View,
            "virtual" => TableKind::Virtual,
            "shadow" => TableKind::Shadow,
            _table => TableKind::Table,
        };
        Ok(TableInfo {
            name: row.get(0)?,
            kind,
            sql: row.get(4)?,
            columns: Vec::new(),
            without_rowid: row.get::<_, i64>(2)? != 0,
            strict: row.get::<_, i64>(3)? != 0,
            foreign_keys: Vec::new(),
        })
    })
}

/// Adds the columns of every table and view.
fn add_columns(
    opened: &Opened,
    tables: &mut [TableInfo],
    by_name: &BTreeMap<String, usize>,
) -> Result<(), EngineError> {
    let columns = rows(opened, COLUMNS, |row| {
        Ok((
            row.get::<_, String>(0)?,
            ColumnInfo {
                name: row.get(1)?,
                declared_type: row.get(2)?,
                not_null: row.get::<_, i64>(3)? != 0,
                default: row.get(4)?,
                primary_key: u32::try_from(row.get::<_, i64>(5)?).unwrap_or(0),
                hidden: row.get::<_, i64>(6)? != 0,
            },
        ))
    })?;
    for (table, column) in columns {
        if let Some(&index) = by_name.get(&table) {
            tables[index].columns.push(column);
        }
    }
    Ok(())
}

/// Adds the foreign keys of every table, each with its columns in order.
fn add_foreign_keys(
    opened: &Opened,
    tables: &mut [TableInfo],
    by_name: &BTreeMap<String, usize>,
) -> Result<(), EngineError> {
    let keys = rows(opened, FOREIGN_KEYS, |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;
    let mut last: Option<(String, i64)> = None;
    for (table, id, parent, from, to, on_update, on_delete) in keys {
        let Some(&index) = by_name.get(&table) else {
            continue;
        };
        let same = last.as_ref() == Some(&(table.clone(), id));
        let foreign_keys = &mut tables[index].foreign_keys;
        match foreign_keys.last_mut() {
            Some(key) if same => {
                key.columns.push(from);
                key.parent_columns.push(to);
            }
            _new => foreign_keys.push(ForeignKeyInfo {
                columns: vec![from],
                parent,
                parent_columns: vec![to],
                on_update,
                on_delete,
            }),
        }
        last = Some((table, id));
    }
    Ok(())
}

/// Reads the indexes of every table, each with its columns in order.
fn read_indexes(opened: &Opened) -> Result<Vec<IndexInfo>, EngineError> {
    let listed = rows(opened, INDEXES, |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)? != 0,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)? != 0,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
        ))
    })?;
    let mut indexes: Vec<IndexInfo> = Vec::new();
    for (table, name, unique, origin, partial, column, sql) in listed {
        let same = indexes
            .last()
            .is_some_and(|index| index.table == table && index.name == name);
        if same {
            if let Some(index) = indexes.last_mut() {
                index.columns.push(column);
            }
            continue;
        }
        indexes.push(IndexInfo {
            name,
            table,
            unique,
            automatic: origin != "c",
            partial,
            columns: vec![column],
            sql,
        });
    }
    Ok(indexes)
}

/// Reads the structure of a registered database.
pub fn structure(
    path: &Path,
    identity: &str,
    time: std::time::Duration,
    cancel: &Cancel,
) -> Result<Structure, EngineError> {
    let mut opened = Opened::open(path, identity, Mode::Read, time, cancel)?;
    opened.confine()?;
    let mut tables = read_tables(&opened)?;
    let by_name: BTreeMap<String, usize> = tables
        .iter()
        .enumerate()
        .map(|(index, table)| (table.name.clone(), index))
        .collect();
    add_columns(&opened, &mut tables, &by_name)?;
    add_foreign_keys(&opened, &mut tables, &by_name)?;
    let indexes = read_indexes(&opened)?;
    let triggers = rows(&opened, TRIGGERS, |row| {
        Ok(TriggerInfo {
            name: row.get(0)?,
            table: row.get(1)?,
            sql: row.get(2)?,
        })
    })?;
    Ok(Structure {
        tables,
        indexes,
        triggers,
    })
}
