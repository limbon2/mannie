//! The lines of a command in the background (ADR-0041, decision 11), as the
//! plugin hands them to the core: each stream of the channel becomes whole
//! lines without their line breaks, after the host's password is replaced in
//! its bytes.
//!
//! The redaction runs on the bytes as they arrive and keeps back the bytes
//! at the end of a read that could start the password, until the next read
//! shows whether they do. So a password that a read cuts is still replaced,
//! and so is a password with a line break in it (T-41). A line longer than
//! the core keeps is cut there, with a mark, and the rest of it, up to its
//! line break, is left out, so a host cannot make the plugin hold an
//! endless line.

use mannie_plugin_sdk::contracts::Secret;

use crate::commands::MIN_REDACTED_PASSWORD_BYTES;
use crate::output::{REDACTED, complete_start};

/// The longest line, which the core keeps whole.
const MAX_LINE_BYTES: usize = 64 * 1024;

/// What a cut line ends with, as the core marks its own cut lines.
const CUT_MARK: &str = " [line cut at 64 KiB]";

/// Replaces the host's password in a stream as its bytes arrive.
#[derive(Debug)]
struct Redaction {
    /// The password, or `None` when it is too short to tell apart from
    /// ordinary text, as for `run_ssh_command`.
    password: Option<Secret>,
    /// The bytes at the end of the last read that could start the password.
    carry: Vec<u8>,
}

impl Redaction {
    fn new(password: Secret) -> Self {
        let is_long_enough = password.expose().len() >= MIN_REDACTED_PASSWORD_BYTES as usize;
        Self {
            password: is_long_enough.then_some(password),
            carry: Vec::new(),
        }
    }

    /// The bytes of a read that are known, with the password replaced.
    fn push(&mut self, bytes: &[u8]) -> Vec<u8> {
        let Some(password) = &self.password else {
            return bytes.to_vec();
        };
        let password = password.expose().as_bytes();
        let mut input = std::mem::take(&mut self.carry);
        input.extend_from_slice(bytes);
        let mut output = Vec::with_capacity(input.len());
        let mut rest = input.as_slice();
        while let Some(at) = find(rest, password) {
            output.extend_from_slice(&rest[..at]);
            output.extend_from_slice(REDACTED.as_bytes());
            rest = &rest[at + password.len()..];
        }
        let known = rest.len() - held_back(rest, password);
        output.extend_from_slice(&rest[..known]);
        self.carry = rest[known..].to_vec();
        output
    }

    /// The bytes kept back at the stream's end, which did not become the
    /// password.
    fn finish(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.carry)
    }
}

/// Where `needle` first occurs in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// How many bytes at the end of `bytes` could start the password.
fn held_back(bytes: &[u8], password: &[u8]) -> usize {
    (1..password.len())
        .rev()
        .find(|&length| bytes.ends_with(&password[..length]))
        .unwrap_or(0)
}

/// A line's text, without its carriage return. A cut line drops the pieces
/// of a character that the cut split, and ends with [`CUT_MARK`].
fn decode(bytes: &[u8], cut: bool) -> String {
    let bytes = if cut {
        complete_start(bytes)
    } else {
        bytes.strip_suffix(b"\r").unwrap_or(bytes)
    };
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    if cut {
        text.push_str(CUT_MARK);
    }
    text
}

/// The lines of one stream of a command, as its bytes arrive.
#[derive(Debug)]
pub(crate) struct StreamLines {
    redaction: Redaction,
    /// The bytes of the line that has not ended yet.
    partial: Vec<u8>,
    /// Whether the line that has not ended was cut, so its bytes up to its
    /// line break are left out.
    skipping: bool,
}

impl StreamLines {
    /// The lines of a stream on a host whose password is `password`.
    pub(crate) fn new(password: Secret) -> Self {
        Self {
            redaction: Redaction::new(password),
            partial: Vec::new(),
            skipping: false,
        }
    }

    /// Adds a read's bytes, and returns the lines that they complete.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        let known = self.redaction.push(bytes);
        self.split(&known)
    }

    /// The lines that are left when the stream ends, the last one without
    /// its line break.
    pub(crate) fn finish(&mut self) -> Vec<String> {
        let rest = self.redaction.finish();
        let mut lines = self.split(&rest);
        if !self.skipping && !self.partial.is_empty() {
            lines.push(decode(&self.partial, false));
        }
        self.partial.clear();
        self.skipping = false;
        lines
    }

    fn split(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        let mut rest = bytes;
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
    fn extend(&mut self, piece: &[u8], lines: &mut Vec<String>) {
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
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::Secret;

    use super::{CUT_MARK, MAX_LINE_BYTES, StreamLines};

    const PASSWORD: &str = "correct horse battery";

    fn lines() -> StreamLines {
        StreamLines::new(Secret::new(PASSWORD.to_owned()))
    }

    #[test]
    fn lines_end_at_line_breaks_across_reads_without_their_carriage_returns() {
        let mut stream = lines();

        let first = stream.push(b"one\r\ntw");
        let second = stream.push(b"o\nthree");
        let last = stream.finish();

        assert_eq!(first, ["one"]);
        assert_eq!(second, ["two"]);
        assert_eq!(last, ["three"]);
    }

    #[test]
    fn a_password_that_reads_cut_is_still_replaced() {
        let mut stream = lines();

        let mut seen = stream.push(b"the password is correct ho");
        seen.extend(stream.push(b"rse"));
        seen.extend(stream.push(b" battery, twice: correct horse battery\nnext\n"));
        seen.extend(stream.finish());

        assert_eq!(
            seen,
            ["the password is [REDACTED], twice: [REDACTED]", "next"]
        );
    }

    #[test]
    fn a_password_with_a_line_break_is_replaced_across_the_lines() {
        let mut stream = StreamLines::new(Secret::new("first line\nsecond".to_owned()));

        let mut seen = stream.push(b"before first line\nsec");
        seen.extend(stream.push(b"ond after\n"));

        assert_eq!(seen, ["before [REDACTED] after"]);
    }

    #[test]
    fn bytes_that_only_start_the_password_show_once_the_stream_moves_on_or_ends() {
        let mut stream = lines();

        let held = stream.push(b"correct h");
        let shown = stream.push(b"ouse\n");
        let ending = stream.push(b"correct");
        let rest = stream.finish();

        assert_eq!(held, Vec::<String>::new());
        assert_eq!(shown, ["correct house"]);
        assert_eq!(ending, Vec::<String>::new());
        assert_eq!(rest, ["correct"]);
    }

    #[test]
    fn a_short_password_is_left_as_it_is() {
        let mut stream = StreamLines::new(Secret::new("hunter2".to_owned()));

        assert_eq!(stream.push(b"hunter2\n"), ["hunter2"]);
    }

    #[test]
    fn a_long_line_is_cut_with_a_mark_and_its_rest_left_out() {
        let mut stream = lines();
        let long = vec![b'x'; MAX_LINE_BYTES + 100];

        let mut seen = stream.push(&long);
        seen.extend(stream.push(b"yyy\nnext\n"));

        assert_eq!(seen.len(), 2);
        assert!(seen[0].ends_with(CUT_MARK));
        assert_eq!(seen[0].len(), MAX_LINE_BYTES + CUT_MARK.len());
        assert_eq!(seen[1], "next");
    }

    #[test]
    fn invalid_utf8_is_replaced() {
        let mut stream = lines();

        assert_eq!(stream.push(b"caf\xe9\n"), ["caf\u{fffd}"]);
    }
}
