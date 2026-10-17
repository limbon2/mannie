//! Provider conformance (V8): every fixture in `tests/fixtures/` runs through
//! its adapter over real HTTP, with the stream cut into chunks in several
//! ways, and must produce the normalized events of its `expected.json`.
//!
//! A fixture folder holds:
//!
//! - `manifest.json`: the server, the interface, the request, and the
//!   provenance (`recorded` from a live server, or `constructed` from the
//!   server's documentation for cases a live server does not produce on
//!   demand);
//! - `response.json`: the status and the headers that matter;
//! - `body.sse` or `body.json`: the response body, byte for byte;
//! - `expected.json`: the events or the error the adapter must produce, which
//!   a person reviewed;
//! - `request.json`, optional: the request body the adapter must send.
//!
//! `MANNIE_UPDATE_EXPECTED=1` writes `expected.json` from the adapter's
//! output instead of comparing, for a new fixture; review the result before
//! committing it.

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures_util::StreamExt;
use mannie_provider::testing::{Chunking, FixtureResponse, FixtureServer};
use mannie_provider::{
    Content, HttpClient, ImageInput, Message, ModelRequest, ProviderError, ProviderEvent,
    ProviderRegistry, Role, Target, ToolChoice, ToolDefinition, quirks_for,
};
use mannie_types::clock::{ManualClock, UtcMillis};
use mannie_types::images::{ImageBytes, ImageFormat};
use mannie_types::provider::{ApiInterface, ReasoningEffort, ServerKind};
use serde::Deserialize;
use serde_json::{Value, json};

/// 2026-10-13T12:00:00Z, the clock of every fixture run.
const NOON: i64 = 1_791_892_800_000;

/// The ways each stream is cut: whole, and at random with fixed seeds.
const CHUNKINGS: [Chunking; 4] = [
    Chunking::Whole,
    Chunking::Random { seed: 1, max: 7 },
    Chunking::Random { seed: 2, max: 64 },
    Chunking::Random { seed: 3, max: 512 },
];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    server: ServerKind,
    interface: ApiInterface,
    provenance: String,
    request: RequestManifest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestManifest {
    model: String,
    #[serde(default)]
    instructions: String,
    messages: Vec<MessageManifest>,
    max_output_tokens: u32,
    #[serde(default)]
    reasoning_effort: ReasoningEffort,
    /// The tools offered to the model.
    #[serde(default)]
    tools: Vec<ToolManifest>,
    /// `auto`, the default, or `none`.
    #[serde(default)]
    tool_choice: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolManifest {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageManifest {
    /// `user`, `assistant` or `tool`.
    role: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    item: Option<Value>,
    /// The tool calls of an assistant message, after its text.
    #[serde(default)]
    tool_calls: Vec<ToolCallManifest>,
    /// The results of a tool message.
    #[serde(default)]
    tool_results: Vec<ToolResultManifest>,
    /// The images of a user message, after its text.
    #[serde(default)]
    images: Vec<ImageManifest>,
}

#[derive(Debug, Deserialize)]
struct ImageManifest {
    format: ImageFormat,
    width: u32,
    height: u32,
    /// The bytes, as base64.
    data: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolCallManifest {
    call_id: String,
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolResultManifest {
    call_id: String,
    output: String,
}

#[derive(Debug, Deserialize)]
struct ResponseManifest {
    status: u16,
    #[serde(default)]
    headers: serde_json::Map<String, Value>,
}

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn fixture_folders() -> Vec<PathBuf> {
    let mut folders = Vec::new();
    for server in std::fs::read_dir(fixtures_root()).unwrap() {
        let server = server.unwrap().path();
        if !server.is_dir() {
            continue;
        }
        for scenario in std::fs::read_dir(&server).unwrap() {
            let scenario = scenario.unwrap().path();
            if scenario.join("manifest.json").is_file() {
                folders.push(scenario);
            }
        }
    }
    folders.sort();
    folders
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> T {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn model_request(manifest: &RequestManifest) -> ModelRequest {
    let messages = manifest
        .messages
        .iter()
        .map(|message| {
            let role = match message.role.as_str() {
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                _ => Role::User,
            };
            let mut content = Vec::new();
            if let Some(item) = &message.item {
                content.push(Content::ProviderItem(item.to_string()));
            }
            if let Some(text) = &message.text {
                content.push(Content::Text(text.clone()));
            }
            for image in &message.images {
                content.push(Content::Image(ImageInput {
                    format: image.format,
                    width: image.width,
                    height: image.height,
                    data: ImageBytes::from(BASE64.decode(&image.data).unwrap()),
                }));
            }
            for call in &message.tool_calls {
                content.push(Content::ToolCall {
                    call_id: call.call_id.clone(),
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                });
            }
            for result in &message.tool_results {
                content.push(Content::ToolResult {
                    call_id: result.call_id.clone(),
                    output: result.output.clone(),
                });
            }
            Message { role, content }
        })
        .collect();
    let tools = manifest
        .tools
        .iter()
        .map(|tool| ToolDefinition {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: tool.parameters.clone(),
        })
        .collect();
    let tool_choice = match manifest.tool_choice.as_deref() {
        Some("none") => ToolChoice::None,
        _ => ToolChoice::Auto,
    };
    ModelRequest {
        model: manifest.model.clone(),
        instructions: manifest.instructions.clone(),
        messages,
        max_output_tokens: manifest.max_output_tokens,
        reasoning_effort: manifest.reasoning_effort,
        tools,
        tool_choice,
    }
}

fn describe_event(event: &ProviderEvent) -> Value {
    match event {
        ProviderEvent::TextDelta(text) => json!({ "text": text }),
        ProviderEvent::ReasoningDelta(text) => json!({ "reasoning": text }),
        ProviderEvent::ProviderItem(item) => {
            json!({ "item": serde_json::from_str::<Value>(item).unwrap_or(Value::String(item.clone())) })
        }
        ProviderEvent::ToolCallStarted { call_id, name } => {
            json!({ "toolCallStarted": { "callId": call_id, "name": name } })
        }
        ProviderEvent::ToolCall {
            call_id,
            name,
            arguments,
        } => json!({ "toolCall": { "callId": call_id, "name": name, "arguments": arguments } }),
        ProviderEvent::Usage(usage) => json!({ "usage": usage }),
        ProviderEvent::Finished(reason) => json!({ "finished": reason }),
    }
}

fn describe_error(error: &ProviderError) -> Value {
    match error {
        ProviderError::Transient { cause, retry_after } => json!({
            "class": "transient",
            "failure": cause,
            "retryAfterMs": retry_after.map(|wait| u64::try_from(wait.as_millis()).unwrap_or(u64::MAX)),
        }),
        ProviderError::ContextOverflow => json!({ "class": "context_overflow" }),
        ProviderError::Permanent(failure) => json!({ "class": "permanent", "failure": failure }),
    }
}

async fn run_fixture(folder: &Path, chunking: Chunking) -> (Value, Option<Value>) {
    let manifest: Manifest = read_json(&folder.join("manifest.json"));
    let response: ResponseManifest = read_json(&folder.join("response.json"));
    let body_path = ["body.sse", "body.json"]
        .into_iter()
        .map(|name| folder.join(name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("{}: no body.sse or body.json", folder.display()));
    let body = std::fs::read(&body_path).unwrap();
    let server = FixtureServer::start().await.unwrap();
    let mut fixture = FixtureResponse::json(response.status, body);
    fixture.headers = response
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.as_str().unwrap_or_default().to_owned()))
        .collect();
    fixture.chunking = chunking;
    server.respond(fixture);

    let clock = Arc::new(ManualClock::new(UtcMillis::from_millis(NOON)));
    let client = HttpClient::new(clock).unwrap();
    let registry = ProviderRegistry::with_builtin(&client);
    let adapter = registry.adapter(manifest.interface).unwrap();
    let target = Target {
        base_url: server.base_url(),
        credential: None,
        quirks: quirks_for(manifest.interface, Some(manifest.server)),
    };
    let mut stream = adapter.stream(&target, model_request(&manifest.request));
    let mut events = Vec::new();
    let mut error = Value::Null;
    while let Some(item) = stream.next().await {
        match item {
            Ok(event) => events.push(describe_event(&event)),
            Err(failure) => {
                error = describe_error(&failure);
                break;
            }
        }
    }
    let sent = server
        .requests()
        .first()
        .map(|request| serde_json::from_slice::<Value>(&request.body).unwrap());
    (json!({ "events": events, "error": error }), sent)
}

#[tokio::test]
async fn every_fixture_produces_its_expected_events_however_its_stream_is_cut() {
    let update = std::env::var_os("MANNIE_UPDATE_EXPECTED").is_some();
    let folders = fixture_folders();
    assert!(
        !folders.is_empty(),
        "no fixtures found in {}",
        fixtures_root().display()
    );
    let mut failures = Vec::new();
    for folder in &folders {
        let name = folder
            .strip_prefix(fixtures_root())
            .unwrap()
            .display()
            .to_string();
        let expected_path = folder.join("expected.json");
        for chunking in CHUNKINGS {
            let (actual, sent) = run_fixture(folder, chunking).await;
            if update && chunking == Chunking::Whole {
                let text = serde_json::to_string_pretty(&actual).unwrap();
                std::fs::write(&expected_path, format!("{text}\n")).unwrap();
                continue;
            }
            let expected: Value = read_json(&expected_path);
            if actual != expected {
                failures.push(format!(
                    "{name} ({chunking:?}):\n  expected {expected}\n  actual   {actual}"
                ));
            }
            let request_path = folder.join("request.json");
            if request_path.is_file() {
                let expected_request: Value = read_json(&request_path);
                if sent.as_ref() != Some(&expected_request) {
                    failures.push(format!("{name}: the request differs:\n  expected {expected_request}\n  actual   {sent:?}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn every_manifest_names_its_provenance() {
    for folder in fixture_folders() {
        let manifest: Manifest = read_json(&folder.join("manifest.json"));
        assert!(
            ["recorded", "constructed"].contains(&manifest.provenance.as_str()),
            "{}: provenance {:?}",
            folder.display(),
            manifest.provenance
        );
    }
}
