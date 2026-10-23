//! Killing Mannie during a task never repeats what a child did
//! (docs/features/child-agents/tasks.md, recovery; ADR-0024, decision 10).
//!
//! The test runs a child process (this test binary, running the ignored
//! `child_task` test) whose conversation agent gives a general child a task.
//! The child runs a command that appends a marker line to a file and then
//! waits; the process approves the command from the parent's task part. The
//! test kills the process while the child's command executes, after its
//! marker, after the child reported, and after the parent's reply ended.
//! It then starts a runtime on the same database and checks the states of
//! the recovery table, that the marker appears at most once, that Retry
//! starts no child, and that a follow-up continues the child's conversation
//! without running the command again.

#![cfg(test)]
#![cfg(any(windows, target_os = "linux", target_os = "freebsd"))]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mannie_provider::testing::Step;
use mannie_provider::{Content, ModelRequest, Role};
use mannie_runtime::testing::{
    FakeModel, MemoryVault, Transcript, fast_config, open_store, reply, start_runtime_with_tools,
    test_tools, tool_calls, wait_for_leftover_command,
};
use mannie_runtime::{AgentRegistry, ConversationSubscription, Runtime, texts};
use mannie_store::Store;
use mannie_types::chat::{SendMessage, StartConversation, SubscribeConversation};
use mannie_types::ids::{ConversationId, RunId};
use mannie_types::provider::{ApiInterface, CreateProviderProfile, ProfileDraft, ReasoningEffort};
use mannie_types::run::RunState;
use mannie_types::secret::Secret;
use mannie_types::tasks::TaskState;
use mannie_types::tools::{ApprovalChoice, DecideApproval, ProcessRecord, ToolCallStatus};
use mannie_types::transcript::{ConversationEvent, ConversationUpdate, PartContent};
use serde_json::json;

const CHILD_DIRECTORY: &str = "MANNIE_TASK_CRASH_CHILD_DIRECTORY";

/// Set when the parent's reply ends after the report instead of waiting.
const PARENT_ENDS: &str = "MANNIE_TASK_CRASH_PARENT_ENDS";

/// The marker in the child's instruction, which routes its model calls.
const CHILD: &str = "CHILD-K";

/// When the test kills the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KillPoint {
    /// The child's command is stored as executing; it may not have started.
    Executing,
    /// The command wrote its marker and still runs.
    AfterTheMarker,
    /// The child reported, and the parent's reply still runs.
    Reported,
    /// The parent's reply ended.
    ParentEnded,
}

impl KillPoint {
    /// What a restart leaves: the parent's run, the task, the child's run
    /// and the child's command.
    fn recovered(self) -> (RunState, TaskState, RunState, ToolCallStatus) {
        match self {
            Self::Executing | Self::AfterTheMarker => (
                RunState::Interrupted,
                TaskState::Interrupted,
                RunState::Interrupted,
                ToolCallStatus::Uncertain,
            ),
            Self::Reported => (
                RunState::Interrupted,
                TaskState::Reported,
                RunState::Completed,
                ToolCallStatus::Completed,
            ),
            Self::ParentEnded => (
                RunState::Completed,
                TaskState::Reported,
                RunState::Completed,
                ToolCallStatus::Completed,
            ),
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

fn start_arguments() -> String {
    json!({
        "activity": "Marking",
        "title": "Mark it",
        "agent": "general",
        "instruction": format!("{CHILD}: run the marker command once."),
        "background": false,
    })
    .to_string()
}

/// Prints the statuses of the child's tool calls, for the test to read.
async fn print_child_calls(mut subscription: ConversationSubscription) {
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
        if let PartContent::ToolCall { call } = &part.content {
            let status = serde_json::to_value(call.status).unwrap();
            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "event command {}", status.as_str().unwrap()).unwrap();
            stdout.flush().unwrap();
        }
    }
}

/// Prints a line for the test to read.
fn print_event(line: &str) {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{line}").unwrap();
    stdout.flush().unwrap();
}

/// Starts the process's runtime with the scripted parent and child, and the
/// parent's conversation in the workspace's project.
async fn start_conversation(directory: &Path, parent_ends: bool) -> (Runtime, ConversationId) {
    let store = open_store(&directory.join("mannie.sqlite"));
    let model = FakeModel::new(ApiInterface::ChatCompletions);
    model.push_reply(tool_calls(&[("call_1", "start_task", &start_arguments())]));
    model.push_reply(if parent_ends {
        reply("It is marked.")
    } else {
        vec![Step::Hang]
    });
    let arguments = json!({ "activity": "Marking", "command": command() }).to_string();
    model.push_reply_for(CHILD, tool_calls(&[("child_1", "run_command", &arguments)]));
    model.push_reply_for(CHILD, reply("marked"));
    model.push_reply_for(CHILD, reply("Report: marked once."));
    let runtime = start_runtime_with_tools(
        store,
        Arc::new(model),
        Arc::new(MemoryVault::new()),
        fast_config(),
        test_tools(directory),
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
        .add_project(workspace(directory).to_str().unwrap().to_owned())
        .await
        .unwrap();
    let started = runtime
        .start_conversation(StartConversation::new("OWNER: mark it").with_project(Some(project.id)))
        .await
        .unwrap();
    (runtime, started.conversation_id)
}

/// Follows the parent: prints the task's states and the end of the reply,
/// follows the child's calls, and approves the child's command from the
/// parent's task part.
async fn follow_and_approve(runtime: Runtime, conversation: ConversationId) {
    let mut subscription = runtime
        .subscribe(SubscribeConversation {
            conversation_id: conversation,
            after_seq: Some(0),
        })
        .await
        .unwrap();
    let mut approved = false;
    while let Some(update) = subscription.next().await {
        let ConversationUpdate::Event { event, .. } = update else {
            continue;
        };
        if let ConversationEvent::RunUpdated { run } = &event
            && run.state == RunState::Completed
        {
            print_event("event parent completed");
        }
        let (ConversationEvent::PartUpdated { part } | ConversationEvent::PartFinished { part }) =
            &event
        else {
            continue;
        };
        let PartContent::Task { call } = &part.content else {
            continue;
        };
        let Some(task) = &call.task else {
            continue;
        };
        let state = serde_json::to_value(task.state).unwrap();
        print_event(&format!("event task {}", state.as_str().unwrap()));
        if let Some(approval) = &task.approval
            && !approved
        {
            // The approval confirms that the child conversation exists.
            // Subscribe before granting it so Executing cannot be missed.
            let child_subscription = runtime
                .subscribe(SubscribeConversation {
                    conversation_id: task.conversation_id.clone(),
                    after_seq: Some(0),
                })
                .await
                .unwrap();
            tokio::spawn(print_child_calls(child_subscription));
            approved = true;
            runtime
                .decide_approval(DecideApproval {
                    conversation_id: task.conversation_id.clone(),
                    part_id: approval.part_id.clone(),
                    digest: approval.call.request.as_ref().unwrap().digest.clone(),
                    choice: ApprovalChoice::AllowOnce,
                })
                .await
                .unwrap();
        }
    }
}

#[test]
#[ignore = "runs only as the child process of killing_mannie_during_a_task_never_repeats_what_the_child_did"]
fn child_task() {
    let Ok(directory) = std::env::var(CHILD_DIRECTORY) else {
        return;
    };
    let directory = PathBuf::from(directory);
    let parent_ends = std::env::var_os(PARENT_ENDS).is_some();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (runtime, conversation) = start_conversation(&directory, parent_ends).await;
        // The test runner has written the test's name without a line break.
        print_event("");
        follow_and_approve(runtime, conversation).await;
    });
}

/// Runs the child process until the kill point, and kills it there.
fn run_child_until(directory: &Path, kill: KillPoint) {
    let test_binary = std::env::current_exe().unwrap();
    let mut command = Command::new(&test_binary);
    command
        .args([
            "child_task",
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_DIRECTORY, directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    if kill == KillPoint::ParentEnded {
        command.env(PARENT_ENDS, "1");
    }
    let mut child = command.spawn().unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let wanted = match kill {
        KillPoint::Executing | KillPoint::AfterTheMarker => "event command executing",
        KillPoint::Reported => "event task reported",
        KillPoint::ParentEnded => "event parent completed",
    };
    let mut reached = false;
    for line in stdout.lines() {
        let line = line.unwrap();
        if line.trim_end() == wanted {
            if kill == KillPoint::AfterTheMarker {
                let deadline = Instant::now() + Duration::from_secs(20);
                while marker_lines(directory) == 0 && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            reached = true;
            child.kill().unwrap();
            break;
        }
    }
    let status = child.wait().unwrap();
    assert!(reached, "child exited {status} before {wanted} at {kill:?}");
}

/// Follows a conversation until its latest run, `run`, has ended.
async fn wait_for_run(store: &Store, conversation: &ConversationId, run: &RunId) -> RunState {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let snapshot = store
            .snapshot(conversation, 50, 1_000)
            .await
            .unwrap()
            .unwrap();
        let current = snapshot
            .turns
            .iter()
            .filter_map(|turn| turn.run.as_ref())
            .find(|view| view.id == *run);
        if let Some(view) = current
            && !view.state.is_active()
        {
            return view.state;
        }
        assert!(Instant::now() < deadline, "the run {run} did not end");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn transcript(store: &Store, conversation: &ConversationId) -> Transcript {
    let snapshot = store
        .snapshot(conversation, 50, 1_000)
        .await
        .unwrap()
        .unwrap();
    let mut transcript = Transcript::default();
    transcript.apply(&ConversationUpdate::Snapshot { snapshot });
    transcript
}

/// The tool results of a request.
fn results(request: &ModelRequest) -> Vec<String> {
    request
        .messages
        .iter()
        .filter(|message| message.role == Role::Tool)
        .flat_map(|message| &message.content)
        .filter_map(|content| match content {
            Content::ToolResult { output, .. } => Some(output.clone()),
            _ => None,
        })
        .collect()
}

/// Starts a runtime on the database that the killed process left.
async fn restart(directory: &Path) -> (Runtime, Store, FakeModel) {
    let store = open_store(&directory.join("mannie.sqlite"));
    let profile = store.default_profile().await.unwrap().unwrap();
    let vault = Arc::new(MemoryVault::new());
    vault.insert(profile.credential_ref.as_deref().unwrap(), "sk-crash-test");
    let model = FakeModel::new(ApiInterface::ChatCompletions);
    let runtime = start_runtime_with_tools(
        store.clone(),
        Arc::new(model.clone()),
        vault,
        fast_config(),
        test_tools(directory),
        AgentRegistry::with_builtin(),
    )
    .await;
    (runtime, store, model)
}

/// Checks the states that the restart left, as the recovery table says, and
/// returns the parent's and the child's conversations.
async fn check_recovered(
    store: &Store,
    kill: KillPoint,
    label: &str,
) -> (ConversationId, ConversationId) {
    let listed = store.list_conversations().await.unwrap();
    assert_eq!(listed.len(), 1, "{label}: only the parent is listed");
    let conversation = listed[0].id.clone();
    let children = store.task_conversations(&conversation).await.unwrap();
    let [child] = children.as_slice() else {
        panic!("{label}: the parent has {} children", children.len());
    };
    let (parent_state, task_state, child_state, command_status) = kill.recovered();
    let parent = transcript(store, &conversation).await;
    let parent_run = parent.turns[&0].run.clone().unwrap();
    assert_eq!(parent_run.state, parent_state, "{label}");
    assert_eq!(parent.tasks(0)[0].state, task_state, "{label}");
    let child_transcript = transcript(store, child).await;
    assert_eq!(
        child_transcript.turns[&0].run.as_ref().unwrap().state,
        child_state,
        "{label}"
    );
    assert_eq!(
        child_transcript.tool_calls(0)[0].status,
        command_status,
        "{label}"
    );
    (conversation, child.clone())
}

/// The shell of the child's command, as the journal recorded it.
async fn child_command(store: &Store, child: &ConversationId) -> Option<ProcessRecord> {
    let turns = store.model_turns(child, None).await.unwrap();
    turns
        .iter()
        .flat_map(|turn| &turn.parts)
        .find_map(|part| part.tool_call.as_ref())
        .and_then(|call| call.process.clone())
}

/// Retries the parent's reply, which reads the task's end and starts no
/// child.
async fn retry_starts_no_child(
    runtime: &Runtime,
    store: &Store,
    model: &FakeModel,
    conversation: &ConversationId,
    kill: KillPoint,
    label: &str,
) {
    model.push_reply(reply("Checked."));
    let retried = runtime.retry_reply(conversation).await.unwrap();
    let state = wait_for_run(store, conversation, &retried.run_id).await;
    assert_eq!(state, RunState::Completed, "{label}");
    let request = model.requests_for("OWNER:").pop().unwrap();
    let (_parent, task_state, _child, _command) = kill.recovered();
    let expected = match task_state {
        TaskState::Reported => texts::task_reported(1, texts::THE_CHILD, "Report: marked once."),
        _ => texts::task_interrupted(1, texts::THE_CHILD),
    };
    assert_eq!(results(&request), [expected], "{label}");
    let children = store.task_conversations(conversation).await.unwrap();
    assert_eq!(children.len(), 1, "{label}: the retry started a child");
}

/// Follows up the child, which continues its conversation and reads its
/// command's result instead of running the command again.
async fn follow_up_reads_the_result(
    runtime: &Runtime,
    store: &Store,
    model: &FakeModel,
    (conversation, child): (&ConversationId, &ConversationId),
    kill: KillPoint,
    label: &str,
) {
    let follow_up = json!({
        "activity": "Checking",
        "task": 1,
        "instruction": "Did the command run? Do not run it again.",
        "background": false,
    })
    .to_string();
    model.push_reply(tool_calls(&[("call_9", "follow_up_task", &follow_up)]));
    model.push_reply(reply("It ran once."));
    model.push_reply_for(CHILD, reply("It ran."));
    model.push_reply_for(CHILD, reply("Report: the command ran."));
    let sent = runtime
        .send_message(SendMessage::new(conversation.clone(), "Did it run?"))
        .await
        .unwrap();
    let state = wait_for_run(store, conversation, &sent.run_id).await;
    assert_eq!(state, RunState::Completed, "{label}");
    let child_request = model.requests_for(CHILD).remove(0);
    let command_result = results(&child_request).remove(0);
    let (_parent, _task, _child, command_status) = kill.recovered();
    if command_status == ToolCallStatus::Uncertain {
        assert_eq!(command_result, texts::UNCERTAIN, "{label}");
    } else {
        assert!(
            command_result.starts_with("Exit code: 0"),
            "{label}: {command_result}"
        );
    }
    let parent = transcript(store, conversation).await;
    let follow_up_task = parent.tasks(1)[0].clone();
    assert_eq!(follow_up_task.state, TaskState::Reported, "{label}");
    assert_eq!(follow_up_task.conversation_id, *child, "{label}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn killing_mannie_during_a_task_never_repeats_what_the_child_did() {
    for kill in [
        KillPoint::Executing,
        KillPoint::AfterTheMarker,
        KillPoint::Reported,
        KillPoint::ParentEnded,
    ] {
        let label = format!("killed at {kill:?}");
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(workspace(directory.path())).unwrap();
        run_child_until(directory.path(), kill);

        let (runtime, store, model) = restart(directory.path()).await;
        let (conversation, child) = check_recovered(&store, kill, &label).await;
        // A command that outlived the process may still write its marker
        // until the recovery has ended it.
        wait_for_leftover_command(child_command(&store, &child).await.as_ref()).await;
        let markers_before = marker_lines(directory.path());
        assert!(
            markers_before <= 1,
            "{label}: the marker appears {markers_before} times"
        );
        if kill != KillPoint::Executing {
            assert_eq!(markers_before, 1, "{label}");
        }
        let (parent_state, _task, _child, _command) = kill.recovered();
        if parent_state != RunState::Completed {
            retry_starts_no_child(&runtime, &store, &model, &conversation, kill, &label).await;
        }
        follow_up_reads_the_result(
            &runtime,
            &store,
            &model,
            (&conversation, &child),
            kill,
            &label,
        )
        .await;

        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            marker_lines(directory.path()),
            markers_before,
            "{label}: the command ran again"
        );
        runtime.shutdown().await;
    }
}
