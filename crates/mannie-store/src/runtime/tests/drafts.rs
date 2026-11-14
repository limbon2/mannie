use super::*;

/// A new conversation of a place, made a draft in the batch that creates it.
fn draft_changes(conversation: &ConversationId, project_id: Option<ProjectId>) -> Vec<Change> {
    vec![
        Change::CreateConversation {
            conversation: NewConversation {
                id: conversation.clone(),
                title: String::new(),
                profile_id: None,
                project_id,
                task: None,
                home: None,
                agent_key: None,
            },
        },
        Change::MarkDraft,
    ]
}

#[tokio::test]
async fn a_place_has_one_draft_which_the_store_finds() {
    let fixture = Fixture::new();
    let project = fixture.project("/work/mannie").await;
    let general = fixture.conversation_id();
    let second = fixture.conversation_id();
    let in_project = fixture.conversation_id();

    let created = fixture.apply(&general, draft_changes(&general, None)).await;
    let refused = fixture
        .store
        .apply(&second, draft_changes(&second, None), Durability::Full)
        .await;
    fixture
        .apply(
            &in_project,
            draft_changes(&in_project, Some(project.clone())),
        )
        .await;

    assert!(created.summary.unwrap().draft);
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(
        fixture.store.draft_conversation(None, None).await.unwrap(),
        Some(general)
    );
    assert_eq!(
        fixture
            .store
            .draft_conversation(Some(&project), None)
            .await
            .unwrap(),
        Some(in_project)
    );
}

#[tokio::test]
async fn only_a_new_conversation_becomes_a_draft() {
    let fixture = Fixture::new();
    let (conversation, _turn, _run, _applied) = fixture.start("Hello").await;

    let refused = fixture
        .store
        .apply(&conversation, vec![Change::MarkDraft], Durability::Full)
        .await;

    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(
        fixture.store.draft_conversation(None, None).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn undrafting_titles_the_conversation_unless_the_owner_renamed_it() {
    let fixture = Fixture::new();
    let first = fixture.conversation_id();
    fixture.apply(&first, draft_changes(&first, None)).await;

    let applied = fixture
        .apply(
            &first,
            vec![Change::Undraft {
                title: "From the message".to_owned(),
            }],
        )
        .await;
    let second = fixture.conversation_id();
    fixture.apply(&second, draft_changes(&second, None)).await;
    fixture
        .apply(
            &second,
            vec![Change::Rename {
                title: "The owner's".to_owned(),
            }],
        )
        .await;
    fixture
        .apply(
            &second,
            vec![Change::Undraft {
                title: "From the message".to_owned(),
            }],
        )
        .await;
    let again = fixture
        .apply(
            &first,
            vec![Change::Undraft {
                title: "Another".to_owned(),
            }],
        )
        .await;

    let summary = applied.summary.unwrap();
    assert!(!summary.draft);
    assert_eq!(summary.title, "From the message");
    let renamed = fixture
        .store
        .conversation_summary(&second)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(renamed.title, "The owner's");
    assert!(
        again.summary.is_none(),
        "a conversation that is not a draft stays as it is"
    );
    assert_eq!(
        fixture
            .store
            .conversation_summary(&first)
            .await
            .unwrap()
            .unwrap()
            .title,
        "From the message"
    );
}
