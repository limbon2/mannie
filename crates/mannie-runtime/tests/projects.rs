//! Projects in the runtime (docs/features/chat/projects.md, ADR-0023): the
//! project commands and the list's channel, conversations that start in a
//! project, the move of a general conversation into one, and grants for a
//! project or for every conversation.

#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mannie_runtime::ListSubscription;
use mannie_runtime::testing::{Harness, HarnessOptions, SeenPart, Transcript, reply, tool_calls};
use mannie_types::chat::{ChatError, StartConversation};
use mannie_types::ids::{ConversationId, PartId, ProjectId, RunId};
use mannie_types::projects::{ProjectView, RemoveProject, RenameProject};
use mannie_types::run::RunState;
use mannie_types::tools::{
    Access, ApprovalChoice, ApprovalReason, ApprovalRequest, DecideApproval, Decision, PathRefusal,
    ToolCallStatus, ToolFailure, ToolOutcome, ToolsError,
};
use mannie_types::transcript::{ConversationListUpdate, PartContent};
use serde_json::json;
use tempfile::TempDir;

/// A runtime whose profile uses tools, and folders to add as projects.
struct Setup {
    harness: Harness,
    folders: TempDir,
}

impl Setup {
    async fn new() -> Self {
        let harness = Harness::with_options(HarnessOptions {
            use_tools: true,
            ..HarnessOptions::default()
        })
        .await;
        Self {
            harness,
            folders: tempfile::tempdir().unwrap(),
        }
    }

    /// A folder with a file `a.txt`, added as a project.
    async fn project(&self, name: &str) -> ProjectView {
        let folder = self.folders.path().join(name);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("a.txt"), format!("a in {name}")).unwrap();
        self.harness
            .runtime
            .add_project(folder.to_str().unwrap().to_owned())
            .await
            .unwrap()
    }

    /// A folder outside every project, with a file `b.txt`.
    fn outside(&self) -> PathBuf {
        let folder = self.folders.path().join("outside");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("b.txt"), "b").unwrap();
        folder.join("b.txt")
    }

    async fn start(&self, project: Option<&ProjectId>, text: &str) -> (ConversationId, RunId) {
        let started = self
            .harness
            .runtime
            .start_conversation(StartConversation {
                mentions: Vec::new(),
                images: Vec::new(),
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

    async fn decide(
        &self,
        conversation: &ConversationId,
        (part_id, request): &(PartId, ApprovalRequest),
        choice: ApprovalChoice,
    ) -> Result<(), ToolsError> {
        self.harness
            .runtime
            .decide_approval(DecideApproval {
                conversation_id: conversation.clone(),
                part_id: part_id.clone(),
                digest: request.digest.clone(),
                choice,
            })
            .await
    }

    async fn project_of(&self, conversation: &ConversationId) -> Option<ProjectId> {
        self.harness
            .store
            .conversation_summary(conversation)
            .await
            .unwrap()
            .unwrap()
            .project_id
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

fn move_to(project: &str) -> String {
    json!({ "project": project, "activity": "Moving to the project" }).to_string()
}

fn read(path: &Path) -> String {
    json!({ "path": path, "activity": "Reading a file" }).to_string()
}

/// The names of the tools a request offered.
fn offered(request: &mannie_provider::ModelRequest) -> Vec<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
}

/// The next update of the list that `wanted` accepts.
async fn next_matching(
    subscription: &mut ListSubscription,
    wanted: impl Fn(&ConversationListUpdate) -> bool,
) -> ConversationListUpdate {
    let wait = async {
        loop {
            let update = subscription.next().await.expect("the list goes on");
            if wanted(&update) {
                return update;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(20), wait)
        .await
        .expect("the update arrives within 20 seconds")
}

#[tokio::test]
async fn projects_reach_the_list_when_they_are_added_and_renamed() {
    let setup = Setup::new().await;
    let mut list = setup.harness.runtime.subscribe_list().await.unwrap();
    let ConversationListUpdate::Snapshot { projects, .. } = list.next().await.unwrap() else {
        panic!("the list starts with a snapshot");
    };
    assert_eq!(projects, Vec::new());

    let project = setup.project("mannie").await;
    let again = setup
        .harness
        .runtime
        .add_project(project.folder.clone())
        .await
        .unwrap();
    let added = next_matching(&mut list, |update| {
        matches!(update, ConversationListUpdate::ProjectUpserted { .. })
    })
    .await;
    let renamed = setup
        .harness
        .runtime
        .rename_project(RenameProject {
            project_id: project.id.clone(),
            name: "  Mannie app ".to_owned(),
        })
        .await
        .unwrap();
    let upserted = next_matching(&mut list, |update| {
        matches!(update, ConversationListUpdate::ProjectUpserted { .. })
    })
    .await;

    assert_eq!(project.name, "mannie");
    assert_eq!(again, project);
    assert_eq!(
        added,
        ConversationListUpdate::ProjectUpserted {
            project: project.clone()
        }
    );
    assert_eq!(renamed.name, "Mannie app");
    assert_eq!(
        upserted,
        ConversationListUpdate::ProjectUpserted { project: renamed }
    );
}

#[tokio::test]
async fn a_project_leaves_the_list_after_its_conversations_once_the_owner_confirmed_their_number() {
    let setup = Setup::new().await;
    let project = setup.project("mannie").await;
    let mut list = setup.harness.runtime.subscribe_list().await.unwrap();
    list.next().await.unwrap();

    setup.harness.model.push_reply(reply("Hello."));
    let (conversation, run) = setup.start(Some(&project.id), "Hi").await;
    setup.harness.wait_for_run(&conversation, &run).await;
    let changed = setup
        .harness
        .runtime
        .remove_project(RemoveProject {
            project_id: project.id.clone(),
            conversations: 0,
        })
        .await;
    assert_eq!(changed, Err(ChatError::ProjectChanged { conversations: 1 }));
    setup
        .harness
        .runtime
        .remove_project(RemoveProject {
            project_id: project.id.clone(),
            conversations: 1,
        })
        .await
        .unwrap();
    let removed = next_matching(&mut list, |update| {
        matches!(update, ConversationListUpdate::Removed { .. })
    })
    .await;
    let project_removed = next_matching(&mut list, |update| {
        matches!(update, ConversationListUpdate::ProjectRemoved { .. })
    })
    .await;

    assert_eq!(
        removed,
        ConversationListUpdate::Removed {
            id: conversation.clone()
        }
    );
    assert_eq!(
        project_removed,
        ConversationListUpdate::ProjectRemoved {
            id: project.id.clone()
        }
    );
    assert_eq!(
        setup
            .harness
            .store
            .conversation_summary(&conversation)
            .await
            .unwrap(),
        None
    );
    assert_eq!(setup.harness.store.projects().await.unwrap(), Vec::new());
    assert_eq!(
        setup
            .harness
            .runtime
            .rename_project(RenameProject {
                project_id: project.id,
                name: "Gone".to_owned(),
            })
            .await,
        Err(ChatError::ProjectNotFound)
    );
}

#[tokio::test]
async fn only_an_existing_folder_becomes_a_project_and_names_are_checked() {
    let setup = Setup::new().await;
    let file = setup.outside();
    let missing = setup.folders.path().join("missing");
    let add = |folder: String| setup.harness.runtime.add_project(folder);

    assert_eq!(
        add(missing.to_str().unwrap().to_owned()).await,
        Err(ChatError::FolderNotFound)
    );
    assert_eq!(
        add(file.to_str().unwrap().to_owned()).await,
        Err(ChatError::NotAFolder)
    );
    assert_eq!(
        add("relative/folder".to_owned()).await,
        Err(ChatError::PathRefused {
            reason: PathRefusal::NotAbsolute
        })
    );
    let project = setup.project("named").await;
    assert_eq!(
        setup
            .harness
            .runtime
            .rename_project(RenameProject {
                project_id: project.id,
                name: " \n ".to_owned(),
            })
            .await,
        Err(ChatError::ProjectNameInvalid {
            max_characters: 100
        })
    );
    let unknown = setup
        .harness
        .runtime
        .start_conversation(StartConversation {
            mentions: Vec::new(),
            images: Vec::new(),
            text: "Hi".to_owned(),
            profile_id: None,
            project_id: Some(ProjectId::parse("0199e0f2-0000-7000-a000-000000000099").unwrap()),
        })
        .await;
    assert!(
        matches!(unknown, Err(ChatError::ProjectNotFound)),
        "{unknown:?}"
    );
}

#[tokio::test]
async fn a_general_conversation_moves_into_the_project_its_message_names() {
    let setup = Setup::new().await;
    let project = setup.project("mannie").await;
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        "move_to_project",
        &move_to("mannie"),
    )]));
    model.push_reply(tool_calls(&[(
        "call_2",
        "read_file",
        &read(Path::new("a.txt")),
    )]));
    model.push_reply(reply("It says a in mannie."));

    let (conversation, run) = setup.start(None, "What does a.txt in Mannie say?").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let calls = transcript.tool_calls(0);
    assert_eq!(calls[0].decision, Some(Decision::OwnerNamedProject));
    assert_eq!(
        calls[0].accesses,
        vec![Access::Project {
            id: project.id.clone(),
            name: "mannie".to_owned(),
            folder: project.folder.clone(),
        }]
    );
    assert_eq!(
        calls[0].result.as_ref().map(|result| &result.outcome),
        Some(&ToolOutcome::MovedToProject {
            name: "mannie".to_owned(),
            folder: project.folder.clone(),
        })
    );
    assert_eq!(calls[1].decision, Some(Decision::Workspace));
    assert_eq!(setup.project_of(&conversation).await, Some(project.id));
    let requests = model.reply_requests();
    assert!(offered(&requests[0]).contains(&"move_to_project".to_owned()));
    assert!(
        requests[0]
            .instructions
            .contains(&format!("  - mannie: {}", project.folder)),
        "{}",
        requests[0].instructions
    );
    assert!(!offered(&requests[1]).contains(&"move_to_project".to_owned()));
    assert!(
        requests[1].instructions.contains(&format!(
            "- Workspace: {}, the folder of the project mannie",
            project.folder
        )),
        "{}",
        requests[1].instructions
    );
}

#[tokio::test]
async fn a_move_that_the_message_does_not_name_asks_the_owner_first() {
    let setup = Setup::new().await;
    let project = setup.project("mannie").await;
    setup.harness.model.push_reply(tool_calls(&[(
        "call_1",
        "move_to_project",
        &move_to(&project.folder),
    )]));
    setup.harness.model.push_reply(reply("Moved."));

    let (conversation, run) = setup.start(None, "Read the notes").await;
    let waiting = setup.awaiting(&conversation).await;

    assert_eq!(waiting.1.reasons, [ApprovalReason::MoveToProject]);
    assert_eq!(waiting.1.grants, Vec::new());
    assert_eq!(waiting.1.project, None);
    assert_eq!(
        setup
            .decide(
                &conversation,
                &waiting,
                ApprovalChoice::AllowForConversation
            )
            .await,
        Err(ToolsError::GrantNotPossible)
    );
    assert_eq!(setup.project_of(&conversation).await, None);
    setup
        .decide(&conversation, &waiting, ApprovalChoice::AllowOnce)
        .await
        .unwrap();
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(
        transcript.tool_calls(0)[0].decision,
        Some(Decision::AllowedOnce)
    );
    assert_eq!(setup.project_of(&conversation).await, Some(project.id));
}

/// A tool result can carry an attacker's text that asks for a move. Once
/// another call came first, a move asks the owner, even when the owner's
/// message named the project (ADR-0023, decision 3).
#[tokio::test]
async fn a_move_after_another_tool_call_asks_even_when_the_message_names_the_project() {
    let setup = Setup::new().await;
    let project = setup.project("secrets").await;
    setup.harness.model.push_reply(tool_calls(&[
        (
            "call_1",
            "system_info",
            &json!({ "activity": "Checking the system" }).to_string(),
        ),
        ("call_2", "move_to_project", &move_to("secrets")),
    ]));
    setup.harness.model.push_reply(reply("I did not move."));

    let (conversation, run) = setup
        .start(None, "Tell me about this computer, not about secrets")
        .await;
    let waiting = setup.awaiting(&conversation).await;
    assert_eq!(waiting.1.reasons, [ApprovalReason::MoveToProject]);
    setup
        .decide(&conversation, &waiting, ApprovalChoice::Deny)
        .await
        .unwrap();
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.tool_calls(0)[1].status, ToolCallStatus::Denied);
    assert_eq!(setup.project_of(&conversation).await, None);
    assert_ne!(setup.project_of(&conversation).await, Some(project.id));
}

#[tokio::test]
async fn a_conversation_moves_once_and_only_a_general_one_is_offered_the_move() {
    let setup = Setup::new().await;
    let first = setup.project("first").await;
    setup.project("second").await;
    setup.harness.model.push_reply(tool_calls(&[
        ("call_1", "move_to_project", &move_to("first")),
        ("call_2", "move_to_project", &move_to("second")),
    ]));
    setup.harness.model.push_reply(reply("Done."));
    setup.harness.model.push_reply(reply("Hello."));

    let (general, run) = setup.start(None, "Work in first, then in second").await;
    let transcript = setup.harness.wait_for_run(&general, &run).await;
    let (in_project, other_run) = setup.start(Some(&first.id), "Hi").await;
    setup.harness.wait_for_run(&in_project, &other_run).await;

    let calls = transcript.tool_calls(0);
    assert_eq!(calls[0].decision, Some(Decision::OwnerNamedProject));
    assert_eq!(
        calls[1].result.as_ref().map(|result| &result.outcome),
        Some(&ToolOutcome::Failed {
            failure: ToolFailure::AlreadyInProject {
                name: "first".to_owned()
            }
        })
    );
    assert_eq!(setup.project_of(&general).await, Some(first.id));
    let requests = setup.harness.model.reply_requests();
    assert!(!offered(requests.last().unwrap()).contains(&"move_to_project".to_owned()));
}

#[tokio::test]
async fn without_projects_a_general_conversation_is_not_offered_the_move() {
    let setup = Setup::new().await;
    setup.harness.model.push_reply(reply("Hello."));

    let (conversation, run) = setup.start(None, "Hi").await;
    setup.harness.wait_for_run(&conversation, &run).await;

    let request = &setup.harness.model.reply_requests()[0];
    assert!(!offered(request).contains(&"move_to_project".to_owned()));
    assert!(offered(request).contains(&"read_file".to_owned()));
    assert!(
        request
            .instructions
            .contains("- Workspace: none, because this conversation belongs to no project"),
        "{}",
        request.instructions
    );
}

#[tokio::test]
async fn grants_for_a_project_and_for_every_conversation_reach_other_conversations() {
    let setup = Setup::new().await;
    let project = setup.project("mannie").await;
    let outside = setup.outside();
    let model = &setup.harness.model;
    for index in 0..4 {
        model.push_reply(tool_calls(&[(
            &format!("call_{index}"),
            "read_file",
            &read(&outside),
        )]));
        model.push_reply(reply("Read it."));
    }

    // A read outside the project asks, and the owner allows it for the
    // project.
    let (first, first_run) = setup.start(Some(&project.id), "Read b.txt").await;
    let waiting = setup.awaiting(&first).await;
    assert_eq!(waiting.1.project.as_ref(), Some(&project.id));
    setup
        .decide(&first, &waiting, ApprovalChoice::AllowForProject)
        .await
        .unwrap();
    setup.harness.wait_for_run(&first, &first_run).await;

    // Another conversation of the project reads without asking.
    let (second, second_run) = setup.start(Some(&project.id), "Read b.txt").await;
    let second_transcript = setup.harness.wait_for_run(&second, &second_run).await;
    assert_eq!(
        second_transcript.tool_calls(0)[0].decision,
        Some(Decision::ProjectGrant)
    );

    // A general conversation asks; "For this project" is not possible there,
    // and the owner allows it always.
    let (general, general_run) = setup.start(None, "Read b.txt").await;
    let waiting = setup.awaiting(&general).await;
    assert_eq!(waiting.1.project, None);
    assert_eq!(
        setup
            .decide(&general, &waiting, ApprovalChoice::AllowForProject)
            .await,
        Err(ToolsError::NotInAProject)
    );
    setup
        .decide(&general, &waiting, ApprovalChoice::AllowAlways)
        .await
        .unwrap();
    setup.harness.wait_for_run(&general, &general_run).await;

    // Every other conversation reads without asking.
    let (another, another_run) = setup.start(None, "Read b.txt").await;
    let another_transcript = setup.harness.wait_for_run(&another, &another_run).await;
    assert_eq!(
        another_transcript.tool_calls(0)[0].decision,
        Some(Decision::AlwaysGrant)
    );
    let grants = setup.harness.runtime.shared_grants().await.unwrap();
    assert_eq!(grants.len(), 2);
    assert_eq!(grants[0].project_id.as_ref(), Some(&project.id));
    assert_eq!(grants[1].project_id, None);
    assert_eq!(
        setup
            .harness
            .runtime
            .conversation_grants(&first)
            .await
            .unwrap(),
        Vec::new()
    );
}

#[tokio::test]
async fn removing_a_project_stops_a_reply_that_waits_for_the_owner() {
    let setup = Setup::new().await;
    let project = setup.project("mannie").await;
    let outside = setup.outside();
    setup
        .harness
        .model
        .push_reply(tool_calls(&[("call_1", "read_file", &read(&outside))]));

    let (conversation, _run) = setup.start(Some(&project.id), "Read b.txt").await;
    setup.awaiting(&conversation).await;
    let busy_removal = setup
        .harness
        .runtime
        .remove_project(RemoveProject {
            project_id: project.id.clone(),
            conversations: 1,
        })
        .await;

    assert_eq!(busy_removal, Ok(()));
    assert_eq!(
        setup
            .harness
            .store
            .conversation_summary(&conversation)
            .await
            .unwrap(),
        None
    );
    assert_eq!(setup.harness.runtime.active_run_count(), 0);
}
