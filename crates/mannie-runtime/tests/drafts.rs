//! Draft conversations (ADR-0068): each place where a new conversation starts
//! has one, which the core keeps until its first message, and which the
//! first message turns into an ordinary conversation.

#![cfg(test)]

use std::fs;

use mannie_provider::testing::{FixtureResponse, FixtureServer};
use mannie_runtime::testing::{Harness, HarnessOptions, reply, sign_in_chatgpt};
use mannie_types::chat::{
    ChatError, OpenDraft, RenameConversation, SendMessage, SetConversationProfile,
    SetConversationReasoningEffort, StartConversation,
};
use mannie_types::context::AttachmentRef;
use mannie_types::ids::{ConversationId, ProjectId};
use mannie_types::projects::RemoveProject;
use mannie_types::provider::{ApiInterface, ReasoningEffort};
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
async fn choosing_default_in_a_draft_pins_the_catalog_model_available_at_first_send() {
    let harness = Harness::with_options(HarnessOptions {
        interface: ApiInterface::Responses,
        ..HarnessOptions::default()
    })
    .await;
    let server = FixtureServer::start().await.unwrap();
    harness
        .runtime
        .delete_profile(&harness.profile.id)
        .await
        .unwrap();
    sign_in_chatgpt(
        &harness.runtime,
        &server,
        "user-1",
        "access",
        "refresh",
        false,
    )
    .await;
    let catalog = serde_json::json!({"models": [
        {"slug": "first", "visibility": "list"},
        {"slug": "second", "visibility": "list"}
    ]});
    server.respond(FixtureResponse::json(200, catalog.to_string()));
    harness.runtime.refresh_chatgpt_catalog().await.unwrap();

    let draft = harness
        .runtime
        .open_draft(OpenDraft::default())
        .await
        .unwrap();
    let first = harness.store.default_profile().await.unwrap().unwrap();
    assert_eq!(first.model, "first");
    for profile_id in [Some(first.id), None] {
        harness
            .runtime
            .set_conversation_profile(SetConversationProfile {
                conversation_id: draft.clone(),
                profile_id,
            })
            .await
            .unwrap();
    }
    assert_eq!(summary(&harness, &draft).await.unwrap().profile_id, None);

    server.respond(FixtureResponse::json(
        200,
        r#"{"models":[{"slug":"second","visibility":"list"}]}"#,
    ));
    harness.runtime.refresh_chatgpt_catalog().await.unwrap();
    send(&harness, &draft, "Use the current default").await;

    let current = harness.store.default_profile().await.unwrap().unwrap();
    assert_eq!(harness.model.reply_requests()[0].model, "second");
    assert_eq!(
        summary(&harness, &draft).await.unwrap().profile_id,
        Some(current.id)
    );
}

#[tokio::test]
async fn new_chats_copy_the_last_explicit_effort_while_existing_drafts_keep_their_choice() {
    let harness = Harness::start().await;
    let folders = tempfile::tempdir().unwrap();
    let project = project(&harness, &folders, "effort").await;
    let project_draft = OpenDraft::default().with_project(Some(project));
    let saved = harness
        .runtime
        .open_draft(project_draft.clone())
        .await
        .unwrap();
    let general = harness
        .runtime
        .open_draft(OpenDraft::default())
        .await
        .unwrap();
    harness
        .runtime
        .set_conversation_reasoning_effort(SetConversationReasoningEffort {
            conversation_id: general.clone(),
            reasoning_effort: Some(ReasoningEffort::High),
        })
        .await
        .unwrap();

    assert_eq!(
        harness.runtime.open_draft(project_draft).await.unwrap(),
        saved
    );
    assert_eq!(
        summary(&harness, &saved).await.unwrap().reasoning_effort,
        None
    );
    send(&harness, &general, "Work carefully").await;

    let next = harness
        .runtime
        .open_draft(OpenDraft::default())
        .await
        .unwrap();
    assert_eq!(
        summary(&harness, &next).await.unwrap().reasoning_effort,
        Some(ReasoningEffort::High)
    );

    harness.model.push_reply(reply("Done."));

    let direct = harness
        .runtime
        .start_conversation(StartConversation::new("One more"))
        .await
        .unwrap();

    harness
        .wait_for_run(&direct.conversation_id, &direct.run_id)
        .await;
    assert!(
        harness
            .model
            .reply_requests()
            .iter()
            .all(|request| request.reasoning_effort == ReasoningEffort::High)
    );

    harness
        .runtime
        .set_conversation_reasoning_effort(SetConversationReasoningEffort {
            conversation_id: saved,
            reasoning_effort: None,
        })
        .await
        .unwrap();
    assert_eq!(
        summary(&harness, &general).await.unwrap().reasoning_effort,
        Some(ReasoningEffort::High)
    );
    harness.model.push_reply(reply("Done."));

    let reset = harness
        .runtime
        .start_conversation(StartConversation::new("Use the profile"))
        .await
        .unwrap();

    harness
        .wait_for_run(&reset.conversation_id, &reset.run_id)
        .await;
    assert_eq!(
        harness
            .model
            .reply_requests()
            .last()
            .unwrap()
            .reasoning_effort,
        ReasoningEffort::ServerDefault
    );
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
