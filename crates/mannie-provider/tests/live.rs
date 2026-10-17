//! Live conformance (V8): the scenarios of the fixtures, run against a real
//! server that `MANNIE_LIVE_*` variables name. The tests are ignored unless
//! run on purpose: `task provider:live` runs them, and the provider-live
//! workflow runs them every night for each server that its secrets name.
//!
//! | Variable                         | Meaning                                                        |
//! | -------------------------------- | -------------------------------------------------------------- |
//! | `MANNIE_LIVE_SERVER`             | `open_ai`, `open_router`, `vllm`, `sglang`, `lm_studio`, `ollama` |
//! | `MANNIE_LIVE_INTERFACE`          | `responses` or `chat_completions`                              |
//! | `MANNIE_LIVE_BASE_URL`           | The server's base URL                                          |
//! | `MANNIE_LIVE_MODEL`              | The model                                                      |
//! | `MANNIE_LIVE_API_KEY`            | The credential, if the server needs one                        |
//! | `MANNIE_LIVE_REASONING_EFFORT`   | An effort such as `low`, for the reasoning scenarios           |
//! | `MANNIE_LIVE_CONTEXT_WINDOW`     | The model's context window, for the overflow scenario          |
//! | `MANNIE_LIVE_SERVER_VERSION`     | The server's version, for the fixtures' manifests              |
//! | `MANNIE_LIVE_IMAGES`             | `1` when the model reads images, for the image scenario        |
//! | `MANNIE_RECORD_FIXTURES`         | `1` writes each scenario as a recorded fixture                 |
//!
//! Recording never writes a file that contains the credential.

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures_util::StreamExt;
use mannie_provider::testing::{Exchange, RecordingProxy};
use mannie_provider::{
    Content, HttpClient, ImageInput, Message, ModelList, ModelRequest, ProviderError,
    ProviderEvent, ProviderRegistry, Role, TOOL_PROMPT, Target, ToolChoice, ToolDefinition,
    list_models, quirks_for,
};
use mannie_types::clock::{Clock, SystemClock};
use mannie_types::images::{ImageBytes, ImageFormat};
use mannie_types::provider::{ApiInterface, ReasoningEffort, ServerKind};
use mannie_types::run::{FinishReason, RunFailure};
use mannie_types::secret::Secret;
use reqwest::Url;
use serde_json::{Value, json};

/// The live server, as the environment names it.
struct Live {
    server: ServerKind,
    interface: ApiInterface,
    base_url: Url,
    model: String,
    credential: Option<Secret>,
    effort: ReasoningEffort,
    context_window: Option<u32>,
    server_version: Option<String>,
    images: bool,
    record: bool,
}

impl Live {
    fn from_environment() -> Option<Self> {
        let variable = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        };
        let server = serde_json::from_value(json!(variable("MANNIE_LIVE_SERVER")?)).ok()?;
        let interface = serde_json::from_value(json!(
            variable("MANNIE_LIVE_INTERFACE").unwrap_or_else(|| "chat_completions".to_owned())
        ))
        .ok()?;
        let effort = variable("MANNIE_LIVE_REASONING_EFFORT")
            .and_then(|effort| serde_json::from_value(json!(effort)).ok())
            .unwrap_or_default();
        Some(Self {
            server,
            interface,
            base_url: Url::parse(&variable("MANNIE_LIVE_BASE_URL")?).ok()?,
            model: variable("MANNIE_LIVE_MODEL")?,
            credential: variable("MANNIE_LIVE_API_KEY").map(Secret::new),
            effort,
            context_window: variable("MANNIE_LIVE_CONTEXT_WINDOW")
                .and_then(|text| text.parse().ok()),
            server_version: variable("MANNIE_LIVE_SERVER_VERSION"),
            images: variable("MANNIE_LIVE_IMAGES").as_deref() == Some("1"),
            record: variable("MANNIE_RECORD_FIXTURES").as_deref() == Some("1"),
        })
    }

    fn request(
        &self,
        prompt: &str,
        max_output_tokens: u32,
        effort: ReasoningEffort,
    ) -> ModelRequest {
        ModelRequest {
            model: self.model.clone(),
            instructions: String::new(),
            messages: vec![Message::text(Role::User, prompt)],
            max_output_tokens,
            reasoning_effort: effort,
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
        }
    }
}

/// The tool of the tool scenarios, as the provider test offers it.
fn status_tool() -> ToolDefinition {
    ToolDefinition {
        name: "report_status".to_owned(),
        description: "Reports the status of the test.".to_owned(),
        parameters: json!({
            "type": "object",
            "properties": {
                "status": { "type": "string", "description": "The status to report." },
            },
            "required": ["status"],
            "additionalProperties": false,
        }),
    }
}

/// What one call produced.
#[derive(Debug, Default)]
struct Outcome {
    events: Vec<ProviderEvent>,
    error: Option<ProviderError>,
}

impl Outcome {
    fn text(&self) -> String {
        self.events
            .iter()
            .filter_map(|event| match event {
                ProviderEvent::TextDelta(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn finish(&self) -> Option<FinishReason> {
        self.events.iter().find_map(|event| match event {
            ProviderEvent::Finished(reason) => Some(*reason),
            _ => None,
        })
    }

    fn item(&self) -> Option<String> {
        self.events.iter().find_map(|event| match event {
            ProviderEvent::ProviderItem(item) => Some(item.clone()),
            _ => None,
        })
    }

    fn tool_call(&self) -> Option<(String, String, String)> {
        self.events.iter().find_map(|event| match event {
            ProviderEvent::ToolCall {
                call_id,
                name,
                arguments,
            } => Some((call_id.clone(), name.clone(), arguments.clone())),
            _ => None,
        })
    }
}

struct Runner {
    live: Live,
    client: HttpClient,
    registry: ProviderRegistry,
}

impl Runner {
    fn new(live: Live) -> Self {
        let client = HttpClient::new(Arc::new(SystemClock)).unwrap();
        let registry = ProviderRegistry::with_builtin(&client);
        Self {
            live,
            client,
            registry,
        }
    }

    async fn call(
        &self,
        base_url: &Url,
        credential: Option<Secret>,
        request: ModelRequest,
    ) -> Outcome {
        let adapter = self.registry.adapter(self.live.interface).unwrap();
        let target = Target {
            base_url: base_url.clone(),
            credential,
            quirks: quirks_for(self.live.interface, Some(self.live.server)),
        };
        let mut stream = adapter.stream(&target, request);
        let mut outcome = Outcome::default();
        while let Some(item) = stream.next().await {
            match item {
                Ok(event) => outcome.events.push(event),
                Err(error) => {
                    outcome.error = Some(error);
                    break;
                }
            }
        }
        outcome
    }

    /// Runs a scenario through the recording proxy when recording, and
    /// directly otherwise.
    async fn scenario(
        &self,
        name: &str,
        credential: Option<Secret>,
        request: ModelRequest,
    ) -> Outcome {
        if !self.live.record {
            return self.call(&self.live.base_url, credential, request).await;
        }
        let proxy = RecordingProxy::start(self.live.base_url.clone())
            .await
            .unwrap();
        let outcome = self
            .call(&proxy.base_url(), credential, request.clone())
            .await;
        let exchange = proxy
            .exchanges()
            .pop()
            .expect("the proxy forwarded the call");
        self.write_fixture(name, &request, &exchange);
        outcome
    }

    fn write_fixture(&self, scenario: &str, request: &ModelRequest, exchange: &Exchange) {
        let folder = fixture_folder(self.live.server, scenario);
        std::fs::create_dir_all(&folder).unwrap();
        let today = time::OffsetDateTime::from_unix_timestamp(SystemClock.now().as_millis() / 1000)
            .unwrap()
            .date()
            .to_string();
        let messages: Vec<Value> = request.messages.iter().map(manifest_message).collect();
        let manifest = json!({
            "server": self.live.server,
            "serverVersion": self.live.server_version,
            "interface": self.live.interface,
            "provenance": "recorded",
            "recordedOn": today,
            "request": {
                "model": request.model,
                "instructions": request.instructions,
                "messages": messages,
                "maxOutputTokens": request.max_output_tokens,
                "reasoningEffort": request.reasoning_effort,
                "tools": request.tools.iter().map(|tool| json!({
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                })).collect::<Vec<_>>(),
                "toolChoice": match request.tool_choice {
                    ToolChoice::Auto => "auto",
                    ToolChoice::None => "none",
                },
            },
        });
        let headers: serde_json::Map<String, Value> = exchange
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), json!(value)))
            .collect();
        let response = json!({ "status": exchange.status, "headers": headers });
        let is_stream = exchange
            .headers
            .iter()
            .any(|(name, value)| name == "content-type" && value.starts_with("text/event-stream"));
        let body_name = if is_stream { "body.sse" } else { "body.json" };
        let files: [(&str, Vec<u8>); 4] = [
            ("manifest.json", pretty(&manifest)),
            ("response.json", pretty(&response)),
            (body_name, exchange.body.clone()),
            ("request.json", exchange.request.body.clone()),
        ];
        for (name, bytes) in &files {
            if let Some(credential) = &self.live.credential {
                let secret = credential.expose().as_bytes();
                let leaks = bytes.windows(secret.len()).any(|window| window == secret);
                assert!(
                    !leaks,
                    "{name} of {scenario} contains the credential; nothing was written"
                );
            }
        }
        for (name, bytes) in files {
            std::fs::write(folder.join(name), bytes).unwrap();
        }
        println!("recorded {}", folder.display());
    }
}

/// A message of a request as a fixture's manifest records it.
fn manifest_message(message: &Message) -> Value {
    let role = match message.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };
    let mut entry = json!({ "role": role });
    let mut calls = Vec::new();
    let mut results = Vec::new();
    let mut images = Vec::new();
    for content in &message.content {
        match content {
            Content::Text(text) => entry["text"] = json!(text),
            Content::Image(image) => images.push(json!({
                "format": image.format,
                "width": image.width,
                "height": image.height,
                "data": BASE64.encode(image.data.as_slice()),
            })),
            Content::ProviderItem(item) => {
                entry["item"] = serde_json::from_str(item).unwrap();
            }
            Content::ToolCall {
                call_id,
                name,
                arguments,
            } => calls.push(json!({ "callId": call_id, "name": name, "arguments": arguments })),
            Content::ToolResult { call_id, output } => {
                results.push(json!({ "callId": call_id, "output": output }));
            }
        }
    }
    if !calls.is_empty() {
        entry["toolCalls"] = json!(calls);
    }
    if !results.is_empty() {
        entry["toolResults"] = json!(results);
    }
    if !images.is_empty() {
        entry["images"] = json!(images);
    }
    entry
}

/// A PNG of one color, with its pixels in stored deflate blocks, so that the
/// test needs no image encoder.
fn solid_png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    // Each row starts with its filter type, 0 for none.
    let mut row = vec![0];
    row.extend(rgb.repeat(usize::try_from(width).unwrap()));
    let raw = row.repeat(usize::try_from(height).unwrap());
    let mut zlib = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (index, block) in blocks.iter().enumerate() {
        let last = u8::from(index + 1 == blocks.len());
        let length = u16::try_from(block.len()).unwrap();
        zlib.push(last);
        zlib.extend_from_slice(&length.to_le_bytes());
        zlib.extend_from_slice(&(!length).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    for (kind, data) in [(*b"IHDR", header), (*b"IDAT", zlib), (*b"IEND", Vec::new())] {
        png.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
        let mut chunk = kind.to_vec();
        chunk.extend_from_slice(&data);
        png.extend_from_slice(&chunk);
        png.extend_from_slice(&crc32(&chunk).to_be_bytes());
    }
    png
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1_u32, 0_u32);
    for byte in bytes {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn pretty(value: &Value) -> Vec<u8> {
    let mut text = serde_json::to_string_pretty(value).unwrap();
    text.push('\n');
    text.into_bytes()
}

fn fixture_folder(server: ServerKind, scenario: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(server.as_str())
        .join(scenario)
}

impl Runner {
    async fn check_text_and_length(&self) {
        let live = &self.live;
        let text = self
            .scenario(
                "text",
                live.credential.clone(),
                live.request("Reply with the single word: ready", 2_048, live.effort),
            )
            .await;
        assert_eq!(text.error, None, "text");
        assert!(!text.text().trim().is_empty(), "text: no answer");
        assert_eq!(text.finish(), Some(FinishReason::Stop), "text");

        let length = self
            .scenario(
                "length",
                live.credential.clone(),
                live.request(
                    "Count from 1 to 300, separated by spaces.",
                    16,
                    ReasoningEffort::ServerDefault,
                ),
            )
            .await;
        assert_eq!(length.error, None, "length");
        assert_eq!(length.finish(), Some(FinishReason::OutputLimit), "length");
    }

    async fn check_refusals(&self) {
        let live = &self.live;
        let unknown = self
            .scenario(
                "unknown_model",
                live.credential.clone(),
                ModelRequest {
                    model: "mannie-unknown-model".to_owned(),
                    ..live.request("Hello", 64, ReasoningEffort::ServerDefault)
                },
            )
            .await;
        assert!(
            matches!(unknown.error, Some(ProviderError::Permanent(_))),
            "unknown model: {:?}",
            unknown.error
        );
        if live.credential.is_some() {
            let invalid = self
                .scenario(
                    "invalid_key",
                    Some(Secret::new("sk-mannie-invalid-0000000000000000".to_owned())),
                    live.request("Hello", 64, ReasoningEffort::ServerDefault),
                )
                .await;
            assert_eq!(
                invalid.error,
                Some(ProviderError::Permanent(RunFailure::InvalidCredential)),
                "invalid key"
            );
        }
    }

    async fn check_reasoning(&self) {
        let live = &self.live;
        if live.effort.api_name().is_none() {
            return;
        }
        let question = "What is 17 times 23? Answer with the number only.";
        let reasoning = self
            .scenario(
                "reasoning",
                live.credential.clone(),
                live.request(question, 8_192, live.effort),
            )
            .await;
        assert_eq!(reasoning.error, None, "reasoning");
        let has_reasoning = reasoning.events.iter().any(|event| {
            matches!(
                event,
                ProviderEvent::ReasoningDelta(_) | ProviderEvent::ProviderItem(_)
            )
        });
        assert!(has_reasoning, "reasoning: none arrived");
        if live.interface != ApiInterface::Responses {
            return;
        }
        let item = reasoning
            .item()
            .expect("the reasoning came with an encrypted item");
        let replay = ModelRequest {
            messages: vec![
                Message::text(Role::User, question),
                Message {
                    role: Role::Assistant,
                    content: vec![Content::ProviderItem(item), Content::Text(reasoning.text())],
                },
                Message::text(Role::User, "And 18 times 23?"),
            ],
            ..live.request("", 8_192, live.effort)
        };
        let replayed = self
            .scenario("reasoning_replay", live.credential.clone(), replay)
            .await;
        assert_eq!(replayed.error, None, "reasoning replay");
    }

    async fn check_overflow(&self) {
        let live = &self.live;
        let Some(window) = live.context_window else {
            return;
        };
        let words = usize::try_from(window)
            .unwrap_or(usize::MAX / 8)
            .saturating_mul(2);
        let long = "word ".repeat(words);
        let overflow = self
            .scenario(
                "overflow",
                live.credential.clone(),
                live.request(&long, 64, ReasoningEffort::ServerDefault),
            )
            .await;
        if live.server == ServerKind::Ollama {
            // Ollama shortens the prompt instead of failing.
            println!("overflow on Ollama: {:?}", overflow.error);
        } else {
            assert_eq!(
                overflow.error,
                Some(ProviderError::ContextOverflow),
                "overflow"
            );
        }
    }

    /// A tool call, and the answer after its result.
    async fn check_tools(&self) {
        let live = &self.live;
        let asking = Message::text(Role::User, TOOL_PROMPT);
        let call = self
            .scenario(
                "tool_call",
                live.credential.clone(),
                ModelRequest {
                    messages: vec![asking.clone()],
                    tools: vec![status_tool()],
                    ..live.request("", 2_048, ReasoningEffort::ServerDefault)
                },
            )
            .await;
        assert_eq!(call.error, None, "tool call");
        let (call_id, name, arguments) = call.tool_call().expect("the model called the tool");
        let replay = ModelRequest {
            messages: vec![
                asking,
                Message {
                    role: Role::Assistant,
                    content: vec![Content::ToolCall {
                        call_id: call_id.clone(),
                        name,
                        arguments,
                    }],
                },
                Message {
                    role: Role::Tool,
                    content: vec![Content::ToolResult {
                        call_id,
                        output: "The status was received. Now reply with the single word: done"
                            .to_owned(),
                    }],
                },
            ],
            tools: vec![status_tool()],
            ..live.request("", 2_048, ReasoningEffort::ServerDefault)
        };
        let answered = self
            .scenario("tool_replay", live.credential.clone(), replay)
            .await;
        assert_eq!(answered.error, None, "tool replay");
    }

    /// A model that reads images answers about one (ADR-0033).
    async fn check_image(&self) {
        let live = &self.live;
        let image = Content::Image(ImageInput {
            format: ImageFormat::Png,
            width: 64,
            height: 64,
            data: ImageBytes::from(solid_png(64, 64, [220, 30, 30])),
        });
        let request = ModelRequest {
            messages: vec![Message {
                role: Role::User,
                content: vec![
                    Content::Text(
                        "What is the main color of this image? Reply with one word.".to_owned(),
                    ),
                    image,
                ],
            }],
            ..live.request("", 2_048, live.effort)
        };
        let answer = self
            .scenario("image", live.credential.clone(), request)
            .await;
        assert_eq!(answer.error, None, "image");
        assert!(!answer.text().trim().is_empty(), "image: no answer");
    }

    async fn check_model_list(&self) {
        let listed = list_models(
            &self.client,
            &self.live.base_url,
            self.live.credential.as_ref(),
        )
        .await;
        assert!(
            matches!(listed, ModelList::Listed(_) | ModelList::Unavailable),
            "models: {listed:?}"
        );
    }
}

#[tokio::test]
#[ignore = "calls the live server that the MANNIE_LIVE_* variables name"]
async fn the_live_server_behaves_as_the_fixtures_say() {
    let Some(live) = Live::from_environment() else {
        panic!("set MANNIE_LIVE_SERVER, MANNIE_LIVE_BASE_URL and MANNIE_LIVE_MODEL");
    };
    let runner = Runner::new(live);

    runner.check_text_and_length().await;
    runner.check_refusals().await;
    runner.check_reasoning().await;
    runner.check_overflow().await;
    runner.check_tools().await;
    runner.check_model_list().await;
    if runner.live.images {
        runner.check_image().await;
    }
}
