//! What the adapters write to the log. A credential that a server repeats in
//! an error message never reaches it (security architecture, rules for
//! handling secrets), and an answer that Mannie cannot read is described by
//! its names, sizes and faults, never by its content (engineering standards,
//! logging).
//!
//! These tests have a binary of their own: tracing caches which callsites are
//! enabled across the whole process, so a capturing subscriber shares it with
//! no other test.

#![cfg(test)]

use std::io::Write;
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use mannie_provider::testing::{FixtureResponse, FixtureServer};
use mannie_provider::{
    HttpClient, Message, ModelRequest, ProviderError, ProviderRegistry, Role, Target, quirks_for,
};
use mannie_types::clock::{ManualClock, UtcMillis};
use mannie_types::provider::{ApiInterface, ReasoningEffort};
use mannie_types::run::RunFailure;
use mannie_types::secret::Secret;

const PLANTED: &str = "sk-planted-log-secret-0123456789";

/// Collects what the tracing subscriber writes.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }

    /// The first line that holds `message`.
    fn line(&self, message: &str) -> String {
        let log = self.text();
        let Some(line) = log.lines().find(|line| line.contains(message)) else {
            panic!("no line holds {message:?}: {log}");
        };
        line.to_owned()
    }
}

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Captures every level of the log on this thread while the guard lives.
fn capture() -> (Captured, tracing::subscriber::DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    (captured, tracing::subscriber::set_default(subscriber))
}

/// The first error of a call to the fixture server's Responses API.
async fn first_error(server: &FixtureServer) -> Option<ProviderError> {
    let client = HttpClient::new(Arc::new(ManualClock::new(UtcMillis::from_millis(0)))).unwrap();
    let registry = ProviderRegistry::with_builtin(&client);
    let adapter = registry.adapter(ApiInterface::Responses).unwrap();
    let target = Target {
        base_url: server.base_url(),
        credential: Some(Secret::new(PLANTED.to_owned())),
        quirks: quirks_for(ApiInterface::Responses, None),
    };
    let request = ModelRequest {
        model: "gpt-6-luna".to_owned(),
        instructions: String::new(),
        messages: vec![Message::text(Role::User, "Hello")],
        max_output_tokens: 64,
        reasoning_effort: ReasoningEffort::ServerDefault,
        tools: Vec::new(),
        tool_choice: mannie_provider::ToolChoice::Auto,
    };
    let mut events = adapter.stream(&target, request);
    while let Some(item) = events.next().await {
        if let Err(error) = item {
            return Some(error);
        }
    }
    None
}

#[tokio::test]
async fn a_credential_that_the_server_repeats_never_reaches_the_log() {
    let (captured, _guard) = capture();
    let server = FixtureServer::start().await.unwrap();
    server.respond(FixtureResponse::json(
        401,
        format!("{{\"error\":{{\"message\":\"Incorrect API key provided: {PLANTED}\",\"code\":\"invalid_api_key\"}}}}"),
    ));

    let error = first_error(&server).await;

    assert_eq!(
        error,
        Some(ProviderError::Permanent(RunFailure::InvalidCredential))
    );
    let log = captured.text();
    assert!(log.contains("the provider refused the call"), "{log}");
    assert!(log.contains("[REDACTED]"), "{log}");
    assert!(!log.contains(PLANTED), "{log}");
}

#[tokio::test]
async fn an_event_that_is_not_json_is_logged_by_its_name_and_size() {
    const WORDS: &str = "the model's planted words";
    let (captured, _guard) = capture();
    let server = FixtureServer::start().await.unwrap();
    server.respond(FixtureResponse::event_stream(format!(
        "event: response.created\n\
         data: {{\"type\":\"response.created\",\"response\":{{\"id\":\"resp_1\"}}}}\n\n\
         event: keepalive\n\
         data: {WORDS}\n\n"
    )));

    let error = first_error(&server).await;

    assert_eq!(
        error,
        Some(ProviderError::Permanent(RunFailure::InvalidResponse))
    );
    let warning = captured.line("an event of the provider's stream is not JSON");
    assert!(warning.contains("WARN"), "{warning}");
    assert!(warning.contains("keepalive"), "{warning}");
    assert!(
        warning.contains(&format!("bytes={}", WORDS.len())),
        "{warning}"
    );
    assert!(!captured.text().contains(WORDS), "{}", captured.text());
}

#[tokio::test]
async fn an_answer_that_is_not_an_event_stream_is_logged_by_its_type_and_code() {
    let (captured, _guard) = capture();
    let server = FixtureServer::start().await.unwrap();
    server.respond(FixtureResponse::json(
        200,
        r#"{"error":{"code":"planted_code","type":"invalid_request_error","message":"the owner's planted words"}}"#,
    ));

    let error = first_error(&server).await;

    assert_eq!(
        error,
        Some(ProviderError::Permanent(RunFailure::InvalidResponse))
    );
    let warning = captured.line("the provider answered without an event stream");
    for expected in [
        "WARN",
        "status=200",
        "application/json",
        "planted_code",
        "invalid_request_error",
    ] {
        assert!(warning.contains(expected), "{warning}");
    }
    // The message may repeat the owner's content, so only the debug log
    // holds it.
    assert!(!warning.contains("planted words"), "{warning}");
}
