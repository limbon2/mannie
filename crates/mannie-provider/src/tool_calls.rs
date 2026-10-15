//! Tool calls as an adapter assembles them from a stream, checked before the
//! runtime sees them (ADR-0011, decision 4): every call has a name and an ID
//! that is unique within the model call, and its arguments stay within
//! [`MAX_ARGUMENTS_BYTES`].
//!
//! A call whose arguments grow beyond the limit, or that the output limit cut
//! off, is started but never completed, so the runtime tells the model that
//! the call was cut off.

use std::collections::HashSet;

use mannie_types::tools::MAX_ARGUMENTS_BYTES;

use crate::provider::ProviderEvent;

/// One call while it streams.
#[derive(Debug)]
struct PendingCall {
    /// How the adapter finds the call again: the index of a Chat Completions
    /// delta, or the item ID of a Responses event.
    key: String,
    /// The ID from the stream, until the call starts.
    raw_id: Option<String>,
    /// The ID the runtime receives, once the call started.
    call_id: Option<String>,
    name: Option<String>,
    arguments: String,
    oversized: bool,
    ended: bool,
}

/// The tool calls of one model call.
#[derive(Debug, Default)]
pub(crate) struct ToolCalls {
    calls: Vec<PendingCall>,
    used_ids: HashSet<String>,
    generated: u32,
    /// Calls that arrived complete at once, for their keys.
    completed: u32,
}

impl ToolCalls {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn find(&mut self, key: &str) -> Option<&mut PendingCall> {
        self.calls.iter_mut().find(|call| call.key == key)
    }

    /// Records what a chunk says about a call: its ID, its name and more of
    /// its arguments. The call starts once it has a name.
    pub(crate) fn update(
        &mut self,
        key: &str,
        id: Option<&str>,
        name: Option<&str>,
        arguments: &str,
        out: &mut Vec<ProviderEvent>,
    ) {
        if self.find(key).is_none() {
            self.calls.push(PendingCall {
                key: key.to_owned(),
                raw_id: None,
                call_id: None,
                name: None,
                arguments: String::new(),
                oversized: false,
                ended: false,
            });
        }
        let index = self
            .calls
            .iter()
            .position(|call| call.key == key)
            .unwrap_or_default();
        {
            let call = &mut self.calls[index];
            if let Some(id) = id.filter(|id| !id.is_empty())
                && call.raw_id.is_none()
            {
                call.raw_id = Some(id.to_owned());
            }
            if let Some(name) = name.filter(|name| !name.is_empty())
                && call.name.is_none()
            {
                call.name = Some(name.to_owned());
            }
            append_arguments(call, arguments);
        }
        self.start_if_named(index, out);
    }

    /// Replaces a call's arguments with the complete ones, as the Responses
    /// API sends them when they are done.
    pub(crate) fn set_arguments(&mut self, key: &str, arguments: &str) {
        if let Some(call) = self.find(key) {
            call.arguments.clear();
            call.oversized = false;
            append_arguments(call, arguments);
        }
    }

    /// Ends one call: it is complete, unless its arguments were too large.
    pub(crate) fn end(&mut self, key: &str, out: &mut Vec<ProviderEvent>) {
        if let Some(index) = self.calls.iter().position(|call| call.key == key) {
            Self::end_call(&mut self.calls[index], out);
        }
    }

    /// Ends every call that has not ended. When the output limit cut the
    /// answer, the call that streamed last is incomplete and stays open.
    pub(crate) fn end_all(&mut self, cut_off: bool, out: &mut Vec<ProviderEvent>) {
        let last_started = self.calls.iter().rposition(|call| call.call_id.is_some());
        for (index, call) in self.calls.iter_mut().enumerate() {
            let is_cut = cut_off && Some(index) == last_started;
            if !is_cut {
                Self::end_call(call, out);
            }
        }
    }

    /// Adds a call that arrived complete at once, as a quirk finds one in the
    /// text.
    pub(crate) fn complete(&mut self, name: &str, arguments: &str, out: &mut Vec<ProviderEvent>) {
        self.completed += 1;
        let key = format!("complete:{}", self.completed);
        self.update(&key, None, Some(name), arguments, out);
        self.end(&key, out);
    }

    fn start_if_named(&mut self, index: usize, out: &mut Vec<ProviderEvent>) {
        let call = &self.calls[index];
        let Some(name) = call.name.clone() else {
            return;
        };
        if call.call_id.is_some() {
            return;
        }
        let raw = call.raw_id.clone();
        let call_id = match raw {
            Some(raw) if !self.used_ids.contains(&raw) => raw,
            _ => self.generated_id(),
        };
        self.used_ids.insert(call_id.clone());
        self.calls[index].call_id = Some(call_id.clone());
        out.push(ProviderEvent::ToolCallStarted { call_id, name });
    }

    fn generated_id(&mut self) -> String {
        loop {
            self.generated += 1;
            let candidate = format!("call_mannie_{}", self.generated);
            if !self.used_ids.contains(&candidate) {
                return candidate;
            }
        }
    }

    fn end_call(call: &mut PendingCall, out: &mut Vec<ProviderEvent>) {
        if call.ended {
            return;
        }
        call.ended = true;
        let (Some(call_id), Some(name)) = (call.call_id.clone(), call.name.clone()) else {
            tracing::warn!("a server sent a tool call without a name; it is dropped");
            return;
        };
        if call.oversized {
            tracing::warn!(
                tool = %name,
                "a tool call's arguments are larger than Mannie accepts; the call counts as cut off"
            );
            return;
        }
        out.push(ProviderEvent::ToolCall {
            call_id,
            name,
            arguments: std::mem::take(&mut call.arguments),
        });
    }
}

fn append_arguments(call: &mut PendingCall, arguments: &str) {
    if call.oversized {
        return;
    }
    if call.arguments.len() + arguments.len() > MAX_ARGUMENTS_BYTES {
        call.oversized = true;
        call.arguments.clear();
        return;
    }
    call.arguments.push_str(arguments);
}

#[cfg(test)]
mod tests {
    use mannie_types::tools::MAX_ARGUMENTS_BYTES;

    use super::ToolCalls;
    use crate::provider::ProviderEvent;

    fn started(call_id: &str, name: &str) -> ProviderEvent {
        ProviderEvent::ToolCallStarted {
            call_id: call_id.to_owned(),
            name: name.to_owned(),
        }
    }

    fn complete(call_id: &str, name: &str, arguments: &str) -> ProviderEvent {
        ProviderEvent::ToolCall {
            call_id: call_id.to_owned(),
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        }
    }

    #[test]
    fn a_call_starts_with_its_name_and_completes_with_its_arguments() {
        let mut calls = ToolCalls::new();
        let mut out = Vec::new();

        calls.update("0", Some("call_a"), None, "", &mut out);
        calls.update("0", None, Some("read_file"), "{\"pa", &mut out);
        calls.update("0", None, None, "th\":\"a\"}", &mut out);
        calls.end_all(false, &mut out);

        assert_eq!(
            out,
            [
                started("call_a", "read_file"),
                complete("call_a", "read_file", "{\"path\":\"a\"}"),
            ]
        );
    }

    #[test]
    fn missing_and_repeated_ids_are_replaced() {
        let mut calls = ToolCalls::new();
        let mut out = Vec::new();

        calls.update("0", None, Some("list_directory"), "{}", &mut out);
        calls.update("1", Some("same"), Some("read_file"), "{}", &mut out);
        calls.update("2", Some("same"), Some("read_file"), "{}", &mut out);
        calls.end_all(false, &mut out);

        let ids: Vec<_> = out
            .iter()
            .filter_map(|event| match event {
                ProviderEvent::ToolCallStarted { call_id, .. } => Some(call_id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(ids, ["call_mannie_1", "same", "call_mannie_2"]);
    }

    #[test]
    fn a_call_without_a_name_never_reaches_the_runtime() {
        let mut calls = ToolCalls::new();
        let mut out = Vec::new();

        calls.update("0", Some("call_a"), None, "{}", &mut out);
        calls.end_all(false, &mut out);

        assert_eq!(out, []);
    }

    #[test]
    fn the_output_limit_leaves_the_last_call_incomplete() {
        let mut calls = ToolCalls::new();
        let mut out = Vec::new();

        calls.update("0", Some("a"), Some("read_file"), "{}", &mut out);
        calls.update("1", Some("b"), Some("write_file"), "{\"path\":", &mut out);
        calls.end_all(true, &mut out);

        assert_eq!(
            out,
            [
                started("a", "read_file"),
                started("b", "write_file"),
                complete("a", "read_file", "{}"),
            ]
        );
    }

    #[test]
    fn oversized_arguments_leave_the_call_incomplete() {
        let mut calls = ToolCalls::new();
        let mut out = Vec::new();
        let chunk = "x".repeat(MAX_ARGUMENTS_BYTES / 2 + 1);

        calls.update("0", Some("a"), Some("write_file"), &chunk, &mut out);
        calls.update("0", None, None, &chunk, &mut out);
        calls.end_all(false, &mut out);

        assert_eq!(out, [started("a", "write_file")]);
    }
}
