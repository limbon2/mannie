//! What the database tools return (plugins/workers/docs/databases.md,
//! agents): rows as lines of values that SQL would write, a structure as a
//! list of tables, and a text within the room of its result. A text that
//! does not fit keeps its first lines, at most 200 rows, with a note, and
//! the whole text is archived under one of the IDs that the core reserved,
//! which the model reads on with `read_tool_output`.

use std::fmt::Write as _;

use mannie_plugin_sdk::contracts::ArchiveId;
use mannie_plugin_sdk::points::tools::{MAX_TOOL_ARCHIVE_BYTES, ToolArchive, ToolResult};
use mannie_plugin_sdk::services::sqlite_files::{
    DatabaseStructure, RowsCut, SqlValue, StatementRows, TableInfo, TableKind,
};

/// The most rows that the model reads in a result; the archive keeps the
/// rest.
pub(crate) const SHOWN_ROWS: usize = 200;

/// The most characters of a value in what the model reads.
const VALUE_CHARS: usize = 200;

/// The most characters of a value in the archive.
const ARCHIVED_VALUE_CHARS: usize = 4_000;

/// The bytes that the note of a cut text takes at most.
const NOTE_BYTES: usize = 400;

/// The room of a result: the bytes that the model reads, and the IDs that
/// the core reserved for archives.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Room<'a> {
    pub(crate) limit: usize,
    pub(crate) archive_ids: &'a [ArchiveId],
}

/// A value as SQL writes it, cut, on one line: line breaks and tabs show as
/// escapes, so each row stays one line.
fn value_text(value: &SqlValue, max_chars: usize) -> String {
    let shown = value.display(max_chars);
    let mut line = String::with_capacity(shown.len());
    for character in shown.chars() {
        match character {
            '\n' => line.push_str("\\n"),
            '\r' => line.push_str("\\r"),
            '\t' => line.push_str("\\t"),
            other if other.is_control() => line.push('\u{FFFD}'),
            other => line.push(other),
        }
    }
    line
}

/// Rows as text: a line of the columns' names, then a line for each row,
/// with values cut to `max_chars`.
fn rows_text(rows: &StatementRows, max_chars: usize) -> String {
    let mut text = rows
        .columns
        .iter()
        .map(|column| column.name.as_str())
        .collect::<Vec<_>>()
        .join(" | ");
    text.push('\n');
    for row in &rows.rows {
        let values: Vec<String> = row
            .iter()
            .map(|value| value_text(value, max_chars))
            .collect();
        text.push_str(&values.join(" | "));
        text.push('\n');
    }
    text
}

/// The note of rows that the read stopped before.
fn cut_note(cut: Option<RowsCut>) -> &'static str {
    match cut {
        None => "",
        Some(RowsCut::Rows) => {
            "[The statement returned more rows than Mannie reads at once: narrow it with WHERE or LIMIT.]\n"
        }
        Some(RowsCut::Bytes) => {
            "[The statement returned more data than Mannie reads at once: select fewer columns or rows.]\n"
        }
    }
}

/// The result of a read: its rows, at most [`SHOWN_ROWS`] of them within
/// `room`, and every row in the archive.
pub(crate) fn query_result(rows: &StatementRows, room: Room<'_>) -> ToolResult {
    if rows.columns.is_empty() {
        return ToolResult::new("The statement returned no columns.");
    }
    let count = rows.rows.len();
    let summary = match count {
        0 => "No rows.\n".to_owned(),
        1 => "1 row:\n".to_owned(),
        _more => format!("{count} rows:\n"),
    };
    let shown = rows_text(rows, VALUE_CHARS);
    let archived = rows_text(rows, ARCHIVED_VALUE_CHARS);
    let fitted = fitted(&shown, &archived, SHOWN_ROWS + 1, room);
    let mut text = summary;
    text.push_str(&fitted.text);
    text.push_str(cut_note(rows.cut));
    let result = ToolResult::new(text);
    match fitted.archive {
        Some(archive) => result.with_archives(vec![archive]),
        None => result,
    }
}

/// A text that fits its room, and the archive of the whole text when it
/// does not.
pub(crate) struct Fitted {
    pub(crate) text: String,
    pub(crate) archive: Option<ToolArchive>,
}

/// The longest start of `text` that is at most `bytes` long and ends at a
/// character.
fn head(text: &str, bytes: usize) -> &str {
    let mut end = bytes.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// `shown` within `room` and at most `max_lines` lines, whole lines only;
/// when lines are left out, a note says how many and where the archive of
/// `archived`, the same lines with longer values, continues.
pub(crate) fn fitted(shown: &str, archived: &str, max_lines: usize, room: Room<'_>) -> Fitted {
    let total_lines = shown.lines().count();
    let fits = shown.len() <= room.limit && total_lines <= max_lines;
    if fits {
        return Fitted {
            text: shown.to_owned(),
            archive: None,
        };
    }
    let budget = room.limit.saturating_sub(NOTE_BYTES);
    let start = head(shown, budget);
    let within_bytes = start.rfind('\n').map_or("", |end| &start[..=end]);
    let kept_lines = within_bytes.lines().count().min(max_lines);
    let mut kept = String::with_capacity(within_bytes.len());
    for line in shown.lines().take(kept_lines) {
        kept.push_str(line);
        kept.push('\n');
    }
    let left_out = total_lines - kept_lines;
    let mut text = kept;
    let Some(id) = room.archive_ids.first() else {
        let _written = writeln!(text, "[{left_out} more lines left out]");
        return Fitted {
            text,
            archive: None,
        };
    };
    let whole = head(archived, MAX_TOOL_ARCHIVE_BYTES);
    let extent = if whole.len() == archived.len() {
        "the whole result is archived"
    } else {
        "the result is archived up to 2 MiB"
    };
    let _written = writeln!(
        text,
        "[{left_out} more lines left out: {extent}; read it with read_tool_output, output_id {id}, offset {}]",
        kept_lines + 1
    );
    Fitted {
        text,
        archive: Some(ToolArchive::new(id.clone(), "rows", whole.to_owned())),
    }
}

/// What a table's kind is called in a structure.
fn kind_name(kind: TableKind) -> &'static str {
    match kind {
        TableKind::Table => "table",
        TableKind::View => "view",
        TableKind::Virtual => "virtual table",
        TableKind::Shadow => "shadow table",
    }
}

/// One table with its columns and foreign keys, as lines.
fn table_text(table: &TableInfo, text: &mut String) {
    let mut traits = vec![kind_name(table.kind).to_owned()];
    if table.without_rowid {
        traits.push("without rowid".to_owned());
    }
    if table.strict {
        traits.push("strict".to_owned());
    }
    let _written = writeln!(text, "{} ({}):", table.name, traits.join(", "));
    for column in &table.columns {
        let mut line = format!("  {}", column.name);
        if !column.declared_type.is_empty() {
            let _written = write!(line, " {}", column.declared_type);
        }
        if column.primary_key > 0 {
            let _written = write!(line, ", primary key {}", column.primary_key);
        }
        if column.not_null {
            line.push_str(", not null");
        }
        if let Some(default) = &column.default {
            let _written = write!(line, ", default {default}");
        }
        if column.hidden {
            line.push_str(", hidden or generated");
        }
        let _written = writeln!(text, "{line}");
    }
    for key in &table.foreign_keys {
        let parents: Vec<&str> = key
            .parent_columns
            .iter()
            .map(|column| column.as_deref().unwrap_or("its primary key"))
            .collect();
        let _written = writeln!(
            text,
            "  foreign key ({}) references {} ({}), on delete {}",
            key.columns.join(", "),
            key.parent,
            parents.join(", "),
            key.on_delete
        );
    }
}

/// A database's structure as text: its tables and views with their columns
/// and foreign keys, its indexes and its triggers. Shadow tables, which
/// virtual tables keep their data in, are left out.
pub(crate) fn structure_text(structure: &DatabaseStructure) -> String {
    let tables: Vec<&TableInfo> = structure
        .tables
        .iter()
        .filter(|table| table.kind != TableKind::Shadow)
        .collect();
    if tables.is_empty() {
        return "The database has no tables or views.\n".to_owned();
    }
    let mut text = String::new();
    for table in tables {
        table_text(table, &mut text);
    }
    let indexes: Vec<String> = structure
        .indexes
        .iter()
        .filter(|index| !index.automatic)
        .map(|index| {
            let columns: Vec<&str> = index
                .columns
                .iter()
                .map(|column| column.as_deref().unwrap_or("an expression"))
                .collect();
            let unique = if index.unique { "unique " } else { "" };
            format!(
                "  {} on {} ({}), {unique}{}",
                index.name,
                index.table,
                columns.join(", "),
                if index.partial { "partial" } else { "whole" }
            )
        })
        .collect();
    if !indexes.is_empty() {
        let _written = writeln!(text, "Indexes:\n{}", indexes.join("\n"));
    }
    if !structure.triggers.is_empty() {
        let triggers: Vec<String> = structure
            .triggers
            .iter()
            .map(|trigger| format!("  {} on {}", trigger.name, trigger.table))
            .collect();
        let _written = writeln!(text, "Triggers:\n{}", triggers.join("\n"));
    }
    text
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::ArchiveId;
    use mannie_plugin_sdk::services::sqlite_files::{
        DatabaseStructure, ResultColumn, RowsCut, SqlValue, StatementRows, TableColumn, TableInfo,
        TableKind,
    };

    use super::{Room, SHOWN_ROWS, query_result, structure_text};

    fn id() -> ArchiveId {
        ArchiveId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap()
    }

    fn rows(count: usize) -> StatementRows {
        StatementRows {
            columns: vec![
                ResultColumn {
                    name: "name".to_owned(),
                    declared_type: Some("TEXT".to_owned()),
                },
                ResultColumn {
                    name: "stock".to_owned(),
                    declared_type: Some("INTEGER".to_owned()),
                },
            ],
            rows: (0..count)
                .map(|index| {
                    vec![
                        SqlValue::text(format!("part\n{index}")),
                        SqlValue::integer(i64::try_from(index).unwrap()),
                    ]
                })
                .collect(),
            cut: None,
        }
    }

    #[test]
    fn a_small_result_shows_every_row_on_one_line_each_without_an_archive() {
        let ids = [id()];

        let result = query_result(
            &rows(2),
            Room {
                limit: 10_000,
                archive_ids: &ids,
            },
        );

        assert_eq!(
            result.output,
            "2 rows:\nname | stock\n'part\\n0' | 0\n'part\\n1' | 1\n"
        );
        assert_eq!(result.archives, Vec::new());
    }

    #[test]
    fn a_long_result_shows_at_most_its_first_rows_and_archives_every_row() {
        let ids = [id()];

        let result = query_result(
            &rows(500),
            Room {
                limit: 1_000_000,
                archive_ids: &ids,
            },
        );

        let shown_rows = result.output.lines().count();
        // The summary, the columns, the rows and the note.
        assert_eq!(shown_rows, 1 + 1 + SHOWN_ROWS + 1);
        assert!(
            result.output.contains("300 more lines left out"),
            "{}",
            result.output
        );
        assert!(result.output.contains("offset 202"), "{}", result.output);
        let [archive] = &result.archives[..] else {
            panic!("one archive: {:?}", result.archives);
        };
        assert_eq!(archive.content.lines().count(), 501);
    }

    #[test]
    fn a_result_that_the_read_stopped_says_so() {
        let mut stopped = rows(1);
        stopped.cut = Some(RowsCut::Rows);

        let result = query_result(
            &stopped,
            Room {
                limit: 10_000,
                archive_ids: &[],
            },
        );

        assert!(result.output.contains("narrow it with WHERE or LIMIT"));
    }

    #[test]
    fn a_structure_lists_tables_with_columns_and_leaves_out_shadow_tables() {
        let column = |name: &str, primary_key: u32| TableColumn {
            name: name.to_owned(),
            declared_type: "INTEGER".to_owned(),
            not_null: primary_key == 0,
            default: None,
            primary_key,
            hidden: false,
        };
        let table = |name: &str, kind: TableKind| TableInfo {
            name: name.to_owned(),
            kind,
            sql: None,
            columns: vec![column("id", 1), column("stock", 0)],
            without_rowid: false,
            strict: true,
            foreign_keys: Vec::new(),
        };
        let structure = DatabaseStructure {
            tables: vec![
                table("parts", TableKind::Table),
                table("notes_data", TableKind::Shadow),
            ],
            indexes: Vec::new(),
            triggers: Vec::new(),
        };

        let text = structure_text(&structure);

        assert!(text.starts_with("parts (table, strict):\n  id INTEGER, primary key 1\n"));
        assert!(text.contains("  stock INTEGER, not null\n"));
        assert!(!text.contains("notes_data"));
    }
}
