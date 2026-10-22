//! Full access on the master (ADR-0029; docs/features/system-tools/
//! operations-and-approvals.md, full access): a conversation in full access
//! runs its operations without asking, Mannie's own folders included, and its
//! model learns that nobody checks them; a new conversation takes the Tools
//! settings' value; the switch applies from the next call; a child agent
//! follows its parent's switch and has none of its own.

#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use mannie_provider::testing::Step;
use mannie_runtime::testing::{Harness, HarnessOptions, Transcript, reply, tool_calls};
use mannie_runtime::texts;
use mannie_types::chat::{SendMessage, StartConversation};
use mannie_types::ids::{ConversationId, ProjectId, RunId};
use mannie_types::run::RunState;
use mannie_types::tasks::TaskState;
use mannie_types::tools::{
    ApprovalChoice, DecideApproval, Decision, SetFullAccess, ToolCallStatus, ToolsError,
    UpdateToolsSettings,
};
use mannie_types::transcript::PartContent;
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
        let harness = Harness::with_options(HarnessOptions {
            use_tools: true,
            ..HarnessOptions::default()
        })
        .await;
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

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Sets whether new conversations start in full access.
    async fn full_access_for_new_conversations(&self, enabled: bool) {
        self.harness
            .runtime
            .update_tools_settings(UpdateToolsSettings {
                allow_changes_in_workspace: false,
                command_environment: Vec::new(),
                full_access_for_new_conversations: enabled,
                wake_on_command_end: true,
            })
            .await
            .unwrap();
    }

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

    async fn send(&self, conversation: &ConversationId, text: &str) -> RunId {
        self.harness
            .runtime
            .send_message(SendMessage::new(conversation.clone(), text))
            .await
            .unwrap()
            .run_id
    }

    async fn switch(&self, conversation: &ConversationId, enabled: bool) -> Result<(), ToolsError> {
        self.harness
            .runtime
            .set_full_access(SetFullAccess {
                conversation_id: conversation.clone(),
                enabled,
            })
            .await
    }

    async fn has_full_access(&self, conversation: &ConversationId) -> bool {
        self.harness
            .store
            .conversation_summary(conversation)
            .await
            .unwrap()
            .unwrap()
            .full_access
    }
}

fn write(path: &Path, content: &str) -> String {
    json!({ "activity": "Writing", "path": path, "content": content }).to_string()
}

/// The decisions of the tool calls of the turn at `position`.
fn decisions(transcript: &Transcript, position: u32) -> Vec<Option<Decision>> {
    transcript
        .tool_calls(position)
        .into_iter()
        .map(|call| call.decision)
        .collect()
}

#[tokio::test]
async fn a_conversation_in_full_access_runs_everything_without_asking() {
    let setup = Setup::new().await;
    setup.full_access_for_new_conversations(true).await;
    let target = setup.path("new.txt");
    let database = setup.harness.database();
    setup.harness.model.push_reply(tool_calls(&[
        ("call_1", "write_file", &write(&target, "made\n")),
        (
            "call_2",
            "file_info",
            &json!({ "activity": "Looking", "path": database }).to_string(),
        ),
    ]));
    setup.harness.model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("Write new.txt and look at the database").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    assert!(setup.has_full_access(&conversation).await);
    assert_eq!(fs::read_to_string(&target).unwrap(), "made\n");
    assert_eq!(
        decisions(&transcript, 0),
        [Some(Decision::FullAccess), Some(Decision::FullAccess)]
    );
    let requests = setup.harness.model.reply_requests();
    assert!(
        requests[0]
            .instructions
            .contains(texts::FULL_ACCESS_TOOL_GUIDANCE),
        "the model learns that nobody checks its operations"
    );
}

#[tokio::test]
async fn the_switch_applies_from_the_next_call() {
    let setup = Setup::new().await;
    let first = setup.path("first.txt");
    let second = setup.path("second.txt");
    setup.harness.model.push_reply(reply("Hello."));
    setup.harness.model.push_reply(tool_calls(&[(
        "call_1",
        "write_file",
        &write(&first, "1\n"),
    )]));
    setup.harness.model.push_reply(reply("Written."));
    setup.harness.model.push_reply(tool_calls(&[(
        "call_2",
        "write_file",
        &write(&second, "2\n"),
    )]));
    setup.harness.model.push_reply(reply("Not written."));

    let (conversation, run) = setup.start("Hello").await;
    setup.harness.wait_for_run(&conversation, &run).await;
    assert!(!setup.has_full_access(&conversation).await);

    setup.switch(&conversation, true).await.unwrap();
    let run = setup.send(&conversation, "Write first.txt").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;
    assert_eq!(decisions(&transcript, 1), [Some(Decision::FullAccess)]);
    assert_eq!(fs::read_to_string(&first).unwrap(), "1\n");

    setup.switch(&conversation, false).await.unwrap();
    let run = setup.send(&conversation, "Write second.txt").await;
    let waiting = setup
        .harness
        .wait_until(&conversation, |transcript| {
            transcript
                .tool_calls(2)
                .first()
                .is_some_and(|call| call.status == ToolCallStatus::AwaitingApproval)
        })
        .await;
    let call = waiting.tool_calls(2)[0].clone();
    let part_id = waiting
        .parts(2)
        .into_iter()
        .find_map(|part| match part.content {
            PartContent::ToolCall { .. } => Some(part.id),
            _ => None,
        })
        .unwrap();
    setup
        .harness
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conversation.clone(),
            part_id,
            digest: call.request.unwrap().digest,
            choice: ApprovalChoice::Deny,
        })
        .await
        .unwrap();
    setup.harness.wait_for_run(&conversation, &run).await;

    assert!(!second.exists());
}

#[tokio::test]
async fn a_child_agent_follows_its_parents_full_access() {
    let setup = Setup::new().await;
    setup.full_access_for_new_conversations(true).await;
    let target = setup.path("child.txt");
    let start = json!({
        "activity": "Delegating",
        "title": "Write a file",
        "agent": "general",
        "instruction": "CHILD-W: write child.txt",
        "background": false,
    })
    .to_string();
    setup
        .harness
        .model
        .push_reply(tool_calls(&[("call_1", "start_task", &start)]));
    setup.harness.model.push_reply(reply("The child wrote it."));
    let child_steps: [Vec<Step>; 3] = [
        tool_calls(&[("w_1", "write_file", &write(&target, "child\n"))]),
        reply("written"),
        reply("Report: written."),
    ];
    for steps in child_steps {
        setup.harness.model.push_reply_for("CHILD-W", steps);
    }

    let (conversation, run) = setup.start("Let a child write child.txt").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let task = transcript.tasks(0)[0].clone();
    assert_eq!(task.state, TaskState::Reported);
    assert_eq!(fs::read_to_string(&target).unwrap(), "child\n");
    let child = setup
        .harness
        .wait_until(&task.conversation_id, |transcript| {
            !transcript.tool_calls(0).is_empty()
        })
        .await;
    assert_eq!(decisions(&child, 0), [Some(Decision::FullAccess)]);
    assert_eq!(
        setup.switch(&task.conversation_id, false).await,
        Err(ToolsError::FullAccessFollowsParent)
    );
}
