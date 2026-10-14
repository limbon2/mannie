use std::sync::Arc;

use mannie_types::clock::{Clock, ManualClock, UtcMillis};
use mannie_types::ids::ProfileId;
use mannie_types::provider::{ApiInterface, ReasoningEffort, ServerKind};

use super::{ProfileFields, ProfileStoreError};
use crate::{Store, StoreOptions};

struct Fixture {
    _directory: tempfile::TempDir,
    store: Store,
    clock: Arc<ManualClock>,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualClock::new(UtcMillis::from_millis(1_791_892_800_000)));
        let shared: Arc<dyn Clock> = Arc::<ManualClock>::clone(&clock);
        let options = StoreOptions::new(directory.path().join("backups"), shared);
        let store = Store::open(&directory.path().join("mannie.sqlite"), &options).unwrap();
        Self {
            _directory: directory,
            store,
            clock,
        }
    }
}

fn id(number: u32) -> ProfileId {
    ProfileId::parse(&format!("0199e0f2-0000-7000-8000-{number:012}")).unwrap()
}

fn fields(name: &str) -> ProfileFields {
    ProfileFields {
        name: name.to_owned(),
        interface: ApiInterface::ChatCompletions,
        base_url: "http://127.0.0.1:8000/v1".to_owned(),
        model: "qwen3".to_owned(),
        credential_ref: None,
        context_window: 32_768,
        output_limit: 4_096,
        reasoning_effort: ReasoningEffort::ServerDefault,
    }
}

#[tokio::test]
async fn the_first_profile_becomes_the_default() {
    let fixture = Fixture::new();

    let first = fixture
        .store
        .insert_profile(&id(1), fields("Local"))
        .await
        .unwrap();
    fixture.clock.advance(std::time::Duration::from_secs(1));
    let second = fixture
        .store
        .insert_profile(&id(2), fields("OpenAI"))
        .await
        .unwrap();

    assert!(first.is_default);
    assert!(!second.is_default);
    assert_eq!(first.config_revision, 1);
    assert_eq!(first.binding(), format!("{}:1", id(1)));
    assert_eq!(
        fixture
            .store
            .default_profile()
            .await
            .unwrap()
            .map(|profile| profile.id),
        Some(id(1))
    );
    let listed: Vec<_> = fixture
        .store
        .list_profiles()
        .await
        .unwrap()
        .into_iter()
        .map(|profile| profile.name)
        .collect();
    assert_eq!(listed, ["Local", "OpenAI"]);
}

#[tokio::test]
async fn names_are_unique_regardless_of_case() {
    let fixture = Fixture::new();
    fixture
        .store
        .insert_profile(&id(1), fields("Ölmodell"))
        .await
        .unwrap();

    let ascii = fixture
        .store
        .insert_profile(&id(2), fields("ölmodell"))
        .await;
    let renamed_onto = {
        fixture
            .store
            .insert_profile(&id(3), fields("Other"))
            .await
            .unwrap();
        fixture
            .store
            .update_profile(&id(3), fields("ÖLMODELL"))
            .await
    };

    assert!(
        matches!(ascii, Err(ProfileStoreError::NameTaken)),
        "{ascii:?}"
    );
    assert!(
        matches!(renamed_onto, Err(ProfileStoreError::NameTaken)),
        "{renamed_onto:?}"
    );
    let keeps_own_name = fixture
        .store
        .update_profile(&id(1), fields("ölmodell"))
        .await;
    assert!(keeps_own_name.is_ok(), "{keeps_own_name:?}");
}

#[tokio::test]
async fn the_revision_increases_only_when_the_connection_changes() {
    let fixture = Fixture::new();
    fixture
        .store
        .insert_profile(&id(1), fields("Local"))
        .await
        .unwrap();
    fixture
        .store
        .record_profile_server(&id(1), ServerKind::Vllm)
        .await
        .unwrap();

    let (_, renamed) = fixture
        .store
        .update_profile(&id(1), fields("Renamed"))
        .await
        .unwrap();
    let limits = ProfileFields {
        context_window: 65_536,
        ..fields("Renamed")
    };
    let (_, limited) = fixture.store.update_profile(&id(1), limits).await.unwrap();
    let model = ProfileFields {
        model: "llama".to_owned(),
        ..fields("Renamed")
    };
    let (before, changed) = fixture.store.update_profile(&id(1), model).await.unwrap();
    let credential = ProfileFields {
        credential_ref: Some("provider/x/2".to_owned()),
        model: "llama".to_owned(),
        ..fields("Renamed")
    };
    let (_, with_credential) = fixture
        .store
        .update_profile(&id(1), credential)
        .await
        .unwrap();

    assert_eq!(renamed.config_revision, 1);
    assert_eq!(renamed.server, Some(ServerKind::Vllm));
    assert_eq!(limited.config_revision, 1);
    assert_eq!(limited.context_window, 65_536);
    assert_eq!(before.model, "qwen3");
    assert_eq!(changed.config_revision, 2);
    assert_eq!(changed.server, None, "a new connection forgets the server");
    assert_eq!(with_credential.config_revision, 3);
    assert!(with_credential.view().has_credential);
}

#[tokio::test]
async fn deleting_the_default_makes_the_oldest_remaining_profile_the_default() {
    let fixture = Fixture::new();
    fixture
        .store
        .insert_profile(&id(1), fields("First"))
        .await
        .unwrap();
    fixture.clock.advance(std::time::Duration::from_secs(1));
    fixture
        .store
        .insert_profile(&id(2), fields("Second"))
        .await
        .unwrap();
    fixture.clock.advance(std::time::Duration::from_secs(1));
    fixture
        .store
        .insert_profile(&id(3), fields("Third"))
        .await
        .unwrap();
    fixture.store.set_default_profile(&id(3)).await.unwrap();

    let deleted = fixture.store.delete_profile(&id(3)).await.unwrap();

    assert_eq!(deleted.id, id(3));
    assert_eq!(
        fixture
            .store
            .default_profile()
            .await
            .unwrap()
            .map(|profile| profile.id),
        Some(id(1))
    );
    assert!(matches!(
        fixture.store.delete_profile(&id(3)).await,
        Err(ProfileStoreError::NotFound)
    ));
    assert!(matches!(
        fixture.store.set_default_profile(&id(9)).await,
        Err(ProfileStoreError::NotFound)
    ));
}

#[tokio::test]
async fn deleting_the_last_profile_leaves_no_default() {
    let fixture = Fixture::new();
    fixture
        .store
        .insert_profile(&id(1), fields("Only"))
        .await
        .unwrap();

    fixture.store.delete_profile(&id(1)).await.unwrap();

    assert_eq!(fixture.store.default_profile().await.unwrap(), None);
    assert!(fixture.store.profile(&id(1)).await.unwrap().is_none());
}
