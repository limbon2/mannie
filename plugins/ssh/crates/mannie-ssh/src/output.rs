//! The output of a command on a host, captured and rendered as `run_command`
//! does it (docs/features/system-tools/commands.md, output): each stream
//! keeps its first 256 KiB and its last 768 KiB with the number of bytes
//! left out, bytes that are not UTF-8 are replaced, CRLF reads as LF, and a
//! stream that does not fit the result is archived as it was captured for
//! `read_tool_output`, with its start and its end in the result.
//!
//! The host's password is replaced wherever it shows in the output, when it
//! has at least 8 bytes, the length from which the log redactor replaces
//! secrets: a shorter one could match ordinary text.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::time::Duration;

use mannie_plugin_sdk::contracts::ArchiveId;
use mannie_plugin_sdk::points::tools::{ToolArchive, ToolResult};

use crate::commands::MIN_REDACTED_PASSWORD_BYTES;

/// The first bytes of each stream that a capture keeps.
const HEAD_BYTES: usize = 256 * 1024;

/// The last bytes of each stream that a capture keeps.
const TAIL_BYTES: usize = 768 * 1024;

/// The part of a stream's room that its start gets when it does not fit.
const HEAD_SHARE: usize = 4;

/// The room that the lines around the streams take.
const FRAME_BYTES: usize = 1024;

/// What replaces the host's password in the output, as in Mannie's logs.
pub(crate) const REDACTED: &str = "[REDACTED]";

/// The start and the end of a stream, as it arrives.
#[derive(Debug, Default)]
pub(crate) struct Capture {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total: u64,
}

impl Capture {
    /// Keeps what fits of `bytes`.
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.total = self
            .total
            .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        let to_head = HEAD_BYTES.saturating_sub(self.head.len()).min(bytes.len());
        self.head.extend_from_slice(&bytes[..to_head]);
        self.tail.extend(&bytes[to_head..]);
        let excess = self.tail.len().saturating_sub(TAIL_BYTES);
        self.tail.drain(..excess);
    }

    /// The stream as text, with a mark where bytes were left out.
    pub(crate) fn finish(&self) -> Captured {
        let kept = self.head.len() + self.tail.len();
        let left_out = self
            .total
            .saturating_sub(u64::try_from(kept).unwrap_or(u64::MAX));
        let tail: Vec<u8> = self.tail.iter().copied().collect();
        if left_out == 0 {
            let whole = [self.head.as_slice(), tail.as_slice()].concat();
            let text = String::from_utf8_lossy(&whole);
            return Captured {
                replaced: matches!(text, Cow::Owned(_)),
                text: text.replace("\r\n", "\n"),
                left_out,
                total: self.total,
            };
        }
        // The cut can split a character: its pieces are dropped, not
        // replaced.
        let head_text = String::from_utf8_lossy(complete_start(&self.head));
        let tail_text = String::from_utf8_lossy(complete_end(&tail));
        let replaced = matches!(head_text, Cow::Owned(_)) || matches!(tail_text, Cow::Owned(_));
        Captured {
            text: format!("{head_text}\n[{left_out} bytes left out]\n{tail_text}")
                .replace("\r\n", "\n"),
            left_out,
            total: self.total,
            replaced,
        }
    }
}

/// The bytes before a character that the end of `bytes` cuts.
pub(crate) fn complete_start(bytes: &[u8]) -> &[u8] {
    match std::str::from_utf8(bytes) {
        Err(error) if error.error_len().is_none() => &bytes[..error.valid_up_to()],
        _whole => bytes,
    }
}

/// The bytes after a character that the start of `bytes` cuts: up to three
/// continuation bytes.
fn complete_end(bytes: &[u8]) -> &[u8] {
    let skip = bytes
        .iter()
        .take(3)
        .take_while(|byte| (**byte & 0b1100_0000) == 0b1000_0000)
        .count();
    &bytes[skip..]
}

/// What a stream produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Captured {
    /// The text, with a mark where bytes were left out.
    pub(crate) text: String,
    /// The bytes between the start and the end that the capture did not
    /// keep.
    pub(crate) left_out: u64,
    /// The size of the whole stream in bytes.
    pub(crate) total: u64,
    /// Whether bytes that are not valid UTF-8 were replaced.
    pub(crate) replaced: bool,
}

impl Captured {
    /// The same output, with every occurrence of `password` replaced, when
    /// the password is long enough to tell apart from ordinary text.
    pub(crate) fn redacted(mut self, password: &str) -> Self {
        let is_long_enough = password.len() >= MIN_REDACTED_PASSWORD_BYTES as usize;
        if is_long_enough && self.text.contains(password) {
            self.text = self.text.replace(password, REDACTED);
        }
        self
    }
}

/// How a command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ending {
    /// It exited with this status.
    Exited(u32),
    /// A signal ended it.
    Signaled {
        /// The signal's name, such as `TERM`.
        signal: String,
        /// Whether the host wrote a core dump.
        core_dumped: bool,
    },
    /// Its time ran out, and the plugin asked the host to end it.
    TimedOut,
    /// The host closed the channel without saying how the command ended.
    Unknown,
}

/// What a finished command produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Finished {
    pub(crate) ending: Ending,
    pub(crate) duration: Duration,
    pub(crate) stdout: Captured,
    pub(crate) stderr: Captured,
}

/// The rendering's limits: the room that the model reads, and the IDs that
/// the core reserved for archives.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Room<'a> {
    pub(crate) result_limit: usize,
    pub(crate) archive_ids: &'a [ArchiveId],
}

/// The result of a command: where it ran, how it ended, how long it ran,
/// and its output within the result limit.
pub(crate) fn render(
    ran_on: &str,
    timeout_seconds: u32,
    finished: &Finished,
    room: Room<'_>,
) -> ToolResult {
    let ending = match &finished.ending {
        Ending::Exited(code) => format!("Exit code: {code}\n"),
        Ending::Signaled {
            signal,
            core_dumped,
        } => {
            let dumped = if *core_dumped {
                " The host wrote a core dump."
            } else {
                ""
            };
            format!("Ended by the signal {signal}.{dumped}\n")
        }
        Ending::TimedOut => format!(
            "Timed out after {timeout_seconds} s: Mannie asked the host to end the command and closed its channel. Processes that it started may still run on the host.\n"
        ),
        Ending::Unknown => "The host closed the channel without an exit status.\n".to_owned(),
    };
    let header = format!(
        "{ran_on}\n{ending}Ran for {:.1} s.\n",
        finished.duration.as_secs_f64()
    );
    let budget = room.result_limit.saturating_sub(header.len() + FRAME_BYTES);
    let (stdout_room, stderr_room) = share(
        budget,
        finished.stdout.text.len(),
        finished.stderr.text.len(),
    );
    let mut ids = room.archive_ids.iter();
    let mut archives = Vec::new();
    let stdout = stream(
        ("stdout", "Standard output"),
        &finished.stdout,
        stdout_room,
        &mut ids,
        &mut archives,
    );
    let stderr = stream(
        ("stderr", "Standard error"),
        &finished.stderr,
        stderr_room,
        &mut ids,
        &mut archives,
    );
    let replaced = if finished.stdout.replaced || finished.stderr.replaced {
        "Bytes that are not valid UTF-8 were replaced with U+FFFD.\n"
    } else {
        ""
    };
    ToolResult::new(format!("{header}{stdout}{stderr}{replaced}")).with_archives(archives)
}

/// How the room is shared between the two streams: a stream that needs less
/// than half keeps all that it needs, and the other gets the rest.
fn share(budget: usize, stdout: usize, stderr: usize) -> (usize, usize) {
    if stdout + stderr <= budget {
        return (stdout, stderr);
    }
    let half = budget / 2;
    match (stdout <= half, stderr <= half) {
        (true, _) => (stdout, budget - stdout),
        (_, true) => (budget - stderr, stderr),
        _neither => (half, budget - half),
    }
}

/// One stream's part of the result: the whole stream when it fits, else its
/// start and its end, with the stream archived as it was captured under the
/// next reserved ID.
fn stream<'a>(
    (name, title): (&str, &str),
    captured: &Captured,
    room: usize,
    ids: &mut impl Iterator<Item = &'a ArchiveId>,
    archives: &mut Vec<ToolArchive>,
) -> String {
    if captured.text.is_empty() {
        return format!("{title}: none.\n");
    }
    let mut part = format!("{title} ({} bytes):\n", captured.total);
    if captured.text.len() <= room {
        part.push_str(&captured.text);
    } else {
        let cut = Cut::of(&captured.text, room);
        part.push_str(cut.head);
        // A start that ends inside a line still puts the note on a line of
        // its own.
        if !cut.head.is_empty() && !cut.head.ends_with('\n') {
            part.push('\n');
        }
        let note = match ids.next() {
            Some(id) => {
                archives.push(ToolArchive::new(id.clone(), name, captured.text.clone()));
                left_out_note(&cut, captured.left_out, id)
            }
            None => format!("[{}]", cut.left_out()),
        };
        let _written = writeln!(part, "{note}");
        part.push_str(cut.tail);
    }
    if !part.ends_with('\n') {
        part.push('\n');
    }
    part
}

/// The note between the start and the end of a stream: what the result
/// leaves out, and where the archive is. The archive lacks the middle of a
/// stream that was longer than the capture keeps, and the note says so.
fn left_out_note(cut: &Cut<'_>, not_captured: u64, id: &ArchiveId) -> String {
    let archived = if not_captured == 0 {
        "the whole output is archived".to_owned()
    } else {
        format!(
            "the output is archived without the {not_captured} bytes in its middle that Mannie did not keep"
        )
    };
    format!(
        "[{}: {archived}; read it with read_tool_output, output_id {id}, offset {}]",
        cut.left_out(),
        cut.first
    )
}

/// The start and the end of a text that does not fit its room, and what the
/// result leaves out between them.
#[derive(Debug)]
struct Cut<'a> {
    head: &'a str,
    tail: &'a str,
    /// The first line left out, in whole or in part, counting from 1.
    first: usize,
    /// The last line left out, in whole or in part.
    last: usize,
    /// The bytes between the start and the end.
    bytes: usize,
    /// Whether the start or the end stops inside a line, which then shows in
    /// part.
    splits_a_line: bool,
}

impl<'a> Cut<'a> {
    /// The start and the end of `text`, which is longer than `room` bytes,
    /// cut at line breaks. A first or a last line that alone is too long for
    /// its share is cut at a character, so the start shows its beginning and
    /// the end shows its end.
    fn of(text: &'a str, room: usize) -> Self {
        let head_room = room / HEAD_SHARE;
        let tail_room = room - head_room;
        let mut head_end = 0;
        for line in text.split_inclusive('\n') {
            if head_end + line.len() > head_room {
                break;
            }
            head_end += line.len();
        }
        if head_end == 0 {
            head_end = text.floor_char_boundary(head_room);
        }
        let mut tail_start = text.len();
        for line in text[head_end..].split_inclusive('\n').rev() {
            if text.len() - tail_start + line.len() > tail_room {
                break;
            }
            tail_start -= line.len();
        }
        if tail_start == text.len() {
            tail_start =
                text.ceil_char_boundary(text.len().saturating_sub(tail_room).max(head_end));
        }
        let head = &text[..head_end];
        let tail = &text[tail_start..];
        let before_tail = &text[..tail_start];
        let tail_starts_a_line = before_tail.ends_with('\n');
        let head_splits = !head.is_empty() && !head.ends_with('\n');
        let tail_splits = !tail.is_empty() && !tail_starts_a_line;
        let first = head.matches('\n').count() + 1;
        // The last line left out holds the byte just before the end: a line
        // break there ends the line before the end's first line.
        let last = before_tail.matches('\n').count() + usize::from(!tail_starts_a_line);
        Self {
            head,
            tail,
            first,
            last: last.max(first),
            bytes: tail_start.saturating_sub(head_end),
            splits_a_line: head_splits || tail_splits,
        }
    }

    /// What the note says is left out: whole lines, or the bytes of the lines
    /// that the start or the end cuts.
    fn left_out(&self) -> String {
        match (self.splits_a_line, self.first == self.last) {
            (false, true) => format!("line {} left out", self.first),
            (false, false) => format!("lines {} to {} left out", self.first, self.last),
            (true, true) => format!("{} bytes of line {} left out", self.bytes, self.first),
            (true, false) => format!(
                "{} bytes left out, from line {} to line {}",
                self.bytes, self.first, self.last
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use mannie_plugin_sdk::contracts::ArchiveId;

    use super::{
        Capture, Captured, Cut, Ending, Finished, HEAD_BYTES, REDACTED, Room, TAIL_BYTES, render,
        share,
    };

    fn captured(text: &str) -> Captured {
        let mut capture = Capture::default();
        capture.push(text.as_bytes());
        capture.finish()
    }

    fn finished(stdout: &str, stderr: &str, ending: Ending) -> Finished {
        Finished {
            ending,
            duration: Duration::from_millis(1_250),
            stdout: captured(stdout),
            stderr: captured(stderr),
        }
    }

    fn archive_ids() -> Vec<ArchiveId> {
        (1..=4)
            .map(|index| {
                ArchiveId::parse(&format!("0199e0f2-0000-7000-8000-00000000000{index}")).unwrap()
            })
            .collect()
    }

    #[test]
    fn a_stream_keeps_its_start_and_its_end_and_reads_crlf_as_lf() {
        let mut capture = Capture::default();
        capture.push(b"first\r\n");
        capture.push(&vec![b'x'; HEAD_BYTES + TAIL_BYTES]);
        capture.push(b"\r\nlast\r\n");

        let captured = capture.finish();

        assert!(captured.text.starts_with("first\nxxx"));
        assert!(captured.text.ends_with("xxx\nlast\n"));
        // The 15 bytes of the two short lines are what does not fit.
        assert!(captured.text.contains("\n[15 bytes left out]\n"));
        assert_eq!(captured.left_out, 15);
        assert_eq!(captured.total, (HEAD_BYTES + TAIL_BYTES + 15) as u64);
        assert!(!captured.replaced);
    }

    #[test]
    fn bytes_that_are_not_utf8_are_replaced_and_said_so() {
        let mut capture = Capture::default();
        capture.push(&[b'o', b'k', 0xff, b'\n']);

        let captured = capture.finish();

        assert_eq!(captured.text, "ok\u{fffd}\n");
        assert!(captured.replaced);
    }

    #[test]
    fn the_password_is_replaced_when_it_has_at_least_eight_bytes() {
        let long = captured("login as deploy with correct horse, then correct horse again\n")
            .redacted("correct horse");
        let short = captured("the pin is 1234\n").redacted("1234");

        assert_eq!(
            long.text,
            format!("login as deploy with {REDACTED}, then {REDACTED} again\n")
        );
        assert_eq!(short.text, "the pin is 1234\n");
    }

    #[test]
    fn a_result_says_where_and_how_the_command_ran() {
        let room = Room {
            result_limit: 64 * 1024,
            archive_ids: &archive_ids(),
        };
        let ran_on = "Ran on build-box (deploy@build-box.lan:22).";

        let exited = render(
            ran_on,
            60,
            &finished("Linux box\n", "", Ending::Exited(0)),
            room,
        );
        let signaled = render(
            ran_on,
            60,
            &finished(
                "",
                "",
                Ending::Signaled {
                    signal: "KILL".to_owned(),
                    core_dumped: false,
                },
            ),
            room,
        );
        let timed_out = render(ran_on, 5, &finished("", "", Ending::TimedOut), room);

        assert_eq!(
            exited.output,
            "Ran on build-box (deploy@build-box.lan:22).\nExit code: 0\nRan for 1.2 s.\nStandard output (10 bytes):\nLinux box\nStandard error: none.\n"
        );
        assert_eq!(exited.archives, Vec::new());
        assert!(signaled.output.contains("Ended by the signal KILL."));
        assert!(timed_out.output.contains("Timed out after 5 s"));
        assert!(timed_out.output.contains("may still run on the host"));
    }

    #[test]
    fn a_stream_that_does_not_fit_is_archived_whole_under_a_reserved_id() {
        let ids = archive_ids();
        let mut lines = String::new();
        for line in 1..=2_000 {
            lines.push_str("line ");
            lines.push_str(&line.to_string());
            lines.push('\n');
        }
        let room = Room {
            result_limit: 4 * 1024,
            archive_ids: &ids,
        };

        let result = render(
            "Ran on box.",
            60,
            &finished(&lines, "warning\n", Ending::Exited(1)),
            room,
        );

        assert_eq!(result.archives.len(), 1);
        assert_eq!(result.archives[0].id, ids[0]);
        assert_eq!(result.archives[0].name, "stdout");
        assert_eq!(result.archives[0].content, lines);
        assert!(
            result.output.contains(&format!("output_id {}", ids[0])),
            "{}",
            result.output
        );
        assert!(result.output.contains("line 1\n"));
        assert!(result.output.contains("line 2000\n"));
        assert!(
            result
                .output
                .contains("Standard error (8 bytes):\nwarning\n")
        );
        assert!(result.output.len() <= 4 * 1024);
    }

    #[test]
    fn a_single_long_line_shows_its_beginning_and_its_end_within_the_limit() {
        let ids = archive_ids();
        let room = Room {
            result_limit: 64 * 1024,
            archive_ids: &ids,
        };
        let line = "x".repeat(300_011);

        let result = render(
            "Ran on box.",
            60,
            &finished(&line, "", Ending::Exited(0)),
            room,
        );

        assert!(
            result.output.len() <= 64 * 1024,
            "{} bytes",
            result.output.len()
        );
        assert!(
            result
                .output
                .contains("Standard output (300011 bytes):\nxxx")
        );
        assert!(
            result
                .output
                .contains(" bytes of line 1 left out: the whole output is archived;"),
            "{}",
            &result.output[..300]
        );
        assert!(result.output.contains("xxx\nStandard error: none.\n"));
        assert_eq!(result.archives[0].content, line);
    }

    #[test]
    fn an_output_longer_than_the_capture_is_not_called_whole() {
        let ids = archive_ids();
        let room = Room {
            result_limit: 64 * 1024,
            archive_ids: &ids,
        };
        // 150,000 lines of 46 bytes, of which the capture keeps 1 MiB.
        let mut capture = Capture::default();
        for number in 1..=150_000 {
            capture
                .push(format!("line {number:>6} of the stress test output, padded\n").as_bytes());
        }
        let finished = Finished {
            ending: Ending::Exited(0),
            duration: Duration::from_secs(1),
            stdout: capture.finish(),
            stderr: Captured::default(),
        };

        let result = render("Ran on box.", 60, &finished, room);

        assert!(
            result.output.contains(
                "the output is archived without the 5851424 bytes in its middle that Mannie did not keep"
            ),
            "{}",
            result.output.lines().find(|line| line.starts_with('[')).unwrap_or("")
        );
        assert!(!result.output.contains("whole"));
        assert!(
            result.archives[0]
                .content
                .contains("\n[5851424 bytes left out]\n")
        );
    }

    #[test]
    fn a_cut_through_characters_keeps_them_whole() {
        // Each character has 2 bytes, and 102 bytes of room give the start
        // 25 and the end 77, so both cuts fall inside a character.
        let text = "é".repeat(500);

        let cut = Cut::of(&text, 102);

        assert_eq!((cut.head.len(), cut.tail.len()), (24, 76));
        assert_eq!(cut.left_out(), "900 bytes of line 1 left out");
    }

    #[test]
    fn streams_share_the_room() {
        assert_eq!(share(100, 30, 40), (30, 40));
        assert_eq!(share(100, 10, 400), (10, 90));
        assert_eq!(share(100, 400, 10), (90, 10));
        assert_eq!(share(100, 400, 400), (50, 50));
    }
}
