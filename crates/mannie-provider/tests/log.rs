//! A credential that a server repeats in an error message never reaches the
//! log (security architecture, rules for handling secrets).
//!
//! This test has a binary of its own: tracing caches which callsites are
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

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn a_credential_that_the_server_repeats_never_reaches_the_log() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let server = FixtureServer::start().await.unwrap();
    server.respond(FixtureResponse::json(
        401,
        format!("{{\"error\":{{\"message\":\"Incorrect API key provided: {PLANTED}\",\"code\":\"invalid_api_key\"}}}}"),
    ));

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
    let error = match adapter.stream(&target, request).next().await {
        Some(Err(error)) => Some(error),
        _ => None,
    };

    assert_eq!(
        error,
        Some(ProviderError::Permanent(RunFailure::InvalidCredential))
    );
    let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains("the provider refused the call"), "{log}");
    assert!(log.contains("[REDACTED]"), "{log}");
    assert!(!log.contains(PLANTED), "{log}");
}
