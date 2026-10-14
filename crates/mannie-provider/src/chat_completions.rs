//! The adapter for Chat Completions on OpenAI and compatible servers: vLLM,
//! SGLang, LM Studio, Ollama and OpenRouter (ADR-0011, decision 1). The
//! server's quirk profile decides the fields that differ between them.

use mannie_types::provider::ApiInterface;
use mannie_types::run::{FinishReason, RunFailure, Usage};
use serde_json::{Map, Value, json};

use crate::errors::{self, Context, ErrorDetails};
use crate::provider::{
    Content, EventStream, ModelRequest, Provider, ProviderError, ProviderEvent, Role, Target,
};
use crate::quirks::{EffortShape, OutputLimitField, Quirks};
use crate::sse::SseEvent;
use crate::stream::{self, Call, Flow, Translator};
use crate::transport::{HttpClient, endpoint};

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
        let translator = ChatTranslator::new(&self.client, &request.model, target.quirks);
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
    let mut messages: Vec<(Role, String)> = Vec::new();
    for message in &request.messages {
        let text = message
            .content
            .iter()
            .filter_map(|content| match content {
                Content::Text(text) => Some(text.as_str()),
                // Provider-private data belongs to the Responses API.
                Content::ProviderItem(_) => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        if text.is_empty() {
            continue;
        }
        // Some chat templates refuse two messages of the same role in a row,
        // so consecutive messages of one role become one message.
        match messages.last_mut() {
            Some((role, existing)) if *role == message.role => {
                existing.push_str("\n\n");
                existing.push_str(&text);
            }
            _ => messages.push((message.role, text)),
        }
    }
    let mut wire = Vec::with_capacity(messages.len() + 1);
    if !request.instructions.is_empty() {
        wire.push(json!({ "role": "system", "content": request.instructions }));
    }
    for (role, text) in messages {
        let role = match role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        wire.push(json!({ "role": role, "content": text }));
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
    Value::Object(body)
}

/// Turns streamed chunks into normalized events.
struct ChatTranslator {
    client: HttpClient,
    model: String,
    quirks: Quirks,
    think: ThinkSplitter,
    finish: Option<FinishReason>,
    usage: Option<Usage>,
}

impl ChatTranslator {
    fn new(client: &HttpClient, model: &str, quirks: Quirks) -> Self {
        Self {
            client: client.clone(),
            model: model.to_owned(),
            quirks,
            think: ThinkSplitter::new(quirks.think_tags),
            finish: None,
            usage: None,
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
        self.think.flush(out);
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
            for field in ["reasoning_content", "reasoning"] {
                if let Some(reasoning) = delta.get(field).and_then(Value::as_str)
                    && !reasoning.is_empty()
                {
                    out.push(ProviderEvent::ReasoningDelta(reasoning.to_owned()));
                }
            }
            if let Some(content) = delta.get("content").and_then(Value::as_str) {
                self.think.push(content, out);
            }
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
