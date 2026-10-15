//! The capture of a command's output (docs/features/system-tools/commands.md,
//! output): each stream keeps its first 256 KiB and its last 768 KiB, and
//! marks the cut with the number of bytes left out.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};

/// The first bytes of each stream that a command's capture keeps.
const HEAD_BYTES: usize = 256 * 1024;
/// The last bytes of each stream that a command's capture keeps.
const TAIL_BYTES: usize = 768 * 1024;
/// How long the capture waits for a stream to close after the command's
/// processes ended. Only a process outside the command's tree can keep it
/// open.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

/// What a stream produced: its start and its end, with a mark where bytes
/// were left out. CRLF line endings, which Windows programs write, read as
/// LF.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Captured {
    /// The text.
    pub(crate) text: String,
    /// The bytes left out between the start and the end.
    pub(crate) left_out: u64,
    /// The size of the whole stream in bytes.
    pub(crate) total: u64,
    /// Whether bytes that are not valid UTF-8 were replaced.
    pub(crate) replaced: bool,
}

/// The start and the end of a stream, as it arrives.
#[derive(Debug, Default)]
struct Capture {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total: u64,
}

impl Capture {
    fn push(&mut self, bytes: &[u8]) {
        self.total = self
            .total
            .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        let to_head = HEAD_BYTES.saturating_sub(self.head.len()).min(bytes.len());
        self.head.extend_from_slice(&bytes[..to_head]);
        self.tail.extend(&bytes[to_head..]);
        let excess = self.tail.len().saturating_sub(TAIL_BYTES);
        self.tail.drain(..excess);
    }

    fn finish(&self) -> Captured {
        let kept = self.head.len() + self.tail.len();
        let left_out = self
            .total
            .saturating_sub(u64::try_from(kept).unwrap_or(u64::MAX));
        let tail: Vec<u8> = self.tail.iter().copied().collect();
        if left_out == 0 {
            let whole = [self.head.as_slice(), tail.as_slice()].concat();
            let text = String::from_utf8_lossy(&whole);
            return Captured {
                replaced: matches!(text, std::borrow::Cow::Owned(_)),
                text: text.replace("\r\n", "\n"),
                left_out,
                total: self.total,
            };
        }
        // The cut can split a character: its pieces are dropped, not
        // replaced.
        let head = complete_start(&self.head);
        let tail = complete_end(&tail);
        let head_text = String::from_utf8_lossy(head);
        let tail_text = String::from_utf8_lossy(tail);
        let replaced = matches!(head_text, std::borrow::Cow::Owned(_))
            || matches!(tail_text, std::borrow::Cow::Owned(_));
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
fn complete_start(bytes: &[u8]) -> &[u8] {
    match std::str::from_utf8(bytes) {
        Err(error) if error.error_len().is_none() => &bytes[..error.valid_up_to()],
        _ => bytes,
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

/// A stream being captured. The capture is shared, so what arrived is kept
/// even when the stream never closes.
pub(super) struct Stream {
    capture: Arc<Mutex<Capture>>,
    reader: tokio::task::JoinHandle<()>,
}

impl Stream {
    pub(super) fn capture(pipe: impl AsyncRead + Unpin + Send + 'static) -> Self {
        let capture = Arc::new(Mutex::new(Capture::default()));
        let shared = Arc::clone(&capture);
        let reader = tokio::spawn(async move {
            let mut pipe = pipe;
            let mut buffer = vec![0_u8; 64 * 1024];
            loop {
                match pipe.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(read) => shared
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(&buffer[..read]),
                }
            }
        });
        Self { capture, reader }
    }

    /// What the stream produced, once it closed or its time to close ran
    /// out.
    pub(super) async fn finish(stream: Option<Self>) -> Captured {
        let Some(mut stream) = stream else {
            return Captured::default();
        };
        if tokio::time::timeout(DRAIN_TIMEOUT, &mut stream.reader)
            .await
            .is_err()
        {
            stream.reader.abort();
        }
        stream
            .capture
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{Capture, HEAD_BYTES, TAIL_BYTES};

    #[test]
    fn small_streams_are_kept_whole() {
        let mut capture = Capture::default();
        capture.push(b"one\n");
        capture.push(b"two\n");

        let captured = capture.finish();

        assert_eq!(captured.text, "one\ntwo\n");
        assert_eq!(captured.left_out, 0);
        assert_eq!(captured.total, 8);
        assert!(!captured.replaced);
    }

    #[test]
    fn large_streams_keep_their_start_and_end() {
        let mut capture = Capture::default();
        capture.push(&vec![b'a'; HEAD_BYTES]);
        capture.push(&[b'b'; 100]);
        capture.push(&vec![b'c'; TAIL_BYTES]);

        let captured = capture.finish();

        assert_eq!(captured.left_out, 100);
        let (head, tail) = captured
            .text
            .split_once("\n[100 bytes left out]\n")
            .unwrap();
        assert_eq!(head, "a".repeat(HEAD_BYTES));
        assert_eq!(tail, "c".repeat(TAIL_BYTES));
    }

    #[test]
    fn crlf_reads_as_lf() {
        let mut capture = Capture::default();
        capture.push(b"one\r\ntwo\r\n");

        assert_eq!(capture.finish().text, "one\ntwo\n");
    }

    #[test]
    fn invalid_utf8_is_replaced_and_reported() {
        let mut capture = Capture::default();
        capture.push(b"caf\xe9\n");

        let captured = capture.finish();

        assert_eq!(captured.text, "caf\u{fffd}\n");
        assert!(captured.replaced);
    }

    #[test]
    fn a_cut_through_a_character_drops_its_pieces() {
        let mut capture = Capture::default();
        let mut head = vec![b'a'; HEAD_BYTES - 1];
        // The first byte of a two-byte character ends the head.
        head.push(0xc3);
        capture.push(&head);
        capture.push(&[0xa9]);
        capture.push(&[b'x'; 10]);
        capture.push(&vec![b'c'; TAIL_BYTES]);

        let captured = capture.finish();

        assert!(
            !captured.replaced,
            "{}",
            &captured.text[HEAD_BYTES - 4..HEAD_BYTES + 30]
        );
    }
}
