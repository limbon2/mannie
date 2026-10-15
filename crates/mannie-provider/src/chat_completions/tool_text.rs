//! The quirk for tool calls that a model writes as text: a `<tool_call>`
//! block with a JSON object of `name` and `arguments`, as models in the Hermes
//! format write it when the server parses no tool calls
//! (docs/features/system-tools/tool-calls.md, quirk profiles).
//!
//! The splitter passes text through and holds back only what may be the start
//! of a block, so that text still streams. A block whose content is not such a
//! JSON object stays text.

use serde_json::Value;

use crate::provider::ProviderEvent;
use crate::tool_calls::ToolCalls;

const OPEN: &str = "<tool_call>";
const CLOSE: &str = "</tool_call>";

/// Whether the text a splitter handles is the answer or the reasoning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stream {
    Text,
    Reasoning,
}

/// Finds tool calls written as text in one stream of text.
#[derive(Debug)]
pub(crate) struct ToolTextSplitter {
    enabled: bool,
    stream: Stream,
    /// Text held back: the start of a possible block, or a whole block that
    /// is not closed yet.
    held: String,
}

impl ToolTextSplitter {
    pub(crate) fn new(enabled: bool, stream: Stream) -> Self {
        Self {
            enabled,
            stream,
            held: String::new(),
        }
    }

    fn emit(&self, text: String, out: &mut Vec<ProviderEvent>) {
        if text.is_empty() {
            return;
        }
        out.push(match self.stream {
            Stream::Text => ProviderEvent::TextDelta(text),
            Stream::Reasoning => ProviderEvent::ReasoningDelta(text),
        });
    }

    pub(crate) fn push(&mut self, text: &str, calls: &mut ToolCalls, out: &mut Vec<ProviderEvent>) {
        if !self.enabled {
            self.emit(text.to_owned(), out);
            return;
        }
        self.held.push_str(text);
        loop {
            let Some(start) = self.held.find(OPEN) else {
                // Keep back an end that may start a block.
                let keep = partial_suffix(&self.held, OPEN);
                let cut = self.held.len() - keep;
                let before: String = self.held.drain(..cut).collect();
                self.emit(before, out);
                return;
            };
            let Some(end) = self.held[start..].find(CLOSE).map(|end| start + end) else {
                // A block that is not closed yet; the text before it goes out.
                let before: String = self.held.drain(..start).collect();
                self.emit(before, out);
                return;
            };
            let block_end = end + CLOSE.len();
            let body = self.held[start + OPEN.len()..end].to_owned();
            if let Some((name, arguments)) = parse_call(&body) {
                let before: String = self.held.drain(..start).collect();
                self.emit(before, out);
                self.held.drain(..block_end - start);
                calls.complete(&name, &arguments, out);
            } else {
                // Not a call: the block is text like any other.
                let block: String = self.held.drain(..block_end).collect();
                self.emit(block, out);
            }
        }
    }

    /// Emits what is held back when the stream ends: an unclosed block stays
    /// text.
    pub(crate) fn flush(&mut self, out: &mut Vec<ProviderEvent>) {
        let held = std::mem::take(&mut self.held);
        self.emit(held, out);
    }
}

/// The name and the arguments, as JSON text, of a block's content.
fn parse_call(body: &str) -> Option<(String, String)> {
    let value: Value = serde_json::from_str(body.trim()).ok()?;
    let name = value.get("name")?.as_str()?.trim();
    if name.is_empty() {
        return None;
    }
    let arguments = match value.get("arguments") {
        // Some models write the arguments as a JSON string.
        Some(Value::String(text)) => text.clone(),
        Some(arguments @ Value::Object(_)) => arguments.to_string(),
        None => "{}".to_owned(),
        Some(_) => return None,
    };
    Some((name.to_owned(), arguments))
}

/// The length of the longest end of `text` that is a start of `tag`.
fn partial_suffix(text: &str, tag: &str) -> usize {
    (1..tag.len())
        .rev()
        .find(|&length| text.ends_with(&tag[..length]))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{Stream, ToolTextSplitter};
    use crate::provider::ProviderEvent;
    use crate::tool_calls::ToolCalls;

    const ANSWER: &str = "Let me look.\n<tool_call>\n{\"name\": \"read_file\", \"arguments\": {\"path\": \"a.txt\"}}\n</tool_call>";

    fn split(pieces: &[&str], enabled: bool) -> Vec<ProviderEvent> {
        let mut splitter = ToolTextSplitter::new(enabled, Stream::Text);
        let mut calls = ToolCalls::new();
        let mut out = Vec::new();
        for piece in pieces {
            splitter.push(piece, &mut calls, &mut out);
        }
        splitter.flush(&mut out);
        out
    }

    fn text_of(events: &[ProviderEvent]) -> String {
        events
            .iter()
            .filter_map(|event| match event {
                ProviderEvent::TextDelta(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn calls_of(events: &[ProviderEvent]) -> Vec<(String, String)> {
        events
            .iter()
            .filter_map(|event| match event {
                ProviderEvent::ToolCall {
                    name, arguments, ..
                } => Some((name.clone(), arguments.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_block_becomes_a_tool_call_and_the_text_before_it_stays() {
        let events = split(&[ANSWER], true);

        assert_eq!(text_of(&events), "Let me look.\n");
        assert_eq!(
            calls_of(&events),
            [("read_file".to_owned(), "{\"path\":\"a.txt\"}".to_owned())]
        );
    }

    #[test]
    fn without_the_quirk_or_with_another_block_the_text_stays() {
        let disabled = split(&[ANSWER], false);
        let not_a_call = split(&["<tool_call>not json</tool_call> after"], true);
        let unclosed = split(&["<tool_call>{\"name\": \"x\""], true);

        assert_eq!(text_of(&disabled), ANSWER);
        assert_eq!(calls_of(&disabled), []);
        assert_eq!(
            text_of(&not_a_call),
            "<tool_call>not json</tool_call> after"
        );
        assert_eq!(text_of(&unclosed), "<tool_call>{\"name\": \"x\"");
    }

    proptest! {
        #[test]
        fn any_split_of_the_text_gives_the_same_events(cuts in proptest::collection::vec(0..ANSWER.len(), 0..12)) {
            let mut cuts: Vec<usize> = cuts.into_iter().filter(|cut| ANSWER.is_char_boundary(*cut)).collect();
            cuts.sort_unstable();
            cuts.dedup();
            let mut pieces = Vec::new();
            let mut start = 0;
            for cut in cuts {
                pieces.push(&ANSWER[start..cut]);
                start = cut;
            }
            pieces.push(&ANSWER[start..]);

            let events = split(&pieces, true);

            prop_assert_eq!(text_of(&events), "Let me look.\n");
            prop_assert_eq!(calls_of(&events).len(), 1);
        }
    }
}
