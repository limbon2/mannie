//! Killing Mannie during a tool call never runs its operation twice
//! (docs/features/system-tools/operations-and-approvals.md, recovery; ADR-0013).
//!
//! The test runs a child process (this test binary, running the ignored
//! `child_tool_call` test) whose model calls `run_command` with a command that
//! appends a marker line to a file and then waits. The child approves the
//! call after a pause and prints the call's statuses. The parent kills the
//! child while the call is received, while it awaits approval, while it
//! executes (before and after the marker) and after it completed, then starts
//! a runtime on the same database and checks the call's status, that the
//! marker appears at most once, and that Retry repeats nothing.

#![cfg(test)]
#![cfg(any(windows, target_os = "linux", target_os = "freebsd"))]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mannie_provider::testing::Step;
use mannie_provider::{Content, ProviderEvent, Role};
use mannie_runtime::testing::{
    FakeModel, MemoryVault, fast_config, open_store, reply, start_runtime_with_tools, test_tools,
    wait_for_leftover_command,
};
use mannie_runtime::{AgentRegistry, texts};
use mannie_types::chat::{StartConversation, SubscribeConversation};
use mannie_types::provider::{ApiInterface, CreateProviderProfile, ProfileDraft, ReasoningEffort};
use mannie_types::run::{FinishReason, RunState};
use mannie_types::secret::Secret;
use mannie_types::tools::{ApprovalChoice, DecideApproval, ToolCallStatus};
use mannie_types::transcript::{ConversationEvent, ConversationUpdate, PartContent};
use serde_json::json;

const CHILD_DIRECTORY: &str = "MANNIE_TOOL_CRASH_CHILD_DIRECTORY";

/// When the parent kills the child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KillPoint {
    /// The call's part started, before its arguments arrived.
    Receiving,
    /// The call waits for the owner.
    AwaitingApproval,
    /// The call is stored as executing; the command may not have started.
    Executing,
    /// The command wrote its marker and still runs.
    AfterTheMarker,
    /// The call completed.
    Completed,
}

impl KillPoint {
    /// The status a restart leaves the call in.
    fn recovered_status(self) -> ToolCallStatus {
        match self {
            Self::Receiving | Self::AwaitingApproval => ToolCallStatus::Canceled,
            Self::Executing | Self::AfterTheMarker => ToolCallStatus::Uncertain,
            Self::Completed => ToolCallStatus::Completed,
        }
    }
}

fn draft() -> ProfileDraft {
    ProfileDraft {
        name: "Test".to_owned(),
        interface: ApiInterface::ChatCompletions,
        base_url: "http://127.0.0.1:9/v1".to_owned(),
        model: "test-model".to_owned(),
        context_window: 128_000,
        output_limit: 16_384,
        reasoning_effort: ReasoningEffort::ServerDefault,
        use_tools: true,
    }
}

/// The command: it appends a marker line, then waits two seconds.
fn command() -> &'static str {
    if cfg!(windows) {
        "Add-Content -Path marker.txt -Value ran; Start-Sleep -Seconds 2"
    } else {
        "echo ran >> marker.txt; sleep 2"
    }
}

fn workspace(directory: &Path) -> PathBuf {
    directory.join("work")
}

fn marker_lines(directory: &Path) -> usize {
    std::fs::read_to_string(workspace(directory).join("marker.txt")).map_or(0, |text| {
        text.lines().filter(|line| line.trim() == "ran").count()
    })
}

#[test]
#[ignore = "runs only as the child process of killing_mannie_during_a_tool_call_never_repeats_it"]
fn child_tool_call() {
    let Ok(directory) = std::env::var(CHILD_DIRECTORY) else {
        return;
    };
    let directory = PathBuf::from(directory);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let store = open_store(&directory.join("mannie.sqlite"));
        let model = FakeModel::new(ApiInterface::ChatCompletions);
        let arguments = json!({ "command": command() }).to_string();
        model.push_reply(vec![
            Step::Event(ProviderEvent::ToolCallStarted {
                call_id: "call_1".to_owned(),
                name: "run_command".to_owned(),
            }),
            // The call is stored as received before its arguments arrive.
            Step::Delay(Duration::from_millis(300)),
            Step::Event(ProviderEvent::ToolCall {
                call_id: "call_1".to_owned(),
                name: "run_command".to_owned(),
                arguments,
            }),
            Step::Event(ProviderEvent::Finished(FinishReason::Other)),
        ]);
        model.push_reply(vec![Step::Hang]);
        let runtime = start_runtime_with_tools(
            store,
            Arc::new(model),
            Arc::new(MemoryVault::new()),
            fast_config(),
            test_tools(&directory),
            AgentRegistry::with_builtin(),
        )
        .await;
        runtime
            .create_profile(CreateProviderProfile {
                draft: draft(),
                credential: Some(Secret::new("sk-crash-test".to_owned())),
            })
            .await
            .unwrap();
        let project = runtime
            .add_project(workspace(&directory).to_str().unwrap().to_owned())
            .await
            .unwrap();
        let started = runtime
            .start_conversation(StartConversation {
                mentions: Vec::new(),
                images: Vec::new(),
                text: "Mark it".to_owned(),
                profile_id: None,
                project_id: Some(project.id),
            })
            .await
            .unwrap();
        let mut subscription = runtime
            .subscribe(SubscribeConversation {
                conversation_id: started.conversation_id.clone(),
                after_seq: Some(0),
            })
            .await
            .unwrap();
        let mut stdout = std::io::stdout().lock();
        // The test runner has written the test's name without a line break.
        writeln!(stdout).unwrap();
        let mut approved = false;
        while let Some(update) = subscription.next().await {
            let ConversationUpdate::Event { event, .. } = update else {
                continue;
            };
            let (ConversationEvent::PartStarted { part }
            | ConversationEvent::PartUpdated { part }
            | ConversationEvent::PartFinished { part }) = &event
            else {
                continue;
            };
            let PartContent::ToolCall { call } = &part.content else {
                continue;
            };
            let status = serde_json::to_value(call.status).unwrap();
            writeln!(stdout, "event call {}", status.as_str().unwrap()).unwrap();
            stdout.flush().unwrap();
            if call.status == ToolCallStatus::AwaitingApproval && !approved {
                approved = true;
                // The parent may kill the child while it waits.
                tokio::time::sleep(Duration::from_millis(400)).await;
                runtime
                    .decide_approval(DecideApproval {
                        conversation_id: started.conversation_id.clone(),
                        part_id: part.id.clone(),
                        digest: call.request.as_ref().unwrap().digest.clone(),
                        choice: ApprovalChoice::AllowOnce,
                    })
                    .await
                    .unwrap();
            }
        }
    });
}

/// Runs the child until the kill point, and kills it there.
fn run_child_until(directory: &Path, kill: KillPoint) {
    let test_binary = std::env::current_exe().unwrap();
    let mut child = Command::new(&test_binary)
        .args([
            "child_tool_call",
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
    let wanted = match kill {
        KillPoint::Receiving => "receiving",
        KillPoint::AwaitingApproval => "awaiting_approval",
        KillPoint::Executing | KillPoint::AfterTheMarker => "executing",
        KillPoint::Completed => "completed",
    };
    for line in stdout.lines() {
        let line = line.unwrap();
        if line.trim_end() == format!("event call {wanted}") {
            if kill == KillPoint::AfterTheMarker {
                let deadline = Instant::now() + Duration::from_secs(20);
                while marker_lines(directory) == 0 && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            child.kill().unwrap();
            break;
        }
    }
    child.wait().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn killing_mannie_during_a_tool_call_never_repeats_it() {
    for kill in [
        KillPoint::Receiving,
        KillPoint::AwaitingApproval,
        KillPoint::Executing,
        KillPoint::AfterTheMarker,
        KillPoint::Completed,
    ] {
        let label = format!("killed at {kill:?}");
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(workspace(directory.path())).unwrap();
        run_child_until(directory.path(), kill);

        let store = open_store(&directory.path().join("mannie.sqlite"));
        let profile = store.default_profile().await.unwrap().unwrap();
        let vault = Arc::new(MemoryVault::new());
        vault.insert(profile.credential_ref.as_deref().unwrap(), "sk-crash-test");
        let model = FakeModel::new(ApiInterface::ChatCompletions);
        model.push_reply(reply("I checked; it ran once."));
        let runtime = start_runtime_with_tools(
            store.clone(),
            Arc::new(model.clone()),
            vault,
            fast_config(),
            test_tools(directory.path()),
            AgentRegistry::with_builtin(),
        )
        .await;
        let conversation = store.list_conversations().await.unwrap().remove(0).id;
        let turns = store.model_turns(&conversation, None).await.unwrap();
        let turn = &turns[0];
        assert_eq!(turn.run_state, Some(RunState::Interrupted), "{label}");
        let call = turn
            .parts
            .iter()
            .find_map(|part| part.tool_call.clone())
            .unwrap_or_else(|| panic!("{label}: no tool call was stored"));
        assert_eq!(call.status, kill.recovered_status(), "{label}: {call:?}");
        // A command that outlived the child may still write its marker until
        // the recovery has ended it.
        wait_for_leftover_command(call.process.as_ref()).await;
        let markers_before = marker_lines(directory.path());
        assert!(
            markers_before <= 1,
            "{label}: the marker appears {markers_before} times"
        );
        if kill == KillPoint::Completed {
            assert_eq!(markers_before, 1, "{label}");
        }
        if kill == KillPoint::Receiving || kill == KillPoint::AwaitingApproval {
            assert_eq!(markers_before, 0, "{label}");
        }

        let retried = runtime.retry_reply(&conversation).await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let snapshot = store
                .snapshot(&conversation, 10, 100)
                .await
                .unwrap()
                .unwrap();
            let run = snapshot.turns[0].run.clone().unwrap();
            if run.id == retried.run_id && !run.state.is_active() {
                assert_eq!(run.state, RunState::Completed, "{label}");
                break;
            }
            assert!(Instant::now() < deadline, "{label}: the retry did not end");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // The retry told the model what happened and ran nothing again.
        let request = model.reply_requests().pop().unwrap();
        let result = request
            .messages
            .iter()
            .filter(|message| message.role == Role::Tool)
            .flat_map(|message| &message.content)
            .find_map(|content| match content {
                Content::ToolResult { output, .. } => Some(output.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{label}: the retry's request has no tool result"));
        match kill.recovered_status() {
            ToolCallStatus::Canceled => assert_eq!(result, texts::CANCELED, "{label}"),
            ToolCallStatus::Uncertain => assert_eq!(result, texts::UNCERTAIN, "{label}"),
            _ => assert!(result.starts_with("Exit code: 0"), "{label}: {result}"),
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            marker_lines(directory.path()),
            markers_before,
            "{label}: the operation ran again"
        );
        runtime.shutdown().await;
    }
}
