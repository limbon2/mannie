//! Draft conversations (ADR-0068): each place where a new conversation starts
//! has one, which the core keeps until its first message, and which the
//! first message turns into an ordinary conversation.

#![cfg(test)]

use std::fs;

use mannie_runtime::testing::{Harness, reply};
use mannie_types::chat::{ChatError, OpenDraft, RenameConversation, SendMessage};
use mannie_types::context::AttachmentRef;
use mannie_types::ids::{ConversationId, ProjectId};
use mannie_types::projects::RemoveProject;
use mannie_types::transcript::{ConversationSummary, TitleSource};

async fn summary(harness: &Harness, conversation: &ConversationId) -> Option<ConversationSummary> {
    harness
        .store
        .conversation_summary(conversation)
        .await
        .unwrap()
}

async fn project(harness: &Harness, folders: &tempfile::TempDir, name: &str) -> ProjectId {
    let folder = folders.path().join(name);
    fs::create_dir_all(&folder).unwrap();
    harness
        .runtime
        .add_project(folder.to_str().unwrap().to_owned())
        .await
        .unwrap()
        .id
}

async fn send(harness: &Harness, conversation: &ConversationId, text: &str) {
    harness.model.push_reply(reply("Done."));
    let started = harness
        .runtime
        .send_message(SendMessage::new(conversation.clone(), text))
        .await
        .unwrap();
    harness.wait_for_run(conversation, &started.run_id).await;
}

#[tokio::test]
async fn each_place_keeps_one_draft_without_turns_or_title() {
    let harness = Harness::start().await;
    let folders = tempfile::tempdir().unwrap();
    let project = project(&harness, &folders, "mannie").await;

    let general = harness
        .runtime
        .open_draft(OpenDraft::default())
        .await
        .unwrap();
    let again = harness
        .runtime
        .open_draft(OpenDraft::default())
        .await
        .unwrap();
    let in_project = harness
        .runtime
        .open_draft(OpenDraft::default().with_project(Some(project.clone())))
        .await
        .unwrap();

    assert_eq!(general, again, "the general conversations keep one draft");
    assert_ne!(general, in_project, "a project has a draft of its own");
    let draft = summary(&harness, &in_project).await.unwrap();
    assert!(draft.draft);
    assert_eq!(draft.title, "");
    assert_eq!(draft.project_id, Some(project));
    assert!(harness.snapshot(&general).await.content().is_empty());
}

#[tokio::test]
async fn opening_two_drafts_of_a_place_at_once_creates_one() {
    let harness = Harness::start().await;

    let (first, second) = tokio::join!(
        harness.runtime.open_draft(OpenDraft::default()),
        harness.runtime.open_draft(OpenDraft::default()),
    );

    assert_eq!(first.unwrap(), second.unwrap());
}

#[tokio::test]
async fn the_first_message_makes_the_draft_a_titled_conversation_and_the_place_gets_a_new_draft() {
    let harness = Harness::start().await;
    let draft = harness
        .runtime
        .open_draft(OpenDraft::default())
        .await
        .unwrap();

    send(&harness, &draft, "Plan the release\nwith details").await;

    let sent = summary(&harness, &draft).await.unwrap();
    assert!(!sent.draft);
    assert_eq!(sent.title, "Plan the release");
    assert_eq!(sent.title_source, TitleSource::FirstMessage);
    let next = harness
        .runtime
        .open_draft(OpenDraft::default())
        .await
        .unwrap();
    assert_ne!(
        next, draft,
        "the place gets a new draft after the first message"
    );
}

#[tokio::test]
async fn a_draft_that_the_owner_renamed_keeps_the_owners_title() {
    let harness = Harness::start().await;
    let draft = harness
        .runtime
        .open_draft(OpenDraft::default())
        .await
        .unwrap();
    harness
        .runtime
        .rename_conversation(RenameConversation {
            conversation_id: draft.clone(),
            title: "Release notes".to_owned(),
        })
        .await
        .unwrap();

    send(&harness, &draft, "Write them").await;

    let sent = summary(&harness, &draft).await.unwrap();
    assert!(!sent.draft);
    assert_eq!(sent.title, "Release notes");
}

#[tokio::test]
async fn a_draft_takes_the_agent_that_it_is_opened_with() {
    let harness = Harness::start().await;
    let draft = harness
        .runtime
        .open_draft(OpenDraft::default())
        .await
        .unwrap();

    let unknown = harness
        .runtime
        .open_draft(OpenDraft::default().with_agent(Some("no-such-agent".to_owned())))
        .await;
    let built_in = harness
        .runtime
        .open_draft(OpenDraft::default().with_agent(Some("conversation".to_owned())))
        .await
        .unwrap();

    assert_eq!(unknown, Err(ChatError::AgentNotFound));
    assert_eq!(built_in, draft);
    assert_eq!(summary(&harness, &draft).await.unwrap().agent_key, None);
}

#[tokio::test]
async fn a_draft_with_a_home_has_no_project() {
    let harness = Harness::start().await;
    let folders = tempfile::tempdir().unwrap();
    let project = project(&harness, &folders, "mannie").await;

    let refused = harness
        .runtime
        .open_draft(
            OpenDraft::default()
                .with_project(Some(project))
                .with_home(Some(AttachmentRef {
                    provider: "notebooks".to_owned(),
                    key: "n1".to_owned(),
                })),
        )
        .await;

    assert_eq!(refused, Err(ChatError::HomeConversation));
}

#[tokio::test]
async fn removing_a_project_counts_its_listed_conversations_and_deletes_its_draft() {
    let harness = Harness::start().await;
    let folders = tempfile::tempdir().unwrap();
    let project = project(&harness, &folders, "mannie").await;
    let draft = harness
        .runtime
        .open_draft(OpenDraft::default().with_project(Some(project.clone())))
        .await
        .unwrap();

    harness
        .runtime
        .remove_project(RemoveProject {
            project_id: project,
            conversations: 0,
        })
        .await
        .unwrap();

    assert_eq!(summary(&harness, &draft).await, None);
}
