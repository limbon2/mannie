use std::sync::Arc;

use mannie_types::clock::{Clock, ManualClock, UtcMillis};
use mannie_types::ids::{ConversationId, GrantId, ProjectId, RuleId};
use mannie_types::tools::{AccessMode, GrantScope};

use super::ToolsStoreError;
use crate::{Change, Durability, NewConversation, Store, StoreOptions};

/// 2026-10-14T12:00:00Z
const NOON: i64 = 1_791_979_200_000;

struct Fixture {
    _directory: tempfile::TempDir,
    store: Store,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let clock: Arc<dyn Clock> = Arc::new(ManualClock::new(UtcMillis::from_millis(NOON)));
        let options = StoreOptions::new(directory.path().join("backups"), clock);
        let store = Store::open(&directory.path().join("mannie.sqlite"), &options).unwrap();
        Self {
            _directory: directory,
            store,
        }
    }

    async fn conversation(&self, number: u32) -> ConversationId {
        let id = ConversationId::parse(&format!("0199e0f2-7a1b-7c3d-8e4f-{number:012}")).unwrap();
        self.store
            .apply(
                &id,
                vec![Change::CreateConversation {
                    conversation: NewConversation {
                        id: id.clone(),
                        title: "Tools".to_owned(),
                        profile_id: None,
                        project_id: None,
                    },
                }],
                Durability::Full,
            )
            .await
            .unwrap();
        id
    }
}

fn grant_id(number: u32) -> GrantId {
    GrantId::parse(&format!("0199e0f2-0000-7000-8000-{number:012}")).unwrap()
}

fn rule_id(number: u32) -> RuleId {
    RuleId::parse(&format!("0199e0f2-0000-7000-9000-{number:012}")).unwrap()
}

fn writes_in(folder: &str) -> GrantScope {
    GrantScope::Path {
        mode: AccessMode::Write,
        folder: folder.to_owned(),
    }
}

#[tokio::test]
async fn grants_belong_to_their_conversation_and_are_added_once() {
    let fixture = Fixture::new();
    let first = fixture.conversation(1).await;
    let second = fixture.conversation(2).await;

    let added = fixture
        .store
        .add_grants(
            &first,
            vec![
                (grant_id(1), writes_in("/work/src")),
                (grant_id(2), writes_in("/work/src")),
            ],
        )
        .await
        .unwrap();
    let again = fixture
        .store
        .add_grants(&first, vec![(grant_id(3), writes_in("/work/src"))])
        .await
        .unwrap();

    assert_eq!(added.len(), 1);
    assert_eq!(again, Vec::new());
    assert_eq!(fixture.store.grants(&first).await.unwrap(), added);
    assert_eq!(fixture.store.grants(&second).await.unwrap(), Vec::new());
}

#[tokio::test]
async fn a_grant_is_revoked_only_in_its_conversation() {
    let fixture = Fixture::new();
    let first = fixture.conversation(1).await;
    let second = fixture.conversation(2).await;
    fixture
        .store
        .add_grants(&first, vec![(grant_id(1), writes_in("/work"))])
        .await
        .unwrap();

    let from_other = fixture
        .store
        .revoke_grant(&second, &grant_id(1))
        .await
        .unwrap();
    let from_owner = fixture
        .store
        .revoke_grant(&first, &grant_id(1))
        .await
        .unwrap();

    assert!(!from_other);
    assert!(from_owner);
    assert_eq!(fixture.store.grants(&first).await.unwrap(), Vec::new());
}

#[tokio::test]
async fn grants_need_a_conversation_and_end_with_it() {
    let fixture = Fixture::new();
    let missing = ConversationId::parse("0199e0f2-7a1b-7c3d-8e4f-000000000099").unwrap();
    let conversation = fixture.conversation(1).await;
    fixture
        .store
        .add_grants(&conversation, vec![(grant_id(1), writes_in("/work"))])
        .await
        .unwrap();

    let refused = fixture
        .store
        .add_grants(&missing, vec![(grant_id(2), writes_in("/work"))])
        .await;
    fixture
        .store
        .delete_conversation(&conversation)
        .await
        .unwrap();

    assert!(matches!(
        refused,
        Err(ToolsStoreError::ConversationNotFound)
    ));
    assert_eq!(
        fixture.store.grants(&conversation).await.unwrap(),
        Vec::new()
    );
}

#[tokio::test]
async fn the_same_rule_is_saved_once_and_deleted() {
    let fixture = Fixture::new();
    let words = vec!["cargo".to_owned(), "test".to_owned()];

    let saved = fixture
        .store
        .add_rule(&rule_id(1), words.clone(), Some("/work".to_owned()))
        .await
        .unwrap();
    let again = fixture
        .store
        .add_rule(&rule_id(2), words.clone(), Some("/work".to_owned()))
        .await
        .unwrap();
    let anywhere = fixture
        .store
        .add_rule(&rule_id(3), words.clone(), None)
        .await
        .unwrap();

    assert_eq!(again, saved);
    assert_ne!(anywhere.id, saved.id);
    assert_eq!(
        fixture.store.rules().await.unwrap(),
        vec![saved.clone(), anywhere.clone()]
    );
    assert!(fixture.store.delete_rule(&saved.id).await.unwrap());
    assert!(!fixture.store.delete_rule(&saved.id).await.unwrap());
    assert_eq!(fixture.store.rules().await.unwrap(), vec![anywhere]);
}

#[tokio::test]
async fn shared_grants_belong_to_a_project_or_to_everyone_and_go_with_their_project() {
    let fixture = Fixture::new();
    let project = ProjectId::parse("0199e0f2-0000-7000-a000-000000000001").unwrap();
    let removed = ProjectId::parse("0199e0f2-0000-7000-a000-000000000002").unwrap();
    fixture.store.add_project(&project, "/work").await.unwrap();
    fixture.store.add_project(&removed, "/gone").await.unwrap();

    let for_project = fixture
        .store
        .add_shared_grants(
            Some(&project),
            vec![
                (grant_id(1), writes_in("/work")),
                (grant_id(2), writes_in("/work")),
            ],
        )
        .await
        .unwrap();
    let for_everyone = fixture
        .store
        .add_shared_grants(None, vec![(grant_id(3), writes_in("/work"))])
        .await
        .unwrap();
    let repeated = fixture
        .store
        .add_shared_grants(Some(&project), vec![(grant_id(4), writes_in("/work"))])
        .await
        .unwrap();
    fixture
        .store
        .add_shared_grants(Some(&removed), vec![(grant_id(5), writes_in("/gone"))])
        .await
        .unwrap();
    let missing = fixture
        .store
        .add_shared_grants(
            Some(&ProjectId::parse("0199e0f2-0000-7000-a000-000000000009").unwrap()),
            vec![(grant_id(6), writes_in("/nowhere"))],
        )
        .await;
    fixture.store.delete_project(&removed).await.unwrap();

    assert_eq!(for_project.len(), 1);
    assert_eq!(for_project[0].project_id.as_ref(), Some(&project));
    assert_eq!(for_everyone.len(), 1);
    assert_eq!(for_everyone[0].project_id, None);
    assert_eq!(repeated, Vec::new());
    assert!(
        matches!(missing, Err(ToolsStoreError::ProjectNotFound)),
        "{missing:?}"
    );
    let grants = fixture.store.shared_grants().await.unwrap();
    assert_eq!(
        grants.iter().map(|grant| &grant.id).collect::<Vec<_>>(),
        [&grant_id(1), &grant_id(3)]
    );
    assert!(
        fixture
            .store
            .revoke_shared_grant(&grant_id(1))
            .await
            .unwrap()
    );
    assert!(
        !fixture
            .store
            .revoke_shared_grant(&grant_id(1))
            .await
            .unwrap()
    );
    assert_eq!(fixture.store.shared_grants().await.unwrap().len(), 1);
}
