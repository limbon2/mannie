//! Command blocks (plugins/terminal/docs/README.md, the command list): each
//! command that ran in a terminal, who ran it, its exit code, its times and
//! the rows of its output, between the marker that started it and the next
//! prompt.
//!
//! A block's text is its rows joined into lines where they wrapped. Rows that
//! the emulator no longer has leave a gap that the text names, and a text
//! keeps its first 256 KiB and its last 768 KiB.

use mannie_plugin_sdk::contracts::UtcMillis;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::emulator::{HEAD_BYTES, Position, RowText};

/// The most blocks that a terminal keeps; the oldest go first.
pub(crate) const MAX_BLOCKS: usize = 200;

/// The most bytes of a block's text that come from its end.
const TAIL_BYTES: usize = 768 * 1024;

/// Who ran a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Author {
    /// The owner typed it.
    Owner,
    /// An agent typed it with `run_in_terminal`.
    Agent,
}

/// A command that ran in a terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Block {
    /// The block's number in its terminal, from 1.
    pub(crate) number: u32,
    /// Who ran it.
    pub(crate) author: Author,
    /// The command line, as the shell or the screen gave it.
    pub(crate) command: String,
    /// Where its output starts.
    pub(crate) start: Position,
    /// Where the next prompt started, once the command ended.
    pub(crate) end: Option<Position>,
    /// The exit code, when the command ended and the shell gave one.
    pub(crate) exit_code: Option<i32>,
    /// When it started.
    pub(crate) started_at: UtcMillis,
    /// When it ended.
    pub(crate) ended_at: Option<UtcMillis>,
    /// The folder it ran in, as the shell reported it last.
    pub(crate) folder: Option<String>,
}

impl Block {
    /// The rows of the block's output, from its first to before `to`: the
    /// row of its end when that row holds output, the one after it when the
    /// end is at the row's start.
    pub(crate) fn rows(&self, cursor: Position) -> (u64, u64) {
        let end = self.end.unwrap_or(cursor);
        let to = if end.column == 0 {
            end.row
        } else {
            end.row + 1
        };
        (self.start.row, to.max(self.start.row))
    }

    /// How long it ran, in milliseconds, once it ended.
    pub(crate) fn duration_ms(&self) -> Option<u64> {
        let ended = self.ended_at?;
        u64::try_from(ended.as_millis() - self.started_at.as_millis()).ok()
    }
}

/// A block's output as lines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct BlockText {
    /// The lines, numbered from 1 by their places.
    pub(crate) lines: Vec<String>,
    /// How many rows the emulator no longer had, or that the size left out.
    pub(crate) omitted: u64,
}

impl BlockText {
    /// The lines as one text.
    pub(crate) fn joined(&self) -> String {
        self.lines.join("\n")
    }
}

/// The text of a block's rows: `rows` from its first row, the columns before
/// `start_column` of the first row and from `end_column` of the last left
/// out, rows that `head` kept where the emulator lost them, wrapped rows
/// joined, trailing blank lines dropped, and the size caps applied.
pub(crate) fn text_of(
    rows: Vec<Option<RowText>>,
    first_row: u64,
    start_column: u16,
    end: Option<Position>,
    head: &[(u64, RowText)],
) -> BlockText {
    let mut lines: Vec<String> = Vec::new();
    let mut omitted = 0;
    let mut current: Option<String> = None;
    let mut in_gap = false;
    let last_row = first_row + rows.len() as u64;
    for (offset, row) in rows.into_iter().enumerate() {
        let number = first_row + offset as u64;
        let row = row.or_else(|| {
            head.iter()
                .find(|(kept, _text)| *kept == number)
                .map(|(_kept, text)| text.clone())
        });
        let Some(row) = row else {
            if let Some(line) = current.take() {
                lines.push(line);
            }
            if !in_gap {
                lines.push(String::from("…"));
                in_gap = true;
            }
            omitted += 1;
            continue;
        };
        in_gap = false;
        let mut text = row.text;
        if number + 1 == last_row
            && let Some(end) = end
            && end.row == number
        {
            text = text.chars().take(usize::from(end.column)).collect();
        }
        if number == first_row && start_column > 0 {
            text = text.chars().skip(usize::from(start_column)).collect();
        }
        let line = current.get_or_insert_with(String::new);
        line.push_str(&text);
        if !row.wrapped {
            lines.push(current.take().unwrap_or_default());
        }
    }
    if let Some(line) = current {
        lines.push(line);
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    capped(BlockText { lines, omitted })
}

/// The text within its caps: its first [`HEAD_BYTES`] and its last
/// [`TAIL_BYTES`], with a line that says how many lines are left out.
fn capped(text: BlockText) -> BlockText {
    let total: usize = text.lines.iter().map(|line| line.len() + 1).sum();
    if total <= HEAD_BYTES + TAIL_BYTES {
        return text;
    }
    let mut head_end = 0;
    let mut bytes = 0;
    while head_end < text.lines.len() && bytes + text.lines[head_end].len() < HEAD_BYTES {
        bytes += text.lines[head_end].len() + 1;
        head_end += 1;
    }
    let mut tail_start = text.lines.len();
    let mut tail_bytes = 0;
    while tail_start > head_end && tail_bytes + text.lines[tail_start - 1].len() < TAIL_BYTES {
        tail_bytes += text.lines[tail_start - 1].len() + 1;
        tail_start -= 1;
    }
    let left_out = (tail_start - head_end) as u64;
    let mut lines: Vec<String> = text.lines[..head_end].to_vec();
    lines.push(String::from("…"));
    lines.extend_from_slice(&text.lines[tail_start..]);
    BlockText {
        lines,
        omitted: text.omitted + left_out,
    }
}

#[cfg(test)]
mod tests {
    use super::{Author, Block, text_of};
    use crate::emulator::{Position, RowText};
    use mannie_plugin_sdk::contracts::UtcMillis;

    fn row(text: &str, wrapped: bool) -> RowText {
        RowText {
            text: text.to_owned(),
            wrapped,
        }
    }

    #[test]
    fn wrapped_rows_join_and_trailing_blanks_go() {
        let rows = vec![
            Some(row("a long li", true)),
            Some(row("ne", false)),
            Some(row("next", false)),
            Some(row("", false)),
        ];

        let text = text_of(rows, 10, 0, None, &[]);

        assert_eq!(text.lines, ["a long line", "next"]);
        assert_eq!(text.omitted, 0);
    }

    #[test]
    fn a_gap_is_named_once_and_the_head_fills_what_it_kept() {
        let rows = vec![None, None, None, Some(row("last", false))];
        let head = vec![(
            5,
            RowText {
                text: "first".to_owned(),
                wrapped: false,
            },
        )];

        let text = text_of(rows, 5, 0, None, &head);

        assert_eq!(text.lines, ["first", "…", "last"]);
        assert_eq!(text.omitted, 2);
    }

    #[test]
    fn the_columns_of_the_ends_are_left_out() {
        let rows = vec![Some(row("$ out", false)), Some(row("tail$ prompt", false))];
        let end = Position { row: 1, column: 4 };

        let text = text_of(rows, 0, 2, Some(end), &[]);

        assert_eq!(text.lines, ["out", "tail"]);
    }

    #[test]
    fn a_large_text_keeps_its_start_and_end() {
        let line = "x".repeat(1000);
        let rows: Vec<_> = (0..2000).map(|_| Some(row(&line, false))).collect();

        let text = text_of(rows, 0, 0, None, &[]);

        assert!(text.omitted > 900);
        assert_eq!(
            text.lines
                .iter()
                .filter(|line| line.as_str() == "…")
                .count(),
            1
        );
        assert!(text.joined().len() <= 1024 * 1024);
    }

    #[test]
    fn a_block_ends_before_a_prompt_at_a_rows_start() {
        let block = Block {
            number: 1,
            author: Author::Owner,
            command: "ls".to_owned(),
            start: Position { row: 4, column: 0 },
            end: Some(Position { row: 9, column: 0 }),
            exit_code: Some(0),
            started_at: UtcMillis::from_millis(1_000),
            ended_at: Some(UtcMillis::from_millis(1_250)),
            folder: None,
        };
        let cursor = Position { row: 12, column: 3 };

        assert_eq!(block.rows(cursor), (4, 9));
        assert_eq!(block.duration_ms(), Some(250));
    }
}
