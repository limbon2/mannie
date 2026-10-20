//! A tool's text within the room of its result (plugins/git/docs/tools.md,
//! every tool): the whole text when it fits, else the lines that fit with a
//! note of what is left out, and the whole text archived under one of the IDs
//! that the core reserved, which the model reads on with `read_tool_output`.
//! Tools that can page, such as `git_log` and `git_diff`, also say how to
//! continue with their own arguments.

use std::fmt::Write as _;

use mannie_plugin_sdk::contracts::ArchiveId;
use mannie_plugin_sdk::points::tools::{MAX_TOOL_ARCHIVE_BYTES, ToolArchive, ToolResult};

/// The bytes that the note of a cut text takes at most.
const NOTE_BYTES: usize = 400;

/// The room of a result: the bytes that the model reads, and the IDs that
/// the core reserved for archives.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Room<'a> {
    pub(crate) limit: usize,
    pub(crate) archive_ids: &'a [ArchiveId],
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

/// The result of a tool whose text is `text`, within `room`.
pub(crate) fn fitted(text: String, room: Room<'_>) -> ToolResult {
    if text.len() <= room.limit {
        return ToolResult::new(text);
    }
    let budget = room.limit.saturating_sub(NOTE_BYTES);
    let start = head(&text, budget);
    // The lines that fit whole, so the note follows a complete line.
    let kept = start.rfind('\n').map_or(start, |end| &start[..=end]);
    let first_left_out = kept.lines().count() + 1;
    let left_out = text[kept.len()..].lines().count();
    let mut result = kept.to_owned();
    let Some(id) = room.archive_ids.first() else {
        let _written = writeln!(result, "[{left_out} more lines left out]");
        return ToolResult::new(result);
    };
    let archived = head(&text, MAX_TOOL_ARCHIVE_BYTES);
    let whole = if archived.len() == text.len() {
        "the whole output is archived"
    } else {
        "the output is archived up to 2 MiB"
    };
    let _written = writeln!(
        result,
        "[{left_out} more lines left out: {whole}; read it with read_tool_output, output_id {id}, offset {first_left_out}]"
    );
    ToolResult::new(result).with_archives(vec![ToolArchive::new(
        id.clone(),
        "output",
        archived.to_owned(),
    )])
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use mannie_plugin_sdk::contracts::ArchiveId;

    use super::{Room, fitted, head};

    fn id() -> ArchiveId {
        ArchiveId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap()
    }

    #[test]
    fn a_text_that_fits_stays_whole_without_an_archive() {
        let ids = [id()];

        let result = fitted(
            "one\ntwo\n".to_owned(),
            Room {
                limit: 1000,
                archive_ids: &ids,
            },
        );

        assert_eq!(result.output, "one\ntwo\n");
        assert_eq!(result.archives, Vec::new());
    }

    #[test]
    fn a_long_text_keeps_whole_lines_and_archives_everything() {
        let ids = [id()];
        let text = (1..=200).fold(String::new(), |mut text, number| {
            let _written = writeln!(text, "line {number}");
            text
        });

        let result = fitted(
            text.clone(),
            Room {
                limit: 600,
                archive_ids: &ids,
            },
        );

        assert!(result.output.len() <= 600, "{}", result.output.len());
        assert!(result.output.starts_with("line 1\nline 2\n"));
        assert!(
            result
                .output
                .contains("read it with read_tool_output, output_id 0199e0f2")
        );
        let kept = result.output.lines().count() - 1;
        assert!(result.output.contains(&format!("offset {}", kept + 1)));
        assert_eq!(result.archives.len(), 1);
        assert_eq!(result.archives[0].content, text);
        let none = fitted(
            text,
            Room {
                limit: 600,
                archive_ids: &[],
            },
        );
        assert!(none.output.ends_with("more lines left out]\n"));
    }

    #[test]
    fn a_head_ends_at_a_character() {
        assert_eq!(head("ééé", 3), "é");
        assert_eq!(head("abc", 10), "abc");
    }
}
