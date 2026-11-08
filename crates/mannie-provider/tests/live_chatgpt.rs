//! Live checks of the ChatGPT plan (ADR-0062; docs/roadmap/m19-validation.md):
//! the owner signs in with ChatGPT in the browser, and the checks call OpenAI
//! on the owner's plan. They are ignored unless run on purpose:
//! `task provider:live:chatgpt` runs them, and `task provider:record:chatgpt`
//! also records the plan's fixtures.
//!
//! | Variable                       | Meaning                                                          |
//! | ------------------------------ | ---------------------------------------------------------------- |
//! | `MANNIE_LIVE_CHATGPT_MODEL`    | The model's slug; the first model of the account's catalog else  |
//! | `MANNIE_LIVE_REASONING_EFFORT` | An effort such as `low`; `low` else                              |
//! | `MANNIE_RECORD_FIXTURES`       | `1` writes the scenarios as recorded fixtures                    |
//!
//! The tokens live only in this process's memory, and the check revokes the
//! session at its end. Recording never writes a file that contains a token.
//! The first sign-in needs port 1455 free.

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use mannie_provider::chatgpt::{
    AuthorizationRequest, CallbackListener, ChatGptEndpoints, PLAN_SCOPE, TokenError, TokenSet,
    authorization, exchange_code, new_host_id, refresh_tokens, revoke_refresh_token,
    verify_id_token,
};
use mannie_provider::testing::{Exchange, RecordingProxy};
use mannie_provider::{
    Content, HttpClient, Message, ModelRequest, ProviderError, ProviderEvent, ProviderRegistry,
    Role, TOOL_PROMPT, Target, ToolChoice, ToolDefinition, chatgpt_plan_quirks, list_plan_models,
};
use mannie_types::clock::{Clock as _, SystemClock};
use mannie_types::provider::{ApiInterface, ChatGptCallbackPage, ReasoningEffort};
use mannie_types::run::FinishReason;
use mannie_types::secret::Secret;
use reqwest::Url;
use serde_json::{Value, json};

/// How long the owner has to sign in.
const SIGN_IN_TIMEOUT: Duration = Duration::from_mins(5);

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
        strict: true,
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

    fn items(&self) -> Vec<String> {
        self.events
            .iter()
            .filter_map(|event| match event {
                ProviderEvent::ProviderItem(item) => Some(item.clone()),
                _ => None,
            })
            .collect()
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

/// The signed-in session, in memory only.
struct Session {
    client_id: String,
    tokens: TokenSet,
}

impl Session {
    /// Every token, which no recorded file may contain.
    fn secrets(&self) -> Vec<String> {
        [
            Some(&self.tokens.access_token),
            self.tokens.refresh_token.as_ref(),
            self.tokens.id_token.as_ref(),
        ]
        .into_iter()
        .flatten()
        .map(|secret| secret.expose().to_owned())
        .collect()
    }
}

struct Runner {
    registry: ProviderRegistry,
    endpoints: ChatGptEndpoints,
    model: String,
    effort: ReasoningEffort,
    record: bool,
}

impl Runner {
    async fn call(&self, base_url: &Url, token: &Secret, request: ModelRequest) -> Outcome {
        let adapter = self.registry.adapter(ApiInterface::Responses).unwrap();
        let target = Target {
            base_url: base_url.clone(),
            credential: Some(token.clone()),
            quirks: chatgpt_plan_quirks(),
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
    async fn scenario(&self, name: &str, session: &Session, request: ModelRequest) -> Outcome {
        let token = &session.tokens.access_token;
        if !self.record {
            return self.call(&self.endpoints.api_base, token, request).await;
        }
        let proxy = RecordingProxy::start(self.endpoints.api_base.clone())
            .await
            .unwrap();
        let outcome = self.call(&proxy.base_url(), token, request.clone()).await;
        let exchange = proxy
            .exchanges()
            .pop()
            .expect("the proxy forwarded the call");
        write_fixture(name, &request, &exchange, &session.secrets());
        outcome
    }

    fn request(&self, messages: Vec<Message>, tools: Vec<ToolDefinition>) -> ModelRequest {
        ModelRequest {
            model: self.model.clone(),
            instructions: "You are a test of Mannie. Answer briefly.".to_owned(),
            messages,
            max_output_tokens: 2_048,
            reasoning_effort: self.effort,
            tools,
            tool_choice: ToolChoice::Auto,
        }
    }
}

/// Signs in with ChatGPT through the browser, as the first sign-in of an
/// installation does.
async fn sign_in(client: &HttpClient, endpoints: &ChatGptEndpoints) -> Session {
    let host_id = new_host_id().unwrap();
    let listener = CallbackListener::bind(endpoints.registration_port)
        .await
        .expect("port 1455 is free");
    let authorization = authorization(&AuthorizationRequest {
        endpoints,
        host_id: &host_id,
        port: listener.port(),
        client_id: None,
        login_hint: None,
    })
    .unwrap();
    println!(
        "Open this page and sign in with ChatGPT:\n\n{}\n",
        authorization.url
    );
    let page = ChatGptCallbackPage {
        title: "Mannie live check".to_owned(),
        signed_in: "Signed in. Go back to the terminal.".to_owned(),
        failed: "The sign-in failed. Go back to the terminal.".to_owned(),
    };
    let callback = listener
        .wait(&authorization.state, &page, SIGN_IN_TIMEOUT)
        .await
        .expect("the browser came back with a code");
    let client_id = callback
        .client_id
        .clone()
        .expect("the return of a first sign-in names the issued client");
    let exchanged = exchange_code(
        client,
        endpoints,
        &client_id,
        &callback.code,
        &authorization,
    )
    .await;
    callback.answer(&page, exchanged.is_ok()).await;
    let tokens = exchanged.expect("the code is redeemed");
    let id_token = tokens.id_token.clone().expect("the answer has an ID token");
    verify_id_token(
        client,
        endpoints,
        &id_token,
        &client_id,
        &authorization.nonce,
    )
    .await
    .expect("the ID token passes its checks");
    assert!(
        tokens.grants(PLAN_SCOPE),
        "the account allowed Mannie to use its ChatGPT plan"
    );
    println!("signed in; the plan's scope was granted");
    Session { client_id, tokens }
}

#[tokio::test]
#[ignore = "signs in with ChatGPT in the browser and calls OpenAI on the owner's plan"]
async fn the_chatgpt_plan_behaves_as_mannie_expects() {
    let client = HttpClient::new(Arc::new(SystemClock)).unwrap();
    let endpoints = ChatGptEndpoints::openai();
    let mut session = sign_in(&client, &endpoints).await;
    let runner = runner_for(&client, &endpoints, &session).await;

    check_text(&runner, &session).await;
    check_tools(&runner, &session).await;
    probe_shapes(&runner, &session).await;
    check_refresh_and_revocation(&client, &runner, &mut session).await;
}

/// Reads the catalog and chooses the model of the checks.
async fn runner_for(
    client: &HttpClient,
    endpoints: &ChatGptEndpoints,
    session: &Session,
) -> Runner {
    let variable = |name: &str| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
    };
    let models = list_plan_models(client, &endpoints.api_base, &session.tokens.access_token)
        .await
        .expect("the catalog is readable");
    assert_ne!(models.len(), 0, "the catalog offers models");
    println!(
        "catalog: {}",
        models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let model = variable("MANNIE_LIVE_CHATGPT_MODEL").unwrap_or_else(|| models[0].id.clone());
    let effort = variable("MANNIE_LIVE_REASONING_EFFORT")
        .and_then(|effort| serde_json::from_value(json!(effort)).ok())
        .unwrap_or(ReasoningEffort::Low);
    Runner {
        registry: ProviderRegistry::with_builtin(client),
        endpoints: endpoints.clone(),
        model,
        effort,
        record: variable("MANNIE_RECORD_FIXTURES").as_deref() == Some("1"),
    }
}

/// A reply without an output limit.
async fn check_text(runner: &Runner, session: &Session) {
    let text = runner
        .scenario(
            "plan_text",
            session,
            runner.request(
                vec![Message::text(
                    Role::User,
                    "Reply with the single word: ready",
                )],
                Vec::new(),
            ),
        )
        .await;
    assert_eq!(text.error, None, "plan_text");
    assert_ne!(text.text().trim(), "", "plan_text: no answer");
    assert_eq!(text.finish(), Some(FinishReason::Stop), "plan_text");
}

/// A call of a tool in the namespace, then the call and its reasoning
/// replayed with the namespace and a phase.
async fn check_tools(runner: &Runner, session: &Session) {
    let asking = Message::text(Role::User, TOOL_PROMPT);
    let first = runner
        .scenario(
            "plan_tool_call",
            session,
            runner.request(vec![asking.clone()], vec![status_tool()]),
        )
        .await;
    assert_eq!(first.error, None, "plan_tool_call");
    let (call_id, name, arguments) = first.tool_call().expect("plan_tool_call: no tool call");
    assert_eq!(name, "report_status");

    let mut answer = vec![Content::Text("I will report the status.".to_owned())];
    answer.splice(0..0, first.items().into_iter().map(Content::ProviderItem));
    answer.push(Content::ToolCall {
        call_id: call_id.clone(),
        name,
        arguments,
    });
    let result = Message {
        role: Role::Tool,
        content: vec![Content::ToolResult {
            call_id,
            output: "The status was received. Now reply with the single word: done".to_owned(),
            images: Vec::new(),
        }],
    };
    let answered = Message {
        role: Role::Assistant,
        content: answer,
    };
    let replay = runner
        .scenario(
            "plan_tool_replay",
            session,
            runner.request(vec![asking, answered, result], vec![status_tool()]),
        )
        .await;
    assert_eq!(replay.error, None, "plan_tool_replay");
    assert_eq!(
        replay.finish(),
        Some(FinishReason::Stop),
        "plan_tool_replay"
    );
}

/// What the route says to the shapes that Mannie avoids, to settle the
/// quirks (docs/roadmap/m19-validation.md): flat function tools, and a
/// replayed call of a tool that the namespace no longer offers.
async fn probe_shapes(runner: &Runner, session: &Session) {
    let status = json!({
        "type": "function",
        "name": "report_status",
        "description": "Reports the status of the test.",
        "parameters": status_tool().parameters,
    });
    let flat = probe(
        &runner.endpoints.api_base,
        &session.tokens.access_token,
        &json!({
            "model": runner.model,
            "input": [{ "role": "user", "content": [{ "type": "input_text", "text": TOOL_PROMPT }] }],
            "tools": [status],
            "stream": true,
            "store": false,
        }),
    )
    .await;
    println!("flat function tools: {flat}");
    let missing = probe(
        &runner.endpoints.api_base,
        &session.tokens.access_token,
        &json!({
            "model": runner.model,
            "input": [
                { "role": "user", "content": [{ "type": "input_text", "text": "What does notes.txt say?" }] },
                { "type": "function_call", "call_id": "call_gone", "name": "read_file", "namespace": "mannie", "arguments": "{\"path\":\"notes.txt\"}" },
                { "type": "function_call_output", "call_id": "call_gone", "output": "hello" },
            ],
            "tools": [{
                "type": "namespace",
                "name": "mannie",
                "description": "Tools that Mannie runs for this conversation.",
                "tools": [status],
            }],
            "stream": true,
            "store": false,
        }),
    )
    .await;
    println!("a replayed call of a tool the namespace lacks: {missing}");
}

/// A refresh replaces the tokens and the new access token works, then
/// revocation ends the session.
async fn check_refresh_and_revocation(client: &HttpClient, runner: &Runner, session: &mut Session) {
    let endpoints = &runner.endpoints;
    let refresh = session
        .tokens
        .refresh_token
        .clone()
        .expect("the sign-in gave a refresh token");
    let refreshed = refresh_tokens(client, endpoints, &session.client_id, &refresh)
        .await
        .expect("the refresh succeeds");
    assert_ne!(
        refreshed.access_token.expose(),
        session.tokens.access_token.expose(),
        "the refresh gave a new access token"
    );
    let rotated = refreshed
        .refresh_token
        .clone()
        .unwrap_or_else(|| refresh.clone());
    session.tokens = TokenSet {
        refresh_token: Some(rotated.clone()),
        ..refreshed
    };
    let after_refresh = runner
        .call(
            &endpoints.api_base,
            &session.tokens.access_token,
            runner.request(
                vec![Message::text(
                    Role::User,
                    "Reply with the single word: ready",
                )],
                Vec::new(),
            ),
        )
        .await;
    assert_eq!(after_refresh.error, None, "a call after the refresh");

    // The replaced refresh token no longer works, when the refresh rotated it.
    if rotated.expose() != refresh.expose() {
        let reused = refresh_tokens(client, endpoints, &session.client_id, &refresh).await;
        println!("a replaced refresh token gives: {reused:?}");
    }

    revoke_refresh_token(client, endpoints, &session.client_id, &rotated)
        .await
        .expect("the revocation is confirmed");
    let revoked = refresh_tokens(client, endpoints, &session.client_id, &rotated)
        .await
        .unwrap_err();
    assert!(
        matches!(revoked, TokenError::Terminal { .. }),
        "a revoked refresh token is gone for good: {revoked:?}"
    );
}

/// Posts a raw request to the plan's Responses API and describes the answer:
/// its status and the start of its body, for the shapes that the adapter
/// never sends.
async fn probe(api_base: &Url, token: &Secret, body: &Value) -> String {
    let url = format!("{}/responses", api_base.as_str().trim_end_matches('/'));
    // The process installed ring as rustls's provider when it built the
    // shared client, so a plain client builds too.
    let answer = reqwest::Client::new()
        .post(url)
        .bearer_auth(token.expose())
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.to_string())
        .send()
        .await;
    match answer {
        Ok(response) => {
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            let start: String = text.chars().take(400).collect();
            format!("status {status}: {start}")
        }
        Err(error) => format!("no answer: {error}"),
    }
}

/// Writes a recorded fixture of the plan, unless a file would contain a
/// token.
fn write_fixture(scenario: &str, request: &ModelRequest, exchange: &Exchange, secrets: &[String]) {
    let folder = fixture_folder(scenario);
    std::fs::create_dir_all(&folder).unwrap();
    let today = time::OffsetDateTime::from_unix_timestamp(SystemClock.now().as_millis() / 1000)
        .unwrap()
        .date()
        .to_string();
    let manifest = json!({
        "server": "open_ai",
        "interface": "responses",
        "access": "chatgpt_plan",
        "provenance": "recorded",
        "recordedOn": today,
        "request": {
            "model": request.model,
            "instructions": request.instructions,
            "messages": request.messages.iter().map(manifest_message).collect::<Vec<_>>(),
            "maxOutputTokens": request.max_output_tokens,
            "reasoningEffort": request.reasoning_effort,
            "tools": request.tools.iter().map(|tool| json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            })).collect::<Vec<_>>(),
            "toolChoice": "auto",
        },
    });
    let headers: serde_json::Map<String, Value> = exchange
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), json!(value)))
        .collect();
    let response = json!({ "status": exchange.status, "headers": headers });
    let files: [(&str, Vec<u8>); 4] = [
        ("manifest.json", pretty(&manifest)),
        ("response.json", pretty(&response)),
        ("body.sse", exchange.body.clone()),
        ("request.json", exchange.request.body.clone()),
    ];
    for (name, bytes) in &files {
        for secret in secrets {
            let leaks = bytes
                .windows(secret.len())
                .any(|window| window == secret.as_bytes());
            assert!(
                !leaks,
                "{name} of {scenario} contains a token; nothing was written"
            );
        }
    }
    for (name, bytes) in files {
        std::fs::write(folder.join(name), bytes).unwrap();
    }
    println!("recorded {}", folder.display());
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
    for content in &message.content {
        match content {
            Content::Text(text) => entry["text"] = json!(text),
            Content::ProviderItem(item) => {
                entry["item"] = serde_json::from_str(item).unwrap_or(Value::Null);
            }
            Content::ToolCall {
                call_id,
                name,
                arguments,
            } => calls.push(json!({ "callId": call_id, "name": name, "arguments": arguments })),
            Content::ToolResult {
                call_id, output, ..
            } => results.push(json!({ "callId": call_id, "output": output })),
            Content::Image(_) => {}
        }
    }
    if !calls.is_empty() {
        entry["toolCalls"] = json!(calls);
    }
    if !results.is_empty() {
        entry["toolResults"] = json!(results);
    }
    entry
}

fn pretty(value: &Value) -> Vec<u8> {
    let mut text = serde_json::to_string_pretty(value).unwrap();
    text.push('\n');
    text.into_bytes()
}

fn fixture_folder(scenario: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/open_ai")
        .join(scenario)
}
