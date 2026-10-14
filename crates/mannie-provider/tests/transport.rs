//! The transport of provider calls over real HTTP: no redirect is followed,
//! the credential travels only as a bearer header, a response must be an event
//! stream, and a silent server times out. `tests/log.rs` checks the log.

#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use mannie_provider::testing::{FixtureResponse, FixtureServer};
use mannie_provider::{
    HttpClient, Message, ModelRequest, ProviderError, ProviderEvent, ProviderRegistry, Role,
    Target, Timeouts, quirks_for,
};
use mannie_types::clock::{ManualClock, UtcMillis};
use mannie_types::provider::{ApiInterface, ReasoningEffort};
use mannie_types::run::RunFailure;
use mannie_types::secret::Secret;

const PLANTED: &str = "sk-planted-transport-secret-0123456789";

fn client(timeouts: Timeouts) -> HttpClient {
    HttpClient::with_timeouts(
        Arc::new(ManualClock::new(UtcMillis::from_millis(0))),
        timeouts,
    )
    .unwrap()
}

fn request() -> ModelRequest {
    ModelRequest {
        model: "gpt-6-luna".to_owned(),
        instructions: String::new(),
        messages: vec![Message::text(Role::User, "Hello")],
        max_output_tokens: 64,
        reasoning_effort: ReasoningEffort::ServerDefault,
    }
}

async fn call(
    client: &HttpClient,
    server: &FixtureServer,
    credential: Option<Secret>,
) -> (Vec<ProviderEvent>, Option<ProviderError>) {
    let registry = ProviderRegistry::with_builtin(client);
    let adapter = registry.adapter(ApiInterface::Responses).unwrap();
    let target = Target {
        base_url: server.base_url(),
        credential,
        quirks: quirks_for(ApiInterface::Responses, None),
    };
    let mut stream = adapter.stream(&target, request());
    let mut events = Vec::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(event) => events.push(event),
            Err(error) => return (events, Some(error)),
        }
    }
    (events, None)
}

#[tokio::test]
async fn a_redirect_is_refused_and_not_followed() {
    let server = FixtureServer::start().await.unwrap();
    let mut redirect = FixtureResponse::json(302, "");
    redirect.headers.push((
        "location".to_owned(),
        "http://127.0.0.1:9/v1/responses".to_owned(),
    ));
    server.respond(redirect);

    let (_, error) = call(
        &client(Timeouts::default()),
        &server,
        Some(Secret::new(PLANTED.to_owned())),
    )
    .await;

    assert_eq!(
        error,
        Some(ProviderError::Permanent(RunFailure::RedirectRefused))
    );
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn the_credential_travels_only_as_a_bearer_header() {
    let server = FixtureServer::start().await.unwrap();
    server.respond(FixtureResponse::event_stream(
        "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n",
    ));

    let (events, error) = call(
        &client(Timeouts::default()),
        &server,
        Some(Secret::new(PLANTED.to_owned())),
    )
    .await;

    assert_eq!(error, None);
    assert_eq!(events.len(), 1);
    let requests = server.requests();
    let sent = &requests[0];
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.path, "/v1/responses");
    assert_eq!(
        sent.header("authorization"),
        Some(format!("Bearer {PLANTED}").as_str())
    );
    let body = String::from_utf8(sent.body.clone()).unwrap();
    assert!(
        !body.contains(PLANTED),
        "the body never carries the credential"
    );
    assert!(
        !sent.path.contains(PLANTED),
        "the URL never carries the credential"
    );
}

#[tokio::test]
async fn a_response_that_is_not_an_event_stream_is_invalid() {
    let server = FixtureServer::start().await.unwrap();
    server.respond(FixtureResponse::json(200, "{\"output\":[]}"));

    let (_, error) = call(&client(Timeouts::default()), &server, None).await;

    assert_eq!(
        error,
        Some(ProviderError::Permanent(RunFailure::InvalidResponse))
    );
}

#[tokio::test]
async fn a_server_that_goes_silent_times_out() {
    let server = FixtureServer::start().await.unwrap();
    let mut stalled = FixtureResponse::event_stream(
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hel\"}\n\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"lo\"}\n\n",
    );
    stalled.chunking = mannie_provider::testing::Chunking::Random { seed: 9, max: 40 };
    stalled.stall = true;
    server.respond(stalled);
    let timeouts = Timeouts {
        first_event: Duration::from_millis(500),
        idle: Duration::from_millis(500),
        ..Timeouts::default()
    };

    let (_, error) = call(&client(timeouts), &server, None).await;

    assert_eq!(
        error,
        Some(ProviderError::Transient {
            cause: RunFailure::TimedOut,
            retry_after: None
        })
    );
}

#[tokio::test]
async fn an_unreachable_server_is_a_transient_failure() {
    let server = FixtureServer::start().await.unwrap();
    let address = server.base_url();
    drop(server);
    let registry = ProviderRegistry::with_builtin(&client(Timeouts::default()));
    let adapter = registry.adapter(ApiInterface::Responses).unwrap();
    let target = Target {
        base_url: address,
        credential: None,
        quirks: quirks_for(ApiInterface::Responses, None),
    };

    let first = adapter.stream(&target, request()).next().await;

    assert!(
        matches!(
            first,
            Some(Err(ProviderError::Transient {
                cause: RunFailure::ConnectionFailed,
                ..
            }))
        ),
        "{first:?}"
    );
}
