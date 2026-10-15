//! Tasks of child agents (docs/features/child-agents/tasks.md): a start
//! returns the child's confirmed report, tool sets are enforced, slots limit
//! the children, background tasks report through `wait_for_tasks`,
//! follow-ups keep a child's context, a child's approvals show in its parent,
//! stopping reaches every child, and the owner's commands refuse task
//! conversations.
//!
//! Each child's model calls are scripted by a marker in its instruction, the
//! first message of its conversation; the parent's play in order.

#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
#[cfg(any(windows, target_os = "linux"))]
use std::time::Instant;

use mannie_provider::testing::Step;
use mannie_provider::{Content, ModelRequest, Role};
use mannie_runtime::testing::{Harness, HarnessOptions, Transcript, reply, tool_calls};
use mannie_runtime::{
    AgentDefinition, AgentRegistry, CONVERSATION, EXPLORE, GENERAL, TASK_TOOLS, texts,
};
use mannie_types::chat::{
    ChatError, RenameConversation, SendMessage, SetConversationProfile, StartConversation,
    StopOutcome, SubscribeConversation,
};
use mannie_types::ids::{ConversationId, PartId, ProjectId, RunId};
use mannie_types::run::{RunActivity, RunState};
use mannie_types::tasks::{
    MAX_REPORT_VIEW_BYTES, TaskConversation, TaskState, TaskView, UpdateChildAgentsSettings,
};
use mannie_types::tools::{
    ApprovalChoice, DecideApproval, Decision, NotRunReason, ToolCallStatus, ToolsError,
};
use mannie_types::transcript::{ConversationUpdate, PartContent, SubscriptionEnd};
use serde_json::json;
use tempfile::TempDir;

/// A runtime whose profile uses tools, and a project whose folder is the
/// workspace of the conversations that start in it.
struct Setup {
    harness: Harness,
    _workspace: TempDir,
    root: PathBuf,
    project: ProjectId,
}

impl Setup {
    async fn new() -> Self {
        Self::with_options(HarnessOptions {
            use_tools: true,
            ..HarnessOptions::default()
        })
        .await
    }

    async fn with_options(options: HarnessOptions) -> Self {
        let harness = Harness::with_options(options).await;
        let workspace = tempfile::tempdir().unwrap();
        let project = harness
            .runtime
            .add_project(workspace.path().to_str().unwrap().to_owned())
            .await
            .unwrap();
        Self {
            harness,
            _workspace: workspace,
            root: PathBuf::from(&project.folder),
            project: project.id,
        }
    }

    fn path(&self, relative: &str) -> PathBuf {
        let mut path = self.root.clone();
        for name in relative.split('/') {
            path.push(name);
        }
        path
    }

    fn file(&self, relative: &str, content: &str) {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    /// Starts a conversation in the project with the owner's message.
    async fn start(&self, text: &str) -> (ConversationId, RunId) {
        let started = self
            .harness
            .runtime
            .start_conversation(StartConversation {
                text: text.to_owned(),
                profile_id: None,
                project_id: Some(self.project.clone()),
            })
            .await
            .unwrap();
        (started.conversation_id, started.run_id)
    }

    /// The parent's model scripts, in order.
    fn parent(&self, steps: Vec<Step>) {
        self.harness.model.push_reply(steps);
    }

    /// A child's model scripts, in order, by the marker in its instruction.
    fn child(&self, marker: &str, steps: Vec<Step>) {
        self.harness.model.push_reply_for(marker, steps);
    }

    /// A general child that writes `path`, then reports.
    fn writer(&self, marker: &str, call_id: &str, path: &str) {
        self.child(marker, tool_calls(&[(call_id, "write_file", &write(path))]));
        self.child(marker, reply("written"));
        self.child(marker, reply(&format!("Report {marker}.")));
    }

    /// Whether the conversation list marks the conversation as waiting for
    /// the owner.
    async fn marked_waiting(&self, conversation: &ConversationId) -> bool {
        self.harness
            .store
            .conversation_summary(conversation)
            .await
            .unwrap()
            .unwrap()
            .awaiting_approval
    }

    /// How many grants "for this conversation" a conversation has.
    async fn grants(&self, conversation: &ConversationId) -> usize {
        self.harness
            .runtime
            .conversation_grants(conversation)
            .await
            .unwrap()
            .len()
    }

    async fn set_limit(&self, limit: u32) {
        self.harness
            .runtime
            .update_child_agents_settings(UpdateChildAgentsSettings {
                parallel_children: limit,
            })
            .await
            .unwrap();
    }

    /// Follows the parent until its tasks satisfy `done`.
    async fn tasks_until(
        &self,
        conversation: &ConversationId,
        done: impl Fn(&[TaskView]) -> bool,
    ) -> Transcript {
        self.harness
            .wait_until(conversation, |transcript| done(&transcript.tasks(0)))
            .await
    }
}

fn start(title: &str, agent: &str, instruction: &str, background: bool) -> String {
    json!({
        "activity": format!("Starting {title}"),
        "title": title,
        "agent": agent,
        "instruction": instruction,
        "background": background,
    })
    .to_string()
}

fn follow_up(task: u32, instruction: &str, background: bool) -> String {
    json!({
        "activity": "Following up",
        "task": task,
        "instruction": instruction,
        "background": background,
    })
    .to_string()
}

fn wait(tasks: &[u32]) -> String {
    json!({ "activity": "Waiting for reports", "tasks": tasks }).to_string()
}

fn write(path: &str) -> String {
    json!({ "activity": format!("Writing {path}"), "path": path, "content": "made\n" }).to_string()
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

fn tool_names(request: &ModelRequest) -> BTreeSet<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
}

/// The parent's part of the first task call.
fn first_task_part(transcript: &Transcript) -> PartId {
    transcript
        .parts(0)
        .into_iter()
        .find(|part| matches!(part.content, PartContent::Task { .. }))
        .map(|part| part.id)
        .unwrap()
}

#[tokio::test]
async fn a_task_returns_the_childs_confirmed_report() {
    let setup = Setup::new().await;
    setup.file("notes.txt", "hello\n");
    setup.parent(tool_calls(&[(
        "call_1",
        "start_task",
        &start(
            "Read the notes",
            "explore",
            "CHILD-1: Read notes.txt and say what it says.",
            false,
        ),
    )]));
    setup.parent(reply("The notes say hello."));
    setup.child(
        "CHILD-1",
        tool_calls(&[("child_1", "read_file", r#"{"path":"notes.txt"}"#)]),
    );
    setup.child("CHILD-1", reply("It says hello."));
    setup.child("CHILD-1", reply("notes.txt says hello."));

    let (conversation, run) = setup.start("OWNER: What do the notes say?").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let [task] = transcript.tasks(0).try_into().unwrap();
    assert_eq!(task.number, 1);
    assert_eq!(task.agent, "explore");
    assert_eq!(task.title, "Read the notes");
    assert_eq!(task.state, TaskState::Reported);
    assert_eq!(task.report.as_deref(), Some("notes.txt says hello."));
    assert_eq!(task.tool_calls, 1);
    assert!(task.instruction.starts_with("CHILD-1"));
    let child = setup.harness.model.requests_for("CHILD-1");
    assert_eq!(child.len(), 3);
    // The child is asked once to confirm, and it reads the parent's
    // workspace.
    let confirmations = |request: &ModelRequest| {
        user_texts(request)
            .iter()
            .filter(|text| *text == texts::CONFIRMATION_NOTE)
            .count()
    };
    assert_eq!(confirmations(&child[1]), 0);
    assert_eq!(confirmations(&child[2]), 1);
    assert!(result(&child[1], "child_1").contains("     1\thello"));
    assert!(
        child[0]
            .instructions
            .starts_with(texts::CHILD_BASE_INSTRUCTIONS)
    );
    assert!(child[0].instructions.contains(texts::EXPLORE_INSTRUCTIONS));
    assert!(!child[0].instructions.contains(texts::TASKS_GUIDANCE));
    let parent = setup.harness.model.requests_for("OWNER:");
    assert!(parent[0].instructions.contains(texts::TASKS_GUIDANCE));
    assert_eq!(
        result(&parent[1], "call_1"),
        texts::task_reported(1, "notes.txt says hello.")
    );
    // The child's conversation is a task conversation, which the list leaves
    // out.
    let summary = setup
        .harness
        .store
        .conversation_summary(&task.conversation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        summary.task,
        Some(TaskConversation {
            parent_id: conversation.clone(),
            number: 1,
            agent: "explore".to_owned(),
        })
    );
    assert_eq!(summary.project_id, None);
    let listed = setup.harness.store.list_conversations().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, conversation);
}

#[tokio::test]
async fn children_get_only_their_agents_tools_and_never_the_task_tools() {
    let setup = Setup::new().await;
    setup.parent(tool_calls(&[
        (
            "call_1",
            "start_task",
            &start("Look around", "explore", "CHILD-E: look", false),
        ),
        (
            "call_2",
            "start_task",
            &start("Change things", "general", "CHILD-G: change", false),
        ),
    ]));
    setup.parent(reply("Done."));
    setup.child(
        "CHILD-E",
        tool_calls(&[
            ("e_1", "write_file", &write("x.txt")),
            (
                "e_2",
                "start_task",
                &start("Deeper", "explore", "CHILD-N: nested", false),
            ),
        ]),
    );
    setup.child("CHILD-E", reply("I could not write."));
    setup.child("CHILD-E", reply("Report E."));
    setup.child("CHILD-G", reply("Nothing to do."));
    setup.child("CHILD-G", reply("Report G."));

    let (conversation, run) = setup.start("OWNER: split it").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let explore = setup.harness.model.requests_for("CHILD-E");
    let general = setup.harness.model.requests_for("CHILD-G");
    let explore_tools: BTreeSet<String> = EXPLORE
        .tools
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    assert_eq!(tool_names(&explore[0]), explore_tools);
    let general_tools = tool_names(&general[0]);
    assert!(general_tools.contains("write_file"), "{general_tools:?}");
    assert!(!general_tools.contains("move_to_project"));
    for task_tool in TASK_TOOLS {
        assert!(!general_tools.contains(task_tool));
    }
    assert_eq!(
        general_tools.len(),
        GENERAL
            .tools
            .iter()
            .filter(|name| setup.harness.model.requests()[0]
                .tools
                .iter()
                .any(|tool| tool.name == **name))
            .count()
    );
    // A call beyond its tools does not run, task tools included, and
    // nothing is written.
    assert_eq!(
        result(&explore[1], "e_1"),
        texts::unknown_tool("write_file")
    );
    assert_eq!(
        result(&explore[1], "e_2"),
        texts::unknown_tool("start_task")
    );
    assert!(!setup.path("x.txt").exists());
    assert_eq!(setup.harness.model.requests_for("CHILD-N"), Vec::new());
    let child = &transcript.tasks(0)[0].conversation_id;
    let child_transcript = setup.harness.snapshot(child).await;
    let calls = child_transcript.tool_calls(0);
    assert_eq!(calls.len(), 2);
    assert!(
        calls
            .iter()
            .all(|call| call.not_run == Some(NotRunReason::UnknownTool))
    );
    assert_eq!(child_transcript.task_calls(0), Vec::new());
}

/// The task tools as a model receives them, compared with a reviewed copy in
/// `tests/fixtures/task_definitions.json`: a change to a description or a
/// schema changes how models give tasks, so it shows in review.
/// `MANNIE_UPDATE_EXPECTED=1` writes the file instead of comparing; review
/// the result before committing it.
#[tokio::test]
async fn the_task_tool_definitions_match_their_reviewed_copy() {
    let setup = Setup::new().await;
    setup.parent(reply("Hello."));
    let (conversation, run) = setup.start("OWNER: hello").await;
    setup.harness.wait_for_run(&conversation, &run).await;

    let request = &setup.harness.model.reply_requests()[0];
    let actual: Vec<serde_json::Value> = request
        .tools
        .iter()
        .filter(|tool| TASK_TOOLS.contains(&tool.name.as_str()))
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            })
        })
        .collect();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("task_definitions.json");
    let actual = serde_json::Value::Array(actual);
    if std::env::var_os("MANNIE_UPDATE_EXPECTED").is_some() {
        let text = serde_json::to_string_pretty(&actual).unwrap();
        fs::write(&fixture, format!("{text}\n")).unwrap();
        return;
    }
    let expected: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&fixture).unwrap()).unwrap();
    assert_eq!(
        actual,
        expected,
        "the task tool definitions changed; review them and update {}",
        fixture.display()
    );
}

#[tokio::test]
async fn a_profile_without_tools_is_offered_no_task_tools() {
    let harness = Harness::start().await;
    harness.model.push_reply(reply("Hello."));

    let started = harness
        .runtime
        .start_conversation(StartConversation {
            text: "Hi".to_owned(),
            profile_id: None,
            project_id: None,
        })
        .await
        .unwrap();
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    let request = &harness.model.reply_requests()[0];
    assert_eq!(request.tools, Vec::new());
    assert!(!request.instructions.contains(texts::TASKS_GUIDANCE));
}

#[tokio::test]
async fn a_limit_of_two_runs_two_children_and_a_larger_limit_starts_the_third() {
    let setup = Setup::new().await;
    setup.set_limit(2).await;
    setup.parent(tool_calls(&[
        (
            "call_1",
            "start_task",
            &start("One", "explore", "CHILD-1", true),
        ),
        (
            "call_2",
            "start_task",
            &start("Two", "explore", "CHILD-2", true),
        ),
        (
            "call_3",
            "start_task",
            &start("Three", "explore", "CHILD-3", true),
        ),
    ]));
    setup.parent(reply("Started three."));
    for marker in ["CHILD-1", "CHILD-2", "CHILD-3"] {
        setup.child(marker, vec![Step::Hang]);
    }

    let (conversation, run) = setup.start("OWNER: three tasks").await;
    let count = |tasks: &[TaskView], state: TaskState| {
        tasks.iter().filter(|task| task.state == state).count()
    };
    let transcript = setup
        .tasks_until(&conversation, |tasks| {
            tasks.len() == 3
                && count(tasks, TaskState::Running) == 2
                && count(tasks, TaskState::Accepted) == 1
        })
        .await;
    let waiting = transcript
        .tasks(0)
        .into_iter()
        .find(|task| task.state == TaskState::Accepted)
        .unwrap();
    let child = setup
        .harness
        .wait_until(&waiting.conversation_id, |child| {
            child.turns[&0]
                .run
                .as_ref()
                .is_some_and(|run| run.activity == Some(RunActivity::WaitingForSlot))
        })
        .await;
    assert_eq!(
        child.turns[&0].run.as_ref().unwrap().state,
        RunState::Queued
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    let still = setup.harness.snapshot(&conversation).await;
    assert_eq!(count(&still.tasks(0), TaskState::Accepted), 1);
    // The reply waits for its tasks.
    let parent_run = still.run(&run).unwrap();
    assert_eq!(
        parent_run.activity,
        Some(RunActivity::WaitingForTasks { count: 3 })
    );
    assert_eq!(setup.harness.runtime.active_run_count(), 1);

    setup.set_limit(3).await;
    setup
        .tasks_until(&conversation, |tasks| count(tasks, TaskState::Running) == 3)
        .await;
    setup.harness.runtime.stop_reply(&conversation).await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Stopped));
    assert_eq!(count(&transcript.tasks(0), TaskState::Canceled), 3);
}

#[tokio::test]
async fn background_tasks_report_through_wait_for_tasks() {
    let setup = Setup::new().await;
    setup.parent(tool_calls(&[
        (
            "call_1",
            "start_task",
            &start("A", "explore", "CHILD-A", true),
        ),
        (
            "call_2",
            "start_task",
            &start("B", "explore", "CHILD-B", true),
        ),
    ]));
    setup.parent(tool_calls(&[("call_3", "wait_for_tasks", &wait(&[]))]));
    setup.parent(tool_calls(&[("call_4", "wait_for_tasks", &wait(&[2, 9]))]));
    setup.parent(reply("Both reported."));
    setup.child("CHILD-A", reply("a"));
    setup.child("CHILD-A", reply("Report A."));
    setup.child("CHILD-B", reply("b"));
    setup.child("CHILD-B", reply("Report B."));

    let (conversation, run) = setup.start("OWNER: two in the background").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let parent = setup.harness.model.requests_for("OWNER:");
    assert_eq!(parent.len(), 4);
    assert_eq!(
        result(&parent[1], "call_1"),
        texts::task_started_in_background(1, "explore")
    );
    assert_eq!(
        result(&parent[1], "call_2"),
        texts::task_started_in_background(2, "explore")
    );
    assert_eq!(
        result(&parent[2], "call_3"),
        format!(
            "{}\n\n{}",
            texts::task_reported(1, "Report A."),
            texts::task_reported(2, "Report B.")
        )
    );
    // A report that was read is returned again; an unknown number says so.
    assert_eq!(
        result(&parent[3], "call_4"),
        format!(
            "{}\n\n{}",
            texts::task_reported(2, "Report B."),
            texts::no_such_task(9)
        )
    );
    // The model read every report, so no note asked for them.
    let notes: Vec<String> = parent
        .iter()
        .flat_map(user_texts)
        .filter(|text| text.starts_with("Task"))
        .collect();
    assert_eq!(notes, Vec::<String>::new());
    let waits: Vec<_> = transcript
        .task_calls(0)
        .into_iter()
        .filter(|call| call.tool == "wait_for_tasks")
        .collect();
    assert_eq!(waits.len(), 2);
    let numbers: Vec<(u32, TaskState)> = waits[0]
        .awaited
        .iter()
        .map(|task| (task.number, task.state))
        .collect();
    assert_eq!(
        numbers,
        [(1, TaskState::Reported), (2, TaskState::Reported)]
    );
}

#[tokio::test]
async fn a_reply_with_unread_reports_waits_and_asks_once_without_their_text() {
    let setup = Setup::new().await;
    setup.parent(tool_calls(&[(
        "call_1",
        "start_task",
        &start("Unread", "explore", "CHILD-U", true),
    )]));
    setup.parent(reply("Started it."));
    setup.parent(reply("Finished without reading."));
    setup.child("CHILD-U", reply("u"));
    setup.child("CHILD-U", reply("Report U, secret details."));

    let (conversation, run) = setup.start("OWNER: one in the background").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let parent = setup.harness.model.requests_for("OWNER:");
    assert_eq!(parent.len(), 3);
    let notes: Vec<String> = user_texts(&parent[2])
        .into_iter()
        .filter(|text| *text == texts::unread_reports(&[1]))
        .collect();
    assert_eq!(notes.len(), 1);
    for request in &parent {
        for text in user_texts(request) {
            assert!(!text.contains("Report U"), "{text}");
        }
    }
    let waited = transcript.events.iter().any(|event| {
        matches!(
            event,
            mannie_types::transcript::ConversationEvent::RunUpdated { run }
                if run.activity == Some(RunActivity::WaitingForTasks { count: 1 })
        )
    });
    assert!(waited);
    assert_eq!(transcript.tasks(0)[0].state, TaskState::Reported);
}

#[tokio::test]
async fn a_follow_up_continues_the_childs_conversation_with_its_agent() {
    let setup = Setup::new().await;
    setup.parent(tool_calls(&[(
        "call_1",
        "start_task",
        &start("Find it", "explore", "CHILD-F: find the config", false),
    )]));
    setup.parent(tool_calls(&[(
        "call_2",
        "follow_up_task",
        &follow_up(1, "Now count its lines.", false),
    )]));
    setup.parent(reply("Done."));
    setup.child("CHILD-F", reply("found"));
    setup.child("CHILD-F", reply("Report 1: config.toml."));
    setup.child("CHILD-F", reply("counted"));
    setup.child("CHILD-F", reply("Report 2: 12 lines."));

    let (conversation, run) = setup.start("OWNER: follow up").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let parent = setup.harness.model.requests_for("OWNER:");
    assert_eq!(
        result(&parent[2], "call_2"),
        texts::task_reported(1, "Report 2: 12 lines.")
    );
    let [first, second] = transcript.tasks(0).try_into().unwrap();
    assert_eq!(first.conversation_id, second.conversation_id);
    assert_eq!((second.number, second.follow_up), (1, true));
    assert_eq!(second.agent, "explore");
    // The child reads its earlier turn: the first task and its report.
    let child = setup.harness.model.requests_for("CHILD-F");
    assert_eq!(child.len(), 4);
    let texts_of_follow_up: Vec<String> = child[2]
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|content| match content {
            Content::Text(text) => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(texts_of_follow_up.contains(&"Report 1: config.toml.".to_owned()));
    assert_eq!(
        user_texts(&child[2]).last().map(String::as_str),
        Some("Now count its lines.")
    );
    let conversation_of_child = setup.harness.snapshot(&first.conversation_id).await;
    assert_eq!(conversation_of_child.turns.len(), 2);
}

#[tokio::test]
async fn a_follow_up_of_a_running_or_unknown_task_assigns_nothing() {
    let setup = Setup::new().await;
    setup.parent(tool_calls(&[(
        "call_1",
        "start_task",
        &start("Busy", "explore", "CHILD-S", true),
    )]));
    setup.parent(tool_calls(&[
        ("call_2", "follow_up_task", &follow_up(1, "More.", false)),
        ("call_3", "follow_up_task", &follow_up(5, "More.", false)),
    ]));
    setup.parent(reply("Waiting."));
    setup.child("CHILD-S", vec![Step::Hang]);

    let (conversation, run) = setup.start("OWNER: refusals").await;
    setup
        .harness
        .wait_until(&conversation, |transcript| {
            transcript.run(&run).is_some_and(|view| {
                view.activity == Some(RunActivity::WaitingForTasks { count: 1 })
            })
        })
        .await;

    let parent = setup.harness.model.requests_for("OWNER:");
    assert_eq!(result(&parent[2], "call_2"), texts::task_still_running(1));
    assert_eq!(result(&parent[2], "call_3"), texts::no_such_task(5));
    setup.harness.runtime.stop_reply(&conversation).await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;
    assert_eq!(transcript.tasks(0).len(), 1);
}

#[tokio::test]
async fn a_childs_approval_shows_in_its_parent_and_its_grant_covers_the_family() {
    let setup = Setup::new().await;
    setup.file("docs/.keep", "");
    setup.parent(tool_calls(&[(
        "call_1",
        "start_task",
        &start("Write a", "general", "CHILD-W: write docs/a.txt", false),
    )]));
    setup.parent(tool_calls(&[(
        "call_2",
        "start_task",
        &start("Write b", "general", "CHILD-X: write docs/b.txt", false),
    )]));
    setup.parent(reply("Both written."));
    setup.writer("CHILD-W", "w_1", "docs/a.txt");
    setup.writer("CHILD-X", "x_1", "docs/b.txt");

    let (conversation, run) = setup.start("OWNER: write two files").await;
    let transcript = setup
        .harness
        .wait_until(&conversation, |transcript| {
            let asks = transcript
                .tasks(0)
                .first()
                .is_some_and(|task| task.approval.is_some());
            let shows = transcript.run(&run).is_some_and(|view| {
                matches!(view.activity, Some(RunActivity::AwaitingApproval { .. }))
            });
            asks && shows
        })
        .await;

    let task = transcript.tasks(0)[0].clone();
    let approval = task.approval.clone().unwrap();
    assert_eq!(approval.call.tool, "write_file");
    assert_eq!(approval.call.status, ToolCallStatus::AwaitingApproval);
    assert!(approval.call.arguments.contains("docs/a.txt"));
    // The parent shows that it waits, with the task's part.
    let shown = transcript.run(&run).unwrap().activity.clone();
    let part_id = first_task_part(&transcript);
    assert_eq!(shown, Some(RunActivity::AwaitingApproval { part_id }));
    assert!(setup.marked_waiting(&conversation).await);
    // The decision names the child's conversation.
    let request = approval.call.request.clone().unwrap();
    let wrong_conversation = setup.harness.runtime.decide_approval(DecideApproval {
        conversation_id: conversation.clone(),
        part_id: approval.part_id.clone(),
        digest: request.digest.clone(),
        choice: ApprovalChoice::AllowOnce,
    });
    assert_eq!(wrong_conversation, Err(ToolsError::ApprovalNotPending));
    setup
        .harness
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: task.conversation_id.clone(),
            part_id: approval.part_id.clone(),
            digest: request.digest,
            choice: ApprovalChoice::AllowForConversation,
        })
        .unwrap();
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    assert!(setup.path("docs/a.txt").exists());
    assert!(setup.path("docs/b.txt").exists());
    let tasks = transcript.tasks(0);
    assert!(tasks.iter().all(|task| task.approval.is_none()));
    // The grant belongs to the parent, so the second child did not ask.
    let second = setup.harness.snapshot(&tasks[1].conversation_id).await;
    assert_eq!(second.tool_calls(0)[0].decision, Some(Decision::Grant));
    assert_eq!(setup.grants(&conversation).await, 1);
    assert_eq!(setup.grants(&task.conversation_id).await, 0);
    assert!(!setup.marked_waiting(&conversation).await);
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn stopping_the_parent_ends_every_child_within_a_second() {
    let setup = Setup::new().await;
    setup.set_limit(2).await;
    let command = if cfg!(windows) {
        "Start-Sleep -Seconds 30"
    } else {
        "sleep 30"
    };
    // Each start waits a moment in the parent's next model call, so the
    // children take their slots in order.
    setup.parent(tool_calls(&[(
        "call_1",
        "start_task",
        &start("Run it", "general", "CHILD-CMD", true),
    )]));
    let mut second = vec![Step::Delay(Duration::from_millis(300))];
    second.extend(tool_calls(&[(
        "call_2",
        "start_task",
        &start("Ask", "general", "CHILD-ASK", true),
    )]));
    setup.parent(second);
    let mut third = vec![Step::Delay(Duration::from_millis(300))];
    third.extend(tool_calls(&[(
        "call_3",
        "start_task",
        &start("Wait", "explore", "CHILD-SLOT", true),
    )]));
    setup.parent(third);
    setup.parent(reply("Working."));
    setup.child(
        "CHILD-CMD",
        tool_calls(&[(
            "c_1",
            "run_command",
            &json!({ "command": command }).to_string(),
        )]),
    );
    setup.child(
        "CHILD-ASK",
        tool_calls(&[("a_1", "write_file", &write("asked.txt"))]),
    );
    setup.child("CHILD-SLOT", vec![Step::Hang]);

    let (conversation, run) = setup.start("OWNER: three kinds of waiting").await;
    let transcript = setup
        .tasks_until(&conversation, |tasks| {
            tasks.first().is_some_and(|task| task.approval.is_some())
        })
        .await;
    let command_task = transcript.tasks(0)[0].clone();
    let approval = command_task.approval.unwrap();
    setup
        .harness
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: command_task.conversation_id.clone(),
            part_id: approval.part_id,
            digest: approval.call.request.unwrap().digest,
            choice: ApprovalChoice::AllowOnce,
        })
        .unwrap();
    setup
        .harness
        .wait_until(&command_task.conversation_id, |child| {
            child
                .tool_calls(0)
                .first()
                .is_some_and(|call| call.status == ToolCallStatus::Executing)
        })
        .await;
    let transcript = setup
        .tasks_until(&conversation, |tasks| {
            tasks.len() == 3 && tasks[1].approval.is_some() && tasks[2].state == TaskState::Accepted
        })
        .await;
    let asking = transcript.tasks(0)[1].clone();
    // The command's process starts before the stop.
    tokio::time::sleep(Duration::from_millis(1_500)).await;

    let stopping = Instant::now();
    let outcome = setup.harness.runtime.stop_reply(&conversation).await;
    let stopped_in = stopping.elapsed();
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(outcome, StopOutcome::Stopped);
    assert!(stopped_in < Duration::from_secs(1), "{stopped_in:?}");
    assert_eq!(transcript.run_state(&run), Some(RunState::Stopped));
    let states: Vec<TaskState> = transcript.tasks(0).iter().map(|task| task.state).collect();
    assert_eq!(states, [TaskState::Canceled; 3]);
    for task in transcript.tasks(0) {
        let child = setup.harness.snapshot(&task.conversation_id).await;
        assert_eq!(
            child.turns[&0].run.as_ref().unwrap().state,
            RunState::Stopped
        );
    }
    let ask = asking.approval.unwrap();
    let late = setup.harness.runtime.decide_approval(DecideApproval {
        conversation_id: asking.conversation_id,
        part_id: ask.part_id,
        digest: ask.call.request.unwrap().digest,
        choice: ApprovalChoice::AllowOnce,
    });
    assert_eq!(late, Err(ToolsError::ApprovalNotPending));
    assert!(!setup.path("asked.txt").exists());
    assert_eq!(setup.harness.runtime.active_run_count(), 0);
}

#[tokio::test]
async fn stopping_one_child_gives_its_parent_a_result_that_says_so() {
    let setup = Setup::new().await;
    setup.parent(tool_calls(&[(
        "call_1",
        "start_task",
        &start("Hang", "explore", "CHILD-H", false),
    )]));
    setup.parent(reply("It was stopped."));
    setup.child("CHILD-H", vec![Step::Hang]);

    let (conversation, run) = setup.start("OWNER: stop one").await;
    let transcript = setup
        .tasks_until(&conversation, |tasks| {
            tasks
                .first()
                .is_some_and(|task| task.state == TaskState::Running)
        })
        .await;
    let child = transcript.tasks(0)[0].conversation_id.clone();
    let outcome = setup.harness.runtime.stop_reply(&child).await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(outcome, StopOutcome::Stopped);
    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    assert_eq!(transcript.tasks(0)[0].state, TaskState::Canceled);
    let parent = setup.harness.model.requests_for("OWNER:");
    assert_eq!(result(&parent[1], "call_1"), texts::task_stopped(1));
}

#[tokio::test]
async fn quitting_interrupts_the_parent_and_its_children() {
    let setup = Setup::new().await;
    setup.parent(tool_calls(&[(
        "call_1",
        "start_task",
        &start("Hang", "explore", "CHILD-Q", false),
    )]));
    setup.child("CHILD-Q", vec![Step::Hang]);

    let (conversation, run) = setup.start("OWNER: quit").await;
    let transcript = setup
        .tasks_until(&conversation, |tasks| {
            tasks
                .first()
                .is_some_and(|task| task.state == TaskState::Running)
        })
        .await;
    let child = transcript.tasks(0)[0].conversation_id.clone();
    let harness = setup.harness.restart().await;

    let parent = harness.snapshot(&conversation).await;
    assert_eq!(parent.run_state(&run), Some(RunState::Interrupted));
    let task = &parent.tasks(0)[0];
    assert_eq!(task.state, TaskState::Interrupted);
    let child = harness.snapshot(&child).await;
    assert_eq!(
        child.turns[&0].run.as_ref().unwrap().state,
        RunState::Interrupted
    );
}

#[tokio::test]
async fn a_reply_starts_no_more_tasks_than_its_budget() {
    let mut agents = AgentRegistry::new();
    agents
        .register(AgentDefinition {
            max_tasks: 1,
            ..CONVERSATION
        })
        .unwrap();
    agents.register(EXPLORE).unwrap();
    agents.register(GENERAL).unwrap();
    let setup = Setup::with_options(HarnessOptions {
        use_tools: true,
        agents,
        ..HarnessOptions::default()
    })
    .await;
    setup.parent(tool_calls(&[
        (
            "call_1",
            "start_task",
            &start("One", "explore", "CHILD-1", false),
        ),
        (
            "call_2",
            "start_task",
            &start("Two", "explore", "CHILD-2", false),
        ),
    ]));
    setup.parent(reply("One was enough."));
    setup.child("CHILD-1", reply("one"));
    setup.child("CHILD-1", reply("Report 1."));

    let (conversation, run) = setup.start("OWNER: budget").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let parent = setup.harness.model.requests_for("OWNER:");
    assert_eq!(result(&parent[1], "call_2"), texts::TASK_BUDGET_USED);
    assert_eq!(transcript.tasks(0).len(), 1);
    assert_eq!(setup.harness.model.requests_for("CHILD-2"), Vec::new());
}

#[tokio::test]
async fn a_long_report_is_cut_and_archived_whole_in_the_parent() {
    let setup = Setup::new().await;
    let report = "A line of the long report.\n".repeat(5_000);
    setup.parent(tool_calls(&[(
        "call_1",
        "start_task",
        &start("Long", "explore", "CHILD-L", false),
    )]));
    setup.parent(reply("Read it."));
    setup.child("CHILD-L", reply("long"));
    setup.child("CHILD-L", reply(&report));

    let (conversation, run) = setup.start("OWNER: long report").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let parent = setup.harness.model.requests_for("OWNER:");
    let output = result(&parent[1], "call_1");
    assert!(output.len() <= 64 * 1024, "{}", output.len());
    let marker = "archived output ";
    let start_of_id = output.rfind(marker).unwrap() + marker.len();
    let id = &output[start_of_id..start_of_id + 36];
    let archive_id = mannie_types::ids::ArchiveId::parse(id).unwrap();
    assert!(output.ends_with(&texts::report_cut(id)));
    let archive = setup
        .harness
        .store
        .archive(&conversation, &archive_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(archive.content, report.trim());
    let task = &transcript.tasks(0)[0];
    assert!(task.report.as_ref().unwrap().len() <= MAX_REPORT_VIEW_BYTES);
}

#[tokio::test]
async fn the_owners_commands_refuse_a_task_conversation_and_deleting_the_parent_deletes_it() {
    let setup = Setup::new().await;
    setup.parent(tool_calls(&[(
        "call_1",
        "start_task",
        &start("Small", "explore", "CHILD-R", false),
    )]));
    setup.parent(reply("Done."));
    setup.child("CHILD-R", reply("small"));
    setup.child("CHILD-R", reply("Report R."));

    let (conversation, run) = setup.start("OWNER: refusals").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;
    let child = transcript.tasks(0)[0].conversation_id.clone();
    let runtime = &setup.harness.runtime;

    let sent = runtime
        .send_message(SendMessage {
            conversation_id: child.clone(),
            text: "Hello, child.".to_owned(),
        })
        .await;
    assert_eq!(sent.unwrap_err(), ChatError::TaskConversation);
    assert_eq!(
        runtime.retry_reply(&child).await.unwrap_err(),
        ChatError::TaskConversation
    );
    assert_eq!(
        runtime.compact_conversation(&child).await.unwrap_err(),
        ChatError::TaskConversation
    );
    let renamed = runtime
        .rename_conversation(RenameConversation {
            conversation_id: child.clone(),
            title: "Mine".to_owned(),
        })
        .await;
    assert_eq!(renamed.unwrap_err(), ChatError::TaskConversation);
    let profiled = runtime
        .set_conversation_profile(SetConversationProfile {
            conversation_id: child.clone(),
            profile_id: None,
        })
        .await;
    assert_eq!(profiled.unwrap_err(), ChatError::TaskConversation);
    assert_eq!(
        runtime.delete_conversation(&child).await.unwrap_err(),
        ChatError::TaskConversation
    );
    assert_eq!(runtime.stop_reply(&child).await, StopOutcome::NotRunning);

    let mut subscription = runtime
        .subscribe(SubscribeConversation {
            conversation_id: child.clone(),
            after_seq: None,
        })
        .await
        .unwrap();
    runtime.delete_conversation(&conversation).await.unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match subscription.next().await {
                Some(ConversationUpdate::Ended { reason }) => return Some(reason),
                Some(_update) => {}
                None => return None,
            }
        }
    })
    .await
    .unwrap();

    assert_eq!(ended, Some(SubscriptionEnd::Deleted));
    assert_eq!(
        setup
            .harness
            .store
            .conversation_summary(&child)
            .await
            .unwrap(),
        None
    );
}
