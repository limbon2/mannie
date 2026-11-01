//! Browsing a table of a registered database in pages that follow keys
//! (ADR-0056, decision 4): by rowid, or by the primary key of a table
//! `WITHOUT ROWID`. Each page is one statement that the engine reads and
//! finishes in the same call, so no statement stays open between pages and
//! the owner's applications keep their locks (spike S4). A view has no key,
//! so its pages follow an offset. Sorting by a column and filters become SQL
//! with quoted names and parameters, never text of the request.

use std::path::Path;

use super::engine::{self, Cancel, Column, EngineError, Limits, Mode, Opened, quote};
use super::structure::{TableInfo, TableKind};
use super::values::Value;

/// The most rows of a page.
pub const MAX_PAGE_ROWS: usize = 500;

/// How a filter compares a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOperator {
    /// `=`.
    Equal,
    /// `<>`.
    NotEqual,
    /// `<`.
    Less,
    /// `<=`.
    LessOrEqual,
    /// `>`.
    Greater,
    /// `>=`.
    GreaterOrEqual,
    /// `LIKE`, with `%` and `_` as SQLite reads them.
    Like,
    /// `IS NULL`, which takes no value.
    IsNull,
    /// `IS NOT NULL`, which takes no value.
    IsNotNull,
}

/// A condition on one column.
#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    /// The column.
    pub column: String,
    /// How it compares.
    pub operator: FilterOperator,
    /// The value that it compares with, which `IsNull` and `IsNotNull`
    /// ignore.
    pub value: Value,
}

/// The column that a page sorts by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Order {
    /// The column.
    pub column: String,
    /// Whether the largest values come first.
    pub descending: bool,
}

/// Where the next page starts: after the last row of the page before.
#[derive(Debug, Clone, PartialEq)]
pub enum Cursor {
    /// After the row with this key and, when the page sorts by a column,
    /// this value of the column.
    After {
        /// The value of the column that the pages sort by.
        order: Option<Value>,
        /// The row's key, in the order of the key's columns.
        key: Vec<Value>,
    },
    /// After this many rows of a view.
    Offset(u64),
}

/// What a page asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct PageRequest {
    /// The table or view.
    pub table: String,
    /// The column that the pages sort by, or `None` for the key's order.
    pub order: Option<Order>,
    /// The conditions that every row meets.
    pub filters: Vec<Filter>,
    /// Where the page starts, or `None` for the first page.
    pub after: Option<Cursor>,
    /// The most rows of the page, at most [`MAX_PAGE_ROWS`].
    pub rows: usize,
}

/// One row of a page.
#[derive(Debug, Clone, PartialEq)]
pub struct PageRow {
    /// The row's key, in the order of the page's key, which names the row
    /// for an edit or a deletion. A view's rows have none.
    pub key: Vec<Value>,
    /// The row's values, one for each column of the page.
    pub values: Vec<Value>,
}

/// A page of a table.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    /// The columns of the key: `rowid`, a primary key's columns, or none
    /// for a view.
    pub key: Vec<String>,
    /// The columns of the rows.
    pub columns: Vec<Column>,
    /// The rows.
    pub rows: Vec<PageRow>,
    /// Where the next page starts, or `None` after the last page.
    pub next: Option<Cursor>,
}

/// The key that names a table's rows: a name of the rowid that no column
/// takes, the primary key of a table `WITHOUT ROWID`, or none for a view.
pub(super) fn key_of(table: &TableInfo) -> Result<Vec<String>, EngineError> {
    if table.kind == TableKind::View {
        return Ok(Vec::new());
    }
    if table.without_rowid {
        let mut primary: Vec<_> = table
            .columns
            .iter()
            .filter(|column| column.primary_key > 0)
            .collect();
        primary.sort_by_key(|column| column.primary_key);
        return Ok(primary.iter().map(|column| column.name.clone()).collect());
    }
    ["rowid", "_rowid_", "oid"]
        .into_iter()
        .find(|name| {
            !table
                .columns
                .iter()
                .any(|column| column.name.eq_ignore_ascii_case(name))
        })
        .map(|name| vec![name.to_owned()])
        .ok_or(EngineError::NoKey)
}

/// The SQL of one filter, with its parameter if it takes one.
fn filter_sql(filter: &Filter, parameters: &mut Vec<Value>) -> String {
    let column = quote(&filter.column);
    let operator = match filter.operator {
        FilterOperator::IsNull => return format!("{column} IS NULL"),
        FilterOperator::IsNotNull => return format!("{column} IS NOT NULL"),
        FilterOperator::Equal => "=",
        FilterOperator::NotEqual => "<>",
        FilterOperator::Less => "<",
        FilterOperator::LessOrEqual => "<=",
        FilterOperator::Greater => ">",
        FilterOperator::GreaterOrEqual => ">=",
        FilterOperator::Like => "LIKE",
    };
    parameters.push(filter.value.clone());
    format!("{column} {operator} ?{}", parameters.len())
}

/// The condition that a row comes after the cursor's key: a comparison of
/// row values, since a key's columns are never `NULL`.
fn after_key(key: &[String], values: &[Value], parameters: &mut Vec<Value>) -> String {
    let columns: Vec<String> = key.iter().map(|name| quote(name)).collect();
    let mut places = Vec::with_capacity(values.len());
    for value in values {
        parameters.push(value.clone());
        places.push(format!("?{}", parameters.len()));
    }
    format!("({}) > ({})", columns.join(", "), places.join(", "))
}

/// The condition that a row comes after the cursor in the order of a
/// column. SQLite sorts `NULL` first in ascending order and last in
/// descending order, which comparisons with `NULL` never match, so the
/// condition names `NULL` itself.
fn after_order(
    order: &Order,
    value: &Value,
    key: &[String],
    key_values: &[Value],
    parameters: &mut Vec<Value>,
) -> String {
    let column = quote(&order.column);
    let is_null = *value == Value::Null;
    let comparison = if order.descending { "<" } else { ">" };
    if is_null {
        let same = after_key(key, key_values, parameters);
        return if order.descending {
            format!("({column} IS NULL AND {same})")
        } else {
            format!("(({column} IS NULL AND {same}) OR {column} IS NOT NULL)")
        };
    }
    parameters.push(value.clone());
    let place = parameters.len();
    let same = after_key(key, key_values, parameters);
    let beyond = format!("{column} {comparison} ?{place} OR ({column} = ?{place} AND {same})");
    if order.descending {
        format!("({beyond} OR {column} IS NULL)")
    } else {
        format!("({beyond})")
    }
}

/// The statement of a page and its parameters.
fn page_sql(request: &PageRequest, key: &[String], rows: usize) -> (String, Vec<Value>) {
    let mut parameters = Vec::new();
    let mut conditions: Vec<String> = request
        .filters
        .iter()
        .map(|filter| filter_sql(filter, &mut parameters))
        .collect();
    let mut offset = None;
    match &request.after {
        Some(Cursor::After {
            order: Some(value),
            key: key_values,
        }) if !key.is_empty() => {
            if let Some(order) = &request.order {
                conditions.push(after_order(order, value, key, key_values, &mut parameters));
            }
        }
        Some(Cursor::After {
            key: key_values, ..
        }) if !key.is_empty() => {
            conditions.push(after_key(key, key_values, &mut parameters));
        }
        Some(Cursor::Offset(rows)) => offset = Some(*rows),
        _none => {}
    }
    let key_columns: Vec<String> = key.iter().map(|name| quote(name)).collect();
    let mut order_by: Vec<String> = Vec::new();
    if let Some(order) = &request.order {
        let direction = if order.descending { "DESC" } else { "ASC" };
        order_by.push(format!("{} {direction}", quote(&order.column)));
    }
    order_by.extend(key_columns.iter().cloned());
    let selected = if key_columns.is_empty() {
        "*".to_owned()
    } else {
        format!("{}, *", key_columns.join(", "))
    };
    let mut parts = vec![format!("SELECT {selected} FROM {}", quote(&request.table))];
    if !conditions.is_empty() {
        parts.push(format!("WHERE {}", conditions.join(" AND ")));
    }
    if !order_by.is_empty() {
        parts.push(format!("ORDER BY {}", order_by.join(", ")));
    }
    // One row more than the page shows tells whether another page follows.
    parts.push(format!("LIMIT {}", rows + 1));
    if let Some(offset) = offset {
        parts.push(format!("OFFSET {offset}"));
    }
    (parts.join(" "), parameters)
}

/// Reads one page of a table of a registered database.
pub fn page(
    path: &Path,
    identity: &str,
    table: &TableInfo,
    request: &PageRequest,
    limits: Limits,
    cancel: &Cancel,
) -> Result<Page, EngineError> {
    let key = key_of(table)?;
    let rows = request.rows.clamp(1, MAX_PAGE_ROWS).min(limits.rows);
    let (text, parameters) = page_sql(request, &key, rows);
    let mut opened = Opened::open(path, identity, Mode::Read, limits.time, cancel)?;
    opened.confine()?;
    let (mut statement, _class) = opened.prepare(&text)?;
    engine::bind(&mut statement, &parameters)?;
    let page_limits = Limits {
        rows: rows + 1,
        ..limits
    };
    let read = engine::collect(&opened, &mut statement, page_limits)?;
    let mut columns = read.columns;
    let key_width = key.len();
    columns.drain(..key_width.min(columns.len()));
    let more = read.rows.len() > rows || read.cut.is_some();
    let page_rows: Vec<PageRow> = read
        .rows
        .into_iter()
        .take(rows)
        .map(|mut values| {
            let key_values: Vec<Value> = values.drain(..key_width.min(values.len())).collect();
            PageRow {
                key: key_values,
                values,
            }
        })
        .collect();
    let next = if more {
        next_cursor(request, &key, &columns, &page_rows)
    } else {
        None
    };
    Ok(Page {
        key,
        columns,
        rows: page_rows,
        next,
    })
}

/// Where the page after these rows starts.
fn next_cursor(
    request: &PageRequest,
    key: &[String],
    columns: &[Column],
    rows: &[PageRow],
) -> Option<Cursor> {
    let last = rows.last()?;
    if key.is_empty() {
        let before = match &request.after {
            Some(Cursor::Offset(offset)) => *offset,
            _first => 0,
        };
        return Some(Cursor::Offset(before + rows.len() as u64));
    }
    let order = request.order.as_ref().and_then(|order| {
        columns
            .iter()
            .position(|column| column.name == order.column)
            .and_then(|index| last.values.get(index).cloned())
    });
    Some(Cursor::After {
        order,
        key: last.key.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::{Filter, FilterOperator, Order, PageRequest, page_sql};
    use crate::dbfiles::values::Value;

    #[test]
    fn a_page_quotes_names_and_passes_values_as_parameters() {
        let request = PageRequest {
            table: "items\" WHERE 1; --".to_owned(),
            order: None,
            filters: vec![Filter {
                column: "na\"me".to_owned(),
                operator: FilterOperator::Like,
                value: Value::Text("%'; DROP TABLE items; --".to_owned()),
            }],
            after: None,
            rows: 2,
        };

        let (text, parameters) = page_sql(&request, &["rowid".to_owned()], 2);

        assert_eq!(
            text,
            "SELECT \"rowid\", * FROM \"items\"\" WHERE 1; --\" WHERE \"na\"\"me\" LIKE ?1 ORDER BY \"rowid\" LIMIT 3"
        );
        assert_eq!(
            parameters,
            [Value::Text("%'; DROP TABLE items; --".to_owned())]
        );
    }

    #[test]
    fn a_sorted_page_continues_after_the_last_row_also_through_null() {
        let request = |value: Value, descending: bool| PageRequest {
            table: "items".to_owned(),
            order: Some(Order {
                column: "name".to_owned(),
                descending,
            }),
            filters: Vec::new(),
            after: Some(super::Cursor::After {
                order: Some(value),
                key: vec![Value::Integer(7)],
            }),
            rows: 10,
        };
        let key = ["rowid".to_owned()];

        let (ascending, _) = page_sql(&request(Value::Text("b".to_owned()), false), &key, 10);
        let (from_null, _) = page_sql(&request(Value::Null, false), &key, 10);
        let (descending, _) = page_sql(&request(Value::Text("b".to_owned()), true), &key, 10);

        assert!(
            ascending.contains("WHERE (\"name\" > ?1 OR (\"name\" = ?1 AND (\"rowid\") > (?2)))"),
            "{ascending}"
        );
        assert!(
            from_null
                .contains("((\"name\" IS NULL AND (\"rowid\") > (?1)) OR \"name\" IS NOT NULL)"),
            "{from_null}"
        );
        assert!(
            descending.contains("OR \"name\" IS NULL)") && descending.contains("\"name\" DESC"),
            "{descending}"
        );
    }
}
