//! Killing Mannie while a reply runs leaves an intact transcript: the next
//! start marks the run as interrupted, keeps the text stored so far, and
//! Retry works (docs/features/chat/conversations.md, stop, retry and
//! recovery).
//!
//! The test runs a child process (this test binary, running the ignored
//! `child_reply` test) whose reply fails once, is retried, and then streams
//! reasoning and text. The child prints every event it sees. The parent kills
//! the child after a different event in each round, which covers each point
//! where a run stores something, then starts a runtime on the same database.

// Integration tests are always built with cfg(test); saying so lets Clippy
// treat the helpers below as test code.
#![cfg(test)]

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use mannie_provider::ProviderEvent;
use mannie_provider::testing::Step;
use mannie_runtime::testing::{
    FakeModel, MemoryVault, fast_config, open_store, reply, server_error, start_runtime,
};
use mannie_types::chat::{StartConversation, SubscribeConversation};
use mannie_types::provider::{ApiInterface, CreateProviderProfile, ProfileDraft, ReasoningEffort};
use mannie_types::run::RunState;
use mannie_types::secret::Secret;
use mannie_types::transcript::{ConversationEvent, ConversationUpdate, PartContent, PartState};

const CHILD_DIRECTORY: &str = "MANNIE_CRASH_CHILD_DIRECTORY";

/// The events after which the parent kills the child: the turn and its queued
/// run, the run starting, the failed attempt's text, the discarded attempt
/// and the retry's wait, the reasoning, and the streamed text.
const KILL_AFTER: [usize; 8] = [1, 3, 6, 8, 11, 13, 16, 20];

fn draft() -> ProfileDraft {
    ProfileDraft {
        name: "Test".to_owned(),
        interface: ApiInterface::ChatCompletions,
        base_url: "http://127.0.0.1:9/v1".to_owned(),
        model: "test-model".to_owned(),
        context_window: 128_000,
        output_limit: 16_384,
        reasoning_effort: ReasoningEffort::ServerDefault,
    }
}

#[test]
#[ignore = "runs only as the child process of killing_mannie_during_a_reply_keeps_the_transcript"]
fn child_reply() {
    let Ok(directory) = std::env::var(CHILD_DIRECTORY) else {
        return;
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let store = open_store(&Path::new(&directory).join("mannie.sqlite"));
        let model = FakeModel::new(ApiInterface::ChatCompletions);
        model.push_reply(vec![
            Step::Event(ProviderEvent::TextDelta("lost ".to_owned())),
            Step::Delay(Duration::from_millis(30)),
            Step::Error(server_error()),
        ]);
        let mut streamed = vec![
            Step::Event(ProviderEvent::ReasoningDelta("Thinking.".to_owned())),
            Step::Delay(Duration::from_millis(30)),
        ];
        for index in 0..12 {
            streamed.push(Step::Event(ProviderEvent::TextDelta(format!("t{index} "))));
            streamed.push(Step::Delay(Duration::from_millis(30)));
        }
        streamed.push(Step::Hang);
        model.push_reply(streamed);
        let runtime = start_runtime(
            store,
            Arc::new(model),
            Arc::new(MemoryVault::new()),
            fast_config(),
        )
        .await;
        runtime
            .create_profile(CreateProviderProfile {
                draft: draft(),
                credential: Some(Secret::new("sk-crash-test".to_owned())),
            })
            .await
            .unwrap();
        let started = runtime
            .start_conversation(StartConversation {
                text: "Question".to_owned(),
                profile_id: None,
            })
            .await
            .unwrap();
        let mut subscription = runtime
            .subscribe(SubscribeConversation {
                conversation_id: started.conversation_id,
                after_seq: Some(0),
            })
            .await
            .unwrap();
        let mut stdout = std::io::stdout().lock();
        // The test runner has written the test's name without a line break.
        writeln!(stdout).unwrap();
        while let Some(update) = subscription.next().await {
            let ConversationUpdate::Event { event, .. } = update else {
                continue;
            };
            let line = match &event {
                ConversationEvent::PartStarted { part } => {
                    let kind = match part.content {
                        PartContent::Text { .. } => "text",
                        _ => "other",
                    };
                    format!("part {} {kind}", part.id)
                }
                ConversationEvent::PartDelta { part_id, text } => format!("delta {part_id} {text}"),
                ConversationEvent::AttemptDiscarded { .. } => "discarded".to_owned(),
                other => other.kind().to_owned(),
            };
            writeln!(stdout, "event {line}").unwrap();
            stdout.flush().unwrap();
        }
    });
}

/// What the parent saw before it killed the child.
#[derive(Debug, Default)]
struct Seen {
    events: usize,
    /// The text part of the latest attempt and the text acknowledged for it.
    text_part: Option<String>,
    text: String,
    /// Whether the failed attempt was discarded.
    discarded: bool,
}

impl Seen {
    fn record(&mut self, line: &str) {
        self.events += 1;
        let mut words = line.splitn(3, ' ');
        match (words.next(), words.next(), words.next()) {
            (Some("part"), Some(id), Some("text")) => {
                self.text_part = Some(id.to_owned());
                self.text.clear();
            }
            (Some("delta"), Some(id), Some(text)) if self.text_part.as_deref() == Some(id) => {
                // The child writes a space between the part and the text, and
                // the text itself ends with one.
                self.text.push_str(text);
                self.text.push(' ');
            }
            (Some("discarded"), _, _) => {
                self.text_part = None;
                self.text.clear();
                self.discarded = true;
            }
            _ => {}
        }
    }
}

fn run_child_until(directory: &Path, kill_after: usize) -> Seen {
    let test_binary = std::env::current_exe().unwrap();
    let mut child = Command::new(&test_binary)
        .args([
            "child_reply",
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_DIRECTORY, directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let mut seen = Seen::default();
    for line in stdout.lines() {
        let line = line.unwrap();
        if let Some(event) = line.strip_prefix("event ") {
            seen.record(event.trim_end());
            if seen.events >= kill_after {
                child.kill().unwrap();
                break;
            }
        }
    }
    child.wait().unwrap();
    seen
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn killing_mannie_during_a_reply_keeps_the_transcript() {
    for kill_after in KILL_AFTER {
        let directory = tempfile::tempdir().unwrap();
        let seen = run_child_until(directory.path(), kill_after);
        assert!(
            seen.events >= kill_after,
            "the child stopped early: {seen:?}"
        );

        let store = open_store(&directory.path().join("mannie.sqlite"));
        let profile = store.default_profile().await.unwrap().unwrap();
        let vault = Arc::new(MemoryVault::new());
        vault.insert(profile.credential_ref.as_deref().unwrap(), "sk-crash-test");
        let model = FakeModel::new(ApiInterface::ChatCompletions);
        model.push_reply(reply("Recovered"));
        let runtime = start_runtime(store.clone(), Arc::new(model), vault, fast_config()).await;
        let conversation = store.list_conversations().await.unwrap().remove(0).id;
        let snapshot = store
            .snapshot(&conversation, 10, 100)
            .await
            .unwrap()
            .unwrap();

        let label = format!("killed after event {kill_after}");
        let turn = &snapshot.turns[0];
        assert_eq!(
            turn.run.as_ref().map(|run| run.state),
            Some(RunState::Interrupted),
            "{label}"
        );
        assert!(
            turn.parts
                .iter()
                .all(|part| part.state != PartState::Streaming),
            "{label}: {:#?}",
            turn.parts
        );
        let texts: Vec<(&str, PartState)> = turn
            .parts
            .iter()
            .filter_map(|part| match &part.content {
                PartContent::Text { text } => Some((text.as_str(), part.state)),
                _ => None,
            })
            .collect();
        if seen.discarded {
            assert!(
                texts.iter().all(|(text, _)| !text.contains("lost")),
                "{label}: the failed attempt's text stays discarded: {texts:?}"
            );
        }
        if let Some(part) = &seen.text_part {
            let stored = turn
                .parts
                .iter()
                .find(|stored| stored.id.as_str() == part)
                .unwrap_or_else(|| panic!("{label}: the acknowledged part {part} is gone"));
            let PartContent::Text { text } = &stored.content else {
                panic!("{label}: the part is not text");
            };
            assert!(
                text.starts_with(&seen.text),
                "{label}: stored {text:?}, acknowledged {:?}",
                seen.text
            );
            assert_eq!(stored.state, PartState::Interrupted, "{label}");
        }

        let retried = runtime.retry_reply(&conversation).await.unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let retried_state = loop {
            let snapshot = store
                .snapshot(&conversation, 10, 100)
                .await
                .unwrap()
                .unwrap();
            let run = snapshot.turns[0].run.clone().unwrap();
            if run.id == retried.run_id && !run.state.is_active() {
                break (run.state, snapshot.turns[0].parts.clone());
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{label}: the retry did not end"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        assert_eq!(retried_state.0, RunState::Completed, "{label}");
        assert!(
            retried_state.1.iter().any(|part| part.content
                == PartContent::Text {
                    text: "Recovered".to_owned()
                }),
            "{label}"
        );
        runtime.shutdown().await;
    }
}
