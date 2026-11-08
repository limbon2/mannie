//! The tokens of the ChatGPT account reach OpenAI only where the protocol
//! puts them: the access token in the Authorization header of the plan's
//! calls, and the refresh token in the token endpoint's form. They never
//! reach the log, the database, its write-ahead log or the transcript, even
//! when the server repeats them in its answers (ADR-0062, decision 6;
//! security architecture, rules for handling secrets).
//!
//! A sign-in, a refused token, its refresh, a passing server error and a reply
//! run through Mannie's own Responses adapter against a fixture server. This
//! test has a binary of its own: tracing caches which callsites are enabled
//! across the whole process, so a capturing subscriber shares it with no other
//! test.

// Integration tests are always built with cfg(test); saying so lets Clippy
// treat the helpers below as test code.
#![cfg(test)]

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mannie_provider::testing::{FixtureResponse, FixtureServer};
use mannie_provider::{HttpClient, ProviderRegistry};
use mannie_runtime::testing::{
    MemoryVault, chatgpt_endpoints_at, chatgpt_tokens, fast_config, open_store, plan_profile,
    sign_in_chatgpt,
};
use mannie_runtime::{AgentRegistry, InstructionRegistry, Runtime, RuntimeDeps};
use mannie_types::chat::StartConversation;
use mannie_types::clock::{Clock, SystemClock};
use mannie_types::run::RunState;

const ACCESS_1: &str = "planted-chatgpt-access-one-0123456789";
const REFRESH_1: &str = "planted-chatgpt-refresh-one-0123456789";
const ACCESS_2: &str = "planted-chatgpt-access-two-0123456789";
const REFRESH_2: &str = "planted-chatgpt-refresh-two-0123456789";

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

/// An answer of the plan that repeats a token.
fn echoing(status: u16, code: &str, token: &str) -> FixtureResponse {
    FixtureResponse::json(
        status,
        format!(
            "{{\"error\":{{\"code\":\"{code}\",\"message\":\"The token {token} was not accepted\"}}}}"
        ),
    )
}

fn streamed_reply() -> FixtureResponse {
    let event = |body: &str| format!("data: {body}\n\n");
    let mut stream = String::new();
    stream.push_str(&event(
        r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"Hello"}"#,
    ));
    stream.push_str(&event(
        r#"{"type":"response.completed","response":{"id":"resp_1","status":"completed","usage":{"input_tokens":12,"output_tokens":1}}}"#,
    ));
    FixtureResponse::event_stream(stream)
}

fn files_containing(directory: &std::path::Path, needle: &str) -> Vec<String> {
    std::fs::read_dir(directory)
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            let found = path.is_file()
                && std::fs::read(&path)
                    .unwrap()
                    .windows(needle.len())
                    .any(|window| window == needle.as_bytes());
            found.then(|| path.display().to_string())
        })
        .collect()
}

async fn start(store: mannie_store::Store, vault: Arc<MemoryVault>) -> Runtime {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let client = HttpClient::new(Arc::clone(&clock)).unwrap();
    let registry = ProviderRegistry::with_builtin(&client);
    Runtime::start(
        RuntimeDeps {
            store,
            client,
            registry,
            vault,
            clock,
            agents: AgentRegistry::with_builtin(),
            instructions: InstructionRegistry::with_builtin(),
            tools: mannie_runtime::ToolsDeps {
                registry: mannie_tools::ToolRegistry::with_system_tools(),
                protected: Vec::new(),
                mutations: mannie_tools::MutationLock::new(),
                system: Arc::new(mannie_tools::SystemFacts::detect()),
            },
            machine: mannie_runtime::MachineSettings::default(),
            mentions: mannie_runtime::MentionRegistry::new(),
            skills: None,
        },
        fast_config(),
    )
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn planted_chatgpt_tokens_reach_only_where_the_protocol_puts_them() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let server = FixtureServer::start().await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(&directory.path().join("mannie.sqlite"));
    let vault = Arc::new(MemoryVault::new());
    let runtime = start(store.clone(), Arc::clone(&vault)).await;
    sign_in_chatgpt(&runtime, &server, "user-1", ACCESS_1, REFRESH_1, false).await;
    plan_profile(&runtime, "gpt-6.1-sol", false).await;
    let endpoints = chatgpt_endpoints_at(&server);
    server.respond(echoing(401, "subscription_sharing_invalid_user", ACCESS_1));
    server.respond(chatgpt_tokens(&endpoints, ACCESS_2, REFRESH_2, None));
    server.respond(echoing(
        503,
        "subscription_sharing_usage_unavailable",
        ACCESS_2,
    ));
    server.respond(streamed_reply());

    let started = runtime
        .start_conversation(StartConversation::new("Hello"))
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let snapshot = loop {
        let snapshot = store
            .snapshot(&started.conversation_id, 10, 100)
            .await
            .unwrap()
            .unwrap();
        let ended = snapshot.turns[0]
            .run
            .as_ref()
            .is_some_and(|run| !run.state.is_active());
        if ended && runtime.active_run_count() == 0 {
            break snapshot;
        }
        assert!(std::time::Instant::now() < deadline, "the run did not end");
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    runtime.shutdown().await;

    assert_eq!(
        snapshot.turns[0].run.as_ref().map(|run| run.state),
        Some(RunState::Completed)
    );
    let requests = server.requests();
    let calls: Vec<_> = requests
        .iter()
        .filter(|request| request.path == "/v1/responses")
        .collect();
    let headers: Vec<_> = calls
        .iter()
        .map(|request| request.header("authorization").unwrap_or_default())
        .collect();
    assert_eq!(
        headers,
        [
            format!("Bearer {ACCESS_1}"),
            format!("Bearer {ACCESS_2}"),
            format!("Bearer {ACCESS_2}"),
        ]
    );
    let tokens = [ACCESS_1, REFRESH_1, ACCESS_2, REFRESH_2];
    for request in &requests {
        let body = String::from_utf8_lossy(&request.body);
        for token in tokens {
            assert!(!request.path.contains(token), "{}", request.path);
            // The refresh form carries the refresh token that it redeems.
            let expected = request.path == "/api/accounts/oauth/token" && token == REFRESH_1;
            assert!(
                expected || !body.contains(token),
                "{} carries {token}",
                request.path
            );
        }
    }
    let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains("refreshed the ChatGPT tokens"), "{log}");
    for token in tokens {
        assert!(!log.contains(token), "the log holds {token}: {log}");
        assert!(!serde_json::to_string(&snapshot).unwrap().contains(token));
        assert_eq!(
            files_containing(directory.path(), token),
            Vec::<String>::new(),
            "{token}"
        );
    }
    assert!(
        vault
            .references()
            .iter()
            .all(|reference| reference.starts_with("provider/chatgpt/")),
        "the tokens live only in the vault's parts"
    );
}
