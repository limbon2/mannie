//! A credential that the runtime lends a plugin for a task is logged with the
//! plugin, the task and the profile, and never with the credential itself; a
//! refused lend is logged with its reason (security architecture, rules for
//! handling secrets; ADR-0034, decision 5).
//!
//! This test has a binary of its own: tracing caches which callsites are
//! enabled across the whole process, so a capturing subscriber shares it
//! with no other test.

// Integration tests are always built with cfg(test); saying so lets Clippy
// treat the helpers below as test code.
#![cfg(test)]

use std::io::Write;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::points::executors::{Executors, TaskExecutor, TaskSubmission};
use mannie_plugin_sdk::services::credentials::LendError;
use mannie_runtime::testing::{
    FAKE_EXECUTOR_KIND, FAKE_EXECUTOR_TOOL, FakeExecutor, Harness, HarnessOptions, add_plugin,
    credentials, executor_host, reply, tool_calls,
};
use mannie_types::chat::{StartConversation, StartedConversation};
use mannie_types::provider::{
    ApiInterface, CreateProviderProfile, ProfileDraft, ProviderProfile, ReasoningEffort,
};
use mannie_types::secret::Secret;
use serde_json::json;

const PLANTED: &str = "sk-planted-lent-credential-0123456789";

const SECOND_PLANTED: &str = "sk-planted-second-credential-0123456789";

/// Collects what the tracing subscriber writes.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A runtime whose profile has the planted credential, with the fake plugin
/// on, and a second profile with a credential of its own.
async fn start() -> (Harness, Arc<FakeExecutor>, ProviderProfile) {
    let harness = Harness::with_options(HarnessOptions {
        use_tools: true,
        credential: Some(PLANTED.to_owned()),
        ..HarnessOptions::default()
    })
    .await;
    let executor = FakeExecutor::new();
    let contribution: Arc<dyn TaskExecutor> = Arc::<FakeExecutor>::clone(&executor);
    add_plugin(
        &harness.runtime,
        FAKE_EXECUTOR_KIND,
        Contributions::new().add::<Executors>(contribution),
    )
    .unwrap();
    let second = harness
        .runtime
        .create_profile(CreateProviderProfile {
            draft: ProfileDraft {
                name: "Second".to_owned(),
                interface: ApiInterface::ChatCompletions,
                base_url: "http://127.0.0.1:9/v1".to_owned(),
                model: "test-model".to_owned(),
                context_window: 128_000,
                output_limit: 16_384,
                reasoning_effort: ReasoningEffort::ServerDefault,
                use_tools: true,
            },
            credential: Some(Secret::new(SECOND_PLANTED.to_owned())),
        })
        .await
        .unwrap();
    (harness, executor, second)
}

/// Starts a reply that gives the fake executor a task in the foreground, and
/// returns the reply and the task's submission.
async fn give_task(
    harness: &Harness,
    executor: &FakeExecutor,
) -> (StartedConversation, TaskSubmission) {
    let arguments = json!({
        "activity": "Giving Build it",
        "target": "box-01",
        "title": "Build it",
        "instruction": "Build the backend.",
        "background": false,
    })
    .to_string();
    harness
        .model
        .push_reply(tool_calls(&[("call_1", FAKE_EXECUTOR_TOOL, &arguments)]));
    harness.model.push_reply(reply("Done."));
    let started = harness
        .runtime
        .start_conversation(StartConversation::new("OWNER: build it on the box"))
        .await
        .unwrap();
    let submission = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if let Some(submission) = executor.submissions().into_iter().next() {
                return submission;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the executor got the task in time");
    (started, submission)
}

/// The first line of the log with `message`.
fn line_with<'log>(log: &'log str, message: &str) -> &'log str {
    log.lines()
        .find(|line| line.contains(message))
        .unwrap_or_else(|| panic!("{message:?} is logged: {log}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_lend_log_names_the_task_and_profile_and_never_the_credential() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();
    let (harness, executor, second) = start().await;
    let (started, submission) = give_task(&harness, &executor).await;
    let lender = credentials(&harness.runtime, FAKE_EXECUTOR_KIND);
    let profile = harness.profile.id.clone();

    let lent = lender
        .lend(submission.task_id.clone(), profile.clone())
        .await
        .unwrap()
        .expect("the profile has a credential");
    let refused = lender
        .lend(submission.task_id.clone(), second.id.clone())
        .await;
    executor_host(&harness.runtime, FAKE_EXECUTOR_KIND)
        .publish_task_state(FakeExecutor::reported(&submission, 2, "Built."))
        .await
        .unwrap();
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    harness.runtime.shutdown().await;

    assert_eq!(lent.credential.expose(), PLANTED);
    assert!(
        matches!(refused, Err(LendError::NotPermitted)),
        "{refused:?}"
    );
    let log = captured.text();
    let lent_line = line_with(&log, "lent a credential for a task");
    for named in [
        FAKE_EXECUTOR_KIND,
        submission.task_id.as_str(),
        profile.as_str(),
    ] {
        assert!(lent_line.contains(named), "{named}: {lent_line}");
    }
    let refused_line = line_with(&log, "refused to lend a credential for a task");
    for named in [
        FAKE_EXECUTOR_KIND,
        submission.task_id.as_str(),
        second.id.as_str(),
        "another_profile",
    ] {
        assert!(refused_line.contains(named), "{named}: {refused_line}");
    }
    assert!(!log.contains(PLANTED), "{log}");
    assert!(!log.contains(SECOND_PLANTED), "{log}");
}
