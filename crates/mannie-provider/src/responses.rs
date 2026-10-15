//! The adapter for the OpenAI Responses API (ADR-0011, decision 1).
//!
//! Requests are stateless: `store` is false, and the conversation goes in
//! `input` each time. With a reasoning effort, the request asks for reasoning
//! summaries, which the owner reads, and for encrypted reasoning, which goes
//! back to the model on the next call of the same binding.

use mannie_types::provider::ApiInterface;
use mannie_types::run::{FinishReason, Usage};
use serde_json::{Map, Value, json};

use crate::errors::{self, Context, ErrorDetails};
use crate::provider::{
    Content, EventStream, ModelRequest, Provider, ProviderError, ProviderEvent, Role, Target,
    ToolChoice,
};
use crate::quirks::Quirks;
use crate::sse::SseEvent;
use crate::stream::{self, Call, Flow, Translator};
use crate::tool_calls::ToolCalls;
use crate::transport::{HttpClient, endpoint};

/// The Responses API adapter.
#[derive(Debug, Clone)]
pub struct ResponsesAdapter {
    client: HttpClient,
}

impl ResponsesAdapter {
    /// An adapter that calls through `client`.
    pub fn new(client: HttpClient) -> Self {
        Self { client }
    }
}

impl Provider for ResponsesAdapter {
    fn interface(&self) -> ApiInterface {
        ApiInterface::Responses
    }

    fn stream(&self, target: &Target, request: ModelRequest) -> EventStream {
        let body = request_body(&request, &target.quirks);
        let url = endpoint(&target.base_url, &["responses"]);
        let http = self.client.post(
            url,
            target.credential.as_ref(),
            body.to_string().into_bytes(),
        );
        let translator = ResponsesTranslator::new(&self.client, &request.model, target.quirks);
        stream::drive(
            self.client.clone(),
            Call {
                request: http,
                model: request.model,
                quirks: target.quirks,
                credential: target.credential.clone(),
            },
            translator,
        )
    }
}

/// The request body of a call.
pub(crate) fn request_body(request: &ModelRequest, quirks: &Quirks) -> Value {
    let mut input = Vec::new();
    for message in &request.messages {
        let (role, text_type) = match message.role {
            Role::User | Role::Tool => ("user", "input_text"),
            Role::Assistant => ("assistant", "output_text"),
        };
        let mut parts = Vec::new();
        for content in &message.content {
            match content {
                Content::Text(text) => parts.push(json!({ "type": text_type, "text": text })),
                // Reasoning items precede the message or the calls they led
                // to; the API wants them back with the calls when tools are
                // used.
                Content::ProviderItem(item) => match serde_json::from_str::<Value>(item) {
                    Ok(item) => input.push(item),
                    Err(error) => {
                        tracing::warn!(%error, "skipped a stored reasoning item that is not JSON");
                    }
                },
                Content::ToolCall {
                    call_id,
                    name,
                    arguments,
                } => {
                    // The text written before a call comes before it. The
                    // item ID is left out, because it would tie the call to a
                    // reasoning item that may not be sent.
                    flush_message(&mut input, role, &mut parts);
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call_id,
                        "name": name,
                        "arguments": arguments,
                    }));
                }
                Content::ToolResult { call_id, output } => {
                    flush_message(&mut input, role, &mut parts);
                    input.push(json!({
                        "type": "function_call_output",
                        "call_id": call_id,
                        "output": output,
                    }));
                }
            }
        }
        flush_message(&mut input, role, &mut parts);
    }
    let mut body = Map::new();
    body.insert("model".to_owned(), json!(request.model));
    if !request.instructions.is_empty() {
        body.insert("instructions".to_owned(), json!(request.instructions));
    }
    body.insert("input".to_owned(), Value::Array(input));
    body.insert("stream".to_owned(), json!(true));
    body.insert("store".to_owned(), json!(false));
    body.insert(
        "max_output_tokens".to_owned(),
        json!(request.max_output_tokens),
    );
    if let Some(effort) = request.reasoning_effort.api_name() {
        body.insert(
            "reasoning".to_owned(),
            json!({ "effort": effort, "summary": "auto" }),
        );
        body.insert("include".to_owned(), json!(["reasoning.encrypted_content"]));
    }
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                    "strict": quirks.tools.strict,
                })
            })
            .collect();
        body.insert("tools".to_owned(), Value::Array(tools));
        let choice = match request.tool_choice {
            ToolChoice::Auto => "auto",
            ToolChoice::None => "none",
        };
        body.insert("tool_choice".to_owned(), json!(choice));
    }
    Value::Object(body)
}

/// Adds the text parts collected so far as one message.
fn flush_message(input: &mut Vec<Value>, role: &str, parts: &mut Vec<Value>) {
    if !parts.is_empty() {
        input.push(json!({ "role": role, "content": std::mem::take(parts) }));
    }
}

/// Turns the events of the Responses API into normalized events.
struct ResponsesTranslator {
    client: HttpClient,
    model: String,
    quirks: Quirks,
    /// The reasoning item and summary part that text last arrived for, so
    /// that parts of a summary are separated by a blank line.
    last_summary: Option<(String, u64)>,
    /// The function calls of the response, by their item.
    calls: ToolCalls,
}

impl ResponsesTranslator {
    fn new(client: &HttpClient, model: &str, quirks: Quirks) -> Self {
        Self {
            client: client.clone(),
            model: model.to_owned(),
            quirks,
            last_summary: None,
            calls: ToolCalls::new(),
        }
    }

    /// Records a function call item as it was added or finished, and returns
    /// the key it is tracked under.
    fn function_call(&mut self, item: &Value, out: &mut Vec<ProviderEvent>) -> Option<String> {
        if item.get("type").and_then(Value::as_str) != Some("function_call") {
            return None;
        }
        let call_id = item.get("call_id").and_then(Value::as_str);
        let key = item
            .get("id")
            .and_then(Value::as_str)
            .or(call_id)?
            .to_owned();
        let name = item.get("name").and_then(Value::as_str);
        self.calls.update(&key, call_id, name, "", out);
        if let Some(arguments) = item.get("arguments").and_then(Value::as_str) {
            self.calls.set_arguments(&key, arguments);
        }
        Some(key)
    }

    fn context(&self) -> Context<'_> {
        Context {
            model: &self.model,
            quirks: &self.quirks,
            now: self.client.clock().now(),
        }
    }

    fn summary_delta(&mut self, data: &Value, out: &mut Vec<ProviderEvent>) {
        let Some(delta) = data.get("delta").and_then(Value::as_str) else {
            return;
        };
        let item = data
            .get("item_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let index = data
            .get("summary_index")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let position = (item, index);
        let starts_new_part = self
            .last_summary
            .as_ref()
            .is_some_and(|last| *last != position);
        if starts_new_part {
            out.push(ProviderEvent::ReasoningDelta("\n\n".to_owned()));
        }
        self.last_summary = Some(position);
        out.push(ProviderEvent::ReasoningDelta(delta.to_owned()));
    }

    fn finish(response: &Value, reason: FinishReason, out: &mut Vec<ProviderEvent>) {
        if let Some(usage) = response.get("usage").and_then(usage_from) {
            out.push(ProviderEvent::Usage(usage));
        }
        out.push(ProviderEvent::Finished(reason));
    }
}

impl Translator for ResponsesTranslator {
    fn event(
        &mut self,
        event: SseEvent,
        out: &mut Vec<ProviderEvent>,
    ) -> Result<Flow, ProviderError> {
        let data: Value = serde_json::from_str(&event.data).map_err(|_not_json| {
            ProviderError::Permanent(mannie_types::run::RunFailure::InvalidResponse)
        })?;
        // The `type` in the data names the event; proxies sometimes drop the
        // `event` line.
        let kind = data
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(event.event.as_str())
            .to_owned();
        match kind.as_str() {
            "response.output_text.delta" => {
                if let Some(delta) = data.get("delta").and_then(Value::as_str) {
                    out.push(ProviderEvent::TextDelta(delta.to_owned()));
                }
            }
            "response.reasoning_summary_text.delta" => self.summary_delta(&data, out),
            "response.reasoning_text.delta" => {
                if let Some(delta) = data.get("delta").and_then(Value::as_str) {
                    out.push(ProviderEvent::ReasoningDelta(delta.to_owned()));
                }
            }
            "response.output_item.added" => {
                if let Some(item) = data.get("item") {
                    self.function_call(item, out);
                }
            }
            "response.function_call_arguments.delta" => {
                let key = data.get("item_id").and_then(Value::as_str);
                let delta = data.get("delta").and_then(Value::as_str);
                if let (Some(key), Some(delta)) = (key, delta) {
                    self.calls.update(key, None, None, delta, out);
                }
            }
            "response.function_call_arguments.done" => {
                let key = data.get("item_id").and_then(Value::as_str);
                let arguments = data.get("arguments").and_then(Value::as_str);
                if let (Some(key), Some(arguments)) = (key, arguments) {
                    self.calls.set_arguments(key, arguments);
                }
            }
            "response.output_item.done" => {
                let item = data.get("item").unwrap_or(&Value::Null);
                if let Some(reasoning) = reasoning_item(item) {
                    out.push(ProviderEvent::ProviderItem(reasoning.to_string()));
                } else if let Some(key) = self.function_call(item, out) {
                    self.calls.end(&key, out);
                }
            }
            "response.completed" => {
                let response = data.get("response").unwrap_or(&Value::Null);
                self.calls.end_all(false, out);
                Self::finish(response, FinishReason::Stop, out);
                return Ok(Flow::Finished);
            }
            "response.incomplete" => {
                let response = data.get("response").unwrap_or(&Value::Null);
                let reason = response
                    .pointer("/incomplete_details/reason")
                    .and_then(Value::as_str);
                let finish = match reason {
                    Some("max_output_tokens") => FinishReason::OutputLimit,
                    Some("content_filter") => FinishReason::ContentFilter,
                    _ => FinishReason::Other,
                };
                Self::finish(response, finish, out);
                return Ok(Flow::Finished);
            }
            "response.failed" => {
                let error = data.pointer("/response/error").unwrap_or(&Value::Null);
                return Err(errors::classify_stream_error(
                    &ErrorDetails::from_value(error),
                    self.context(),
                ));
            }
            "error" => {
                return Err(errors::classify_stream_error(
                    &ErrorDetails::from_value(&data),
                    self.context(),
                ));
            }
            // Other events describe the response's structure, which the
            // deltas and the finishing events already carry.
            _ => {}
        }
        Ok(Flow::Continue)
    }

    fn end(&mut self, _out: &mut Vec<ProviderEvent>) -> Result<(), ProviderError> {
        // A response always ends with completed, incomplete or failed.
        Err(stream::broken_stream())
    }
}

/// The part of a reasoning item that goes back to the model: its ID, its
/// encrypted content and its summary. Items without encrypted content are of
/// no use on the next call.
fn reasoning_item(item: &Value) -> Option<Value> {
    let is_reasoning = item.get("type").and_then(Value::as_str) == Some("reasoning");
    let encrypted = item.get("encrypted_content").and_then(Value::as_str)?;
    if !is_reasoning {
        return None;
    }
    let mut kept = Map::new();
    kept.insert("type".to_owned(), json!("reasoning"));
    if let Some(id) = item.get("id") {
        kept.insert("id".to_owned(), id.clone());
    }
    kept.insert("encrypted_content".to_owned(), json!(encrypted));
    kept.insert(
        "summary".to_owned(),
        item.get("summary").cloned().unwrap_or_else(|| json!([])),
    );
    Some(Value::Object(kept))
}

/// Reads the Responses API's usage object.
fn usage_from(usage: &Value) -> Option<Usage> {
    let count = |pointer: &str| {
        usage
            .pointer(pointer)
            .and_then(Value::as_u64)
            .map(|value| u32::try_from(value).unwrap_or(u32::MAX))
    };
    Some(Usage {
        input: count("/input_tokens")?,
        output: count("/output_tokens")?,
        cached_input: count("/input_tokens_details/cached_tokens"),
        reasoning: count("/output_tokens_details/reasoning_tokens"),
    })
}

#[cfg(test)]
mod tests;
