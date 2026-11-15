//! Bounded, historical previews of file writes (system-tools/tools-and-results.md).

use std::time::Duration;

use mannie_types::tools::{FileChange, FileChangeHunk, FileChangeLine, FileChangeLineKind};
use serde::Serialize;
use similar::{Algorithm, ChangeTag, TextDiff};

use crate::results::count;

const MAX_PREVIEW_BYTES: usize = 64 * 1024;
const MAX_PREVIEW_LINES: usize = 500;

/// Compares the normalized text that the file tools actually wrote. The
/// deadline may coarsen the diff, but its operations still describe the
/// whole transformation. Totals come from those same operations, before
/// the preview is shortened. This runs on the file tool's blocking thread.
pub(crate) fn compare(old: &str, new: &str) -> FileChange {
    // A bare carriage return is content, as it is for read_file. Keep LF
    // attached so a missing final newline still counts as a change.
    let old_lines: Vec<_> = old.split_inclusive('\n').collect();
    let new_lines: Vec<_> = new.split_inclusive('\n').collect();
    let diff = TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .timeout(Duration::from_millis(200))
        .diff_slices(&old_lines, &new_lines);
    let mut result = FileChange {
        added: 0,
        removed: 0,
        hunks: Vec::new(),
        truncated: false,
    };
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => result.added += 1,
            ChangeTag::Delete => result.removed += 1,
            ChangeTag::Equal => {}
        }
    }
    // Account for JSON escaping, field names and separators, not just source
    // bytes. Changing false to true only makes the encoding shorter.
    let mut bytes = encoded_bytes(&result);
    let mut lines = 0;
    'hunks: for ops in diff.grouped_ops(3) {
        let (Some(first), Some(last)) = (ops.first(), ops.last()) else {
            continue;
        };
        let old_range = first.old_range().start..last.old_range().end;
        let new_range = first.new_range().start..last.new_range().end;
        let hunk = FileChangeHunk {
            old_start: count(old_range.start + usize::from(!old_range.is_empty())),
            old_lines: count(old_range.len()),
            new_start: count(new_range.start + usize::from(!new_range.is_empty())),
            new_lines: count(new_range.len()),
            lines: Vec::new(),
        };
        let header_bytes = encoded_bytes(&hunk) + usize::from(!result.hunks.is_empty());
        if bytes.saturating_add(header_bytes) > MAX_PREVIEW_BYTES {
            result.truncated = true;
            break;
        }
        bytes += header_bytes;
        result.hunks.push(hunk);
        let hunk = result.hunks.last_mut().expect("the hunk was just pushed");
        for op in &ops {
            for change in diff.iter_changes(op) {
                if lines == MAX_PREVIEW_LINES {
                    result.truncated = true;
                    break 'hunks;
                }
                let value = change.value();
                let text = value.strip_suffix('\n').unwrap_or(value);
                let end = text.floor_char_boundary(MAX_PREVIEW_BYTES.min(text.len()));
                let mut line = FileChangeLine {
                    kind: match change.tag() {
                        ChangeTag::Insert => FileChangeLineKind::Added,
                        ChangeTag::Delete => FileChangeLineKind::Removed,
                        ChangeTag::Equal => FileChangeLineKind::Context,
                    },
                    old: change.old_index().map(|index| count(index + 1)),
                    new: change.new_index().map(|index| count(index + 1)),
                    text: text[..end].to_owned(),
                    no_newline: !value.ends_with('\n'),
                    text_truncated: end < text.len(),
                };
                let separator = usize::from(!hunk.lines.is_empty());
                let available = MAX_PREVIEW_BYTES.saturating_sub(bytes + separator);
                let mut line_bytes = encoded_bytes(&line);
                while line_bytes > available && !line.text.is_empty() {
                    let end = line.text.floor_char_boundary(line.text.len() / 2);
                    line.text.truncate(end);
                    line.text_truncated = true;
                    line_bytes = encoded_bytes(&line);
                }
                if line_bytes > available {
                    result.truncated = true;
                    break 'hunks;
                }
                result.truncated |= line.text_truncated;
                bytes += line_bytes + separator;
                lines += 1;
                hunk.lines.push(line);
            }
        }
    }
    result.hunks.retain(|hunk| !hunk.lines.is_empty());
    result
}

/// These types contain only strings, integers and lists. If serialization
/// ever becomes fallible, omit the preview rather than exceed its limit.
fn encoded_bytes(value: &impl Serialize) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use mannie_types::tools::FileChangeLineKind::{Added, Context, Removed};

    use super::{MAX_PREVIEW_BYTES, MAX_PREVIEW_LINES, compare};

    #[test]
    fn counts_changed_lines_and_keeps_both_line_numbers() {
        let change = compare("first\nold\nlast\n", "first\nnew\nextra\nlast\n");
        assert_eq!((change.added, change.removed), (2, 1));
        let hunk = &change.hunks[0];
        assert_eq!((hunk.old_start, hunk.old_lines), (1, 3));
        assert_eq!((hunk.new_start, hunk.new_lines), (1, 4));
        let lines: Vec<_> = hunk
            .lines
            .iter()
            .map(|line| (line.kind, line.old, line.new, line.text.as_str()))
            .collect();
        assert_eq!(
            lines,
            vec![
                (Context, Some(1), Some(1), "first"),
                (Removed, Some(2), None, "old"),
                (Added, None, Some(2), "new"),
                (Added, None, Some(3), "extra"),
                (Context, Some(3), Some(4), "last"),
            ]
        );
        assert!(!change.truncated);
    }

    #[test]
    fn empty_new_unchanged_and_removed_files() {
        for text in ["", "one\n", "one\ntwo", "\n\n", "ä😀\n"] {
            let unchanged = compare(text, text);
            assert_eq!((unchanged.added, unchanged.removed), (0, 0));
            assert_eq!(
                unchanged.hunks,
                Vec::<mannie_types::tools::FileChangeHunk>::new()
            );
            let created = compare("", text);
            let removed = compare(text, "");
            let lines = u32::try_from(text.lines().count()).unwrap();
            assert_eq!((created.added, created.removed), (lines, 0));
            assert_eq!((removed.added, removed.removed), (0, lines));
            if let Some(hunk) = created.hunks.first() {
                assert_eq!((hunk.old_start, hunk.old_lines, hunk.new_start), (0, 0, 1));
            }
        }
    }

    #[test]
    fn final_newline_is_part_of_the_comparison() {
        let change = compare("one\n", "one");
        assert_eq!((change.added, change.removed), (1, 1));
        assert!(!change.hunks[0].lines[0].no_newline);
        assert!(change.hunks[0].lines[1].no_newline);
    }

    #[test]
    fn bare_carriage_returns_stay_on_their_line() {
        let change = compare("first\nlast\n", "first\rnew\nlast\n");
        assert_eq!((change.added, change.removed), (1, 1));
        let lines = &change.hunks[0].lines;
        assert_eq!(lines[1].text, "first\rnew");
        assert!(!lines[1].no_newline);
        assert_eq!((lines[2].old, lines[2].new), (Some(2), Some(2)));
    }

    #[test]
    fn distant_changes_have_three_context_lines_and_correct_offsets() {
        let mut old = String::new();
        for number in 1..=30 {
            writeln!(old, "{number}").unwrap();
        }
        let new = old.replace("2\n", "two\n");
        let change = compare(&old, &new);
        assert_eq!((change.added, change.removed), (3, 3));
        assert_eq!(change.hunks.len(), 3);
        assert_eq!(change.hunks[1].old_start, 9);
        assert_eq!(change.hunks[1].old_lines, 7);
    }

    #[test]
    fn repeated_lines_still_describe_the_transformation() {
        let change = compare("a\na\nb\na\n", "a\nb\nb\na\n");
        assert_eq!((change.added, change.removed), (1, 1));
        let mut old = String::new();
        let mut new = String::new();
        for line in &change.hunks[0].lines {
            if line.kind != Added {
                writeln!(old, "{}", line.text).unwrap();
            }
            if line.kind != Removed {
                writeln!(new, "{}", line.text).unwrap();
            }
        }
        assert_eq!(old, "a\na\nb\na\n");
        assert_eq!(new, "a\nb\nb\na\n");
    }

    #[test]
    fn preview_limits_do_not_limit_totals() {
        let change = compare("", &"new\n".repeat(800));
        assert_eq!((change.added, change.removed), (800, 0));
        assert!(change.truncated);
        assert_eq!(change.hunks[0].lines.len(), MAX_PREVIEW_LINES);
        assert!(serde_json::to_vec(&change).unwrap().len() <= MAX_PREVIEW_BYTES);
    }

    #[test]
    fn escaped_text_and_long_unicode_lines_fit_the_serialized_byte_limit() {
        let text = format!("{}\nsecond\n", "😀\"\t\\".repeat(30_000));
        let change = compare("", &text);
        assert_eq!(change.added, 2);
        assert!(change.truncated);
        assert!(change.hunks[0].lines[0].text_truncated);
        assert!(serde_json::to_vec(&change).unwrap().len() <= MAX_PREVIEW_BYTES);
    }
}
