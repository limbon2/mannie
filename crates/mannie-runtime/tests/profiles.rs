//! Provider profiles keep their credentials in the credential vault, write a
//! new credential before the profile changes and delete the old one after, and
//! never leave a credential behind that no profile uses
//! (docs/features/chat/providers.md, provider profiles; ADR-0010).

// Integration tests are always built with cfg(test); saying so lets Clippy
// treat the helpers below as test code.
#![cfg(test)]

use std::sync::Arc;

use mannie_provider::testing::{FixtureResponse, FixtureServer, Step};
use mannie_runtime::Runtime;
use mannie_runtime::testing::{
    FakeModel, MemoryVault, fast_config, open_store, reply_with_usage, start_runtime,
};
use mannie_store::Store;
use mannie_types::chat::StartConversation;
use mannie_types::provider::{
    ApiInterface, CheckKind, CheckOutcome, CreateProviderProfile, CredentialChange,
    CredentialStoreProblem, ProfileDraft, ProviderProfile, ProviderProfileError, ReasoningEffort,
    ServerKind,
};
use mannie_types::secret::Secret;

struct Setup {
    runtime: Runtime,
    store: Store,
    vault: Arc<MemoryVault>,
    model: FakeModel,
    directory: tempfile::TempDir,
}

async fn setup() -> Setup {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(&directory.path().join("mannie.sqlite"));
    let vault = Arc::new(MemoryVault::new());
    let model = FakeModel::new(ApiInterface::ChatCompletions);
    let runtime = start_runtime(
        store.clone(),
        Arc::new(model.clone()),
        Arc::clone(&vault),
        fast_config(),
    )
    .await;
    Setup {
        runtime,
        store,
        vault,
        model,
        directory,
    }
}

fn draft(name: &str, base_url: &str) -> ProfileDraft {
    ProfileDraft {
        name: name.to_owned(),
        interface: ApiInterface::ChatCompletions,
        base_url: base_url.to_owned(),
        model: "test-model".to_owned(),
        context_window: 128_000,
        output_limit: 16_384,
        reasoning_effort: ReasoningEffort::ServerDefault,
        use_tools: false,
    }
}

async fn create(
    setup: &Setup,
    name: &str,
    credential: Option<&str>,
) -> Result<ProviderProfile, ProviderProfileError> {
    setup
        .runtime
        .create_profile(CreateProviderProfile {
            draft: draft(name, "https://api.example.com/v1"),
            credential: credential.map(|credential| Secret::new(credential.to_owned())),
        })
        .await
}

async fn reference(setup: &Setup, profile: &ProviderProfile) -> Option<String> {
    setup
        .store
        .profile(&profile.id)
        .await
        .unwrap()
        .and_then(|stored| stored.credential_ref)
}

fn files_contain(directory: &std::path::Path, needle: &str) -> bool {
    std::fs::read_dir(directory).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.is_file()
            && std::fs::read(&path)
                .unwrap()
                .windows(needle.len())
                .any(|window| window == needle.as_bytes())
    })
}

#[tokio::test]
async fn the_store_keeps_only_the_name_of_the_credential() {
    let setup = setup().await;

    let profile = create(&setup, "OpenAI", Some("sk-planted-credential-1"))
        .await
        .unwrap();
    setup.store.flush().await.unwrap();

    let reference = reference(&setup, &profile).await.unwrap();
    assert_eq!(reference, format!("provider/{}/1", profile.id));
    assert_eq!(setup.vault.references(), std::slice::from_ref(&reference));
    assert_eq!(
        setup.vault.credential(&reference).as_deref(),
        Some("sk-planted-credential-1")
    );
    assert!(profile.has_credential);
    assert!(profile.is_default, "the first profile becomes the default");
    assert!(!files_contain(
        setup.directory.path(),
        "sk-planted-credential-1"
    ));
}

#[tokio::test]
async fn a_credential_never_travels_over_plain_http_to_a_remote_host() {
    let setup = setup().await;

    let remote = setup
        .runtime
        .create_profile(CreateProviderProfile {
            draft: draft("Remote", "http://models.example.com/v1"),
            credential: Some(Secret::new("sk-remote".to_owned())),
        })
        .await;
    let local = setup
        .runtime
        .create_profile(CreateProviderProfile {
            draft: draft("Local", "http://192.168.1.20:8000/v1"),
            credential: Some(Secret::new("sk-local".to_owned())),
        })
        .await;
    let without_credential = setup
        .runtime
        .create_profile(CreateProviderProfile {
            draft: draft("Remote without key", "http://models.example.com/v1"),
            credential: None,
        })
        .await;

    assert_eq!(
        remote.unwrap_err(),
        ProviderProfileError::InsecureCredentialTransport
    );
    local.unwrap();
    assert!(!without_credential.unwrap().has_credential);
    assert_eq!(
        setup.vault.references().len(),
        1,
        "only the local profile's credential"
    );
}

#[tokio::test]
async fn a_vault_that_refuses_the_credential_leaves_no_profile() {
    let setup = setup().await;
    setup
        .vault
        .fail_saves(Some(CredentialStoreProblem::AccessDenied));

    let refused = create(&setup, "OpenAI", Some("sk-1")).await;

    assert_eq!(
        refused.unwrap_err(),
        ProviderProfileError::CredentialStoreUnavailable {
            reason: CredentialStoreProblem::AccessDenied
        }
    );
    assert_eq!(setup.runtime.list_profiles().await.unwrap(), Vec::new());
}

#[tokio::test]
async fn a_profile_that_cannot_be_stored_takes_its_new_credential_back() {
    let setup = setup().await;
    let first = create(&setup, "OpenAI", Some("sk-1")).await.unwrap();

    let duplicate = create(&setup, "openai", Some("sk-2")).await;

    assert_eq!(duplicate.unwrap_err(), ProviderProfileError::NameTaken);
    assert_eq!(
        setup.vault.references(),
        [reference(&setup, &first).await.unwrap()]
    );
}

#[tokio::test]
async fn replacing_a_credential_writes_the_new_one_first_and_deletes_the_old_one_after() {
    let setup = setup().await;
    let profile = create(&setup, "OpenAI", Some("sk-1")).await.unwrap();
    let other = create(&setup, "Other", None).await.unwrap();
    let old = reference(&setup, &profile).await.unwrap();

    let replaced = setup
        .runtime
        .update_profile(
            &profile.id,
            draft("OpenAI", "https://api.example.com/v1"),
            CredentialChange::Replace {
                credential: Secret::new("sk-2".to_owned()),
            },
        )
        .await
        .unwrap();
    let new = reference(&setup, &replaced).await.unwrap();
    let refused = setup
        .runtime
        .update_profile(
            &profile.id,
            draft("Other", "https://api.example.com/v1"),
            CredentialChange::Replace {
                credential: Secret::new("sk-3".to_owned()),
            },
        )
        .await;

    assert_eq!(new, format!("provider/{}/2", profile.id));
    assert_eq!(setup.vault.credential(&new).as_deref(), Some("sk-2"));
    assert_eq!(setup.vault.credential(&old), None);
    assert_eq!(refused.unwrap_err(), ProviderProfileError::NameTaken);
    assert_eq!(
        setup.vault.references(),
        std::slice::from_ref(&new),
        "the refused credential is taken back"
    );
    assert_eq!(reference(&setup, &replaced).await, Some(new));
    assert!(!other.has_credential);
}

#[tokio::test]
async fn keeping_and_removing_a_credential() {
    let setup = setup().await;
    let profile = create(&setup, "OpenAI", Some("sk-1")).await.unwrap();
    let original = reference(&setup, &profile).await.unwrap();
    let mut changed_model = draft("OpenAI", "https://api.example.com/v1");
    changed_model.model = "another-model".to_owned();

    let kept = setup
        .runtime
        .update_profile(&profile.id, changed_model.clone(), CredentialChange::Keep)
        .await
        .unwrap();
    let kept_reference = reference(&setup, &kept).await;
    let removed = setup
        .runtime
        .update_profile(&profile.id, changed_model, CredentialChange::Remove)
        .await
        .unwrap();

    assert_eq!(kept_reference, Some(original.clone()));
    assert!(!removed.has_credential);
    assert_eq!(reference(&setup, &removed).await, None);
    assert_eq!(setup.vault.references(), Vec::<String>::new());
}

#[tokio::test]
async fn deleting_a_profile_deletes_its_credential_and_the_oldest_becomes_the_default() {
    let setup = setup().await;
    let first = create(&setup, "First", Some("sk-1")).await.unwrap();
    let second = create(&setup, "Second", Some("sk-2")).await.unwrap();
    let second_reference = reference(&setup, &second).await.unwrap();

    setup.runtime.delete_profile(&first.id).await.unwrap();
    let profiles = setup.runtime.list_profiles().await.unwrap();
    let missing = setup.runtime.delete_profile(&first.id).await;

    assert_eq!(profiles.len(), 1);
    assert!(profiles[0].is_default);
    assert_eq!(setup.vault.references(), [second_reference]);
    assert_eq!(missing.unwrap_err(), ProviderProfileError::NotFound);
}

#[tokio::test]
async fn a_profile_that_a_running_reply_uses_is_kept_until_the_reply_ends() {
    let setup = setup().await;
    let profile = create(&setup, "OpenAI", Some("sk-1")).await.unwrap();
    setup.model.push_reply(vec![Step::Hang]);
    let started = setup
        .runtime
        .start_conversation(StartConversation::new("Question"))
        .await
        .unwrap();

    let in_use = setup.runtime.delete_profile(&profile.id).await;
    setup.runtime.stop_reply(&started.conversation_id).await;
    let after_stop = setup.runtime.delete_profile(&profile.id).await;

    assert_eq!(in_use.unwrap_err(), ProviderProfileError::ProfileInUse);
    after_stop.unwrap();
}

#[tokio::test]
async fn the_provider_test_uses_the_stored_credential_and_records_the_server() {
    let setup = setup().await;
    let server = FixtureServer::start().await.unwrap();
    server.respond(FixtureResponse::json(
        200,
        r#"{"data":[{"id":"test-model"}]}"#,
    ));
    let base_url = server.base_url().to_string();
    let profile = setup
        .runtime
        .create_profile(CreateProviderProfile {
            draft: draft("Local", &base_url),
            credential: Some(Secret::new("sk-local-test".to_owned())),
        })
        .await
        .unwrap();
    setup.model.push_reply(reply_with_usage("ready", 12, 2));

    let report = setup.runtime.test_profile(&profile.id).await.unwrap();
    let stored = setup.runtime.list_profiles().await.unwrap().remove(0);

    let outcome = |kind: CheckKind| {
        report
            .checks
            .iter()
            .find(|check| check.check == kind)
            .map(|check| check.outcome.clone())
    };
    assert_eq!(outcome(CheckKind::Credential), Some(CheckOutcome::Passed));
    assert_eq!(outcome(CheckKind::Model), Some(CheckOutcome::Passed));
    assert_eq!(outcome(CheckKind::Reply), Some(CheckOutcome::Passed));
    assert_eq!(outcome(CheckKind::Usage), Some(CheckOutcome::Passed));
    let models_request = &server.requests()[0];
    assert_eq!(models_request.path, "/v1/models");
    assert_eq!(
        models_request.header("authorization"),
        Some("Bearer sk-local-test")
    );
    assert_eq!(
        setup.model.credentials()[0].as_deref(),
        Some("sk-local-test")
    );
    assert_eq!(report.server, Some(ServerKind::Other));
    assert_eq!(stored.server, Some(ServerKind::Other));
}

#[tokio::test]
async fn a_provider_test_without_the_credential_reports_it_and_calls_nothing() {
    let setup = setup().await;
    let profile = create(&setup, "OpenAI", Some("sk-1")).await.unwrap();
    setup
        .vault
        .remove(&reference(&setup, &profile).await.unwrap());

    let report = setup.runtime.test_profile(&profile.id).await.unwrap();

    assert_eq!(
        report.checks[0].outcome,
        CheckOutcome::Failed {
            failure: mannie_types::run::RunFailure::CredentialMissing
        }
    );
    assert_eq!(setup.model.requests(), Vec::new());
}
