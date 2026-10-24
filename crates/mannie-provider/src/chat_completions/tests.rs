use std::sync::Arc;

use mannie_types::clock::{ManualClock, UtcMillis};
use mannie_types::images::{ImageBytes, ImageFormat};
use mannie_types::provider::{ApiInterface, ReasoningEffort, ServerKind};
use mannie_types::run::{FinishReason, RunFailure, Usage};
use proptest::prelude::*;
use serde_json::json;

use super::{ChatTranslator, ThinkSplitter, request_body};
use crate::provider::{
    Content, ImageInput, Message, ModelRequest, ProviderError, ProviderEvent, Role,
};
use crate::quirks::quirks_for;
use crate::sse::SseEvent;
use crate::stream::{Flow, Translator};
use crate::transport::HttpClient;

fn translator(server: ServerKind) -> ChatTranslator {
    let clock = Arc::new(ManualClock::new(UtcMillis::from_millis(0)));
    let client = HttpClient::new(clock).unwrap();
    ChatTranslator::new(
        &client,
        "qwen3",
        quirks_for(ApiInterface::ChatCompletions, Some(server)),
        true,
    )
}

fn feed(
    translator: &mut ChatTranslator,
    chunks: &[&str],
) -> (Vec<ProviderEvent>, Result<Flow, ProviderError>) {
    let mut out = Vec::new();
    let mut flow = Ok(Flow::Continue);
    for data in chunks {
        let event = SseEvent {
            event: "message".to_owned(),
            data: (*data).to_owned(),
        };
        flow = translator.event(event, &mut out);
        if !matches!(flow, Ok(Flow::Continue)) {
            break;
        }
    }
    (out, flow)
}

fn request(effort: ReasoningEffort) -> ModelRequest {
    ModelRequest {
        model: "qwen3".to_owned(),
        instructions: "Be brief.".to_owned(),
        messages: vec![
            Message::text(Role::User, "First"),
            Message::text(Role::User, "Second"),
            Message {
                role: Role::Assistant,
                content: vec![
                    Content::ProviderItem("{}".to_owned()),
                    Content::Text("Answer".to_owned()),
                ],
            },
            Message::text(Role::User, "Third"),
        ],
        max_output_tokens: 512,
        reasoning_effort: effort,
        tools: Vec::new(),
        tool_choice: crate::provider::ToolChoice::Auto,
    }
}

#[test]
fn the_request_merges_consecutive_roles_and_follows_the_quirks() {
    let generic = request_body(
        &request(ReasoningEffort::High),
        &quirks_for(ApiInterface::ChatCompletions, None),
    );
    let openrouter = request_body(
        &request(ReasoningEffort::High),
        &quirks_for(ApiInterface::ChatCompletions, Some(ServerKind::OpenRouter)),
    );
    let openai = request_body(
        &request(ReasoningEffort::ServerDefault),
        &quirks_for(ApiInterface::ChatCompletions, Some(ServerKind::OpenAi)),
    );

    assert_eq!(
        generic,
        json!({
            "model": "qwen3",
            "messages": [
                { "role": "system", "content": "Be brief." },
                { "role": "user", "content": "First\n\nSecond" },
                { "role": "assistant", "content": "Answer" },
                { "role": "user", "content": "Third" }
            ],
            "stream": true,
            "stream_options": { "include_usage": true },
            "max_tokens": 512,
            "reasoning_effort": "high"
        })
    );
    assert_eq!(openrouter["reasoning"], json!({ "effort": "high" }));
    assert!(openrouter.get("reasoning_effort").is_none());
    assert_eq!(openai["max_completion_tokens"], json!(512));
    assert!(openai.get("reasoning_effort").is_none());
}

fn image(byte: u8) -> Content {
    Content::Image(ImageInput {
        format: ImageFormat::Png,
        width: 2,
        height: 1,
        data: ImageBytes::from(vec![byte, byte]),
    })
}

fn with_messages(messages: Vec<Message>) -> ModelRequest {
    ModelRequest {
        messages,
        instructions: String::new(),
        ..request(ReasoningEffort::ServerDefault)
    }
}

#[test]
fn images_make_an_array_of_parts_in_their_order_and_text_stays_a_string() {
    let body = request_body(
        &with_messages(vec![
            Message::text(Role::User, "Before"),
            Message {
                role: Role::User,
                content: vec![
                    Content::Text("Look".to_owned()),
                    Content::Text("Mentioned".to_owned()),
                    image(1),
                    image(2),
                ],
            },
            Message::text(Role::Assistant, "A cat."),
        ]),
        &quirks_for(ApiInterface::ChatCompletions, None),
    );

    assert_eq!(
        body["messages"],
        json!([
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": "Before\n\nLook\n\nMentioned" },
                    { "type": "image_url", "image_url": { "url": "data:image/png;base64,AQE=" } },
                    { "type": "image_url", "image_url": { "url": "data:image/png;base64,AgI=" } }
                ]
            },
            { "role": "assistant", "content": "A cat." }
        ])
    );
}

#[test]
fn the_images_of_a_step_s_results_follow_its_last_result_in_a_message_of_the_owner() {
    let call = |id: &str| Content::ToolCall {
        call_id: id.to_owned(),
        name: "mcp_browser_screenshot".to_owned(),
        arguments: "{}".to_owned(),
    };
    let result = |id: &str, images: Vec<ImageInput>| Message {
        role: Role::Tool,
        content: vec![Content::ToolResult {
            call_id: id.to_owned(),
            output: format!("Result of {id}."),
            images,
        }],
    };
    let shot = |byte: u8| ImageInput {
        format: ImageFormat::Png,
        width: 2,
        height: 1,
        data: ImageBytes::from(vec![byte, byte]),
    };
    let body = request_body(
        &with_messages(vec![
            Message::text(Role::User, "Show me both pages"),
            Message {
                role: Role::Assistant,
                content: vec![call("call_1"), call("call_2"), call("call_3")],
            },
            result("call_1", vec![shot(1)]),
            result("call_2", Vec::new()),
            result("call_3", vec![shot(2), shot(3)]),
            Message::text(Role::User, "Which one is newer?"),
        ]),
        &quirks_for(ApiInterface::ChatCompletions, None),
    );

    let messages = body["messages"].as_array().unwrap();
    let roles: Vec<&str> = messages
        .iter()
        .map(|message| message["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["user", "assistant", "tool", "tool", "tool", "user"]);
    assert_eq!(messages[2]["content"], json!("Result of call_1."));
    assert_eq!(
        messages[5]["content"],
        json!([
            { "type": "text", "text": "The images of the result of the tool call call_1:" },
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,AQE=" } },
            { "type": "text", "text": "The images of the result of the tool call call_3:" },
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,AgI=" } },
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,AwM=" } },
            { "type": "text", "text": "Which one is newer?" }
        ])
    );
}

#[test]
fn a_request_that_ends_with_results_ends_with_their_images() {
    let body = request_body(
        &with_messages(vec![
            Message::text(Role::User, "Show me"),
            Message {
                role: Role::Assistant,
                content: vec![Content::ToolCall {
                    call_id: "call_1".to_owned(),
                    name: "mcp_browser_screenshot".to_owned(),
                    arguments: "{}".to_owned(),
                }],
            },
            Message {
                role: Role::Tool,
                content: vec![Content::ToolResult {
                    call_id: "call_1".to_owned(),
                    output: String::new(),
                    images: vec![ImageInput {
                        format: ImageFormat::Jpeg,
                        width: 2,
                        height: 1,
                        data: ImageBytes::from(vec![4]),
                    }],
                }],
            },
        ]),
        &quirks_for(ApiInterface::ChatCompletions, None),
    );

    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[2]["role"], json!("tool"));
    assert_eq!(
        messages[3],
        json!({
            "role": "user",
            "content": [
                { "type": "text", "text": "The images of the result of the tool call call_1:" },
                { "type": "image_url", "image_url": { "url": "data:image/jpeg;base64,BA==" } }
            ]
        })
    );
}

#[test]
fn a_message_with_only_an_image_is_sent() {
    let body = request_body(
        &with_messages(vec![Message {
            role: Role::User,
            content: vec![image(1)],
        }]),
        &quirks_for(ApiInterface::ChatCompletions, Some(ServerKind::Ollama)),
    );

    assert_eq!(
        body["messages"],
        json!([{
            "role": "user",
            "content": [
                { "type": "image_url", "image_url": { "url": "data:image/png;base64,AQE=" } }
            ]
        }])
    );
}

#[test]
fn text_after_an_image_is_a_part_of_its_own() {
    let body = request_body(
        &with_messages(vec![
            Message {
                role: Role::User,
                content: vec![image(1)],
            },
            Message::text(Role::User, "Then this"),
        ]),
        &quirks_for(ApiInterface::ChatCompletions, None),
    );

    assert_eq!(
        body["messages"][0]["content"],
        json!([
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,AQE=" } },
            { "type": "text", "text": "Then this" }
        ])
    );
}

#[test]
fn a_stream_yields_reasoning_text_usage_and_its_end() {
    let mut translator = translator(ServerKind::Vllm);

    let (events, flow) = feed(
        &mut translator,
        &[
            r#"{"choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"Think"},"finish_reason":null}]}"#,
            r#"{"choices":[{"index":0,"delta":{"reasoning":"ing"},"finish_reason":null}]}"#,
            r#"{"choices":[{"index":0,"delta":{"content":"Hel"},"finish_reason":null}]}"#,
            r#"{"choices":[{"index":0,"delta":{"content":"lo"},"finish_reason":"stop"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":9,"completion_tokens":4,"completion_tokens_details":{"reasoning_tokens":2}}}"#,
            "[DONE]",
        ],
    );

    assert_eq!(flow, Ok(Flow::Finished));
    assert_eq!(
        events,
        [
            ProviderEvent::ReasoningDelta("Think".to_owned()),
            ProviderEvent::ReasoningDelta("ing".to_owned()),
            ProviderEvent::TextDelta("Hel".to_owned()),
            ProviderEvent::TextDelta("lo".to_owned()),
            ProviderEvent::Usage(Usage {
                input: 9,
                output: 4,
                cached_input: None,
                reasoning: Some(2)
            }),
            ProviderEvent::Finished(FinishReason::Stop),
        ]
    );
}

#[test]
fn a_stream_without_done_ends_well_after_a_finish_reason_and_breaks_without_one() {
    let mut finished = translator(ServerKind::Ollama);
    let (_, flow) = feed(
        &mut finished,
        &[r#"{"choices":[{"index":0,"delta":{"content":"x"},"finish_reason":"length"}]}"#],
    );
    assert_eq!(flow, Ok(Flow::Continue));
    let mut out = Vec::new();
    let ended = finished.end(&mut out);

    let mut cut = translator(ServerKind::Ollama);
    let (_, flow) = feed(
        &mut cut,
        &[r#"{"choices":[{"index":0,"delta":{"content":"x"},"finish_reason":null}]}"#],
    );
    assert_eq!(flow, Ok(Flow::Continue));
    let broken = cut.end(&mut Vec::new());

    assert_eq!(ended, Ok(()));
    assert_eq!(
        out.last(),
        Some(&ProviderEvent::Finished(FinishReason::OutputLimit))
    );
    assert_eq!(
        broken,
        Err(ProviderError::Transient {
            cause: RunFailure::BrokenStream,
            retry_after: None
        })
    );
}

#[test]
fn an_error_object_in_the_stream_is_classified() {
    let (_, flow) = feed(
        &mut translator(ServerKind::OpenRouter),
        &[
            r#"{"error":{"code":502,"message":"This model's maximum context length is 8192 tokens"}}"#,
        ],
    );

    assert_eq!(flow, Err(ProviderError::ContextOverflow));
}

#[test]
fn think_tags_at_the_start_become_reasoning() {
    let mut translator = translator(ServerKind::Other);

    let (events, _) = feed(
        &mut translator,
        &[
            r#"{"choices":[{"delta":{"content":"\n<thi"},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{"content":"nk>Plan</th"},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{"content":"ink>\n\nDone"},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ],
    );

    assert_eq!(
        events,
        [
            ProviderEvent::ReasoningDelta("Plan".to_owned()),
            ProviderEvent::TextDelta("Done".to_owned()),
            ProviderEvent::Finished(FinishReason::Stop),
        ]
    );
}

#[test]
fn content_that_does_not_open_with_a_think_tag_is_answer() {
    let mut splitter = ThinkSplitter::new(true);
    let mut out = Vec::new();

    splitter.push("<b>", &mut out);
    splitter.push(" bold", &mut out);
    splitter.flush(&mut out);

    assert_eq!(
        out,
        [
            ProviderEvent::TextDelta("<b>".to_owned()),
            ProviderEvent::TextDelta(" bold".to_owned()),
        ]
    );
}

proptest! {
    #[test]
    fn any_split_of_the_content_gives_the_same_reasoning_and_answer(
        reasoning in "[a-z <>/]{0,30}",
        answer in "[a-z <>/]{0,30}",
        cuts in proptest::collection::vec(any::<prop::sample::Index>(), 0..8),
    ) {
        // A reasoning text that contains the closing tag would end early; the
        // strategy's alphabet can produce it, so such cases are skipped.
        prop_assume!(!reasoning.contains("</think>"));
        let content = format!("<think>{reasoning}</think>{answer}");
        let mut positions: Vec<usize> = cuts.iter().map(|cut| cut.index(content.len() + 1)).collect();
        positions.sort_unstable();
        let mut splitter = ThinkSplitter::new(true);
        let mut out = Vec::new();
        let mut start = 0;
        for position in positions {
            splitter.push(&content[start..position], &mut out);
            start = position;
        }
        splitter.push(&content[start..], &mut out);
        splitter.flush(&mut out);

        let mut got_reasoning = String::new();
        let mut got_answer = String::new();
        for event in out {
            match event {
                ProviderEvent::ReasoningDelta(text) => got_reasoning.push_str(&text),
                ProviderEvent::TextDelta(text) => got_answer.push_str(&text),
                _ => {}
            }
        }
        prop_assert_eq!(got_reasoning, reasoning);
        prop_assert_eq!(got_answer, answer.trim_start());
    }
}

#[test]
fn a_loose_tool_goes_without_strict_mode_beside_strict_ones() {
    let tool = |name: &str, strict: bool| crate::provider::ToolDefinition {
        name: name.to_owned(),
        description: "A tool.".to_owned(),
        parameters: json!({ "type": "object", "properties": {} }),
        strict,
    };
    let mut request = request(ReasoningEffort::ServerDefault);
    request.tools = vec![
        tool("read_file", true),
        tool("mcp_github_list_issues", false),
    ];

    let openai = request_body(
        &request,
        &quirks_for(ApiInterface::ChatCompletions, Some(ServerKind::OpenAi)),
    );
    let vllm = request_body(
        &request,
        &quirks_for(ApiInterface::ChatCompletions, Some(ServerKind::Vllm)),
    );

    assert_eq!(openai["tools"][0]["function"]["strict"], json!(true));
    assert!(openai["tools"][1]["function"].get("strict").is_none());
    assert!(vllm["tools"][0]["function"].get("strict").is_none());
}
