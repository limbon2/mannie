//! Background commands through the runtime (docs/features/system-tools/
//! background-commands.md and wake-ups.md): a command outlives its call and
//! its reply, its output reads after other calls, the notes tell the model of
//! ends without their output, Stop of a reply leaves commands running, and a
//! task's end, a deletion and quitting end them.
//!
//! The commands are real processes in the platform's shell, so the tests run
//! where commands run.

#![cfg(test)]
#![cfg(any(windows, target_os = "linux", target_os = "freebsd"))]

use std::sync::Arc;
use std::time::{Duration, Instant};

use mannie_provider::testing::Step;
use mannie_provider::{Content, ModelRequest, ProviderEvent, Role};
use mannie_runtime::RuntimeConfig;
use mannie_runtime::testing::{
    Harness, HarnessOptions, Transcript, fast_config, reply, tool_calls,
};
use mannie_types::chat::{SendMessage, StartConversation};
use mannie_types::ids::{ConversationId, ProjectId, RunId};
use mannie_types::run::{FinishReason, RunState};
use mannie_types::tools::{
    ApprovalChoice, BackgroundEnding, BackgroundLimit, CommandEndView, DecideApproval, GrantScope,
    ToolCallStatus, ToolFailure, ToolOutcome, UpdateToolsSettings,
};
use mannie_types::transcript::PartContent;
use serde_json::json;
use tempfile::TempDir;

/// A runtime whose profile uses tools, and a project whose folder is the
/// workspace of the conversations that start in it. Its conversations start
/// in full access, so commands run without asking, and no end wakes the
/// agent unless a test turns wake-ups on.
struct Setup {
    harness: Harness,
    _workspace: TempDir,
    project: ProjectId,
}

impl Setup {
    async fn new() -> Self {
        Self::with(true, false, RuntimeConfig::default().wake_up_delay).await
    }

    async fn with_full_access(full_access: bool) -> Self {
        Self::with(full_access, false, RuntimeConfig::default().wake_up_delay).await
    }

    /// A setup whose ends wake the agent after `delay`.
    async fn waking(delay: Duration) -> Self {
        Self::with(true, true, delay).await
    }

    async fn with(full_access: bool, wake: bool, delay: Duration) -> Self {
        let harness = Harness::with_options(HarnessOptions {
            use_tools: true,
            config: RuntimeConfig {
                wake_up_delay: delay,
                ..fast_config()
            },
            ..HarnessOptions::default()
        })
        .await;
        harness
            .runtime
            .update_tools_settings(UpdateToolsSettings {
                allow_changes_in_workspace: true,
                command_environment: Vec::new(),
                full_access_for_new_conversations: full_access,
                wake_on_command_end: wake,
            })
            .await
            .unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let project = harness
            .runtime
            .add_project(workspace.path().to_str().unwrap().to_owned())
            .await
            .unwrap();
        Self {
            harness,
            _workspace: workspace,
            project: project.id,
        }
    }

    /// Starts a conversation in the project with the owner's message.
    async fn start(&self, text: &str) -> (ConversationId, RunId) {
        let started = self
            .harness
            .runtime
            .start_conversation(
                StartConversation::new(text).with_project(Some(self.project.clone())),
            )
            .await
            .unwrap();
        (started.conversation_id, started.run_id)
    }

    /// Restarts Mannie on the same store, as quitting and starting again
    /// does.
    async fn restart(self) -> Self {
        let Self {
            harness,
            _workspace: workspace,
            project,
        } = self;
        Self {
            harness: harness.restart().await,
            _workspace: workspace,
            project,
        }
    }

    /// Sends the owner's next message.
    async fn send(&self, conversation: &ConversationId, text: &str) -> RunId {
        self.harness
            .runtime
            .send_message(SendMessage::new(conversation.clone(), text))
            .await
            .unwrap()
            .run_id
    }

    /// Waits until the conversation has `turns` turns and the reply of the
    /// last one ended.
    async fn replied(&self, conversation: &ConversationId, turns: usize) -> Transcript {
        self.harness
            .wait_until(conversation, |transcript| {
                transcript.turns.len() == turns
                    && transcript.turns.values().last().is_some_and(|turn| {
                        turn.run.as_ref().is_some_and(|run| !run.state.is_active())
                    })
            })
            .await
    }

    /// How many turns the conversation has, after `pause` for anything that
    /// would start one.
    async fn turns_after(&self, conversation: &ConversationId, pause: Duration) -> usize {
        tokio::time::sleep(pause).await;
        self.harness.snapshot(conversation).await.turns.len()
    }

    /// Waits until a command of the conversation ended, as its subscribers
    /// see it.
    async fn ended(&self, conversation: &ConversationId, number: u32) -> Transcript {
        self.harness
            .wait_until(conversation, |transcript| {
                transcript
                    .background_commands
                    .get(&number)
                    .is_some_and(|command| command.ending.is_some())
            })
            .await
    }
}

/// A command in the platform's shell: PowerShell on Windows, sh elsewhere.
fn shell(windows: &str, unix: &str) -> String {
    if cfg!(windows) {
        windows.to_owned()
    } else {
        unix.to_owned()
    }
}

/// A command that writes a line, then runs for a minute.
fn long_running() -> String {
    shell(
        "Write-Output first; Start-Sleep -Seconds 60",
        "echo first; sleep 60",
    )
}

fn start(command: &str) -> String {
    json!({ "command": command }).to_string()
}

/// The tool results of a request, by call ID.
fn results(request: &ModelRequest) -> Vec<(String, String)> {
    request
        .messages
        .iter()
        .filter(|message| message.role == Role::Tool)
        .flat_map(|message| &message.content)
        .filter_map(|content| match content {
            Content::ToolResult { call_id, output } => Some((call_id.clone(), output.clone())),
            _ => None,
        })
        .collect()
}

/// The result of the call `call_id` in a request.
fn result(request: &ModelRequest, call_id: &str) -> String {
    results(request)
        .into_iter()
        .find(|(id, _output)| id == call_id)
        .map_or_else(|| panic!("no result for {call_id}"), |(_id, output)| output)
}

/// The texts of a request's user messages, in order.
fn user_texts(request: &ModelRequest) -> Vec<String> {
    request
        .messages
        .iter()
        .filter(|message| message.role == Role::User)
        .flat_map(|message| &message.content)
        .filter_map(|content| match content {
            Content::Text(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// The notes about background commands that a request carries.
fn notes(request: &ModelRequest) -> Vec<String> {
    user_texts(request)
        .into_iter()
        .filter(|text| text.starts_with("Background commands of this conversation:"))
        .collect()
}

#[tokio::test]
async fn a_command_outlives_its_reply_and_reads_return_its_new_lines() {
    let setup = Setup::new().await;
    let command = shell(
        "Write-Output first; Start-Sleep -Seconds 1; Write-Output 'ready on 5173'; Start-Sleep -Seconds 60",
        "echo first; sleep 1; echo 'ready on 5173'; sleep 60",
    );
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[("call_1", "start_command", &start(&command))]));
    model.push_reply(reply("The server starts."));
    model.push_reply(tool_calls(&[(
        "call_2",
        "read_command_output",
        &json!({ "command": 1, "wait_seconds": 60, "until": "ready on \\d+" }).to_string(),
    )]));
    model.push_reply(tool_calls(&[(
        "call_3",
        "read_command_output",
        &json!({ "command": 1 }).to_string(),
    )]));
    model.push_reply(tool_calls(&[(
        "call_4",
        "stop_command",
        &json!({ "command": 1 }).to_string(),
    )]));
    model.push_reply(reply("It is up, and now stopped."));

    let (conversation, first) = setup.start("Start the server").await;
    let transcript = setup.harness.wait_for_run(&conversation, &first).await;

    // The call returned at once, and the command runs after the reply.
    let started = transcript.tool_calls(0)[0].clone();
    assert_eq!(started.status, ToolCallStatus::Completed);
    assert_eq!(
        started.result.unwrap().outcome,
        ToolOutcome::CommandStarted {
            number: 1,
            command: command.clone(),
        }
    );
    assert!(transcript.background_commands[&1].is_running());
    // A call without an activity leaves the command to be named by its text.
    assert_eq!(transcript.background_commands[&1].activity, None);
    assert_eq!(setup.harness.runtime.running_background_count(), 1);
    let requests = model.reply_requests();
    assert!(
        result(&requests[1], "call_1").starts_with("Command 1 started in the background, in "),
        "{}",
        result(&requests[1], "call_1")
    );

    let second = setup.send(&conversation, "Is it up?").await;
    let transcript = setup.harness.wait_for_run(&conversation, &second).await;

    let requests = model.reply_requests();
    // The owner's message carries the note of the command that runs, with
    // its unread lines and none of its output.
    let opening = notes(&requests[2]);
    assert_eq!(opening.len(), 1, "{:?}", user_texts(&requests[2]));
    assert!(
        opening[0].contains(&format!(
            "- Command 1 ({}) has run for ",
            command.lines().next().unwrap()
        )),
        "{}",
        opening[0]
    );
    // The note's lines are its heading, one line for the command, and how to
    // read the output.
    assert_eq!(opening[0].lines().count(), 3, "{}", opening[0]);
    // The wait ends at the line that matches, with the lines before it.
    let waited = result(&requests[3], "call_2");
    assert!(waited.contains("first\n"), "{waited}");
    assert!(waited.contains("ready on 5173\n"), "{waited}");
    assert!(waited.contains("Line 2 matches until."), "{waited}");
    // The next read finds nothing new, and the stop says how it ended.
    let again = result(&requests[4], "call_3");
    assert!(
        again.contains("No new lines since your last read; it wrote 2 lines so far."),
        "{again}"
    );
    let stopped = result(&requests[5], "call_4");
    assert!(
        stopped.contains("is stopped. You stopped it after"),
        "{stopped}"
    );
    // A Unix shell can write a termination diagnostic when its child stops.
    let unread_lines = transcript.background_commands[&1].lines - 2;
    assert!(
        stopped.contains(&format!("{unread_lines} of its lines are unread.")),
        "{stopped}"
    );
    assert_eq!(setup.harness.runtime.running_background_count(), 0);
    let ended = &transcript.background_commands[&1];
    assert_eq!(ended.ending, Some(BackgroundEnding::StoppedByAgent));
    // The agent's own stop is told by its result, so no note follows the
    // opening note of the owner's message.
    assert_eq!(notes(&requests[5]), opening);
    let stored = setup
        .harness
        .store
        .background_command(&conversation, 1)
        .await
        .unwrap()
        .unwrap();
    assert!(stored.document.told);
    let archive = setup
        .harness
        .store
        .archive(&conversation, stored.document.archive_id.as_ref().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(archive.name, "output");
    assert!(
        archive.content.starts_with("first\nready on 5173\n"),
        "{}",
        archive.content
    );
    assert_eq!(
        u32::try_from(archive.content.lines().count()).unwrap(),
        ended.lines
    );
    setup.harness.runtime.shutdown().await;
}

#[tokio::test]
async fn a_command_is_named_by_the_activity_of_its_start_call() {
    let setup = Setup::new().await;
    let model = &setup.harness.model;
    let arguments = json!({ "activity": "Running the test ticks", "command": long_running() });
    model.push_reply(tool_calls(&[(
        "call_1",
        "start_command",
        &arguments.to_string(),
    )]));
    model.push_reply(reply("The ticks run."));

    let (conversation, run) = setup.start("Run the ticks").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let command = &transcript.background_commands[&1];
    assert_eq!(command.activity.as_deref(), Some("Running the test ticks"));
    assert_eq!(command.command, long_running());
    // The record keeps it, so the name outlives a restart.
    let stored = setup
        .harness
        .store
        .background_command(&conversation, 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.document.activity.as_deref(),
        Some("Running the test ticks")
    );
    setup.harness.runtime.shutdown().await;
}

#[tokio::test]
async fn an_end_during_a_reply_reaches_the_model_before_its_next_call() {
    let setup = Setup::new().await;
    let quick = shell("Start-Sleep -Seconds 1; exit 3", "sleep 1; exit 3");
    let longer = shell("Start-Sleep -Seconds 6", "sleep 6");
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[("call_1", "start_command", &start(&quick))]));
    model.push_reply(tool_calls(&[("call_2", "run_command", &start(&longer))]));
    model.push_reply(reply("The check failed with code 3."));

    let (conversation, run) = setup.start("Run the check").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let requests = model.reply_requests();
    assert_eq!(notes(&requests[1]), Vec::<String>::new());
    let told = notes(&requests[2]);
    assert_eq!(told.len(), 1, "{:?}", user_texts(&requests[2]));
    assert!(
        told[0].contains("- Command 1 (") && told[0].contains(") ended with exit code 3 after "),
        "{}",
        told[0]
    );
    // The note comes after the results of the step's calls.
    let last = requests[2].messages.last().unwrap();
    assert_eq!(last.role, Role::User);
    // An end is told once.
    let stored = setup
        .harness
        .store
        .background_command(&conversation, 1)
        .await
        .unwrap()
        .unwrap();
    assert!(stored.document.told);
    assert_eq!(stored.document.exit_code, Some(3));
    setup.harness.runtime.shutdown().await;
}

#[tokio::test]
async fn a_reply_that_would_end_with_an_end_untold_continues_once() {
    let setup = Setup::new().await;
    let quick = shell("Start-Sleep -Seconds 1; exit 0", "sleep 1; exit 0");
    let release = Arc::new(tokio::sync::Notify::new());
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[("call_1", "start_command", &start(&quick))]));
    model.push_reply(vec![
        Step::Wait(Arc::clone(&release)),
        Step::Event(ProviderEvent::TextDelta("It started.".to_owned())),
        Step::Event(ProviderEvent::Finished(FinishReason::Stop)),
    ]);
    model.push_reply(reply("It also finished, with code 0."));

    let (conversation, run) = setup.start("Run the build").await;
    setup.ended(&conversation, 1).await;
    release.notify_one();
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let requests = model.reply_requests();
    assert_eq!(requests.len(), 3);
    let told = notes(&requests[2]);
    assert_eq!(told.len(), 1);
    assert!(told[0].contains("ended with exit code 0"), "{}", told[0]);
    setup.harness.runtime.shutdown().await;
}

#[tokio::test]
async fn stop_of_a_reply_ends_a_wait_and_leaves_the_command_running() {
    let setup = Setup::new().await;
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[
        ("call_1", "start_command", &start(&long_running())),
        (
            "call_2",
            "read_command_output",
            &json!({ "command": 1, "wait_seconds": 600 }).to_string(),
        ),
    ]));

    let (conversation, run) = setup.start("Follow the log").await;
    setup
        .harness
        .wait_until(&conversation, |transcript| {
            transcript
                .tool_calls(0)
                .get(1)
                .is_some_and(|call| call.status == ToolCallStatus::Running)
        })
        .await;
    let stopping = Instant::now();
    setup.harness.runtime.stop_reply(&conversation).await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert!(
        stopping.elapsed() < Duration::from_secs(2),
        "{:?}",
        stopping.elapsed()
    );
    assert_eq!(transcript.run_state(&run), Some(RunState::Stopped));
    assert!(transcript.background_commands[&1].is_running());
    assert_eq!(setup.harness.runtime.running_background_count(), 1);
    setup.harness.runtime.shutdown().await;
}

#[tokio::test]
async fn a_child_s_commands_end_with_its_task() {
    let setup = Setup::new().await;
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        "start_task",
        &json!({
            "activity": "Starting the watcher",
            "title": "Watch",
            "agent": "general",
            "instruction": "CHILD-1: start the watcher and report.",
            "background": false,
        })
        .to_string(),
    )]));
    model.push_reply(reply("The child reported."));
    model.push_reply_for(
        "CHILD-1",
        tool_calls(&[("child_1", "start_command", &start(&long_running()))]),
    );
    model.push_reply_for("CHILD-1", reply("Watching."));
    model.push_reply_for("CHILD-1", reply("Report: the watcher ran."));

    let (conversation, run) = setup.start("OWNER: watch").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let [task] = transcript.tasks(0).try_into().unwrap();
    let child = setup
        .harness
        .store
        .background_command(&task.conversation_id, 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(child.document.ending, Some(BackgroundEnding::TaskEnded));
    assert_eq!(setup.harness.runtime.running_background_count(), 0);
    setup.harness.runtime.shutdown().await;
}

#[tokio::test]
async fn deleting_a_conversation_ends_its_commands() {
    let setup = Setup::new().await;
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        "start_command",
        &start(&long_running()),
    )]));
    model.push_reply(reply("Started."));
    let (conversation, run) = setup.start("Start it").await;
    setup.harness.wait_for_run(&conversation, &run).await;
    assert_eq!(setup.harness.runtime.running_background_count(), 1);

    setup
        .harness
        .runtime
        .delete_conversation(&conversation)
        .await
        .unwrap();

    assert_eq!(setup.harness.runtime.running_background_count(), 0);
    assert_eq!(
        setup
            .harness
            .store
            .background_commands(&conversation)
            .await
            .unwrap(),
        Vec::new()
    );
    setup.harness.runtime.shutdown().await;
}

#[tokio::test]
async fn quitting_interrupts_commands_keeps_their_output_and_the_next_reply_tells_the_model() {
    let setup = Setup::new().await;
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[
        ("call_1", "start_command", &start(&long_running())),
        (
            "call_2",
            "read_command_output",
            &json!({ "command": 1, "wait_seconds": 60, "until": "first" }).to_string(),
        ),
    ]));
    model.push_reply(reply("It runs."));
    let (conversation, run) = setup.start("Start it").await;
    setup.harness.wait_for_run(&conversation, &run).await;

    let setup = setup.restart().await;
    let harness = &setup.harness;

    let stored = harness
        .store
        .background_command(&conversation, 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.document.ending, Some(BackgroundEnding::Interrupted));
    assert!(!stored.document.told);
    let archive = harness
        .store
        .archive(&conversation, stored.document.archive_id.as_ref().unwrap())
        .await
        .unwrap()
        .unwrap();
    // Keep the command's output and any termination diagnostic from its shell.
    assert!(
        archive.content.starts_with("first\n"),
        "{}",
        archive.content
    );
    let archived_lines = archive.content.lines().count();
    assert_eq!(harness.runtime.running_background_count(), 0);

    setup.harness.model.push_reply(tool_calls(&[(
        "call_3",
        "read_command_output",
        &json!({ "command": 1, "from_line": 1 }).to_string(),
    )]));
    setup.harness.model.push_reply(reply("It was interrupted."));
    let next = setup.send(&conversation, "Is it still running?").await;
    setup.harness.wait_for_run(&conversation, &next).await;

    let requests = setup.harness.model.reply_requests();
    let opening = notes(&requests[2]);
    assert_eq!(opening.len(), 1);
    assert!(
        opening[0].contains("ended when Mannie quit"),
        "{}",
        opening[0]
    );
    // A read after the end comes from the archive, with the same numbers.
    let archived = result(&requests[3], "call_3");
    assert!(
        archived.contains(&format!(
            "Lines 1 to {archived_lines} of {archived_lines}:\n{}",
            archive.content
        )),
        "{archived}"
    );
    setup.harness.runtime.shutdown().await;
}

#[tokio::test]
async fn a_ninth_command_in_a_conversation_is_refused() {
    let setup = Setup::new().await;
    let command = long_running();
    let ids: Vec<String> = (1..=9).map(|index| format!("call_{index}")).collect();
    let arguments = start(&command);
    let calls: Vec<(&str, &str, &str)> = ids
        .iter()
        .map(|id| (id.as_str(), "start_command", arguments.as_str()))
        .collect();
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&calls));
    model.push_reply(reply("Eight run."));

    let (conversation, run) = setup.start("Start nine").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let calls = transcript.tool_calls(0);
    assert_eq!(calls.len(), 9);
    assert_eq!(
        calls[8].result.as_ref().unwrap().outcome,
        ToolOutcome::Failed {
            failure: ToolFailure::TooManyBackgroundCommands {
                scope: BackgroundLimit::Conversation,
                limit: 8,
            }
        }
    );
    assert_eq!(setup.harness.runtime.running_background_count(), 8);
    setup.harness.runtime.shutdown().await;
}

#[tokio::test]
async fn a_grant_from_the_foreground_asks_again_before_the_same_command_runs_in_the_background() {
    let setup = Setup::with_full_access(false).await;
    let command = shell("Write-Output hi", "echo hi");
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[("call_1", "run_command", &start(&command))]));
    model.push_reply(tool_calls(&[("call_2", "start_command", &start(&command))]));
    model.push_reply(tool_calls(&[("call_3", "run_command", &start(&command))]));
    model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("Say hi").await;
    let mut asked = Vec::new();
    for call in 0..2 {
        let transcript = setup
            .harness
            .wait_until(&conversation, |transcript| {
                transcript
                    .tool_calls(0)
                    .get(call)
                    .is_some_and(|view| view.status == ToolCallStatus::AwaitingApproval)
            })
            .await;
        let part = transcript
            .parts(0)
            .into_iter()
            .filter(|part| matches!(part.content, PartContent::ToolCall { .. }))
            .nth(call)
            .unwrap();
        let request = transcript.tool_calls(0)[call].request.clone().unwrap();
        asked.push(request.grants.clone());
        setup
            .harness
            .runtime
            .decide_approval(DecideApproval {
                conversation_id: conversation.clone(),
                part_id: part.id,
                digest: request.digest,
                choice: ApprovalChoice::AllowForConversation,
            })
            .await
            .unwrap();
    }
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    // The foreground's grant did not cover the background, and the
    // background's grant covers the foreground: the third call ran without
    // asking.
    let in_background = |grants: &[GrantScope]| {
        grants.iter().any(|grant| {
            matches!(
                grant,
                GrantScope::Command {
                    background: true,
                    ..
                }
            )
        })
    };
    assert!(!in_background(&asked[0]));
    assert!(in_background(&asked[1]));
    let calls = transcript.tool_calls(0);
    assert_eq!(calls[2].status, ToolCallStatus::Completed);
    assert_eq!(calls[2].request, None);
    setup.harness.runtime.shutdown().await;
}

#[tokio::test]
async fn the_tools_of_background_commands_come_with_start_command_and_the_guidance() {
    let setup = Setup::new().await;
    setup.harness.model.push_reply(reply("Hello."));

    let (conversation, run) = setup.start("Hi").await;
    setup.harness.wait_for_run(&conversation, &run).await;

    let request = &setup.harness.model.reply_requests()[0];
    let names: Vec<&str> = request
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect();
    for tool in ["start_command", "read_command_output", "stop_command"] {
        assert!(names.contains(&tool), "{names:?}");
    }
    assert!(
        request
            .instructions
            .contains(mannie_runtime::texts::BACKGROUND_GUIDANCE)
    );
    setup.harness.runtime.shutdown().await;
}

// Wake-ups (docs/features/system-tools/wake-ups.md).

/// The delay that gathers ends in tests that need no particular delay.
const SHORT_DELAY: Duration = Duration::from_millis(200);

/// A command that ends with `code` after `seconds`.
fn ends_after(seconds: u32, code: i32) -> String {
    shell(
        &format!("Start-Sleep -Seconds {seconds}; exit {code}"),
        &format!("sleep {seconds}; exit {code}"),
    )
}

/// The ends that the request of a turn names, when it is a wake-up's.
fn woken_by(transcript: &Transcript, position: u32) -> Option<Vec<CommandEndView>> {
    transcript
        .parts(position)
        .into_iter()
        .find_map(|part| match part.content {
            PartContent::CommandNote { note } => Some(note.ends),
            _ => None,
        })
}

#[tokio::test]
async fn an_end_of_its_own_wakes_an_idle_conversation_with_a_note_of_mannie_s() {
    let setup = Setup::waking(SHORT_DELAY).await;
    let command = ends_after(1, 2);
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[("call_1", "start_command", &start(&command))]));
    model.push_reply(reply("The build runs."));
    model.push_reply(reply("The build failed with code 2."));

    let (conversation, _run) = setup.start("Build it").await;
    let transcript = setup.replied(&conversation, 2).await;

    let ends = woken_by(&transcript, 1).expect("the second turn is a wake-up");
    assert_eq!(ends.len(), 1);
    assert_eq!(ends[0].number, 1);
    assert_eq!(ends[0].ending, BackgroundEnding::Exited);
    assert_eq!(ends[0].exit_code, Some(2));
    // The model reads Mannie's note and what the reply is for, and no
    // output.
    let request = model.reply_requests().pop().unwrap();
    let last = user_texts(&request).pop().unwrap();
    assert!(
        last.starts_with("Background commands of this conversation:\n- Command 1 ("),
        "{last}"
    );
    assert!(
        last.ends_with(mannie_runtime::texts::WAKE_UP_REQUEST),
        "{last}"
    );
    let stored = setup
        .harness
        .store
        .background_command(&conversation, 1)
        .await
        .unwrap()
        .unwrap();
    assert!(stored.document.told);
    // One end wakes the conversation once.
    assert_eq!(setup.turns_after(&conversation, SHORT_DELAY * 5).await, 2);
    setup.harness.runtime.shutdown().await;
}

#[tokio::test]
async fn ends_that_come_together_share_one_wake_up() {
    let setup = Setup::waking(Duration::from_secs(2)).await;
    let first = ends_after(1, 0);
    let second = ends_after(1, 1);
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[
        ("call_1", "start_command", &start(&first)),
        ("call_2", "start_command", &start(&second)),
    ]));
    model.push_reply(reply("Both run."));
    model.push_reply(reply("One passed, one failed."));

    let (conversation, _run) = setup.start("Run both").await;
    let transcript = setup.replied(&conversation, 2).await;

    let ends = woken_by(&transcript, 1).expect("the second turn is a wake-up");
    let numbers: Vec<u32> = ends.iter().map(|end| end.number).collect();
    assert_eq!(numbers, [1, 2]);
    assert_eq!(
        setup
            .turns_after(&conversation, Duration::from_secs(3))
            .await,
        2
    );
    setup.harness.runtime.shutdown().await;
}

#[tokio::test]
async fn the_owner_s_stop_pauses_wake_ups_until_their_next_message() {
    let setup = Setup::waking(SHORT_DELAY).await;
    let release = Arc::new(tokio::sync::Notify::new());
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        "start_command",
        &start(&ends_after(2, 0)),
    )]));
    model.push_reply(vec![Step::Wait(Arc::clone(&release))]);
    model.push_reply(reply("It ended while you were away."));

    let (conversation, run) = setup.start("Run it").await;
    // The stop ends the second model call, which waits, so the next message
    // plays the last script.
    tokio::time::timeout(Duration::from_secs(20), async {
        while model.reply_requests().len() < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the second model call starts");
    setup.harness.runtime.stop_reply(&conversation).await;
    setup.harness.wait_for_run(&conversation, &run).await;
    setup.ended(&conversation, 1).await;

    assert_eq!(setup.turns_after(&conversation, SHORT_DELAY * 5).await, 1);
    let next = setup.send(&conversation, "Did it end?").await;
    setup.harness.wait_for_run(&conversation, &next).await;
    let request = model.reply_requests().pop().unwrap();
    let opening = notes(&request);
    assert_eq!(opening.len(), 1);
    assert!(
        opening[0].contains("ended with exit code 0"),
        "{}",
        opening[0]
    );
    setup.harness.runtime.shutdown().await;
}

#[tokio::test]
async fn with_the_setting_off_no_end_wakes_and_the_next_reply_tells_it() {
    let setup = Setup::new().await;
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        "start_command",
        &start(&ends_after(1, 0)),
    )]));
    model.push_reply(reply("It runs."));
    model.push_reply(reply("It ended."));

    let (conversation, run) = setup.start("Run it").await;
    setup.harness.wait_for_run(&conversation, &run).await;
    setup.ended(&conversation, 1).await;

    assert_eq!(
        setup
            .turns_after(&conversation, Duration::from_secs(3))
            .await,
        1
    );
    let next = setup.send(&conversation, "Did it end?").await;
    setup.harness.wait_for_run(&conversation, &next).await;
    let opening = notes(&model.reply_requests().pop().unwrap());
    assert_eq!(opening.len(), 1);
    assert!(
        opening[0].contains("ended with exit code 0"),
        "{}",
        opening[0]
    );
    setup.harness.runtime.shutdown().await;
}

#[tokio::test]
async fn an_end_that_its_reply_told_wakes_nothing_after_the_reply() {
    let setup = Setup::waking(SHORT_DELAY).await;
    let release = Arc::new(tokio::sync::Notify::new());
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        "start_command",
        &start(&ends_after(1, 0)),
    )]));
    model.push_reply(vec![
        Step::Wait(Arc::clone(&release)),
        Step::Event(ProviderEvent::TextDelta("It started.".to_owned())),
        Step::Event(ProviderEvent::Finished(FinishReason::Stop)),
    ]);
    model.push_reply(reply("And it ended."));

    let (conversation, run) = setup.start("Run it").await;
    setup.ended(&conversation, 1).await;
    release.notify_one();
    setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(setup.turns_after(&conversation, SHORT_DELAY * 5).await, 1);
    setup.harness.runtime.shutdown().await;
}

#[tokio::test]
async fn a_task_conversation_never_wakes() {
    let setup = Setup::waking(SHORT_DELAY).await;
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        "start_task",
        &json!({
            "activity": "Starting the check",
            "title": "Check",
            "agent": "general",
            "instruction": "CHILD-1: run the check and report.",
            "background": false,
        })
        .to_string(),
    )]));
    model.push_reply(reply("The child reported."));
    model.push_reply_for(
        "CHILD-1",
        tool_calls(&[("child_1", "start_command", &start(&ends_after(1, 0)))]),
    );
    model.push_reply_for(
        "CHILD-1",
        vec![
            Step::Delay(Duration::from_secs(3)),
            Step::Event(ProviderEvent::TextDelta("Checked.".to_owned())),
            Step::Event(ProviderEvent::Finished(FinishReason::Stop)),
        ],
    );
    model.push_reply_for("CHILD-1", reply("Report: the check ran."));
    model.push_reply_for("CHILD-1", reply("Report: the check passed."));

    let (conversation, run) = setup.start("OWNER: check").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let [task] = transcript.tasks(0).try_into().unwrap();
    let child = &task.conversation_id;
    assert_eq!(setup.turns_after(child, SHORT_DELAY * 5).await, 1);
    setup.harness.runtime.shutdown().await;
}

#[tokio::test]
async fn a_conversation_wakes_at_most_six_times_an_hour() {
    let setup = Setup::waking(SHORT_DELAY).await;
    let commands: Vec<String> = (1..=7).map(|seconds| ends_after(seconds * 2, 0)).collect();
    let arguments: Vec<String> = commands.iter().map(|command| start(command)).collect();
    let ids: Vec<String> = (1..=7).map(|index| format!("call_{index}")).collect();
    let calls: Vec<(&str, &str, &str)> = ids
        .iter()
        .zip(&arguments)
        .map(|(id, arguments)| (id.as_str(), "start_command", arguments.as_str()))
        .collect();
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&calls));
    model.push_reply(reply("Seven run."));
    for _wake_up in 0..6 {
        model.push_reply(reply("Noted."));
    }
    model.push_reply(reply("The seventh ended too."));

    let (conversation, _run) = setup.start("Run seven").await;
    setup.replied(&conversation, 7).await;
    setup.ended(&conversation, 7).await;

    assert_eq!(
        setup
            .turns_after(&conversation, Duration::from_secs(2))
            .await,
        7
    );
    let next = setup.send(&conversation, "Anything else?").await;
    setup.harness.wait_for_run(&conversation, &next).await;
    // The wake-ups' notes come first, and the note of the owner's message
    // tells only the end that waited.
    let notes = notes(&model.reply_requests().pop().unwrap());
    assert_eq!(notes.len(), 7);
    let opening = notes.last().unwrap();
    assert!(opening.contains("- Command 7 ("), "{opening}");
    assert!(!opening.contains("- Command 6 ("), "{opening}");
    setup.harness.runtime.shutdown().await;
}
