//! Archived outputs (docs/features/system-tools/tools-and-results.md, results
//! and limits): the model reads them with `read_tool_output`, and the owner
//! opens them from the tool call, both in pages of lines.

use mannie_store::StoredArchive;
use mannie_types::tools::{MAX_OUTPUT_PAGE_LINES, ToolOutputPage};

use crate::results::count;
use crate::text::lines;

/// A page of an archived output: up to `limit` lines from `first_line`,
/// counting from 1, and at most [`MAX_OUTPUT_PAGE_LINES`].
pub fn output_page(archive: &StoredArchive, first_line: u32, limit: u32) -> ToolOutputPage {
    let all = lines(&archive.content);
    let first_line = first_line.max(1);
    let limit = limit.clamp(1, MAX_OUTPUT_PAGE_LINES);
    let start = usize::try_from(first_line - 1)
        .unwrap_or(usize::MAX)
        .min(all.len());
    let end = start
        .saturating_add(usize::try_from(limit).unwrap_or(usize::MAX))
        .min(all.len());
    ToolOutputPage {
        lines: all[start..end]
            .iter()
            .map(|line| (*line).to_owned())
            .collect(),
        first_line,
        total_lines: count(all.len()),
    }
}

#[cfg(test)]
mod tests {
    use mannie_store::StoredArchive;

    use super::output_page;

    fn archive(content: &str) -> StoredArchive {
        StoredArchive {
            name: "stdout".to_owned(),
            content: content.to_owned(),
            lines: 0,
        }
    }

    #[test]
    fn pages_hold_the_lines_they_name() {
        let output = archive("one\ntwo\nthree\n");

        let page = output_page(&output, 2, 5);

        assert_eq!(page.lines, ["two", "three"]);
        assert_eq!(page.first_line, 2);
        assert_eq!(page.total_lines, 3);
    }

    #[test]
    fn a_page_past_the_end_is_empty() {
        let page = output_page(&archive("one\n"), 10, 5);

        assert_eq!(page.lines, Vec::<String>::new());
        assert_eq!(page.total_lines, 1);
    }
}
