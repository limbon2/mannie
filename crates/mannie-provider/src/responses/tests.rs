use std::sync::Arc;

use mannie_types::clock::{ManualClock, UtcMillis};
use mannie_types::images::{ImageBytes, ImageFormat};
use mannie_types::provider::{ApiInterface, ReasoningEffort, ServerKind};
use mannie_types::run::{FinishReason, RunFailure, Usage};
use serde_json::json;

use super::{ResponsesTranslator, request_body};
use crate::provider::{
    Content, ImageInput, Message, ModelRequest, ProviderError, ProviderEvent, Role, ToolChoice,
    ToolDefinition,
};
use crate::quirks::{chatgpt_plan_quirks, quirks_for};
use crate::sse::SseEvent;
use crate::stream::{Flow, Translator};
use crate::transport::HttpClient;

fn translator() -> ResponsesTranslator {
    let clock = Arc::new(ManualClock::new(UtcMillis::from_millis(0)));
    let client = HttpClient::new(clock).unwrap();
    ResponsesTranslator::new(
        &client,
        "gpt-6-luna",
        quirks_for(ApiInterface::Responses, None),
    )
}

fn feed(
    translator: &mut ResponsesTranslator,
    events: &[serde_json::Value],
) -> (Vec<ProviderEvent>, Result<Flow, ProviderError>) {
    let mut out = Vec::new();
    let mut flow = Ok(Flow::Continue);
    for data in events {
        let event = SseEvent {
            event: data["type"].as_str().unwrap_or("message").to_owned(),
            data: data.to_string(),
        };
        flow = translator.event(event, &mut out);
        if !matches!(flow, Ok(Flow::Continue)) {
            break;
        }
    }
    (out, flow)
}

#[test]
fn the_request_is_stateless_and_replays_reasoning_items_before_their_answer() {
    let item = r#"{"type":"reasoning","id":"rs_1","encrypted_content":"opaque","summary":[]}"#;
    let request = ModelRequest {
        model: "gpt-6-luna".to_owned(),
        instructions: "Be brief.".to_owned(),
        messages: vec![
            Message::text(Role::User, "Hello"),
            Message {
                role: Role::Assistant,
                content: vec![
                    Content::ProviderItem(item.to_owned()),
                    Content::Text("Hi.".to_owned()),
                ],
            },
            Message::text(Role::User, "And now?"),
        ],
        max_output_tokens: 4_096,
        reasoning_effort: ReasoningEffort::Low,
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
    };

    let body = request_body(
        &request,
        &quirks_for(ApiInterface::Responses, Some(ServerKind::OpenAi)),
    );

    assert_eq!(
        body,
        json!({
            "model": "gpt-6-luna",
            "instructions": "Be brief.",
            "input": [
                { "role": "user", "content": [{ "type": "input_text", "text": "Hello" }] },
                { "type": "reasoning", "id": "rs_1", "encrypted_content": "opaque", "summary": [] },
                { "role": "assistant", "content": [{ "type": "output_text", "text": "Hi." }] },
                { "role": "user", "content": [{ "type": "input_text", "text": "And now?" }] }
            ],
            "stream": true,
            "store": false,
            "max_output_tokens": 4096,
            "reasoning": { "effort": "low", "summary": "auto" },
            "include": ["reasoning.encrypted_content"]
        })
    );
}

#[test]
fn without_an_effort_the_request_asks_for_no_reasoning() {
    let request = ModelRequest {
        model: "gpt-4.1".to_owned(),
        instructions: String::new(),
        messages: vec![Message::text(Role::User, "Hello")],
        max_output_tokens: 100,
        reasoning_effort: ReasoningEffort::ServerDefault,
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
    };

    let body = request_body(
        &request,
        &quirks_for(ApiInterface::Responses, Some(ServerKind::OpenAi)),
    );

    assert!(body.get("reasoning").is_none());
    assert!(body.get("include").is_none());
    assert!(body.get("instructions").is_none());
}

#[test]
fn images_follow_the_text_of_their_message_as_data_urls() {
    let image = |byte: u8| {
        Content::Image(ImageInput {
            format: ImageFormat::Jpeg,
            width: 4,
            height: 3,
            data: ImageBytes::from(vec![byte]),
        })
    };
    let request = ModelRequest {
        model: "gpt-4.1".to_owned(),
        instructions: String::new(),
        messages: vec![
            Message {
                role: Role::User,
                content: vec![Content::Text("What is this?".to_owned()), image(1)],
            },
            Message::text(Role::Assistant, "A cat."),
            Message {
                role: Role::User,
                content: vec![image(2)],
            },
        ],
        max_output_tokens: 100,
        reasoning_effort: ReasoningEffort::ServerDefault,
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
    };

    let body = request_body(
        &request,
        &quirks_for(ApiInterface::Responses, Some(ServerKind::OpenAi)),
    );

    assert_eq!(
        body["input"],
        json!([
            {
                "role": "user",
                "content": [
                    { "type": "input_text", "text": "What is this?" },
                    { "type": "input_image", "image_url": "data:image/jpeg;base64,AQ==" }
                ]
            },
            { "role": "assistant", "content": [{ "type": "output_text", "text": "A cat." }] },
            {
                "role": "user",
                "content": [{ "type": "input_image", "image_url": "data:image/jpeg;base64,Ag==" }]
            }
        ])
    );
}

/// A step that called two tools, of which the first returned an image.
fn step_with_a_result_image() -> Vec<Message> {
    let call = |id: &str| Content::ToolCall {
        call_id: id.to_owned(),
        name: "mcp_browser_screenshot".to_owned(),
        arguments: "{}".to_owned(),
    };
    vec![
        Message::text(Role::User, "Show me the page"),
        Message {
            role: Role::Assistant,
            content: vec![call("call_1"), call("call_2")],
        },
        Message {
            role: Role::Tool,
            content: vec![Content::ToolResult {
                call_id: "call_1".to_owned(),
                output: "The page.".to_owned(),
                images: vec![ImageInput {
                    format: ImageFormat::Png,
                    width: 4,
                    height: 3,
                    data: ImageBytes::from(vec![1]),
                }],
            }],
        },
        Message {
            role: Role::Tool,
            content: vec![Content::ToolResult {
                call_id: "call_2".to_owned(),
                output: "No image.".to_owned(),
                images: Vec::new(),
            }],
        },
    ]
}

#[test]
fn a_result_with_images_sends_them_beside_its_text_in_the_output() {
    let request = ModelRequest {
        model: "gpt-4.1".to_owned(),
        instructions: String::new(),
        messages: step_with_a_result_image(),
        max_output_tokens: 100,
        reasoning_effort: ReasoningEffort::ServerDefault,
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
    };

    let body = request_body(
        &request,
        &quirks_for(ApiInterface::Responses, Some(ServerKind::OpenAi)),
    );

    assert_eq!(
        body["input"][3],
        json!({
            "type": "function_call_output",
            "call_id": "call_1",
            "output": [
                { "type": "input_text", "text": "The page." },
                { "type": "input_image", "image_url": "data:image/png;base64,AQ==" }
            ]
        })
    );
    assert_eq!(
        body["input"][4],
        json!({ "type": "function_call_output", "call_id": "call_2", "output": "No image." })
    );
}

#[test]
fn an_image_shows_only_the_number_of_its_bytes_when_debugged() {
    let image = ImageInput {
        format: ImageFormat::Png,
        width: 1,
        height: 1,
        data: ImageBytes::from(vec![0; 2_048]),
    };

    let debugged = format!("{:?}", Content::Image(image));

    assert!(debugged.contains("ImageBytes(2048 bytes)"), "{debugged}");
    assert!(debugged.len() < 200, "{debugged}");
}

#[test]
fn a_stream_yields_reasoning_text_items_usage_and_its_end() {
    let mut translator = translator();

    let (events, flow) = feed(
        &mut translator,
        &[
            json!({ "type": "response.created", "response": {} }),
            json!({ "type": "response.reasoning_summary_text.delta", "item_id": "rs_1", "summary_index": 0, "delta": "First" }),
            json!({ "type": "response.reasoning_summary_text.delta", "item_id": "rs_1", "summary_index": 1, "delta": "Second" }),
            json!({ "type": "response.output_item.done", "item": {
                "type": "reasoning", "id": "rs_1", "encrypted_content": "opaque",
                "summary": [{ "type": "summary_text", "text": "First" }], "status": "completed"
            } }),
            json!({ "type": "response.output_text.delta", "delta": "Hel" }),
            json!({ "type": "response.output_text.delta", "delta": "lo" }),
            json!({ "type": "response.completed", "response": { "status": "completed", "usage": {
                "input_tokens": 120, "output_tokens": 40,
                "input_tokens_details": { "cached_tokens": 100 },
                "output_tokens_details": { "reasoning_tokens": 30 }
            } } }),
        ],
    );

    assert_eq!(flow, Ok(Flow::Finished));
    assert_eq!(
        events,
        [
            ProviderEvent::ReasoningDelta("First".to_owned()),
            ProviderEvent::ReasoningDelta("\n\n".to_owned()),
            ProviderEvent::ReasoningDelta("Second".to_owned()),
            ProviderEvent::ProviderItem(
                json!({ "type": "reasoning", "id": "rs_1", "encrypted_content": "opaque",
                    "summary": [{ "type": "summary_text", "text": "First" }] })
                .to_string()
            ),
            ProviderEvent::TextDelta("Hel".to_owned()),
            ProviderEvent::TextDelta("lo".to_owned()),
            ProviderEvent::Usage(Usage {
                input: 120,
                output: 40,
                cached_input: Some(100),
                reasoning: Some(30)
            }),
            ProviderEvent::Finished(FinishReason::Stop),
        ]
    );
}

#[test]
fn an_incomplete_response_ends_at_the_output_limit() {
    let mut translator = translator();

    let (events, flow) = feed(
        &mut translator,
        &[json!({ "type": "response.incomplete", "response": {
            "incomplete_details": { "reason": "max_output_tokens" },
            "usage": { "input_tokens": 5, "output_tokens": 16 }
        } })],
    );

    assert_eq!(flow, Ok(Flow::Finished));
    assert_eq!(
        events.last(),
        Some(&ProviderEvent::Finished(FinishReason::OutputLimit))
    );
}

#[test]
fn a_failed_response_and_an_error_event_are_classified() {
    let (_, overflow) = feed(
        &mut translator(),
        &[json!({ "type": "response.failed", "response": { "error": {
            "code": "context_length_exceeded", "message": "too long" } } })],
    );
    let (_, server) = feed(
        &mut translator(),
        &[json!({ "type": "error", "code": "server_error", "message": "boom" })],
    );

    assert_eq!(overflow, Err(ProviderError::ContextOverflow));
    assert_eq!(
        server,
        Err(ProviderError::Transient {
            cause: RunFailure::ServerError { status: 500 },
            retry_after: None
        })
    );
}

#[test]
fn a_stream_that_ends_without_its_last_event_was_cut() {
    let mut translator = translator();
    let (_, flow) = feed(
        &mut translator,
        &[json!({ "type": "response.output_text.delta", "delta": "Hel" })],
    );
    assert_eq!(flow, Ok(Flow::Continue));

    let ended = translator.end(&mut Vec::new());

    assert_eq!(
        ended,
        Err(ProviderError::Transient {
            cause: RunFailure::BrokenStream,
            retry_after: None
        })
    );
}

#[test]
fn reasoning_items_without_encrypted_content_are_not_kept() {
    let (events, _) = feed(
        &mut translator(),
        &[
            json!({ "type": "response.output_item.done", "item": { "type": "reasoning", "id": "rs_2", "summary": [] } }),
        ],
    );

    assert_eq!(events, []);
}

#[test]
fn a_loose_tool_goes_without_strict_mode_beside_strict_ones() {
    let tool = |name: &str, strict: bool| ToolDefinition {
        name: name.to_owned(),
        description: "A tool.".to_owned(),
        parameters: json!({ "type": "object", "properties": {} }),
        strict,
    };
    let request = ModelRequest {
        model: "gpt-6-luna".to_owned(),
        instructions: String::new(),
        messages: vec![Message::text(Role::User, "Hello")],
        max_output_tokens: 100,
        reasoning_effort: ReasoningEffort::ServerDefault,
        tools: vec![
            tool("read_file", true),
            tool("mcp_github_list_issues", false),
        ],
        tool_choice: ToolChoice::Auto,
    };

    let openai = request_body(
        &request,
        &quirks_for(ApiInterface::Responses, Some(ServerKind::OpenAi)),
    );

    assert_eq!(openai["tools"][0]["strict"], json!(true));
    assert_eq!(openai["tools"][1]["strict"], json!(false));
    assert_eq!(openai["tools"][1]["name"], json!("mcp_github_list_issues"));
}

fn read_file_tool() -> ToolDefinition {
    ToolDefinition {
        name: "read_file".to_owned(),
        description: "Reads a file.".to_owned(),
        parameters: json!({ "type": "object", "properties": { "path": { "type": "string" } } }),
        strict: true,
    }
}

/// A conversation in which the model wrote a preamble, called a tool, read
/// its result and answered.
fn conversation_with_a_call() -> ModelRequest {
    let reasoning = r#"{"type":"reasoning","id":"rs_1","encrypted_content":"opaque","summary":[]}"#;
    ModelRequest {
        model: "gpt-6.1-sol".to_owned(),
        instructions: "Be brief.".to_owned(),
        messages: vec![
            Message::text(Role::User, "What is in notes.txt?"),
            Message {
                role: Role::Assistant,
                content: vec![
                    Content::ProviderItem(reasoning.to_owned()),
                    Content::Text("Let me read it.".to_owned()),
                    Content::ToolCall {
                        call_id: "call_1".to_owned(),
                        name: "read_file".to_owned(),
                        arguments: r#"{"path":"notes.txt"}"#.to_owned(),
                    },
                ],
            },
            Message {
                role: Role::Tool,
                content: vec![Content::ToolResult {
                    call_id: "call_1".to_owned(),
                    output: "Buy milk.".to_owned(),
                    images: Vec::new(),
                }],
            },
            Message::text(Role::Assistant, "It says: buy milk."),
            Message::text(Role::User, "Thanks."),
        ],
        max_output_tokens: 8_192,
        reasoning_effort: ReasoningEffort::Medium,
        tools: vec![read_file_tool()],
        tool_choice: ToolChoice::Auto,
    }
}

#[test]
fn a_plan_request_sends_no_output_limit_and_offers_its_tools_in_one_namespace() {
    let body = request_body(&conversation_with_a_call(), &chatgpt_plan_quirks());

    let mut keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "include",
            "input",
            "instructions",
            "model",
            "reasoning",
            "store",
            "stream",
            "tool_choice",
            "tools"
        ]
    );
    assert_eq!(
        body["tools"],
        json!([{
            "type": "namespace",
            "name": "mannie",
            "description": "Tools that Mannie runs for this conversation.",
            "tools": [{
                "type": "function",
                "name": "read_file",
                "description": "Reads a file.",
                "parameters": { "type": "object", "properties": { "path": { "type": "string" } } },
                "strict": true
            }]
        }])
    );
    assert_eq!(body["store"], json!(false));
    assert_eq!(body["stream"], json!(true));
}

#[test]
fn replayed_calls_of_a_plan_request_name_the_namespace_and_assistant_text_has_its_phase() {
    let body = request_body(&conversation_with_a_call(), &chatgpt_plan_quirks());

    assert_eq!(
        body["input"],
        json!([
            { "role": "user", "content": [{ "type": "input_text", "text": "What is in notes.txt?" }] },
            { "type": "reasoning", "id": "rs_1", "encrypted_content": "opaque", "summary": [] },
            {
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": "Let me read it." }],
                "phase": "commentary"
            },
            {
                "type": "function_call",
                "call_id": "call_1",
                "name": "read_file",
                "arguments": "{\"path\":\"notes.txt\"}",
                "namespace": "mannie"
            },
            { "type": "function_call_output", "call_id": "call_1", "output": "Buy milk." },
            {
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": "It says: buy milk." }],
                "phase": "final_answer"
            },
            { "role": "user", "content": [{ "type": "input_text", "text": "Thanks." }] }
        ])
    );
}

#[test]
fn an_endpoint_request_has_no_namespace_and_no_phase() {
    let body = request_body(
        &conversation_with_a_call(),
        &quirks_for(ApiInterface::Responses, Some(ServerKind::OpenAi)),
    );

    assert_eq!(body["max_output_tokens"], json!(8_192));
    assert_eq!(body["tools"][0]["type"], json!("function"));
    let input = body["input"].as_array().unwrap();
    assert!(input.iter().all(|item| item.get("namespace").is_none()));
    assert!(input.iter().all(|item| item.get("phase").is_none()));
    assert_eq!(
        input[2],
        json!({ "role": "assistant", "content": [{ "type": "output_text", "text": "Let me read it." }] })
    );
}

#[test]
fn a_namespaced_call_in_the_stream_keeps_its_plain_name() {
    let clock = Arc::new(ManualClock::new(UtcMillis::from_millis(0)));
    let client = HttpClient::new(clock).unwrap();
    let mut translator = ResponsesTranslator::new(&client, "gpt-6.1-sol", chatgpt_plan_quirks());
    let item = json!({
        "type": "function_call",
        "id": "fc_1",
        "call_id": "call_1",
        "name": "read_file",
        "namespace": "mannie",
        "arguments": "{\"path\":\"notes.txt\"}"
    });

    let (events, flow) = feed(
        &mut translator,
        &[
            json!({ "type": "response.output_item.added", "item": item }),
            json!({ "type": "response.output_item.done", "item": item }),
            json!({ "type": "response.completed", "response": {} }),
        ],
    );

    assert!(matches!(flow, Ok(Flow::Finished)));
    assert!(events.iter().any(|event| matches!(
        event,
        ProviderEvent::ToolCall { name, call_id, .. } if name == "read_file" && call_id == "call_1"
    )));
}
