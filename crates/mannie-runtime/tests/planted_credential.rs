//! A credential reaches the provider only in the Authorization header: never
//! the log, the database, the write-ahead log, the transcript or an error
//! message, even when the server repeats it in its answers (security
//! architecture, rules for handling secrets; ADR-0010).
//!
//! A reply runs through the real Chat Completions adapter against a fixture
//! server that echoes the credential in its errors. This test has a binary of
//! its own: tracing caches which callsites are enabled across the whole
//! process, so a capturing subscriber shares it with no other test.

// Integration tests are always built with cfg(test); saying so lets Clippy
// treat the helpers below as test code.
#![cfg(test)]

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mannie_provider::testing::{FixtureResponse, FixtureServer};
use mannie_provider::{HttpClient, ProviderRegistry};
use mannie_runtime::testing::{MemoryVault, fast_config, open_store};
use mannie_runtime::{AgentRegistry, InstructionRegistry, Runtime, RuntimeDeps};
use mannie_types::chat::StartConversation;
use mannie_types::clock::{Clock, SystemClock};
use mannie_types::ids::{ConversationId, RunId};
use mannie_types::provider::{ApiInterface, CreateProviderProfile, ProfileDraft, ReasoningEffort};
use mannie_types::run::{RunFailure, RunState};
use mannie_types::secret::Secret;
use mannie_types::transcript::PartContent;

const PLANTED: &str = "sk-planted-runtime-secret-0123456789";

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

fn echoing_error(status: u16) -> FixtureResponse {
    FixtureResponse::json(
        status,
        format!(
            "{{\"error\":{{\"message\":\"Incorrect API key provided: {PLANTED}\",\"code\":\"invalid_api_key\"}}}}"
        ),
    )
}

fn streamed_reply() -> FixtureResponse {
    let chunk = |body: &str| format!("data: {body}\n\n");
    let mut stream = String::new();
    stream.push_str(&chunk(r#"{"choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}"#));
    stream.push_str(&chunk(
        r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
    ));
    stream.push_str(&chunk(
        r#"{"choices":[],"usage":{"prompt_tokens":15,"completion_tokens":1,"total_tokens":16}}"#,
    ));
    stream.push_str(&chunk("[DONE]"));
    FixtureResponse::event_stream(stream)
}

async fn wait_for_end(
    store: &mannie_store::Store,
    conversation: &ConversationId,
    run: &RunId,
) -> mannie_types::transcript::ConversationSnapshot {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let snapshot = store
            .snapshot(conversation, 10, 100)
            .await
            .unwrap()
            .unwrap();
        let ended = snapshot.turns[0]
            .run
            .as_ref()
            .is_some_and(|view| view.id == *run && !view.state.is_active());
        if ended {
            return snapshot;
        }
        assert!(std::time::Instant::now() < deadline, "the run did not end");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn files_contain(directory: &std::path::Path, needle: &str) -> Vec<String> {
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

/// A runtime with Mannie's own adapters and a profile with the planted
/// credential for the server at `base_url`.
async fn start_with_adapters(store: mannie_store::Store, base_url: &str) -> Runtime {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let client = HttpClient::new(Arc::clone(&clock)).unwrap();
    let registry = ProviderRegistry::with_builtin(&client);
    let runtime = Runtime::start(
        RuntimeDeps {
            store,
            client,
            registry,
            vault: Arc::new(MemoryVault::new()),
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
        },
        fast_config(),
    )
    .await
    .unwrap();
    runtime
        .create_profile(CreateProviderProfile {
            draft: ProfileDraft {
                name: "Local".to_owned(),
                interface: ApiInterface::ChatCompletions,
                base_url: base_url.to_owned(),
                model: "test-model".to_owned(),
                context_window: 32_768,
                output_limit: 4_096,
                reasoning_effort: ReasoningEffort::ServerDefault,
                use_tools: false,
            },
            credential: Some(Secret::new(PLANTED.to_owned())),
        })
        .await
        .unwrap();
    runtime
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_planted_credential_reaches_only_the_authorization_header() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let server = FixtureServer::start().await.unwrap();
    server.respond(echoing_error(401));
    server.respond(echoing_error(500));
    server.respond(streamed_reply());
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(&directory.path().join("mannie.sqlite"));
    let runtime = start_with_adapters(store.clone(), server.base_url().as_str()).await;

    let started = runtime
        .start_conversation(StartConversation {
            text: "Hello".to_owned(),
            profile_id: None,
            project_id: None,
        })
        .await
        .unwrap();
    let failed = wait_for_end(&store, &started.conversation_id, &started.run_id).await;
    let retried = runtime.retry_reply(&started.conversation_id).await.unwrap();
    let completed = wait_for_end(&store, &started.conversation_id, &retried.run_id).await;
    runtime.shutdown().await;

    let failed_run = failed.turns[0].run.as_ref().unwrap();
    assert_eq!(
        failed_run
            .outcome
            .as_ref()
            .and_then(|outcome| outcome.failure.clone()),
        Some(RunFailure::InvalidCredential)
    );
    let completed_turn = &completed.turns[0];
    assert_eq!(
        completed_turn.run.as_ref().map(|run| run.state),
        Some(RunState::Completed)
    );
    assert!(completed_turn.parts.iter().any(|part| part.content
        == PartContent::Text {
            text: "Hello".to_owned()
        }));

    let requests = server.requests();
    assert_eq!(
        requests.len(),
        3,
        "the failed call, the transient error and the retry"
    );
    for request in &requests {
        let authorization = format!("Bearer {PLANTED}");
        assert_eq!(
            request.header("authorization"),
            Some(authorization.as_str())
        );
        assert!(!request.path.contains(PLANTED));
        assert!(!String::from_utf8_lossy(&request.body).contains(PLANTED));
    }
    let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(
        log.contains("a run ended"),
        "the runtime logged its runs: {log}"
    );
    assert!(!log.contains(PLANTED), "{log}");
    for snapshot in [&failed, &completed] {
        assert!(!serde_json::to_string(snapshot).unwrap().contains(PLANTED));
    }
    assert_eq!(
        files_contain(directory.path(), PLANTED),
        Vec::<String>::new()
    );
}
