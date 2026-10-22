//! Killing Mannie while a background command runs leaves nothing of the
//! command running after the next start (docs/features/system-tools/
//! background-commands.md, recovery; ADR-0041).
//!
//! The test runs a child process (this test binary, running the ignored
//! `child_background_command` test) whose model starts a command in the
//! background that appends a line to a file five times a second. The parent
//! kills the child once the command's record is stored, and once its start
//! call completed, then starts a runtime on the same database: the record is
//! interrupted and untold, and the file stops growing. On Linux and FreeBSD
//! the start ends what is left of the tree; on Windows its job ended with the
//! child.

#![cfg(test)]
#![cfg(any(windows, target_os = "linux", target_os = "freebsd"))]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use mannie_provider::ProviderEvent;
use mannie_provider::testing::Step;
use mannie_runtime::AgentRegistry;
use mannie_runtime::testing::{
    FakeModel, MemoryVault, fast_config, open_store, start_runtime_with_tools, test_tools,
    wait_for_leftover_command,
};
use mannie_types::chat::{StartConversation, SubscribeConversation};
use mannie_types::provider::{ApiInterface, CreateProviderProfile, ProfileDraft, ReasoningEffort};
use mannie_types::run::FinishReason;
use mannie_types::secret::Secret;
use mannie_types::tools::{BackgroundEnding, ToolCallStatus, UpdateToolsSettings};
use mannie_types::transcript::{ConversationEvent, ConversationUpdate, PartContent};
use serde_json::json;

const CHILD_DIRECTORY: &str = "MANNIE_BACKGROUND_CRASH_CHILD_DIRECTORY";

/// When the parent kills the child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KillPoint {
    /// The command's record is stored; its start call may still run.
    Recorded,
    /// The start call completed, and the reply goes on.
    Completed,
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

/// The command: it appends a line five times a second until it is ended.
fn command() -> &'static str {
    if cfg!(windows) {
        "while ($true) { Add-Content -Path ticks.txt -Value tick; Start-Sleep -Milliseconds 200 }"
    } else {
        "while true; do echo tick >> ticks.txt; sleep 0.2; done"
    }
}

fn workspace(directory: &Path) -> PathBuf {
    directory.join("work")
}

fn ticks(directory: &Path) -> usize {
    std::fs::read_to_string(workspace(directory).join("ticks.txt"))
        .map_or(0, |text| text.lines().count())
}

#[test]
#[ignore = "runs only as the child process of killing_mannie_leaves_no_background_command_running"]
fn child_background_command() {
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
                name: "start_command".to_owned(),
            }),
            Step::Event(ProviderEvent::ToolCall {
                call_id: "call_1".to_owned(),
                name: "start_command".to_owned(),
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
        // The command runs without asking.
        runtime
            .update_tools_settings(UpdateToolsSettings {
                allow_changes_in_workspace: true,
                command_environment: Vec::new(),
                full_access_for_new_conversations: true,
                wake_on_command_end: false,
            })
            .await
            .unwrap();
        let project = runtime
            .add_project(workspace(&directory).to_str().unwrap().to_owned())
            .await
            .unwrap();
        let started = runtime
            .start_conversation(StartConversation::new("Tick").with_project(Some(project.id)))
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
        while let Some(update) = subscription.next().await {
            let ConversationUpdate::Event { event, .. } = update else {
                continue;
            };
            match &event {
                ConversationEvent::BackgroundCommandChanged { command } => {
                    writeln!(stdout, "event command {}", command.number).unwrap();
                }
                ConversationEvent::PartUpdated { part }
                | ConversationEvent::PartFinished { part } => {
                    if let PartContent::ToolCall { call } = &part.content
                        && call.status == ToolCallStatus::Completed
                    {
                        writeln!(stdout, "event call completed").unwrap();
                    }
                }
                _ => {}
            }
            stdout.flush().unwrap();
        }
    });
}

/// Runs the child until the kill point, and kills it there.
fn run_child_until(directory: &Path, kill: KillPoint) {
    let test_binary = std::env::current_exe().unwrap();
    let mut child = Command::new(&test_binary)
        .args([
            "child_background_command",
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
        KillPoint::Recorded => "event command 1",
        KillPoint::Completed => "event call completed",
    };
    for line in stdout.lines() {
        if line.unwrap().trim_end() == wanted {
            child.kill().unwrap();
            break;
        }
    }
    child.wait().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn killing_mannie_leaves_no_background_command_running() {
    for kill in [KillPoint::Recorded, KillPoint::Completed] {
        let label = format!("killed at {kill:?}");
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(workspace(directory.path())).unwrap();
        run_child_until(directory.path(), kill);

        let store = open_store(&directory.path().join("mannie.sqlite"));
        let runtime = start_runtime_with_tools(
            store.clone(),
            Arc::new(FakeModel::new(ApiInterface::ChatCompletions)),
            Arc::new(MemoryVault::new()),
            fast_config(),
            test_tools(directory.path()),
            AgentRegistry::with_builtin(),
        )
        .await;
        let conversation = store.list_conversations().await.unwrap().remove(0).id;
        let command = store
            .background_command(&conversation, 1)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{label}: no record was stored"));
        assert_eq!(
            command.document.ending,
            Some(BackgroundEnding::Interrupted),
            "{label}"
        );
        assert!(!command.document.told, "{label}");
        assert_eq!(command.document.archive_id, None, "{label}");
        assert_eq!(runtime.running_background_count(), 0, "{label}");
        // What outlived the child ends once the recovery has ended it.
        wait_for_leftover_command(command.document.process.as_ref()).await;
        let after_recovery = ticks(directory.path());
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(
            ticks(directory.path()),
            after_recovery,
            "{label}: the command still runs"
        );
        runtime.shutdown().await;
    }
}
