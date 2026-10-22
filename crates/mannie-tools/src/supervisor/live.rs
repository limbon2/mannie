//! The output of a command that runs in the background (ADR-0041,
//! docs/features/system-tools/background-commands.md, the log): both of its
//! streams as one log of lines, in the order the lines arrive. Each stream is
//! kept to whole lines, so the lines of the two never mix. The log keeps its
//! first 256 KiB and its last 768 KiB of lines, the sizes that a stream of
//! `run_command` keeps, and numbers every line from 1, so a reader names the
//! lines it read even after the log let some of them go.
//!
//! A log that a command wrote is archived when the command ends, and
//! [`LiveLog::from_archive`] reads the archive back with the same numbers.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use regex::Regex;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::watch;

/// The first bytes of lines that a log keeps.
const HEAD_BYTES: usize = 256 * 1024;
/// The last bytes of lines that a log keeps.
const TAIL_BYTES: usize = 768 * 1024;
/// The longest line. A longer line is cut there, with [`CUT_MARK`], and the
/// rest of it, up to its line break, is left out.
pub const MAX_LINE_BYTES: usize = 64 * 1024;
/// What a cut line ends with.
const CUT_MARK: &str = " [line cut at 64 KiB]";
/// The size of one read from a stream.
const READ_BYTES: usize = 64 * 1024;

/// How far a log has come: the number of its lines, and whether it ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LogMark {
    /// The lines that the command wrote so far, counting the lines that the
    /// log let go.
    pub lines: u64,
    /// Whether the command ended and its streams were read to their end.
    pub ended: bool,
}

/// Lines of a log, in order, and the lines between them that the log no
/// longer holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogLines {
    /// The lines, each with its number.
    pub lines: Vec<(u64, String)>,
    /// The first and the last line of the range asked for that the log no
    /// longer holds, when there are any.
    pub left_out: Option<(u64, u64)>,
}

/// A log as an archive keeps it: its text, with a line that names the lines
/// left out where it lost any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedLog {
    /// The text.
    pub text: String,
    /// How many of the first lines the text holds before the lines left out.
    pub head_lines: u64,
    /// The lines that the log let go, when it let any go.
    pub left_out: Option<(u64, u64)>,
    /// The lines that the command wrote.
    pub lines: u64,
    /// The bytes that the command wrote.
    pub bytes: u64,
    /// Whether bytes that are not valid UTF-8 were replaced.
    pub replaced: bool,
}

/// The lines of a log, as it keeps them.
#[derive(Debug, Default)]
struct State {
    /// The first lines, from line 1.
    head: Vec<String>,
    head_bytes: usize,
    /// Whether a line went past the room of the head, after which every
    /// line goes to the tail, so the head stays the log's first lines.
    head_closed: bool,
    /// The last lines, up to [`State::lines`].
    tail: VecDeque<String>,
    tail_bytes: usize,
    lines: u64,
    bytes: u64,
    replaced: bool,
    ended: bool,
}

impl State {
    fn push(&mut self, line: String) {
        self.lines += 1;
        let size = line.len() + 1;
        if !self.head_closed && self.head_bytes + size <= HEAD_BYTES {
            self.head_bytes += size;
            self.head.push(line);
            return;
        }
        self.head_closed = true;
        self.tail_bytes += size;
        self.tail.push_back(line);
        while self.tail_bytes > TAIL_BYTES {
            let Some(dropped) = self.tail.pop_front() else {
                break;
            };
            self.tail_bytes -= dropped.len() + 1;
        }
    }

    fn mark(&self) -> LogMark {
        LogMark {
            lines: self.lines,
            ended: self.ended,
        }
    }

    /// The number of the last line of the head.
    fn head_end(&self) -> u64 {
        count(self.head.len())
    }

    /// The number of the first line of the tail, or the next line's number
    /// while the tail is empty.
    fn tail_start(&self) -> u64 {
        self.lines - count(self.tail.len()) + 1
    }

    /// The lines that the log let go.
    fn left_out(&self) -> Option<(u64, u64)> {
        let first = self.head_end() + 1;
        let last = self.tail_start() - 1;
        (first <= last).then_some((first, last))
    }

    /// The line with this number, when the log holds it.
    fn line(&self, number: u64) -> Option<&str> {
        if number == 0 || number > self.lines {
            return None;
        }
        if number <= self.head_end() {
            return self.head.get(index(number - 1)).map(String::as_str);
        }
        let start = self.tail_start();
        if number < start {
            return None;
        }
        self.tail.get(index(number - start)).map(String::as_str)
    }

    /// Up to `max_lines` lines from `first` on, with the part of the left-out
    /// lines that the range crosses.
    fn lines_from(&self, first: u64, max_lines: usize) -> LogLines {
        let mut lines = Vec::new();
        let mut left_out = None;
        let mut number = first.max(1);
        while number <= self.lines && lines.len() < max_lines {
            if let Some(line) = self.line(number) {
                lines.push((number, line.to_owned()));
                number += 1;
                continue;
            }
            // The range starts or runs into the lines that the log let go.
            if let Some((gap_first, gap_last)) = self.left_out() {
                left_out = Some((number.max(gap_first), gap_last));
                number = gap_last + 1;
            } else {
                break;
            }
        }
        LogLines { lines, left_out }
    }

    /// The text of the archive: the head, a line that names the lines left
    /// out, and the tail.
    fn archive(&self) -> ArchivedLog {
        let mut text = String::new();
        for line in &self.head {
            text.push_str(line);
            text.push('\n');
        }
        let left_out = self.left_out();
        if let Some((first, last)) = left_out {
            text.push_str(&left_out_line(first, last));
            text.push('\n');
        }
        for line in &self.tail {
            text.push_str(line);
            text.push('\n');
        }
        ArchivedLog {
            text,
            head_lines: self.head_end(),
            left_out,
            lines: self.lines,
            bytes: self.bytes,
            replaced: self.replaced,
        }
    }
}

/// The line of an archive that stands for the lines left out.
fn left_out_line(first: u64, last: u64) -> String {
    format!("[lines {first} to {last} left out]")
}

fn count(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

fn index(number: u64) -> usize {
    usize::try_from(number).unwrap_or(usize::MAX)
}

#[derive(Debug)]
struct Shared {
    state: Mutex<State>,
    changes: watch::Sender<LogMark>,
}

/// The output of a command that runs in the background, which its readers
/// share. Clones share the log.
#[derive(Debug, Clone)]
pub struct LiveLog {
    shared: Arc<Shared>,
}

impl Default for LiveLog {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveLog {
    /// An empty log of a command that runs.
    pub fn new() -> Self {
        let (changes, _receiver) = watch::channel(LogMark::default());
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State::default()),
                changes,
            }),
        }
    }

    /// The log of a command that ended, read back from its archive.
    /// `head_lines` and `left_out` are what [`ArchivedLog`] said when the log
    /// was archived, so the lines keep their numbers.
    pub fn from_archive(text: &str, head_lines: u64, left_out: Option<(u64, u64)>) -> Self {
        let mut state = State {
            head_closed: true,
            ended: true,
            ..State::default()
        };
        let mut lines = text.lines();
        for line in lines.by_ref().take(index(head_lines)) {
            state.head.push(line.to_owned());
            state.head_bytes += line.len() + 1;
        }
        let mut number = head_lines;
        if let Some((_, last)) = left_out {
            // The archive's own line that names the lines left out.
            lines.next();
            number = last;
        }
        for line in lines {
            state.tail.push_back(line.to_owned());
            state.tail_bytes += line.len() + 1;
            number += 1;
        }
        state.lines = number;
        state.bytes = count(state.head_bytes + state.tail_bytes);
        let mark = state.mark();
        let (changes, _receiver) = watch::channel(mark);
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(state),
                changes,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Adds lines that a stream completed, after `bytes` more bytes of it
    /// were read, and wakes the readers.
    fn append(&self, bytes: usize, lines: Vec<Decoded>) {
        let mark = {
            let mut state = self.lock();
            state.bytes = state.bytes.saturating_add(count(bytes));
            if lines.is_empty() {
                return;
            }
            for line in lines {
                state.replaced |= line.replaced;
                state.push(line.text);
            }
            state.mark()
        };
        self.shared.changes.send_replace(mark);
    }

    /// Adds lines that a plugin read from a remote command (ADR-0041,
    /// decision 11), and wakes the readers. A text with line breaks counts as
    /// its lines, and a line longer than [`MAX_LINE_BYTES`] is cut as a
    /// stream's line is, so the archive reads back with the same numbers.
    pub fn push_lines(&self, texts: Vec<String>) {
        let mut bytes = 0;
        let mut lines = Vec::new();
        for text in texts {
            bytes += text.len() + 1;
            for line in text.split('\n') {
                lines.push(remote_line(line));
            }
        }
        self.append(bytes, lines);
    }

    /// Marks the log as ended: the command ended and its streams were read.
    pub fn end(&self) {
        let mark = {
            let mut state = self.lock();
            state.ended = true;
            state.mark()
        };
        self.shared.changes.send_replace(mark);
    }

    /// How far the log has come.
    pub fn mark(&self) -> LogMark {
        self.lock().mark()
    }

    /// Follows the log: the receiver changes whenever lines arrive and when
    /// the log ends.
    pub fn subscribe(&self) -> watch::Receiver<LogMark> {
        self.shared.changes.subscribe()
    }

    /// Every line after the line `after` that the log holds, with the lines
    /// in between that it let go.
    pub fn lines_after(&self, after: u64) -> LogLines {
        self.lines_from(after.saturating_add(1), usize::MAX)
    }

    /// Up to `max_lines` lines from the line `first` on.
    pub fn lines_from(&self, first: u64, max_lines: usize) -> LogLines {
        self.lock().lines_from(first, max_lines)
    }

    /// The number of the first line after the line `after` that matches the
    /// pattern, among the lines that the log holds.
    pub fn first_match_after(&self, after: u64, pattern: &Regex) -> Option<u64> {
        let state = self.lock();
        let head = state
            .head
            .iter()
            .enumerate()
            .map(|(position, line)| (count(position) + 1, line));
        let start = state.tail_start();
        let tail = state
            .tail
            .iter()
            .enumerate()
            .map(|(position, line)| (start + count(position), line));
        head.chain(tail)
            .find(|(number, line)| *number > after && pattern.is_match(line))
            .map(|(number, _)| number)
    }

    /// Whether bytes that are not valid UTF-8 were replaced.
    pub fn replaced(&self) -> bool {
        self.lock().replaced
    }

    /// The log as its archive keeps it.
    pub fn archive(&self) -> ArchivedLog {
        self.lock().archive()
    }
}

/// The bytes before a character that the end of `bytes` cuts.
pub(super) fn complete_start(bytes: &[u8]) -> &[u8] {
    match std::str::from_utf8(bytes) {
        Err(error) if error.error_len().is_none() => &bytes[..error.valid_up_to()],
        _ => bytes,
    }
}

/// A line of a stream, decoded.
#[derive(Debug, PartialEq, Eq)]
struct Decoded {
    text: String,
    replaced: bool,
}

/// Decodes a line's bytes, without its line break. CRLF reads as LF, and
/// bytes that are not valid UTF-8 are replaced. A cut line drops the pieces
/// of a character that the cut split, and ends with [`CUT_MARK`].
fn decode(bytes: &[u8], cut: bool) -> Decoded {
    let bytes = if cut {
        complete_start(bytes)
    } else {
        bytes.strip_suffix(b"\r").unwrap_or(bytes)
    };
    let text = String::from_utf8_lossy(bytes);
    let replaced = matches!(text, std::borrow::Cow::Owned(_));
    let mut text = text.into_owned();
    if cut {
        text.push_str(CUT_MARK);
    }
    Decoded { text, replaced }
}

/// A line of a remote command, without its carriage return, cut at
/// [`MAX_LINE_BYTES`].
fn remote_line(line: &str) -> Decoded {
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line.len() <= MAX_LINE_BYTES {
        return Decoded {
            text: line.to_owned(),
            replaced: false,
        };
    }
    let mut end = MAX_LINE_BYTES;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    let mut text = line[..end].to_owned();
    text.push_str(CUT_MARK);
    Decoded {
        text,
        replaced: false,
    }
}

/// The lines of one stream, as its bytes arrive.
#[derive(Debug, Default)]
struct LineSplitter {
    /// The bytes of the line that has not ended yet.
    partial: Vec<u8>,
    /// Whether the line that has not ended was cut, so its bytes up to its
    /// line break are left out.
    skipping: bool,
}

impl LineSplitter {
    /// Adds bytes, and returns the lines that they complete.
    fn push(&mut self, chunk: &[u8]) -> Vec<Decoded> {
        let mut lines = Vec::new();
        let mut rest = chunk;
        while let Some(at) = rest.iter().position(|byte| *byte == b'\n') {
            self.extend(&rest[..at], &mut lines);
            if self.skipping {
                // The line break ends a line that was cut already.
                self.skipping = false;
            } else {
                lines.push(decode(&self.partial, false));
            }
            self.partial.clear();
            rest = &rest[at + 1..];
        }
        self.extend(rest, &mut lines);
        lines
    }

    /// Adds bytes of a line that has not ended, and cuts the line when it
    /// goes past [`MAX_LINE_BYTES`].
    fn extend(&mut self, piece: &[u8], lines: &mut Vec<Decoded>) {
        if self.skipping {
            return;
        }
        let room = MAX_LINE_BYTES - self.partial.len();
        if piece.len() <= room {
            self.partial.extend_from_slice(piece);
            return;
        }
        self.partial.extend_from_slice(&piece[..room]);
        lines.push(decode(&self.partial, true));
        self.partial.clear();
        self.skipping = true;
    }

    /// The last line of a stream that ended without a line break.
    fn finish(&mut self) -> Vec<Decoded> {
        let mut lines = Vec::new();
        if !self.skipping && !self.partial.is_empty() {
            lines.push(decode(&self.partial, false));
        }
        self.partial.clear();
        self.skipping = false;
        lines
    }
}

/// Reads a stream into the log until it closes or fails.
pub(super) fn read_stream(
    log: LiveLog,
    pipe: impl AsyncRead + Unpin + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut pipe = pipe;
        let mut splitter = LineSplitter::default();
        let mut buffer = vec![0_u8; READ_BYTES];
        loop {
            match pipe.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(read) => log.append(read, splitter.push(&buffer[..read])),
            }
        }
        log.append(0, splitter.finish());
    })
}

#[cfg(test)]
mod tests {
    use regex::Regex;

    use super::{
        CUT_MARK, Decoded, HEAD_BYTES, LineSplitter, LiveLog, LogLines, MAX_LINE_BYTES, TAIL_BYTES,
        decode,
    };

    fn texts(lines: &[Decoded]) -> Vec<&str> {
        lines.iter().map(|line| line.text.as_str()).collect()
    }

    /// A log with these lines, as one stream wrote them.
    fn log_of(lines: impl IntoIterator<Item = String>) -> LiveLog {
        let log = LiveLog::new();
        let decoded: Vec<Decoded> = lines
            .into_iter()
            .map(|text| Decoded {
                text,
                replaced: false,
            })
            .collect();
        log.append(0, decoded);
        log
    }

    #[test]
    fn lines_end_at_line_breaks_across_reads() {
        let mut splitter = LineSplitter::default();

        let first = splitter.push(b"one\ntw");
        let second = splitter.push(b"o\r\nthree");
        let last = splitter.finish();

        assert_eq!(texts(&first), ["one"]);
        assert_eq!(texts(&second), ["two"]);
        assert_eq!(texts(&last), ["three"]);
    }

    #[test]
    fn an_empty_line_is_a_line() {
        let mut splitter = LineSplitter::default();

        assert_eq!(texts(&splitter.push(b"\n\nend\n")), ["", "", "end"]);
        assert_eq!(splitter.finish(), Vec::new());
    }

    #[test]
    fn a_long_line_is_cut_and_its_rest_left_out() {
        let mut splitter = LineSplitter::default();
        let long = vec![b'x'; MAX_LINE_BYTES + 100];

        let mut lines = splitter.push(&long);
        lines.extend(splitter.push(b"yyy\nnext\n"));

        assert_eq!(lines.len(), 2);
        assert!(lines[0].text.ends_with(CUT_MARK));
        assert_eq!(lines[0].text.len(), MAX_LINE_BYTES + CUT_MARK.len());
        assert_eq!(lines[1].text, "next");
    }

    #[test]
    fn a_line_of_the_longest_size_is_kept_whole() {
        let mut splitter = LineSplitter::default();
        let mut bytes = vec![b'x'; MAX_LINE_BYTES];
        bytes.push(b'\n');

        let lines = splitter.push(&bytes);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text.len(), MAX_LINE_BYTES);
    }

    #[test]
    fn invalid_utf8_is_replaced_and_reported() {
        let decoded = decode(b"caf\xe9", false);

        assert_eq!(decoded.text, "caf\u{fffd}");
        assert!(decoded.replaced);
    }

    #[test]
    fn a_cut_through_a_character_drops_its_pieces() {
        let mut bytes = vec![b'a'; MAX_LINE_BYTES - 1];
        // The first byte of a two-byte character ends the room of the line.
        bytes.extend_from_slice("é and more".as_bytes());
        let mut splitter = LineSplitter::default();

        let lines = splitter.push(&bytes);

        assert_eq!(lines.len(), 1);
        assert!(!lines[0].replaced);
        assert!(lines[0].text.starts_with(&"a".repeat(MAX_LINE_BYTES - 1)));
    }

    #[test]
    fn small_logs_keep_every_line_with_its_number() {
        let log = log_of((1..=5).map(|line| format!("line {line}")));

        let read = log.lines_after(2);

        assert_eq!(
            read,
            LogLines {
                lines: vec![
                    (3, "line 3".to_owned()),
                    (4, "line 4".to_owned()),
                    (5, "line 5".to_owned())
                ],
                left_out: None,
            }
        );
        assert_eq!(log.mark().lines, 5);
        assert_eq!(log.lines_after(5), LogLines::default());
    }

    #[test]
    fn large_logs_keep_their_start_and_their_end() {
        // Lines of 1 KiB with their line breaks: the head holds 256 of them,
        // and the tail 768.
        let line = "x".repeat(1023);
        let total = 5_000_u64;
        let log = log_of((0..total).map(|_| line.clone()));
        let head = u64::try_from(HEAD_BYTES / 1024).unwrap();
        let tail = u64::try_from(TAIL_BYTES / 1024).unwrap();

        let read = log.lines_after(0);

        assert_eq!(read.left_out, Some((head + 1, total - tail)));
        assert_eq!(read.lines.len(), usize::try_from(head + tail).unwrap());
        assert_eq!(read.lines.first().unwrap().0, 1);
        assert_eq!(read.lines.last().unwrap().0, total);
        // A read after the lines that the log let go starts in the tail.
        let late = log.lines_after(total - 3);
        assert_eq!(late.lines.len(), 3);
        assert_eq!(late.left_out, None);
        // A page that starts among them names them and goes on in the tail.
        let page = log.lines_from(head + 10, 2);
        assert_eq!(page.left_out, Some((head + 10, total - tail)));
        assert_eq!(page.lines[0].0, total - tail + 1);
    }

    #[test]
    fn a_pattern_finds_the_first_new_line_that_matches() {
        let log = log_of([
            "starting".to_owned(),
            "ready on 3000".to_owned(),
            "ready again".to_owned(),
        ]);
        let ready = Regex::new("^ready").unwrap();

        assert_eq!(log.first_match_after(0, &ready), Some(2));
        assert_eq!(log.first_match_after(2, &ready), Some(3));
        assert_eq!(log.first_match_after(3, &ready), None);
    }

    #[test]
    fn an_archive_reads_back_with_the_same_numbers() {
        let line = "y".repeat(1023);
        let log = log_of((0..3_000).map(|number| format!("{number:04}{}", &line[4..])));
        log.end();

        let archived = log.archive();
        let back = LiveLog::from_archive(&archived.text, archived.head_lines, archived.left_out);

        assert_eq!(back.mark(), log.mark());
        assert_eq!(back.lines_after(0), log.lines_after(0));
        assert_eq!(back.lines_from(2_990, 5), log.lines_from(2_990, 5));
        assert!(archived.text.contains("left out]"));
    }

    #[test]
    fn a_whole_archive_reads_back_with_the_same_numbers() {
        let log = log_of((1..=3).map(|line| format!("line {line}")));
        log.end();

        let archived = log.archive();
        let back = LiveLog::from_archive(&archived.text, archived.head_lines, archived.left_out);

        assert_eq!(archived.text, "line 1\nline 2\nline 3\n");
        assert_eq!(archived.left_out, None);
        assert_eq!(back.lines_after(1), log.lines_after(1));
        assert_eq!(back.mark().lines, 3);
    }

    #[test]
    fn a_remote_command_s_lines_keep_their_numbers_through_the_archive() {
        let log = LiveLog::new();
        let wide_line = "é".repeat(MAX_LINE_BYTES);

        log.push_lines(vec!["one\r".to_owned(), "two\nthree".to_owned()]);
        log.push_lines(vec![wide_line]);
        let archived = log.archive();
        let back = LiveLog::from_archive(&archived.text, archived.head_lines, archived.left_out);

        let lines = log.lines_after(0).lines;
        assert_eq!(
            lines[..3],
            [
                (1, "one".to_owned()),
                (2, "two".to_owned()),
                (3, "three".to_owned())
            ]
        );
        assert!(lines[3].1.ends_with(CUT_MARK));
        assert!(lines[3].1.len() <= MAX_LINE_BYTES + CUT_MARK.len());
        assert_eq!(log.mark().lines, 4);
        assert_eq!(back.lines_after(0), log.lines_after(0));
    }

    #[tokio::test]
    async fn readers_wake_when_lines_arrive_and_when_the_log_ends() {
        let log = LiveLog::new();
        let mut changes = log.subscribe();

        log.append(
            4,
            vec![Decoded {
                text: "one".to_owned(),
                replaced: false,
            }],
        );
        changes.changed().await.unwrap();
        assert_eq!(changes.borrow_and_update().lines, 1);

        log.end();
        changes.changed().await.unwrap();
        assert!(changes.borrow().ended);
    }
}
