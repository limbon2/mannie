//! The tool loop of a run (docs/features/system-tools/tool-calls.md, the
//! loop; operations-and-approvals.md): calls run one at a time between model
//! calls, the policy decides or the owner does, and Stop, budgets and early
//! stops behave as the feature says.

#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};
#[cfg(any(windows, target_os = "linux"))]
use std::time::{Duration, Instant};

use mannie_provider::testing::Step;
use mannie_provider::{Content, ModelRequest, ProviderError, ProviderEvent, Role, ToolChoice};
use mannie_runtime::testing::{Harness, HarnessOptions, SeenPart, Transcript, reply, tool_calls};
use mannie_runtime::{AgentDefinition, AgentRegistry, CONVERSATION, texts};
use mannie_tools::ACTIVITY_ARGUMENT;
use mannie_types::chat::StartConversation;
use mannie_types::ids::{ConversationId, PartId, ProjectId, RunId};
use mannie_types::run::{FinishReason, RunFailure, RunState};
#[cfg(any(windows, target_os = "linux"))]
use mannie_types::tools::CommandEnding;
use mannie_types::tools::{
    Access, AccessMode, ApprovalChoice, ApprovalReason, ApprovalRequest, DecideApproval, Decision,
    GrantScope, NotRunReason, ToolCallStatus, ToolCallView, ToolOutcome, ToolsError,
};
use mannie_types::transcript::PartContent;
use serde_json::json;
use tempfile::TempDir;

/// A runtime whose profile uses tools, and a project whose folder is the
/// workspace of the conversations that start in it.
struct Setup {
    harness: Harness,
    workspace: TempDir,
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
            workspace,
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

    /// Starts a conversation in the project.
    async fn start(&self, text: &str) -> (ConversationId, RunId) {
        self.start_in(Some(&self.project), text).await
    }

    /// Starts a conversation in a project, or a general one.
    async fn start_in(&self, project: Option<&ProjectId>, text: &str) -> (ConversationId, RunId) {
        let started = self
            .harness
            .runtime
            .start_conversation(StartConversation {
                text: text.to_owned(),
                profile_id: None,
                project_id: project.cloned(),
            })
            .await
            .unwrap();
        (started.conversation_id, started.run_id)
    }

    /// Waits until a tool call waits for the owner, and returns its part and
    /// request.
    async fn awaiting(&self, conversation: &ConversationId) -> (PartId, ApprovalRequest) {
        let transcript = self
            .harness
            .wait_until(conversation, |transcript| {
                waiting_call(transcript).is_some()
            })
            .await;
        waiting_call(&transcript).unwrap()
    }

    fn decide(
        &self,
        conversation: &ConversationId,
        (part_id, request): &(PartId, ApprovalRequest),
        choice: ApprovalChoice,
    ) -> Result<(), ToolsError> {
        self.harness.runtime.decide_approval(DecideApproval {
            conversation_id: conversation.clone(),
            part_id: part_id.clone(),
            digest: request.digest.clone(),
            choice,
        })
    }
}

/// The tool call that waits for the owner, if one does.
fn waiting_call(transcript: &Transcript) -> Option<(PartId, ApprovalRequest)> {
    transcript.parts(0).into_iter().find_map(|part| match part {
        SeenPart {
            id,
            content: PartContent::ToolCall { call },
            ..
        } if call.status == ToolCallStatus::AwaitingApproval => {
            call.request.clone().map(|request| (id, request))
        }
        _ => None,
    })
}

fn arguments(path: &Path) -> String {
    json!({ "path": path }).to_string()
}

fn the_call(transcript: &Transcript, index: usize) -> ToolCallView {
    transcript.tool_calls(0)[index].clone()
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

#[tokio::test]
async fn a_read_inside_the_workspace_runs_without_asking() {
    let setup = Setup::new().await;
    setup.file("notes.txt", "hello\n");
    setup.harness.model.push_reply(tool_calls(&[(
        "call_1",
        "read_file",
        &arguments(Path::new("notes.txt")),
    )]));
    setup.harness.model.push_reply(reply("It says hello."));

    let (conversation, run) = setup.start("What does notes.txt say?").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let call = the_call(&transcript, 0);
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert_eq!(call.decision, Some(Decision::Workspace));
    assert!(matches!(
        call.result.as_ref().map(|result| &result.outcome),
        Some(ToolOutcome::Read { .. })
    ));
    let requests = setup.harness.model.reply_requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].instructions.contains(texts::TOOL_GUIDANCE));
    assert_ne!(requests[0].tools, Vec::new());
    let [(call_id, output)] = results(&requests[1]).try_into().unwrap();
    assert_eq!(call_id, "call_1");
    assert!(output.contains("     1\thello"), "{output}");
    assert_eq!(transcript.reply_text(0), "It says hello.");
}

#[tokio::test]
async fn a_change_inside_the_workspace_asks_and_runs_once_allowed() {
    let setup = Setup::new().await;
    let target = setup.path("new.txt");
    setup.harness.model.push_reply(tool_calls(&[(
        "call_1",
        "write_file",
        &json!({ "path": "new.txt", "content": "made\n" }).to_string(),
    )]));
    setup.harness.model.push_reply(reply("Written."));

    let (conversation, run) = setup.start("Write new.txt").await;
    let waiting = setup.awaiting(&conversation).await;

    assert_eq!(waiting.1.reasons, [ApprovalReason::ChangeInWorkspace]);
    assert!(!target.exists());
    setup
        .decide(&conversation, &waiting, ApprovalChoice::AllowOnce)
        .unwrap();
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(fs::read_to_string(&target).unwrap(), "made\n");
    let call = the_call(&transcript, 0);
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert_eq!(call.decision, Some(Decision::AllowedOnce));
    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
}

#[tokio::test]
async fn every_offered_tool_asks_the_model_for_an_activity() {
    let setup = Setup::new().await;
    setup.harness.model.push_reply(reply("Nothing to do."));

    let (conversation, run) = setup.start("Hello").await;
    setup.harness.wait_for_run(&conversation, &run).await;

    let request = &setup.harness.model.reply_requests()[0];
    assert_ne!(request.tools, Vec::new());
    for tool in &request.tools {
        let parameters = &tool.parameters;
        assert_eq!(
            parameters["properties"][ACTIVITY_ARGUMENT]["type"], "string",
            "{}",
            tool.name
        );
        let required = parameters["required"].as_array().unwrap();
        assert!(
            required.iter().any(|name| name == ACTIVITY_ARGUMENT),
            "{}",
            tool.name
        );
    }
}

#[tokio::test]
async fn a_calls_activity_reaches_its_document_and_never_its_tool() {
    let setup = Setup::new().await;
    setup.file("notes.txt", "hello\n");
    let written = json!({ "activity": " Reading\nthe notes ", "path": "notes.txt" }).to_string();
    setup
        .harness
        .model
        .push_reply(tool_calls(&[("call_1", "read_file", &written)]));
    setup.harness.model.push_reply(reply("It says hello."));

    let (conversation, run) = setup.start("What does notes.txt say?").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let call = the_call(&transcript, 0);
    // read_file refuses arguments it does not know, so it never saw the
    // activity.
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert!(
        matches!(
            call.result.as_ref().map(|result| &result.outcome),
            Some(ToolOutcome::Read { .. })
        ),
        "{call:?}"
    );
    assert_eq!(call.activity.as_deref(), Some("Reading the notes"));
    // The model reads its call back as it wrote it.
    assert_eq!(call.arguments, written);
    let sent_back: Vec<_> = setup.harness.model.reply_requests()[1]
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|content| match content {
            Content::ToolCall { arguments, .. } => Some(arguments.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(sent_back, [written]);
}

#[tokio::test]
async fn a_misleading_activity_changes_nothing_that_authorization_decides() {
    let setup = Setup::new().await;
    setup.file("notes.txt", "keep me\n");
    setup.harness.model.push_reply(tool_calls(&[(
        "call_1",
        "delete_path",
        &json!({ "activity": "Reading the notes", "path": "notes.txt" }).to_string(),
    )]));
    setup
        .harness
        .model
        .push_reply(reply("I did not delete it."));

    let (conversation, run) = setup.start("Read notes.txt").await;
    let transcript = setup
        .harness
        .wait_until(&conversation, |transcript| {
            waiting_call(transcript).is_some()
        })
        .await;

    // The request asks for the deletion that the core prepared.
    let call = the_call(&transcript, 0);
    assert_eq!(call.activity.as_deref(), Some("Reading the notes"));
    assert!(
        matches!(
            call.accesses.as_slice(),
            [Access::Path {
                mode: AccessMode::Delete,
                ..
            }]
        ),
        "{:?}",
        call.accesses
    );
    let waiting = waiting_call(&transcript).unwrap();
    assert_eq!(waiting.1.reasons, [ApprovalReason::ChangeInWorkspace]);
    setup
        .decide(&conversation, &waiting, ApprovalChoice::Deny)
        .unwrap();
    setup.harness.wait_for_run(&conversation, &run).await;
    assert_eq!(
        fs::read_to_string(setup.path("notes.txt")).unwrap(),
        "keep me\n"
    );
}

#[tokio::test]
async fn a_denial_reaches_the_model_and_skips_the_rest_of_the_step() {
    let setup = Setup::new().await;
    setup.harness.model.push_reply(tool_calls(&[
        (
            "call_1",
            "write_file",
            &json!({ "path": "a.txt", "content": "a" }).to_string(),
        ),
        (
            "call_2",
            "write_file",
            &json!({ "path": "b.txt", "content": "b" }).to_string(),
        ),
    ]));
    setup
        .harness
        .model
        .push_reply(reply("I could not write them."));

    let (conversation, run) = setup.start("Write two files").await;
    let waiting = setup.awaiting(&conversation).await;
    setup
        .decide(&conversation, &waiting, ApprovalChoice::Deny)
        .unwrap();
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(the_call(&transcript, 0).status, ToolCallStatus::Denied);
    assert_eq!(the_call(&transcript, 1).status, ToolCallStatus::NotRun);
    assert_eq!(
        the_call(&transcript, 1).not_run,
        Some(NotRunReason::EarlierCallDenied)
    );
    assert!(!setup.path("a.txt").exists() && !setup.path("b.txt").exists());
    let requests = setup.harness.model.reply_requests();
    assert_eq!(
        results(&requests[1]),
        [
            ("call_1".to_owned(), texts::DENIED.to_owned()),
            ("call_2".to_owned(), texts::SKIPPED_AFTER_DENIAL.to_owned()),
        ]
    );
}

#[tokio::test]
async fn allowing_for_the_conversation_grants_the_folder() {
    let setup = Setup::new().await;
    let write = |name: &str| json!({ "path": format!("docs/{name}"), "content": name }).to_string();
    setup.file("docs/keep.txt", "keep");
    setup
        .harness
        .model
        .push_reply(tool_calls(&[("call_1", "write_file", &write("a.txt"))]));
    setup
        .harness
        .model
        .push_reply(tool_calls(&[("call_2", "write_file", &write("b.txt"))]));
    setup.harness.model.push_reply(reply("Both written."));

    let (conversation, run) = setup.start("Write two files").await;
    let waiting = setup.awaiting(&conversation).await;
    assert_eq!(
        waiting.1.grants,
        [GrantScope::Path {
            mode: AccessMode::Create,
            folder: setup.path("docs").to_str().unwrap().to_owned(),
        }]
    );
    setup
        .decide(
            &conversation,
            &waiting,
            ApprovalChoice::AllowForConversation,
        )
        .unwrap();
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(
        the_call(&transcript, 0).decision,
        Some(Decision::AllowedForConversation)
    );
    assert_eq!(the_call(&transcript, 1).decision, Some(Decision::Grant));
    assert!(setup.path("docs/b.txt").exists());
    let grants = setup
        .harness
        .runtime
        .conversation_grants(&conversation)
        .await
        .unwrap();
    assert_eq!(grants.len(), 1);
}

#[tokio::test]
async fn a_decision_counts_only_for_the_request_it_names() {
    let setup = Setup::new().await;
    setup.harness.model.push_reply(tool_calls(&[(
        "call_1",
        "write_file",
        &json!({ "path": "a.txt", "content": "a" }).to_string(),
    )]));
    setup.harness.model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("Write a.txt").await;
    let waiting = setup.awaiting(&conversation).await;
    let (part_id, request) = waiting.clone();
    let other_part = PartId::parse("0199e0f2-0000-7000-8000-000000000999").unwrap();

    let changed = setup.harness.runtime.decide_approval(DecideApproval {
        conversation_id: conversation.clone(),
        part_id: part_id.clone(),
        digest: "0".repeat(64),
        choice: ApprovalChoice::AllowOnce,
    });
    let elsewhere = setup.decide(
        &conversation,
        &(other_part, request.clone()),
        ApprovalChoice::AllowOnce,
    );
    let no_rule = setup.decide(
        &conversation,
        &waiting,
        ApprovalChoice::SaveRule {
            prefix_words: 1,
            limit_to_folder: false,
        },
    );

    assert_eq!(changed, Err(ToolsError::ApprovalChanged));
    assert_eq!(elsewhere, Err(ToolsError::ApprovalNotPending));
    assert_eq!(no_rule, Err(ToolsError::RuleNotPossible));
    assert!(!setup.path("a.txt").exists());
    setup
        .decide(&conversation, &waiting, ApprovalChoice::AllowOnce)
        .unwrap();
    setup.harness.wait_for_run(&conversation, &run).await;
    assert_eq!(
        setup.decide(&conversation, &waiting, ApprovalChoice::AllowOnce),
        Err(ToolsError::ApprovalNotPending),
        "a decided request no longer waits"
    );
}

#[tokio::test]
async fn stop_during_an_approval_cancels_the_call() {
    let setup = Setup::new().await;
    setup.harness.model.push_reply(tool_calls(&[(
        "call_1",
        "delete_path",
        &json!({ "path": "doomed.txt" }).to_string(),
    )]));
    setup.file("doomed.txt", "x");

    let (conversation, run) = setup.start("Delete doomed.txt").await;
    let waiting = setup.awaiting(&conversation).await;
    setup.harness.runtime.stop_reply(&conversation).await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Stopped));
    assert_eq!(the_call(&transcript, 0).status, ToolCallStatus::Canceled);
    assert!(setup.path("doomed.txt").exists());
    assert_eq!(
        setup.decide(&conversation, &waiting, ApprovalChoice::AllowOnce),
        Err(ToolsError::ApprovalNotPending)
    );
    let summary = setup
        .harness
        .store
        .conversation_summary(&conversation)
        .await
        .unwrap()
        .unwrap();
    assert!(!summary.awaiting_approval);
}

#[tokio::test]
async fn a_run_that_waits_for_the_owner_holds_no_model_slot() {
    let setup = Setup::with_options(HarnessOptions {
        use_tools: true,
        config: mannie_runtime::RuntimeConfig {
            max_concurrent_calls: 1,
            ..mannie_runtime::testing::fast_config()
        },
        ..HarnessOptions::default()
    })
    .await;
    setup.harness.model.push_reply(tool_calls(&[(
        "call_1",
        "write_file",
        &json!({ "path": "a.txt", "content": "a" }).to_string(),
    )]));
    setup.harness.model.push_reply(reply("Meanwhile."));

    let (waiting_conversation, _) = setup.start("Write a.txt").await;
    let waiting = setup.awaiting(&waiting_conversation).await;
    let summary = setup
        .harness
        .store
        .conversation_summary(&waiting_conversation)
        .await
        .unwrap()
        .unwrap();
    let (other, other_run) = setup.start("Hello").await;
    let transcript = setup.harness.wait_for_run(&other, &other_run).await;

    assert!(summary.awaiting_approval);
    assert_eq!(transcript.run_state(&other_run), Some(RunState::Completed));
    assert_eq!(transcript.reply_text(0), "Meanwhile.");
    setup
        .decide(&waiting_conversation, &waiting, ApprovalChoice::Deny)
        .unwrap();
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn stop_ends_a_running_command_and_its_reply_within_a_second() {
    let setup = Setup::new().await;
    let command = if cfg!(windows) {
        "Start-Sleep -Seconds 30"
    } else {
        "sleep 30"
    };
    setup.harness.model.push_reply(tool_calls(&[(
        "call_1",
        "run_command",
        &json!({ "command": command }).to_string(),
    )]));

    let (conversation, run) = setup.start("Wait").await;
    let waiting = setup.awaiting(&conversation).await;
    setup
        .decide(&conversation, &waiting, ApprovalChoice::AllowOnce)
        .unwrap();
    // Stop once the command runs.
    setup
        .harness
        .wait_until(&conversation, |transcript| {
            transcript
                .tool_calls(0)
                .first()
                .is_some_and(|call| call.status == ToolCallStatus::Executing)
        })
        .await;
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    let stopping = Instant::now();
    setup.harness.runtime.stop_reply(&conversation).await;
    let stopped_in = stopping.elapsed();
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert!(stopped_in < Duration::from_secs(1), "{stopped_in:?}");
    assert_eq!(transcript.run_state(&run), Some(RunState::Stopped));
    let call = the_call(&transcript, 0);
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert!(matches!(
        call.result.map(|result| result.outcome),
        Some(ToolOutcome::CommandRan {
            ending: CommandEnding::Stopped,
            ..
        })
    ));
}

#[tokio::test]
async fn unknown_tools_and_invalid_arguments_do_not_run() {
    let setup = Setup::new().await;
    setup.harness.model.push_reply(tool_calls(&[
        ("call_1", "format_disk", "{}"),
        ("call_2", "read_file", r#"{"path": 7}"#),
    ]));
    setup.harness.model.push_reply(reply("Sorry."));

    let (conversation, run) = setup.start("Go").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(
        the_call(&transcript, 0).not_run,
        Some(NotRunReason::UnknownTool)
    );
    assert!(matches!(
        the_call(&transcript, 1).not_run,
        Some(NotRunReason::InvalidArguments { .. })
    ));
    let results = results(&setup.harness.model.reply_requests()[1]);
    assert_eq!(results[0].1, texts::unknown_tool("format_disk"));
    assert!(
        results[1].1.starts_with("The arguments are not valid: "),
        "{}",
        results[1].1
    );
}

#[tokio::test]
async fn a_reply_that_used_up_its_tools_answers_without_them() {
    let mut agents = AgentRegistry::new();
    agents
        .register(AgentDefinition {
            max_tool_rounds: 1,
            ..CONVERSATION
        })
        .unwrap();
    let setup = Setup::with_options(HarnessOptions {
        use_tools: true,
        agents,
        ..HarnessOptions::default()
    })
    .await;
    setup.file("a.txt", "a");
    let read = arguments(Path::new("a.txt"));
    setup
        .harness
        .model
        .push_reply(tool_calls(&[("call_1", "read_file", &read)]));
    // The model calls a tool although the request asked for none.
    setup
        .harness
        .model
        .push_reply(tool_calls(&[("call_2", "read_file", &read)]));

    let (conversation, run) = setup.start("Read a.txt").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let requests = setup.harness.model.reply_requests();
    assert_eq!(requests[1].tool_choice, ToolChoice::None);
    assert_eq!(
        requests[1].messages.last().map(|message| &message.content),
        Some(&vec![Content::Text(texts::TOOL_BUDGET_USED.to_owned())])
    );
    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    assert_eq!(
        the_call(&transcript, 1).not_run,
        Some(NotRunReason::OverBudget)
    );
}

#[tokio::test]
async fn a_profile_without_tools_sends_none_and_runs_none() {
    let setup = Setup::with_options(HarnessOptions::default()).await;
    // A server that sends a call anyway.
    setup
        .harness
        .model
        .push_reply(tool_calls(&[("call_1", "read_file", "{}")]));

    let (conversation, run) = setup.start("Hello").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let request = &setup.harness.model.reply_requests()[0];
    assert_eq!(request.tools, Vec::new());
    assert!(!request.instructions.contains(texts::TOOL_GUIDANCE));
    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    assert_eq!(
        the_call(&transcript, 0).not_run,
        Some(NotRunReason::ToolsOff)
    );
}

#[tokio::test]
async fn early_stops_after_tools_get_notes() {
    let setup = Setup::new().await;
    setup.file("a.txt", "a");
    setup.harness.model.push_reply(tool_calls(&[(
        "call_1",
        "read_file",
        &arguments(Path::new("a.txt")),
    )]));
    setup
        .harness
        .model
        .push_reply(reply("Next I will read b.txt:"));
    setup
        .harness
        .model
        .push_reply(reply("<tool_call>{\"name\": \"read_file\"}</tool_call>"));
    setup.harness.model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("Read the files").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let notes: Vec<String> = transcript
        .parts(0)
        .into_iter()
        .filter_map(|part| match part.content {
            PartContent::Text { text }
                if text == texts::CONTINUE_NOTE || text == texts::USE_TOOL_INTERFACE =>
            {
                Some(text)
            }
            _ => None,
        })
        .collect();
    let requests = setup.harness.model.reply_requests();
    assert_eq!(requests.len(), 4);
    let user_texts = |request: &ModelRequest| -> Vec<String> {
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
    };
    assert!(user_texts(&requests[2]).contains(&texts::CONTINUE_NOTE.to_owned()));
    assert!(user_texts(&requests[3]).contains(&texts::USE_TOOL_INTERFACE.to_owned()));
    assert!(notes.is_empty(), "notes are for the model: {notes:?}");
    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
}

#[tokio::test]
async fn a_retry_keeps_the_runs_that_made_tool_calls() {
    let setup = Setup::new().await;
    setup.file("a.txt", "a");
    setup.harness.model.push_reply(tool_calls(&[(
        "call_1",
        "read_file",
        &arguments(Path::new("a.txt")),
    )]));
    setup
        .harness
        .model
        .push_reply(vec![Step::Error(ProviderError::Permanent(
            RunFailure::ServerError { status: 500 },
        ))]);
    setup.harness.model.push_reply(reply("It says a."));

    let (conversation, run) = setup.start("Read a.txt").await;
    setup.harness.wait_for_run(&conversation, &run).await;
    let retried = setup
        .harness
        .runtime
        .retry_reply(&conversation)
        .await
        .unwrap();
    let transcript = setup
        .harness
        .wait_for_run(&conversation, &retried.run_id)
        .await;

    // The kept run's call stays in the transcript and in the model's context.
    assert_eq!(transcript.tool_calls(0).len(), 1);
    let request = setup.harness.model.reply_requests().pop().unwrap();
    assert_eq!(results(&request).len(), 1);
    assert!(
        request
            .messages
            .iter()
            .any(|message| message.content
                == vec![Content::Text(texts::INTERRUPTED_MARKER.to_owned())]),
        "{:?}",
        request.messages
    );
    assert_eq!(transcript.reply_text(0), "It says a.");
}

#[tokio::test]
async fn reads_inside_mannies_data_folder_always_ask() {
    let setup = Setup::new().await;
    let database = setup.harness.database();
    setup.harness.model.push_reply(tool_calls(&[(
        "call_1",
        "file_info",
        &arguments(&database),
    )]));
    setup.harness.model.push_reply(reply("Done."));

    // Even a project whose folder holds the data folder does not allow it.
    let data_project = setup
        .harness
        .runtime
        .add_project(setup.harness.data_folder().to_str().unwrap().to_owned())
        .await
        .unwrap();
    let (conversation, _) = setup
        .start_in(Some(&data_project.id), "Look at the database")
        .await;
    let waiting = setup.awaiting(&conversation).await;

    assert_eq!(waiting.1.reasons, [ApprovalReason::MannieFolder]);
    assert_eq!(waiting.1.grants, Vec::new());
    setup
        .decide(&conversation, &waiting, ApprovalChoice::Deny)
        .unwrap();
}

#[tokio::test]
async fn a_finish_reason_after_tool_calls_does_not_end_the_reply() {
    let setup = Setup::new().await;
    setup.file("a.txt", "a");
    let mut steps = tool_calls(&[("call_1", "read_file", &arguments(Path::new("a.txt")))]);
    // Some servers end a call with tool calls with "stop".
    steps.pop();
    steps.push(Step::Event(ProviderEvent::Finished(FinishReason::Stop)));
    setup.harness.model.push_reply(steps);
    setup.harness.model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("Read a.txt").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.reply_text(0), "Done.");
    assert_eq!(setup.harness.model.remaining_replies(), 0);
    drop(setup.workspace);
}

/// Links a folder: a symbolic link on Unix, a junction on Windows.
fn link_folder(link: &Path, target: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J failed");
    }
}

#[tokio::test]
async fn a_folder_swapped_for_a_link_while_the_owner_decides_is_not_written() {
    let setup = Setup::new().await;
    let outside = tempfile::tempdir().unwrap();
    setup.file("docs/keep.txt", "keep");
    setup.harness.model.push_reply(tool_calls(&[(
        "call_1",
        "write_file",
        &json!({ "path": "docs/new.txt", "content": "new" }).to_string(),
    )]));
    setup.harness.model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("Write docs/new.txt").await;
    let waiting = setup.awaiting(&conversation).await;
    // While the owner reads the request, the folder becomes a link.
    fs::rename(setup.path("docs"), setup.path("docs-old")).unwrap();
    link_folder(&setup.path("docs"), outside.path());
    setup
        .decide(&conversation, &waiting, ApprovalChoice::AllowOnce)
        .unwrap();
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let call = the_call(&transcript, 0);
    assert!(
        matches!(
            call.result.map(|result| result.outcome),
            Some(ToolOutcome::Failed {
                failure: mannie_types::tools::ToolFailure::PathThroughLink { .. }
            })
        ),
        "{:?}",
        the_call(&transcript, 0)
    );
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn a_file_replaced_while_the_owner_decides_asks_again() {
    let setup = Setup::new().await;
    setup.file("notes.txt", "old");
    let revision = format!(
        "sha256:{:x}",
        <sha2::Sha256 as sha2::Digest>::digest(b"old")
    );
    setup.harness.model.push_reply(tool_calls(&[(
        "call_1",
        "write_file",
        &json!({ "path": "notes.txt", "content": "new", "expected_revision": revision })
            .to_string(),
    )]));
    setup.harness.model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("Rewrite notes.txt").await;
    let first = setup.awaiting(&conversation).await;
    // Another program replaces the file with one of the same content.
    setup.file("other.txt", "old");
    fs::rename(setup.path("other.txt"), setup.path("notes.txt")).unwrap();
    setup
        .decide(&conversation, &first, ApprovalChoice::AllowOnce)
        .unwrap();
    // The approval covered the old file; the policy asks about the new one.
    let second = setup
        .harness
        .wait_until(&conversation, |transcript| {
            waiting_call(transcript).is_some_and(|(_, request)| request.digest != first.1.digest)
        })
        .await;
    let second = waiting_call(&second).unwrap();
    setup
        .decide(&conversation, &second, ApprovalChoice::Deny)
        .unwrap();
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(the_call(&transcript, 0).status, ToolCallStatus::Denied);
    assert_eq!(fs::read_to_string(setup.path("notes.txt")).unwrap(), "old");
}
