//! The redaction layer: every log line passes through a [`Redactor`] before it
//! reaches a file or the console (engineering standards, observability).
//!
//! The secret type already refuses to format its value, so a secret reaches a
//! log line only through a bug, for example by logging the result of
//! `Secret::expose`. The redactor catches such bugs in two ways: it replaces
//! the values of secrets that the process registered, and it replaces text
//! that looks like a credential: bearer tokens, keys that start with `sk-`,
//! and the values of fields named like credentials.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::io;
use std::sync::{Arc, LazyLock, PoisonError, RwLock};

use mannie_types::secret::Secret;
use regex::Regex;
use tracing_subscriber::fmt::MakeWriter;
use zeroize::Zeroizing;

/// The text that replaces a secret.
pub const REDACTED: &str = "[REDACTED]";

/// Secrets shorter than this are not matched by value: replacing every
/// occurrence of a few characters would garble the logs. Provider credentials
/// are much longer.
const MIN_REGISTERED_LENGTH: usize = 8;

/// Text that looks like a credential, whether or not it was registered.
static CREDENTIAL_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        // Authorization headers and bearer tokens.
        (r"(?i)\b(bearer)\s+[A-Za-z0-9._~+/=-]{8,}", "${1} [REDACTED]"),
        // OpenAI and OpenAI-style keys: sk-..., sk-proj-...
        (r"\bsk-[A-Za-z0-9_-]{16,}", "[REDACTED]"),
        // JSON fields named like credentials: "api_key":"...".
        (
            r#"(?i)("(?:api[_-]?key|authorization|password|passwd|secret|client[_-]?secret|access[_-]?token|refresh[_-]?token|credential)"\s*:\s*)"(?:[^"\\]|\\.)*""#,
            "${1}\"[REDACTED]\"",
        ),
        // key=value pairs named like credentials, as the console format writes them.
        (
            r"(?i)\b((?:api[_-]?key|authorization|password|passwd|secret|client[_-]?secret|access[_-]?token|refresh[_-]?token|credential)\s*=\s*)[^\s,;]+",
            "${1}[REDACTED]",
        ),
    ]
    .into_iter()
    .map(|(pattern, replacement)| (Regex::new(pattern).expect("the credential patterns are valid"), replacement))
    .collect()
});

/// Removes secrets from text. Cloning is cheap, and every clone shares the
/// registered secrets.
#[derive(Clone, Default)]
pub struct Redactor {
    registered: Arc<RwLock<Vec<Zeroizing<String>>>>,
}

impl std::fmt::Debug for Redactor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self
            .registered
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        formatter
            .debug_struct("Redactor")
            .field("registered", &count)
            .finish()
    }
}

impl Redactor {
    /// A redactor with no registered secrets.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a secret whose value must never appear in a log. The code
    /// that reads a secret into the process registers it.
    pub fn register(&self, secret: &Secret) {
        let value = secret.expose();
        if value.len() < MIN_REGISTERED_LENGTH {
            return;
        }
        let mut registered = self
            .registered
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let forms = [value.to_owned(), json_escaped(value)];
        for form in forms {
            if !registered.iter().any(|known| **known == form) {
                registered.push(Zeroizing::new(form));
            }
        }
        // Longer forms first, so a secret that contains another one is
        // replaced as a whole.
        registered.sort_by_key(|form| std::cmp::Reverse(form.len()));
    }

    /// Returns `text` with every registered secret and every credential-like
    /// value replaced by [`REDACTED`].
    pub fn redact<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let mut result = Cow::Borrowed(text);
        {
            let registered = self
                .registered
                .read()
                .unwrap_or_else(PoisonError::into_inner);
            for secret in registered.iter() {
                if result.contains(secret.as_str()) {
                    result = Cow::Owned(result.replace(secret.as_str(), REDACTED));
                }
            }
        }
        for (pattern, replacement) in CREDENTIAL_PATTERNS.iter() {
            if let Cow::Owned(replaced) = pattern.replace_all(&result, *replacement) {
                result = Cow::Owned(replaced);
            }
        }
        result
    }
}

/// How a value appears inside a JSON string, so a secret is also found in the
/// JSON log lines.
fn json_escaped(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            control if control.is_control() => {
                write!(escaped, "\\u{:04x}", u32::from(control))
                    .expect("writing to a String cannot fail");
            }
            other => escaped.push(other),
        }
    }
    escaped
}

/// A `MakeWriter` that redacts every log line before passing it on.
#[derive(Debug, Clone)]
pub(crate) struct RedactingMakeWriter<M> {
    inner: M,
    redactor: Redactor,
}

impl<M> RedactingMakeWriter<M> {
    pub(crate) fn new(inner: M, redactor: Redactor) -> Self {
        Self { inner, redactor }
    }
}

impl<'a, M> MakeWriter<'a> for RedactingMakeWriter<M>
where
    M: MakeWriter<'a>,
{
    type Writer = RedactingWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter {
            inner: self.inner.make_writer(),
            redactor: self.redactor.clone(),
            line: Vec::new(),
        }
    }

    fn make_writer_for(&'a self, meta: &tracing::Metadata<'_>) -> Self::Writer {
        RedactingWriter {
            inner: self.inner.make_writer_for(meta),
            redactor: self.redactor.clone(),
            line: Vec::new(),
        }
    }
}

/// Collects one formatted event and writes it, redacted, when it is flushed
/// or dropped. The formatter creates one writer per event.
pub(crate) struct RedactingWriter<W: io::Write> {
    inner: W,
    redactor: Redactor,
    line: Vec<u8>,
}

impl<W: io::Write> RedactingWriter<W> {
    fn write_line(&mut self) -> io::Result<()> {
        if self.line.is_empty() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&self.line);
        let redacted = self.redactor.redact(&text);
        let result = self.inner.write_all(redacted.as_bytes());
        // The unredacted line must not stay in memory longer than needed.
        zeroize::Zeroize::zeroize(&mut self.line);
        self.line.clear();
        result
    }
}

impl<W: io::Write> io::Write for RedactingWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.line.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.write_line()?;
        self.inner.flush()
    }
}

impl<W: io::Write> Drop for RedactingWriter<W> {
    fn drop(&mut self) {
        // A log line that cannot be written is lost; there is nowhere else to
        // report it.
        if self.write_line().is_err() {
            self.line.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use mannie_types::secret::Secret;

    use super::{REDACTED, RedactingWriter, Redactor};

    const PLANTED: &str = "planted-test-secret-4f9c2a";

    #[test]
    fn registered_secrets_are_replaced_in_raw_and_json_form() {
        let redactor = Redactor::new();
        redactor.register(&Secret::new(PLANTED.to_owned()));
        redactor.register(&Secret::new("quote\"and\\slash-secret".to_owned()));

        let line = format!("request failed with key {PLANTED} at step 3");
        let raw = redactor.redact(&line);
        let json = redactor.redact(r#"{"message":"bad key quote\"and\\slash-secret"}"#);

        assert_eq!(raw, format!("request failed with key {REDACTED} at step 3"));
        assert_eq!(json, format!(r#"{{"message":"bad key {REDACTED}"}}"#));
    }

    #[test]
    fn short_secrets_are_not_matched_by_value() {
        let redactor = Redactor::new();
        redactor.register(&Secret::new("abc".to_owned()));

        assert_eq!(redactor.redact("abc def"), "abc def");
    }

    #[test]
    fn credential_patterns_are_replaced_without_registration() {
        let redactor = Redactor::new();
        let cases = [
            (
                "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload",
                "Authorization: Bearer [REDACTED]",
            ),
            (
                "key sk-proj-abcdefghijklmnop0123 rejected",
                "key [REDACTED] rejected",
            ),
            (
                r#"{"api_key":"abc\"def","model":"gpt"}"#,
                r#"{"api_key":"[REDACTED]","model":"gpt"}"#,
            ),
            (
                r#"{"Authorization":"Basic dXNlcjpwYXNz"}"#,
                r#"{"Authorization":"[REDACTED]"}"#,
            ),
            (
                "connecting api_key=abcdef123456 model=gpt",
                "connecting api_key=[REDACTED] model=gpt",
            ),
            (
                "password = hunter2hunter2, user=owner",
                "password = [REDACTED], user=owner",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(redactor.redact(input), expected, "{input}");
        }
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        let redactor = Redactor::new();
        let text = r#"{"level":"INFO","fields":{"message":"migrated the database","tokens":200,"skip":"sk-short"}}"#;

        assert_eq!(redactor.redact(text), text);
    }

    #[test]
    fn the_writer_redacts_the_whole_line_when_dropped() {
        let redactor = Redactor::new();
        redactor.register(&Secret::new(PLANTED.to_owned()));
        let mut output = Vec::new();
        {
            let mut writer = RedactingWriter {
                inner: &mut output,
                redactor,
                line: Vec::new(),
            };
            // The secret arrives split across two writes.
            writer.write_all(b"value: planted-test-").unwrap();
            writer.write_all(b"secret-4f9c2a\n").unwrap();
        }

        assert_eq!(
            String::from_utf8(output).unwrap(),
            format!("value: {REDACTED}\n")
        );
    }
}
