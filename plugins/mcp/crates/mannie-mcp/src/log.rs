//! The log of each server (plugins/mcp/docs/README.md, Show log): the last
//! [`MAX_LOG_BYTES`] of its error output and of what happened to its
//! connections, in memory only, with the server's secrets replaced.
//!
//! The error output is the server's own text, which the settings show as it
//! is. What happened to a connection is an event with a code, which the UI
//! words from its catalog (ADR-0019).

use std::collections::VecDeque;

use mannie_plugin_sdk::contracts::{Secret, UtcMillis};

use crate::commands::{LogEntry, LogEvent};

/// The most bytes of a server's log. The oldest entries leave first.
pub(crate) const MAX_LOG_BYTES: usize = 64 * 1024;

/// The bytes that an event counts toward the limit.
const EVENT_BYTES: usize = 64;

/// The text that replaces a secret.
pub(crate) const REDACTED: &str = "[REDACTED]";

/// The shortest secret that the plugin finds and replaces in a server's
/// output, as the SSH plugin does with its passwords. A shorter one could
/// match ordinary text, so it is left as it is.
pub(crate) const MIN_REDACTED_BYTES: usize = 8;

/// The values that a server's output must not show: its secrets and its
/// OAuth tokens, longest first, so a secret inside another is replaced with
/// it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Redactions {
    values: Vec<Secret>,
}

impl Redactions {
    /// The values among `secrets` that are long enough to replace.
    pub(crate) fn new(secrets: impl IntoIterator<Item = Secret>) -> Self {
        let mut values: Vec<Secret> = secrets
            .into_iter()
            .filter(|secret| secret.expose().len() >= MIN_REDACTED_BYTES)
            .collect();
        values.sort_by_key(|secret| std::cmp::Reverse(secret.expose().len()));
        Self { values }
    }

    /// The same values with `secrets` among them.
    pub(crate) fn with(&self, secrets: impl IntoIterator<Item = Secret>) -> Self {
        Self::new(self.values.iter().cloned().chain(secrets))
    }

    /// `text` with every value replaced.
    pub(crate) fn apply(&self, text: &str) -> String {
        let mut redacted = text.to_owned();
        for value in &self.values {
            if redacted.contains(value.expose()) {
                redacted = redacted.replace(value.expose(), REDACTED);
            }
        }
        redacted
    }

    /// Whether `text` holds one of the values.
    pub(crate) fn holds(&self, text: &str) -> bool {
        self.values
            .iter()
            .any(|value| text.contains(value.expose()))
    }

    /// Replaces the values in every string of `value`, such as the
    /// descriptions and defaults of a tool's schema. Keys stay as they are,
    /// since a schema's keys are its properties' names.
    pub(crate) fn apply_to_json(&self, value: &mut serde_json::Value) {
        match value {
            serde_json::Value::String(text) if self.holds(text) => *text = self.apply(text),
            serde_json::Value::Array(items) => {
                for item in items {
                    self.apply_to_json(item);
                }
            }
            serde_json::Value::Object(entries) => {
                for entry in entries.values_mut() {
                    self.apply_to_json(entry);
                }
            }
            _other => {}
        }
    }
}

/// A server's log.
#[derive(Debug, Default)]
pub(crate) struct Log {
    entries: VecDeque<(LogEntry, usize)>,
    bytes: usize,
    /// Whether older entries left.
    cut: bool,
}

impl Log {
    /// Adds a line of the server's error output, with its secrets replaced.
    pub(crate) fn output(&mut self, at: UtcMillis, line: &str, redactions: &Redactions) {
        let text = redactions.apply(line);
        let bytes = text.len().max(1);
        self.push(LogEntry::Output { at, text }, bytes);
    }

    /// Adds an event of the server's connections.
    pub(crate) fn event(&mut self, at: UtcMillis, event: LogEvent) {
        self.push(LogEntry::Event { at, event }, EVENT_BYTES);
    }

    fn push(&mut self, entry: LogEntry, bytes: usize) {
        self.entries.push_back((entry, bytes));
        self.bytes += bytes;
        while self.bytes > MAX_LOG_BYTES {
            let Some((_entry, bytes)) = self.entries.pop_front() else {
                break;
            };
            self.bytes -= bytes;
            self.cut = true;
        }
    }

    /// The entries, oldest first, and whether older ones left.
    pub(crate) fn entries(&self) -> (Vec<LogEntry>, bool) {
        let entries = self
            .entries
            .iter()
            .map(|(entry, _bytes)| entry.clone())
            .collect();
        (entries, self.cut)
    }
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::{Secret, UtcMillis};

    use super::{Log, MAX_LOG_BYTES, Redactions};
    use crate::commands::{LogEntry, LogEvent};

    #[test]
    fn secrets_are_replaced_longest_first_and_short_ones_are_left() {
        let redactions = Redactions::new([
            Secret::new("planted-token".to_owned()),
            Secret::new("planted-token-long".to_owned()),
            Secret::new("short".to_owned()),
        ]);

        assert_eq!(
            redactions.apply("a planted-token-long and planted-token, short"),
            "a [REDACTED] and [REDACTED], short"
        );
    }

    #[test]
    fn every_string_of_a_json_value_is_redacted_and_its_keys_stay() {
        let redactions = Redactions::new([Secret::new("planted-token".to_owned())]);
        let mut schema = serde_json::json!({
            "description": "Uses planted-token.",
            "properties": {
                "mode": { "enum": ["plain", "planted-token"], "default": "planted-token" },
                "count": { "type": "integer", "minimum": 1 },
            },
        });

        redactions.apply_to_json(&mut schema);

        assert!(redactions.holds("with planted-token inside"));
        assert!(!redactions.holds("without it"));
        assert_eq!(
            schema,
            serde_json::json!({
                "description": "Uses [REDACTED].",
                "properties": {
                    "mode": { "enum": ["plain", "[REDACTED]"], "default": "[REDACTED]" },
                    "count": { "type": "integer", "minimum": 1 },
                },
            })
        );
    }

    #[test]
    fn the_log_keeps_its_newest_entries_within_its_limit() {
        let mut log = Log::default();
        let line = "x".repeat(1024);
        for _ in 0..80 {
            log.output(UtcMillis::from_millis(1), &line, &Redactions::default());
        }
        log.event(UtcMillis::from_millis(2), LogEvent::Stopped);

        let (entries, cut) = log.entries();

        assert!(cut);
        let bytes: usize = entries
            .iter()
            .map(|entry| match entry {
                LogEntry::Output { text, .. } => text.len(),
                LogEntry::Event { .. } => 64,
            })
            .sum();
        assert!(bytes <= MAX_LOG_BYTES);
        assert!(matches!(
            entries.last(),
            Some(LogEntry::Event {
                event: LogEvent::Stopped,
                ..
            })
        ));
    }
}
