use std::time::Duration;

use mannie_types::agents::{AgentDraft, AgentKind};

use super::*;
use crate::writer::TransactError;

const A_MINUTE: Duration = Duration::from_secs(60);

fn draft(name: &str) -> AgentDraft {
    AgentDraft {
        name: name.to_owned(),
        description: String::new(),
        kind: AgentKind::Conversation,
        read_only: false,
        profile_id: None,
        instructions: "Do the work.".to_owned(),
        steps: Vec::new(),
        tools: vec!["read_file".to_owned()],
        plugins: None,
        children: None,
        max_tasks: 20,
        max_tool_rounds: 100,
        max_tool_calls: 250,
    }
}

fn set_agent(key: &str) -> Change {
    Change::SetAgent {
        agent_key: Some(key.to_owned()),
    }
}

fn save(title: &str, body: &str) -> Change {
    Change::SavePlan {
        title: title.to_owned(),
        body: body.to_owned(),
    }
}

#[tokio::test]
async fn a_new_conversation_keeps_the_agent_it_started_with() {
    let fixture = Fixture::new();
    let conversation = fixture.conversation_id();

    let applied = fixture
        .apply(
            &conversation,
            vec![Change::CreateConversation {
                conversation: NewConversation {
                    id: conversation.clone(),
                    title: "A title".to_owned(),
                    profile_id: None,
                    project_id: None,
                    task: None,
                    home: None,
                    agent_key: Some("custom.plan".to_owned()),
                },
            }],
        )
        .await;

    assert_eq!(
        applied.summary.unwrap().agent_key.as_deref(),
        Some("custom.plan")
    );
}

#[tokio::test]
async fn choosing_an_agent_changes_the_summary_and_can_go_back_to_the_default() {
    let fixture = Fixture::new();
    let (conversation, _, _, _) = fixture.start("Question").await;
    fixture.clock.advance(A_MINUTE);

    let chosen = fixture
        .apply(&conversation, vec![set_agent("custom.mine")])
        .await;

    let summary = chosen.summary.unwrap();
    assert_eq!(summary.agent_key.as_deref(), Some("custom.mine"));
    assert_eq!(
        summary.updated_at.as_millis(),
        NOON,
        "choosing an agent does not reorder the list"
    );
    let listed = fixture.store.list_conversations().await.unwrap();
    assert_eq!(listed[0].agent_key.as_deref(), Some("custom.mine"));

    let back = fixture
        .apply(&conversation, vec![Change::SetAgent { agent_key: None }])
        .await;

    assert_eq!(back.summary.unwrap().agent_key, None);
}

#[tokio::test]
async fn custom_agents_read_the_oldest_first_and_a_key_stays_taken() {
    let fixture = Fixture::new();
    let store = &fixture.store;
    store
        .create_custom_agent("custom.z", &draft("Older"))
        .await
        .unwrap();
    fixture.clock.advance(A_MINUTE);
    store
        .create_custom_agent("custom.a", &draft("Newer"))
        .await
        .unwrap();

    let taken = store.create_custom_agent("custom.a", &draft("Again")).await;

    assert!(matches!(taken, Err(RuntimeStoreError::AgentKeyTaken)));
    let agents = store.custom_agents().await.unwrap();
    let names: Vec<_> = agents
        .iter()
        .map(|agent| agent.draft.name.as_str())
        .collect();
    assert_eq!(names, ["Older", "Newer"]);
}

#[tokio::test]
async fn updating_a_custom_agent_replaces_its_document_under_the_same_key() {
    let fixture = Fixture::new();
    let store = &fixture.store;
    store
        .create_custom_agent("custom.mine", &draft("Mine"))
        .await
        .unwrap();
    fixture.clock.advance(A_MINUTE);

    store
        .update_custom_agent("custom.mine", &draft("Renamed"))
        .await
        .unwrap();

    let agents = store.custom_agents().await.unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].key, "custom.mine");
    assert_eq!(agents[0].draft.name, "Renamed");
    assert_eq!(agents[0].created_at.as_millis(), NOON);
    assert_eq!(
        agents[0].updated_at.as_millis(),
        NOON + 60_000,
        "the change is dated"
    );
    let missing = store
        .update_custom_agent("custom.other", &draft("Other"))
        .await;
    assert!(matches!(missing, Err(RuntimeStoreError::AgentNotFound)));
}

#[tokio::test]
async fn deleting_a_custom_agent_names_the_conversations_that_use_it() {
    let fixture = Fixture::new();
    let store = &fixture.store;
    store
        .create_custom_agent("custom.mine", &draft("Mine"))
        .await
        .unwrap();
    let (first, _, _, _) = fixture.start("One").await;
    let (second, _, _, _) = fixture.start("Two").await;
    let (other, _, _, _) = fixture.start("Three").await;
    fixture.apply(&first, vec![set_agent("custom.mine")]).await;
    fixture.apply(&second, vec![set_agent("custom.mine")]).await;
    fixture.apply(&other, vec![set_agent("custom.other")]).await;

    let named = store.delete_custom_agent("custom.mine").await.unwrap();

    assert_eq!(named, [first, second]);
    assert_eq!(store.custom_agents().await.unwrap(), Vec::new());
    let missing = store.delete_custom_agent("custom.mine").await;
    assert!(matches!(missing, Err(RuntimeStoreError::AgentNotFound)));
}

#[tokio::test]
async fn a_document_that_cannot_be_read_costs_only_its_own_agent() {
    let fixture = Fixture::new();
    let store = &fixture.store;
    store
        .create_custom_agent("custom.bad", &draft("Bad"))
        .await
        .unwrap();
    store
        .create_custom_agent("custom.good", &draft("Good"))
        .await
        .unwrap();
    store
        .transact::<_, RuntimeStoreError, _>(Durability::Full, |transaction| {
            transaction
                .execute(
                    "UPDATE runtime_agents SET document = 'not a document' WHERE key = 'custom.bad'",
                    [],
                )
                .map_err(TransactError::from)?;
            Ok(())
        })
        .await
        .unwrap();

    let agents = store.custom_agents().await.unwrap();

    let keys: Vec<_> = agents.iter().map(|agent| agent.key.as_str()).collect();
    assert_eq!(keys, ["custom.good"]);
}

#[tokio::test]
async fn saving_again_replaces_the_plan_of_a_conversation_under_its_number() {
    let fixture = Fixture::new();
    let project = fixture.project("/work").await;
    let conversation = fixture.conversation_in(&project, "A title").await;

    let applied = fixture
        .apply(&conversation, vec![save("First", "One.")])
        .await;

    assert!(applied.summary.is_some(), "the lists hear of the plan");
    let store = &fixture.store;
    let first = store
        .conversation_plan(&conversation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.title, "First");
    assert_eq!(first.body, "One.");
    assert_eq!(first.conversation_id.as_ref(), Some(&conversation));
    assert_eq!(first.project_id.as_ref(), Some(&project));
    assert!(!first.owner_edited);

    fixture.clock.advance(A_MINUTE);
    fixture
        .apply(&conversation, vec![save("Second", "Two.")])
        .await;

    let plans = store.plans().await.unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].number, first.number);
    assert_eq!(plans[0].title, "Second");
    assert_eq!(plans[0].body, "Two.");
    assert_eq!(plans[0].created_at, first.created_at);
    assert_eq!(plans[0].updated_at.as_millis(), NOON + 60_000);
}

#[tokio::test]
async fn a_plan_saves_with_the_changes_beside_it_or_not_at_all() {
    let fixture = Fixture::new();
    let (conversation, _, _, _) = fixture.start("Question").await;
    let missing = ProjectId::parse(&fixture.uuid()).unwrap();

    let refused = fixture
        .store
        .apply(
            &conversation,
            vec![
                save("Plan", "Text."),
                Change::SetProject {
                    project_id: missing,
                },
            ],
            Durability::Full,
        )
        .await;

    assert!(matches!(refused, Err(RuntimeStoreError::ProjectNotFound)));
    assert_eq!(fixture.store.plans().await.unwrap(), Vec::new());
}

#[tokio::test]
async fn a_plan_follows_its_conversation_into_a_project() {
    let fixture = Fixture::new();
    let (conversation, _, _, _) = fixture.start("Question").await;
    fixture
        .apply(&conversation, vec![save("Plan", "Text.")])
        .await;
    let store = &fixture.store;
    let before = store
        .conversation_plan(&conversation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.project_id, None);
    let project = fixture.project("/work").await;

    fixture
        .apply(
            &conversation,
            vec![Change::SetProject {
                project_id: project.clone(),
            }],
        )
        .await;

    let after = store.plan(before.number).await.unwrap().unwrap();
    assert_eq!(after.project_id, Some(project));
}

#[tokio::test]
async fn the_owner_s_edit_stays_marked_until_the_model_has_read_it_or_saved_again() {
    let fixture = Fixture::new();
    let (conversation, _, _, _) = fixture.start("Question").await;
    fixture
        .apply(&conversation, vec![save("Model's", "Model's text.")])
        .await;
    let store = &fixture.store;
    let number = store
        .conversation_plan(&conversation)
        .await
        .unwrap()
        .unwrap()
        .number;
    fixture.clock.advance(A_MINUTE);

    assert!(
        store
            .edit_plan(number, "Owner's", "Owner's text.")
            .await
            .unwrap()
    );

    let edited = store.plan(number).await.unwrap().unwrap();
    assert_eq!(edited.title, "Owner's");
    assert_eq!(edited.body, "Owner's text.");
    assert!(edited.owner_edited);
    assert_eq!(edited.updated_at.as_millis(), NOON + 60_000);

    fixture
        .apply(&conversation, vec![Change::PlanEditSeen])
        .await;

    let seen = store.plan(number).await.unwrap().unwrap();
    assert!(!seen.owner_edited);
    assert_eq!(seen.body, "Owner's text.", "reading it changes no text");

    store
        .edit_plan(number, "Owner's", "Edited again.")
        .await
        .unwrap();
    fixture
        .apply(&conversation, vec![save("Model's", "Saved over it.")])
        .await;

    let saved = store.plan(number).await.unwrap().unwrap();
    assert!(!saved.owner_edited);
    assert_eq!(saved.body, "Saved over it.");
    assert!(
        !store
            .edit_plan(number + 1, "Other", "Other text.")
            .await
            .unwrap(),
        "there is no such plan"
    );
}

#[tokio::test]
async fn a_plan_outlives_its_conversation_and_its_project() {
    let fixture = Fixture::new();
    let project = fixture.project("/work").await;
    let conversation = fixture.conversation_in(&project, "A title").await;
    fixture
        .apply(&conversation, vec![save("Kept", "Kept text.")])
        .await;
    let store = &fixture.store;
    let number = store
        .conversation_plan(&conversation)
        .await
        .unwrap()
        .unwrap()
        .number;

    assert!(store.delete_conversation(&conversation).await.unwrap());

    let without_conversation = store.plan(number).await.unwrap().unwrap();
    assert_eq!(without_conversation.conversation_id, None);
    assert_eq!(without_conversation.project_id.as_ref(), Some(&project));
    assert_eq!(without_conversation.body, "Kept text.");

    store.delete_project(&project).await.unwrap().unwrap();

    let without_project = store.plan(number).await.unwrap().unwrap();
    assert_eq!(without_project.project_id, None);
    assert_eq!(without_project.title, "Kept");
}

#[tokio::test]
async fn deleting_a_project_with_its_conversation_keeps_the_conversation_s_plan() {
    let fixture = Fixture::new();
    let project = fixture.project("/work").await;
    let conversation = fixture.conversation_in(&project, "A title").await;
    fixture
        .apply(&conversation, vec![save("Kept", "Kept text.")])
        .await;

    fixture
        .store
        .delete_project(&project)
        .await
        .unwrap()
        .unwrap();

    let plans = fixture.store.plans().await.unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].conversation_id, None);
    assert_eq!(plans[0].project_id, None);
    assert_eq!(plans[0].body, "Kept text.");
}

#[tokio::test]
async fn a_deleted_plan_s_number_never_returns() {
    let fixture = Fixture::new();
    let (conversation, _, _, _) = fixture.start("Question").await;
    fixture
        .apply(&conversation, vec![save("First", "One.")])
        .await;
    let store = &fixture.store;
    let first = store
        .conversation_plan(&conversation)
        .await
        .unwrap()
        .unwrap()
        .number;

    assert!(store.delete_plan(first).await.unwrap());
    assert!(!store.delete_plan(first).await.unwrap());
    assert_eq!(store.plan(first).await.unwrap(), None);

    fixture
        .apply(&conversation, vec![save("Second", "Two.")])
        .await;

    let second = store
        .conversation_plan(&conversation)
        .await
        .unwrap()
        .unwrap()
        .number;
    assert!(second > first, "{second} follows {first}");
}

#[tokio::test]
async fn plans_read_the_one_that_changed_last_first() {
    let fixture = Fixture::new();
    let (older, _, _, _) = fixture.start("One").await;
    let (newer, _, _, _) = fixture.start("Two").await;
    fixture.apply(&older, vec![save("Older", "One.")]).await;
    fixture.clock.advance(A_MINUTE);
    fixture.apply(&newer, vec![save("Newer", "Two.")]).await;
    let store = &fixture.store;

    let titles = |plans: Vec<super::super::StoredPlan>| -> Vec<String> {
        plans.into_iter().map(|plan| plan.title).collect()
    };
    assert_eq!(titles(store.plans().await.unwrap()), ["Newer", "Older"]);

    fixture.clock.advance(A_MINUTE);
    let number = store
        .conversation_plan(&older)
        .await
        .unwrap()
        .unwrap()
        .number;
    store.edit_plan(number, "Edited", "One.").await.unwrap();

    assert_eq!(titles(store.plans().await.unwrap()), ["Edited", "Newer"]);
}
