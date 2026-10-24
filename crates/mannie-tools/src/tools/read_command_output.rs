//! `read_command_output`: the state and the output of a command that runs in
//! the background (ADR-0041, docs/features/system-tools/
//! background-commands.md, reading output). Without `from_line` it returns
//! the lines after the model's last read, and new lines that do not fit keep
//! their start and their end; with `from_line` it reads forward in pages.
//! The command belongs to the same conversation, so reading touches nothing
//! that needs authorization.

use std::fmt::Write as _;
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::tools::{Access, BackgroundSource, ToolFailure, ToolOutcome};
use regex::{Regex, RegexBuilder};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::background::{CommandRead, MAX_WAIT_SECONDS, ReadRequest};
use crate::results::cut_line;
use crate::supervisor::LogLines;
use crate::texts;
use crate::tool::{Done, PermissionClass, PrepareError, ToolContext, TypedExecution, TypedTool};

/// The longest pattern of `until`.
const MAX_PATTERN_BYTES: usize = 1_000;
/// How large a compiled pattern may grow, so a pattern cannot take the
/// machine's memory.
const PATTERN_SIZE_LIMIT: usize = 1 << 20;
/// The most lines of one page from `from_line`, as `read_tool_output` pages.
const PAGE_LINES: usize = 2_000;
/// The bytes of a rendering kept free for its header and its notes.
const FRAME_BYTES: usize = 1_024;
/// The share of the room that the start of new lines gets when they do not
/// fit; the end gets the rest, because errors and summaries come last.
const HEAD_SHARE: usize = 4;
/// The most characters of a command's text that a result's header shows.
const HEADER_COMMAND_CHARS: usize = 200;

/// The arguments of `read_command_output`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// The command's number, as `start_command` returned it.
    command: u32,
    /// The first line to read, counting from 1. The lines after your last
    /// read when null.
    #[serde(default)]
    from_line: Option<u32>,
    /// How many seconds to wait first for the command's end or for a new line
    /// that matches `until`: 0 when null, at most 600.
    #[serde(default)]
    wait_seconds: Option<u32>,
    /// A regular expression: the wait ends at the first new line that matches
    /// it. None when null.
    #[serde(default)]
    until: Option<String>,
}

/// The tool.
pub(crate) struct ReadCommandOutput;

impl TypedTool for ReadCommandOutput {
    type Input = Input;
    const NAME: &'static str = "read_command_output";
    const CLASS: PermissionClass = PermissionClass::ReadOnly;

    fn description() -> &'static str {
        texts::READ_COMMAND_OUTPUT
    }

    /// A background command belongs to the conversation, so reading it
    /// touches nothing that needs authorization.
    fn prepare(_context: &ToolContext, input: &Input) -> Result<Vec<Access>, PrepareError> {
        if input.from_line == Some(0) {
            return Err(PrepareError::invalid("from_line counts from 1"));
        }
        if let Some(until) = &input.until {
            pattern(until)?;
        }
        Ok(Vec::new())
    }

    fn execute(execution: TypedExecution<Input>) -> BoxFuture<'static, Result<Done, ToolFailure>> {
        let TypedExecution {
            context,
            input,
            stop,
            ..
        } = execution;
        async move {
            let seconds = input.wait_seconds.unwrap_or(0).min(MAX_WAIT_SECONDS);
            let until = input.until.as_deref().map(pattern).transpose()?;
            let from_line = input.from_line.map(u64::from);
            let request = ReadRequest {
                conversation: context.conversation_id.clone(),
                number: input.command,
                from_line,
                max_lines: PAGE_LINES,
                wait: Duration::from_secs(u64::from(seconds)),
                until,
            };
            let read = context.background.read(request, stop).await?;
            Ok(render(&context, input.command, from_line, &read))
        }
        .boxed()
    }
}

/// The pattern of `until`, compiled within its limits.
fn pattern(text: &str) -> Result<Regex, ToolFailure> {
    if text.len() > MAX_PATTERN_BYTES {
        return Err(ToolFailure::InvalidPattern {
            reason: format!("the pattern is longer than {MAX_PATTERN_BYTES} bytes"),
        });
    }
    RegexBuilder::new(text)
        .size_limit(PATTERN_SIZE_LIMIT)
        .build()
        .map_err(|error| ToolFailure::InvalidPattern {
            reason: error.to_string(),
        })
}

/// Where a command runs, as its results name it.
pub(crate) fn place(source: &BackgroundSource) -> String {
    match source {
        BackgroundSource::Local {
            working_directory, ..
        } => format!("in {working_directory}"),
        BackgroundSource::Remote {
            label, endpoint, ..
        } => format!("on {label} ({endpoint})"),
    }
}

/// The first line of a command's text, cut for a result's header.
pub(crate) fn header_command(command: &str) -> String {
    let first = command.lines().next().unwrap_or_default();
    let (shown, left_out) = cut_line(first, HEADER_COMMAND_CHARS);
    let more = left_out > 0 || command.lines().nth(1).is_some();
    if more {
        format!("{shown} …")
    } else {
        shown.to_owned()
    }
}

/// The result: the command, its place and its state, then its lines within
/// the result limit.
fn render(context: &ToolContext, number: u32, from_line: Option<u64>, read: &CommandRead) -> Done {
    let remote = matches!(read.view.source, BackgroundSource::Remote { .. });
    let mut header = format!(
        "Command {number}: {}\nIt runs {}.\n{}\n",
        header_command(&read.view.command),
        place(&read.view.source),
        texts::command_state(&read.view, read.ran_for, remote)
    );
    if let Some(matched) = read.matched {
        writeln!(header, "Line {matched} matches until.").expect("writing to a String cannot fail");
    }
    let mut notes = String::new();
    if read.replaced {
        notes.push_str("Bytes that are not valid UTF-8 were replaced with U+FFFD.\n");
    }
    let room = context
        .result_limit
        .saturating_sub(header.len() + notes.len() + FRAME_BYTES);
    let rendered = match from_line {
        None => new_lines(number, &read.lines, room),
        Some(first) => page(number, first, &read.lines, read.total_lines, room),
    };
    let body = match (&rendered.shown, from_line) {
        _ if read.lost => texts::COMMAND_OUTPUT_LOST.to_owned(),
        (Some((first, last)), _) => format!(
            "Lines {first} to {last} of {}:\n{}",
            read.total_lines, rendered.text
        ),
        (None, None) => format!(
            "No new lines since your last read; it wrote {} lines so far.\n{}",
            read.total_lines, rendered.text
        ),
        (None, Some(first)) => format!(
            "No line from line {first} on; it wrote {} lines so far.\n{}",
            read.total_lines, rendered.text
        ),
    };
    let (first_line, last_line) = rendered.shown.unwrap_or((0, 0));
    Done {
        outcome: ToolOutcome::CommandOutputRead {
            number,
            first_line: count(first_line),
            last_line: count(last_line),
            total_lines: count(read.total_lines),
            ending: read.view.ending,
            exit_code: read.view.exit_code,
        },
        output: format!("{header}{body}{notes}"),
        archives: Vec::new(),
        images: Vec::new(),
    }
}

fn count(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Lines as a result shows them, and the first and last line shown.
#[derive(Debug, Default, PartialEq, Eq)]
struct Rendered {
    text: String,
    shown: Option<(u64, u64)>,
}

/// One entry of a read: a line, or the lines that the log let go.
enum Entry<'a> {
    Line(u64, &'a str),
    NotKept(u64, u64),
}

impl Entry<'_> {
    fn text(&self) -> String {
        match self {
            Self::Line(_, text) => (*text).to_owned(),
            Self::NotKept(first, last) => texts::command_lines_not_kept(*first, *last),
        }
    }

    fn size(&self) -> usize {
        self.text().len() + 1
    }
}

/// The entries of a read in order, with the lines that the log let go where
/// they would be.
fn entries(lines: &LogLines) -> Vec<Entry<'_>> {
    let mut entries = Vec::with_capacity(lines.lines.len() + 1);
    let mut gap = lines.left_out;
    for (number, text) in &lines.lines {
        if let Some((first, last)) = gap
            && *number > last
        {
            entries.push(Entry::NotKept(first, last));
            gap = None;
        }
        entries.push(Entry::Line(*number, text));
    }
    if let Some((first, last)) = gap {
        entries.push(Entry::NotKept(first, last));
    }
    entries
}

/// The first and last line among entries.
fn span(entries: &[Entry<'_>]) -> Option<(u64, u64)> {
    let numbers = entries.iter().filter_map(|entry| match entry {
        Entry::Line(number, _) => Some(*number),
        Entry::NotKept(..) => None,
    });
    let mut first = None;
    let mut last = None;
    for number in numbers {
        first.get_or_insert(number);
        last = Some(number);
    }
    first.zip(last)
}

/// An entry within `room` bytes: whole, or a line cut at a character.
fn fitted(entry: &Entry<'_>, room: usize) -> String {
    let text = entry.text();
    if text.len() < room {
        return text;
    }
    let chars = room.saturating_sub(64) / 4;
    let (shown, left_out) = cut_line(&text, chars.max(1));
    format!("{shown}{}", texts::line_cut(left_out))
}

/// The lines after the model's last read. When they do not fit, they keep
/// their start and their end, cut between lines, with a note that names the
/// lines left out and how to read them.
fn new_lines(number: u32, lines: &LogLines, room: usize) -> Rendered {
    let entries = entries(lines);
    if entries.is_empty() {
        return Rendered::default();
    }
    let whole: usize = entries.iter().map(Entry::size).sum();
    if whole <= room {
        let mut text = String::with_capacity(whole);
        for entry in &entries {
            text.push_str(&entry.text());
            text.push('\n');
        }
        return Rendered {
            text,
            shown: span(&entries),
        };
    }
    let head_room = room / HEAD_SHARE;
    let mut head_end = 0;
    let mut used = 0;
    while head_end < entries.len() && used + entries[head_end].size() <= head_room {
        used += entries[head_end].size();
        head_end += 1;
    }
    let tail_room = room - used;
    let mut tail_start = entries.len();
    let mut tail_used = 0;
    while tail_start > head_end && tail_used + entries[tail_start - 1].size() <= tail_room {
        tail_used += entries[tail_start - 1].size();
        tail_start -= 1;
    }
    let mut text = String::new();
    for entry in &entries[..head_end] {
        text.push_str(&entry.text());
        text.push('\n');
    }
    // A first line too long for the start shows its beginning.
    if head_end == 0 && tail_start > 0 {
        text.push_str(&fitted(&entries[0], head_room));
        text.push('\n');
        head_end = 1;
    }
    let omitted = span(&entries[head_end..tail_start.max(head_end)]);
    if let Some((first, last)) = omitted {
        text.push_str(&texts::command_lines_left_out(number, first, last));
        text.push('\n');
    }
    for entry in &entries[tail_start.max(head_end)..] {
        text.push_str(&fitted(entry, tail_room));
        text.push('\n');
    }
    Rendered {
        text,
        shown: span(&entries),
    }
}

/// A page of lines from `first` on, within the room, with where the next page
/// starts when more follows.
fn page(number: u32, first: u64, lines: &LogLines, total: u64, room: usize) -> Rendered {
    let entries = entries(lines);
    let mut text = String::new();
    let mut shown: Vec<&Entry<'_>> = Vec::new();
    for entry in &entries {
        let size = entry.size();
        if !shown.is_empty() && text.len() + size > room {
            break;
        }
        if shown.is_empty() && size > room {
            text.push_str(&fitted(entry, room));
        } else {
            text.push_str(&entry.text());
        }
        text.push('\n');
        shown.push(entry);
    }
    let last_shown = shown
        .last()
        .map_or(first.saturating_sub(1), |entry| match entry {
            Entry::Line(number, _) => *number,
            Entry::NotKept(_, last) => *last,
        });
    if last_shown < total {
        text.push_str(&texts::command_read_on(number, last_shown + 1));
        text.push('\n');
    }
    let first_line = shown.iter().find_map(|entry| match entry {
        Entry::Line(number, _) => Some(*number),
        Entry::NotKept(..) => None,
    });
    let last_line = shown.iter().rev().find_map(|entry| match entry {
        Entry::Line(number, _) => Some(*number),
        Entry::NotKept(..) => None,
    });
    Rendered {
        text,
        shown: first_line.zip(last_line),
    }
}

#[cfg(test)]
mod tests {
    use super::{Rendered, header_command, new_lines, page, pattern};
    use crate::supervisor::LogLines;

    fn lines(range: std::ops::RangeInclusive<u64>) -> LogLines {
        LogLines {
            lines: range
                .map(|number| (number, format!("line {number:04}")))
                .collect(),
            left_out: None,
        }
    }

    #[test]
    fn new_lines_that_fit_come_whole() {
        let rendered = new_lines(3, &lines(5..=7), 1_000);

        assert_eq!(
            rendered,
            Rendered {
                text: "line 0005\nline 0006\nline 0007\n".to_owned(),
                shown: Some((5, 7)),
            }
        );
    }

    #[test]
    fn new_lines_that_do_not_fit_keep_their_start_and_their_end() {
        // Each line takes 10 bytes: a room of 200 keeps 5 at the start and 15
        // at the end, minus the note's room.
        let rendered = new_lines(3, &lines(1..=100), 200);

        assert!(
            rendered.text.starts_with("line 0001\n"),
            "{}",
            rendered.text
        );
        assert!(rendered.text.ends_with("line 0100\n"), "{}", rendered.text);
        assert!(
            rendered
                .text
                .contains("left out of this result: read them with read_command_output, command 3, from_line 6."),
            "{}",
            rendered.text
        );
        assert_eq!(rendered.shown, Some((1, 100)));
    }

    #[test]
    fn lines_that_the_log_let_go_are_named_where_they_would_be() {
        let read = LogLines {
            lines: vec![(1, "first".to_owned()), (900, "last".to_owned())],
            left_out: Some((2, 899)),
        };

        let rendered = new_lines(1, &read, 10_000);

        assert_eq!(
            rendered.text,
            "first\n[Lines 2 to 899 are no longer kept: the log keeps the first 256 KiB and the last 768 KiB of a command's output.]\nlast\n"
        );
    }

    #[test]
    fn a_page_says_where_the_next_one_starts() {
        let rendered = page(4, 10, &lines(10..=60), 200, 100);

        assert_eq!(rendered.shown, Some((10, 19)));
        assert!(
            rendered.text.ends_with(
                "More follows: call read_command_output again with command 4 and from_line 20.\n"
            ),
            "{}",
            rendered.text
        );
    }

    #[test]
    fn the_last_page_says_nothing_more() {
        let rendered = page(4, 10, &lines(10..=12), 12, 1_000);

        assert_eq!(rendered.text, "line 0010\nline 0011\nline 0012\n");
    }

    #[test]
    fn a_header_shows_the_first_line_of_a_long_command() {
        assert_eq!(header_command("npm run dev"), "npm run dev");
        assert_eq!(header_command("cd app\nnpm run dev"), "cd app …");
        assert_eq!(
            header_command(&"x".repeat(300)).chars().count(),
            super::HEADER_COMMAND_CHARS + 2
        );
    }

    #[test]
    fn a_pattern_has_limits() {
        assert!(pattern("ready on \\d+").unwrap().is_match("ready on 5173"));
        pattern("(").unwrap_err();
        pattern(&"a".repeat(1_001)).unwrap_err();
    }
}
