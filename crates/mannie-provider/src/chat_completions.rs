//! The adapter for Chat Completions on OpenAI and compatible servers: vLLM,
//! SGLang, LM Studio, Ollama and OpenRouter (ADR-0011, decision 1). The
//! server's quirk profile decides the fields that differ between them.

use mannie_types::provider::ApiInterface;
use mannie_types::run::{FinishReason, RunFailure, Usage};
use serde_json::{Map, Value, json};

use crate::errors::{self, Context, ErrorDetails};
use crate::provider::{
    Content, EventStream, ImageInput, ModelRequest, Provider, ProviderError, ProviderEvent, Role,
    Target, ToolChoice,
};
use crate::quirks::{EffortShape, OutputLimitField, Quirks};
use crate::sse::SseEvent;
use crate::stream::{self, Call, Flow, Translator};
use crate::tool_calls::ToolCalls;
use crate::transport::{HttpClient, endpoint};

mod tool_text;

use tool_text::{Stream as TextStream, ToolTextSplitter};

/// The Chat Completions adapter.
#[derive(Debug, Clone)]
pub struct ChatCompletionsAdapter {
    client: HttpClient,
}

impl ChatCompletionsAdapter {
    /// An adapter that calls through `client`.
    pub fn new(client: HttpClient) -> Self {
        Self { client }
    }
}

impl Provider for ChatCompletionsAdapter {
    fn interface(&self) -> ApiInterface {
        ApiInterface::ChatCompletions
    }

    fn stream(&self, target: &Target, request: ModelRequest) -> EventStream {
        let body = request_body(&request, &target.quirks);
        let url = endpoint(&target.base_url, &["chat", "completions"]);
        let http = self.client.post(
            url,
            target.credential.as_ref(),
            body.to_string().into_bytes(),
        );
        let translator = ChatTranslator::new(
            &self.client,
            &request.model,
            target.quirks,
            !request.tools.is_empty(),
        );
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

/// A message as Chat Completions takes it.
enum Wire<'a> {
    /// Text and images of the owner, or text of the model.
    Text {
        role: Role,
        parts: Vec<WirePart<'a>>,
    },
    /// The model's tool calls, with the text it wrote before them.
    Calls { text: String, calls: Vec<Value> },
    /// The result of one tool call.
    Result { call_id: String, output: String },
}

/// One element of a text message, in order.
enum WirePart<'a> {
    Text(&'a str),
    Image(&'a ImageInput),
}

/// The messages of a request in the shape Chat Completions takes.
fn wire_messages(request: &ModelRequest) -> Vec<Wire<'_>> {
    let mut wire: Vec<Wire<'_>> = Vec::new();
    for message in &request.messages {
        let mut parts = Vec::new();
        let mut calls = Vec::new();
        for content in &message.content {
            match content {
                Content::Text(text) => parts.push(WirePart::Text(text)),
                Content::Image(image) => parts.push(WirePart::Image(image)),
                // Provider-private data belongs to the Responses API.
                Content::ProviderItem(_) => {}
                Content::ToolCall {
                    call_id,
                    name,
                    arguments,
                } => calls.push(json!({
                    "id": call_id,
                    "type": "function",
                    "function": { "name": name, "arguments": arguments },
                })),
                // Each result is a message of its own, and is never merged or
                // dropped: a call without its result is refused.
                Content::ToolResult { call_id, output } => wire.push(Wire::Result {
                    call_id: call_id.clone(),
                    output: output.clone(),
                }),
            }
        }
        if !calls.is_empty() {
            // Only the owner's messages hold images, and the model's calls
            // never do.
            wire.push(Wire::Calls {
                text: joined_text(&parts),
                calls,
            });
            continue;
        }
        let has_images = parts.iter().any(|part| matches!(part, WirePart::Image(_)));
        if !has_images && joined_text(&parts).is_empty() {
            continue;
        }
        // Some chat templates refuse two messages of the same role in a row,
        // so consecutive text messages of one role become one message.
        match wire.last_mut() {
            Some(Wire::Text {
                role,
                parts: existing,
            }) if *role == message.role => existing.extend(parts),
            _ => wire.push(Wire::Text {
                role: message.role,
                parts,
            }),
        }
    }
    wire
}

/// The text parts of a message, separated by empty lines.
fn joined_text(parts: &[WirePart<'_>]) -> String {
    let texts: Vec<&str> = parts
        .iter()
        .filter_map(|part| match part {
            WirePart::Text(text) => Some(*text),
            WirePart::Image(_) => None,
        })
        .collect();
    texts.join("\n\n")
}

/// The content of a text message: a string when it holds only text, as every
/// server takes it, and otherwise an array of text and `image_url` parts, in
/// which the text between two images is one part (ADR-0033, decision 5).
fn wire_content(parts: &[WirePart<'_>]) -> Value {
    let has_images = parts.iter().any(|part| matches!(part, WirePart::Image(_)));
    if !has_images {
        return json!(joined_text(parts));
    }
    let mut content = Vec::new();
    let mut texts: Vec<&str> = Vec::new();
    for part in parts {
        match part {
            WirePart::Text(text) => texts.push(text),
            WirePart::Image(image) => {
                push_text(&mut content, &mut texts);
                content.push(json!({
                    "type": "image_url",
                    "image_url": { "url": image.data_url() },
                }));
            }
        }
    }
    push_text(&mut content, &mut texts);
    Value::Array(content)
}

/// Adds the texts collected so far as one text part, unless they are empty.
fn push_text(content: &mut Vec<Value>, texts: &mut Vec<&str>) {
    let text = texts.join("\n\n");
    texts.clear();
    if !text.is_empty() {
        content.push(json!({ "type": "text", "text": text }));
    }
}

/// The request body of a call.
pub(crate) fn request_body(request: &ModelRequest, quirks: &Quirks) -> Value {
    let messages = wire_messages(request);
    let mut wire = Vec::with_capacity(messages.len() + 1);
    if !request.instructions.is_empty() {
        wire.push(json!({ "role": "system", "content": request.instructions }));
    }
    for message in messages {
        wire.push(match message {
            Wire::Text { role, parts } => {
                let role = match role {
                    Role::User | Role::Tool => "user",
                    Role::Assistant => "assistant",
                };
                json!({ "role": role, "content": wire_content(&parts) })
            }
            // Some chat templates fail on a null content, so a call without
            // text has an empty one.
            Wire::Calls { text, calls } => {
                json!({ "role": "assistant", "content": text, "tool_calls": calls })
            }
            Wire::Result { call_id, output } => {
                json!({ "role": "tool", "tool_call_id": call_id, "content": output })
            }
        });
    }
    let mut body = Map::new();
    body.insert("model".to_owned(), json!(request.model));
    body.insert("messages".to_owned(), Value::Array(wire));
    body.insert("stream".to_owned(), json!(true));
    if quirks.stream_options {
        body.insert(
            "stream_options".to_owned(),
            json!({ "include_usage": true }),
        );
    }
    let limit_field = match quirks.output_limit_field {
        OutputLimitField::MaxCompletionTokens => "max_completion_tokens",
        OutputLimitField::MaxTokens => "max_tokens",
    };
    body.insert(limit_field.to_owned(), json!(request.max_output_tokens));
    if let Some(effort) = request.reasoning_effort.api_name() {
        match quirks.effort {
            EffortShape::ReasoningEffort => {
                body.insert("reasoning_effort".to_owned(), json!(effort));
            }
            EffortShape::ReasoningObject => {
                body.insert("reasoning".to_owned(), json!({ "effort": effort }));
            }
            EffortShape::Omitted => {}
        }
    }
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                let mut function = Map::new();
                function.insert("name".to_owned(), json!(tool.name));
                function.insert("description".to_owned(), json!(tool.description));
                function.insert("parameters".to_owned(), tool.parameters.clone());
                if quirks.tools.strict {
                    function.insert("strict".to_owned(), json!(true));
                }
                json!({ "type": "function", "function": function })
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

/// Turns streamed chunks into normalized events.
struct ChatTranslator {
    client: HttpClient,
    model: String,
    quirks: Quirks,
    think: ThinkSplitter,
    /// Tool calls written as text in the answer, when the quirk applies.
    text_calls: ToolTextSplitter,
    /// Tool calls written as text in the reasoning, when the quirk applies.
    reasoning_calls: ToolTextSplitter,
    calls: ToolCalls,
    finish: Option<FinishReason>,
    usage: Option<Usage>,
}

impl ChatTranslator {
    fn new(client: &HttpClient, model: &str, quirks: Quirks, tools_offered: bool) -> Self {
        // A call written as text is a call only when the model had tools.
        let finds_calls_in_text = quirks.tools.calls_in_text && tools_offered;
        Self {
            client: client.clone(),
            model: model.to_owned(),
            quirks,
            think: ThinkSplitter::new(quirks.think_tags),
            text_calls: ToolTextSplitter::new(finds_calls_in_text, TextStream::Text),
            reasoning_calls: ToolTextSplitter::new(finds_calls_in_text, TextStream::Reasoning),
            calls: ToolCalls::new(),
            finish: None,
            usage: None,
        }
    }

    /// Passes text and reasoning through the splitters that find tool calls
    /// written as text.
    fn route(&mut self, events: Vec<ProviderEvent>, out: &mut Vec<ProviderEvent>) {
        for event in events {
            match event {
                ProviderEvent::TextDelta(text) => self.text_calls.push(&text, &mut self.calls, out),
                ProviderEvent::ReasoningDelta(text) => {
                    self.reasoning_calls.push(&text, &mut self.calls, out);
                }
                other => out.push(other),
            }
        }
    }

    /// Reads the tool calls of a delta: by their index, with an ID and a name
    /// in their first chunk and their arguments in pieces.
    fn tool_calls(&mut self, delta: &Value, out: &mut Vec<ProviderEvent>) {
        let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) else {
            return;
        };
        for (position, call) in calls.iter().enumerate() {
            let index = call
                .get("index")
                .and_then(Value::as_u64)
                .unwrap_or(position as u64);
            let function = call.get("function");
            self.calls.update(
                &index.to_string(),
                call.get("id").and_then(Value::as_str),
                function
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str),
                function
                    .and_then(|function| function.get("arguments"))
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                out,
            );
        }
    }

    fn context(&self) -> Context<'_> {
        Context {
            model: &self.model,
            quirks: &self.quirks,
            now: self.client.clock().now(),
        }
    }

    /// Ends the call: the text the splitter held back, the usage, and the end
    /// with its reason. A stream that never gave a finish reason was cut.
    fn close(&mut self, out: &mut Vec<ProviderEvent>) -> Result<(), ProviderError> {
        let Some(finish) = self.finish else {
            return Err(stream::broken_stream());
        };
        let mut held = Vec::new();
        self.think.flush(&mut held);
        self.route(held, out);
        self.text_calls.flush(out);
        self.reasoning_calls.flush(out);
        // At the output limit, the call that streamed last may be incomplete.
        self.calls.end_all(finish == FinishReason::OutputLimit, out);
        if let Some(usage) = self.usage.take() {
            out.push(ProviderEvent::Usage(usage));
        }
        out.push(ProviderEvent::Finished(finish));
        Ok(())
    }
}

impl Translator for ChatTranslator {
    fn event(
        &mut self,
        event: SseEvent,
        out: &mut Vec<ProviderEvent>,
    ) -> Result<Flow, ProviderError> {
        if event.data.trim() == "[DONE]" {
            self.close(out)?;
            return Ok(Flow::Finished);
        }
        let chunk: Value = serde_json::from_str(&event.data)
            .map_err(|_not_json| ProviderError::Permanent(RunFailure::InvalidResponse))?;
        if chunk.get("error").is_some() {
            return Err(errors::classify_stream_error(
                &ErrorDetails::from_value(&chunk),
                self.context(),
            ));
        }
        if let Some(usage) = chunk.get("usage").and_then(usage_from) {
            self.usage = Some(usage);
        }
        let choice = chunk.pointer("/choices/0");
        if let Some(delta) = choice.and_then(|choice| choice.get("delta")) {
            let mut produced = Vec::new();
            for field in ["reasoning_content", "reasoning"] {
                if let Some(reasoning) = delta.get(field).and_then(Value::as_str)
                    && !reasoning.is_empty()
                {
                    produced.push(ProviderEvent::ReasoningDelta(reasoning.to_owned()));
                }
            }
            if let Some(content) = delta.get("content").and_then(Value::as_str) {
                self.think.push(content, &mut produced);
            }
            self.route(produced, out);
            self.tool_calls(delta, out);
        }
        if let Some(reason) = choice
            .and_then(|choice| choice.get("finish_reason"))
            .and_then(Value::as_str)
        {
            self.finish = Some(match reason {
                "stop" => FinishReason::Stop,
                "length" => FinishReason::OutputLimit,
                "content_filter" => FinishReason::ContentFilter,
                _ => FinishReason::Other,
            });
        }
        Ok(Flow::Continue)
    }

    fn end(&mut self, out: &mut Vec<ProviderEvent>) -> Result<(), ProviderError> {
        // Some servers end the stream after the finish reason without
        // `[DONE]`.
        self.close(out)
    }
}

/// Reads the Chat Completions usage object.
fn usage_from(usage: &Value) -> Option<Usage> {
    let count = |pointer: &str| {
        usage
            .pointer(pointer)
            .and_then(Value::as_u64)
            .map(|value| u32::try_from(value).unwrap_or(u32::MAX))
    };
    Some(Usage {
        input: count("/prompt_tokens")?,
        output: count("/completion_tokens")?,
        cached_input: count("/prompt_tokens_details/cached_tokens"),
        reasoning: count("/completion_tokens_details/reasoning_tokens"),
    })
}

const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";

/// Separates reasoning that a model writes between `<think>` and `</think>`
/// at the start of its content from the answer that follows.
#[derive(Debug)]
pub(crate) struct ThinkSplitter {
    state: ThinkState,
    /// Text held back because it may be the start of a tag.
    held: String,
    /// Whether the answer has not started yet after the reasoning, so that
    /// the blank lines between them are dropped.
    answer_pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThinkState {
    /// At the start of the content: it may open with `<think>`.
    Start,
    /// Inside the reasoning.
    Thinking,
    /// After the reasoning, or without any: everything is answer.
    Answering,
}

impl ThinkSplitter {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            state: if enabled {
                ThinkState::Start
            } else {
                ThinkState::Answering
            },
            held: String::new(),
            answer_pending: false,
        }
    }

    pub(crate) fn push(&mut self, text: &str, out: &mut Vec<ProviderEvent>) {
        self.held.push_str(text);
        loop {
            match self.state {
                ThinkState::Answering => {
                    if self.answer_pending {
                        let start = self.held.len() - self.held.trim_start().len();
                        self.held.drain(..start);
                        self.answer_pending = self.held.is_empty();
                    }
                    if !self.held.is_empty() {
                        out.push(ProviderEvent::TextDelta(std::mem::take(&mut self.held)));
                    }
                    return;
                }
                ThinkState::Start => {
                    let trimmed = self.held.trim_start();
                    if let Some(rest) = trimmed.strip_prefix(THINK_OPEN) {
                        self.held = rest.to_owned();
                        self.state = ThinkState::Thinking;
                        continue;
                    }
                    let may_still_open = THINK_OPEN.starts_with(trimmed);
                    if may_still_open {
                        return;
                    }
                    self.state = ThinkState::Answering;
                }
                ThinkState::Thinking => {
                    if let Some(end) = self.held.find(THINK_CLOSE) {
                        let reasoning = self.held[..end].to_owned();
                        let answer = self.held[end + THINK_CLOSE.len()..].to_owned();
                        if !reasoning.is_empty() {
                            out.push(ProviderEvent::ReasoningDelta(reasoning));
                        }
                        self.held = answer;
                        self.state = ThinkState::Answering;
                        self.answer_pending = true;
                        continue;
                    }
                    // Keep back the end of the text if it may start the
                    // closing tag; everything before it is reasoning.
                    let keep = partial_suffix(&self.held, THINK_CLOSE);
                    let cut = self.held.len() - keep;
                    if cut > 0 {
                        let reasoning: String = self.held.drain(..cut).collect();
                        out.push(ProviderEvent::ReasoningDelta(reasoning));
                    }
                    return;
                }
            }
        }
    }

    /// Emits what was held back when the content ends.
    pub(crate) fn flush(&mut self, out: &mut Vec<ProviderEvent>) {
        if self.held.is_empty() {
            return;
        }
        let held = std::mem::take(&mut self.held);
        match self.state {
            ThinkState::Thinking => out.push(ProviderEvent::ReasoningDelta(held)),
            ThinkState::Start | ThinkState::Answering => out.push(ProviderEvent::TextDelta(held)),
        }
    }
}

/// The length of the longest end of `text` that is a start of `tag`.
fn partial_suffix(text: &str, tag: &str) -> usize {
    (1..tag.len())
        .rev()
        .find(|&length| text.ends_with(&tag[..length]))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
