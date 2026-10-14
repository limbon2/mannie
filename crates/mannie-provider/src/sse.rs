//! A parser for server-sent events (the HTML standard's `text/event-stream`),
//! Mannie's own and small, as ADR-0011 decides.
//!
//! The parser takes bytes in chunks of any size: a line ending, a UTF-8
//! sequence or a field may be split between two chunks. Lines end with CRLF,
//! LF or CR. Comments (lines that start with a colon) are skipped, and so are
//! the `id` and `retry` fields, because Mannie never reconnects a stream on
//! its own: the runtime retries the whole call.

/// The longest event Mannie accepts, in bytes of its data.
pub(crate) const MAX_EVENT_BYTES: usize = 1024 * 1024;

/// The longest line Mannie accepts, in bytes.
const MAX_LINE_BYTES: usize = MAX_EVENT_BYTES + 64;

/// One event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseEvent {
    /// The event's type: the `event` field, or `message` without one.
    pub(crate) event: String,
    /// The event's data: its `data` lines joined with line feeds.
    pub(crate) data: String,
}

/// Why a stream is not valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum SseError {
    /// An event or a line is longer than Mannie accepts.
    #[error("an event is longer than {MAX_EVENT_BYTES} bytes")]
    TooLarge,
    /// A line is not valid UTF-8.
    #[error("the stream is not valid UTF-8")]
    InvalidUtf8,
}

/// The parser's state between chunks.
#[derive(Debug, Default)]
pub(crate) struct SseParser {
    /// The bytes of the line that is not complete yet.
    line: Vec<u8>,
    /// Whether the previous chunk ended with CR, so that an LF at the start of
    /// the next chunk belongs to the same line ending.
    after_cr: bool,
    /// Whether the start of the stream was checked for a byte order mark.
    started: bool,
    /// The type of the event being read.
    event: Option<String>,
    /// The data of the event being read.
    data: String,
    /// Whether the event being read has a data field, even an empty one.
    has_data: bool,
}

impl SseParser {
    /// A parser at the start of a stream.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Reads a chunk and returns the events it completes.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, SseError> {
        let mut events = Vec::new();
        let mut bytes = chunk;
        if !self.started && !bytes.is_empty() {
            self.started = true;
            bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        }
        for &byte in bytes {
            let continues_crlf = self.after_cr && byte == b'\n';
            self.after_cr = false;
            if continues_crlf {
                continue;
            }
            match byte {
                b'\r' | b'\n' => {
                    self.after_cr = byte == b'\r';
                    let line = std::mem::take(&mut self.line);
                    if let Some(event) = self.line_ended(&line)? {
                        events.push(event);
                    }
                }
                _ => {
                    if self.line.len() >= MAX_LINE_BYTES {
                        return Err(SseError::TooLarge);
                    }
                    self.line.push(byte);
                }
            }
        }
        Ok(events)
    }

    /// Whether the stream ended in the middle of a line or an event. The
    /// standard drops such an incomplete event; Mannie reports it, because a
    /// stream that ends there was cut.
    pub(crate) fn has_partial_event(&self) -> bool {
        let has_pending_line = !self.line.is_empty();
        let has_pending_event = self.has_data || self.event.is_some();
        has_pending_line || has_pending_event
    }

    fn line_ended(&mut self, line: &[u8]) -> Result<Option<SseEvent>, SseError> {
        if line.is_empty() {
            return Ok(self.dispatch());
        }
        let text = std::str::from_utf8(line).map_err(|_invalid| SseError::InvalidUtf8)?;
        if text.starts_with(':') {
            return Ok(None);
        }
        let (field, value) = match text.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (text, ""),
        };
        match field {
            "data" => {
                let added = value.len() + usize::from(self.has_data);
                if self.data.len() + added > MAX_EVENT_BYTES {
                    return Err(SseError::TooLarge);
                }
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            "event" => self.event = Some(value.to_owned()),
            // `id` and `retry` serve reconnection, which Mannie leaves to the
            // runtime; other fields are ignored, as the standard says.
            _ => {}
        }
        Ok(None)
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        let event = self.event.take();
        if !self.has_data {
            return None;
        }
        self.has_data = false;
        Some(SseEvent {
            event: event.unwrap_or_else(|| "message".to_owned()),
            data: std::mem::take(&mut self.data),
        })
    }
}

#[cfg(test)]
mod tests;
