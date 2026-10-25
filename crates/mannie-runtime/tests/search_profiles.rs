//! Search profiles and the service `models` (ADR-0047): a profile keeps its
//! credential in the vault as a chat profile does, and a plugin embeds and
//! reranks through it without seeing the server, the credential or the
//! prefixes, which the core adds by the kind of each text.

// Integration tests are always built with cfg(test); saying so lets Clippy
// treat the helpers below as test code.
#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use mannie_plugin_sdk::services::models::{InputKind, ModelsError, SearchModelKind};
use mannie_provider::testing::{FixtureResponse, FixtureServer};
use mannie_runtime::Runtime;
use mannie_runtime::testing::{
    FakeModel, MemoryVault, fast_config, models, open_store, start_runtime,
};
use mannie_store::Store;
use mannie_types::provider::{
    ApiInterface, CreateSearchProfile, CredentialChange, ProviderProfileError, SearchKind,
    SearchProfile, SearchProfileDraft,
};
use mannie_types::secret::Secret;
use serde_json::{Value, json};

const PLANTED: &str = "sk-planted-search-credential-0123456789";

/// The plugin that the service is bound to.
const PLUGIN: &str = "notebooks";

struct Setup {
    runtime: Runtime,
    store: Store,
    vault: Arc<MemoryVault>,
    directory: tempfile::TempDir,
}

async fn setup() -> Setup {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(&directory.path().join("mannie.sqlite"));
    let vault = Arc::new(MemoryVault::new());
    let model = FakeModel::new(ApiInterface::ChatCompletions);
    let runtime = start_runtime(
        store.clone(),
        Arc::new(model),
        Arc::clone(&vault),
        fast_config(),
    )
    .await;
    Setup {
        runtime,
        store,
        vault,
        directory,
    }
}

fn draft(kind: SearchKind, base_url: &str) -> SearchProfileDraft {
    let prefixes = kind == SearchKind::Embeddings;
    SearchProfileDraft {
        name: format!("{} at {base_url}", kind.as_str()),
        kind,
        base_url: base_url.to_owned(),
        model: "a-search-model".to_owned(),
        query_prefix: if prefixes {
            "Q: ".to_owned()
        } else {
            String::new()
        },
        document_prefix: if prefixes {
            "D: ".to_owned()
        } else {
            String::new()
        },
    }
}

async fn create(
    setup: &Setup,
    kind: SearchKind,
    base_url: &str,
    credential: Option<&str>,
) -> SearchProfile {
    setup
        .runtime
        .create_search_profile(CreateSearchProfile {
            draft: draft(kind, base_url),
            credential: credential.map(|credential| Secret::new(credential.to_owned())),
        })
        .await
        .unwrap()
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

fn json_response(status: u16, body: &Value) -> FixtureResponse {
    FixtureResponse::json(status, serde_json::to_vec(body).unwrap())
}

fn sent_body(server: &FixtureServer, request: usize) -> Value {
    serde_json::from_slice(&server.requests()[request].body).unwrap()
}

#[tokio::test]
async fn a_search_profile_keeps_only_the_name_of_its_credential() {
    let setup = setup().await;

    let profile = create(
        &setup,
        SearchKind::Embeddings,
        "https://search.example.com/v1",
        Some(PLANTED),
    )
    .await;
    setup.store.flush().await.unwrap();

    let reference = format!("provider/{}/1", profile.id);
    assert!(profile.has_credential);
    assert_eq!(setup.vault.references(), std::slice::from_ref(&reference));
    assert_eq!(setup.vault.credential(&reference).as_deref(), Some(PLANTED));
    assert!(!files_contain(setup.directory.path(), PLANTED));
    assert_eq!(
        setup.runtime.list_search_profiles().await.unwrap(),
        vec![profile]
    );
}

#[tokio::test]
async fn a_new_credential_replaces_the_old_and_deleting_the_profile_deletes_it() {
    let setup = setup().await;
    let profile = create(
        &setup,
        SearchKind::Rerank,
        "https://search.example.com/v1",
        Some(PLANTED),
    )
    .await;

    let changed = setup
        .runtime
        .update_search_profile(
            &profile.id,
            draft(SearchKind::Rerank, "https://search.example.com/v1"),
            CredentialChange::Replace {
                credential: Secret::new("sk-second".to_owned()),
            },
        )
        .await
        .unwrap();
    let after_change = setup.vault.references();
    setup
        .runtime
        .delete_search_profile(&changed.id)
        .await
        .unwrap();

    assert_eq!(after_change, [format!("provider/{}/2", profile.id)]);
    assert_eq!(setup.vault.references(), Vec::<String>::new());
    assert_eq!(
        setup.runtime.delete_search_profile(&profile.id).await,
        Err(ProviderProfileError::NotFound)
    );
}

#[tokio::test]
async fn a_credential_never_travels_over_plain_http_to_a_remote_host() {
    let setup = setup().await;

    let refused = setup
        .runtime
        .create_search_profile(CreateSearchProfile {
            draft: draft(SearchKind::Embeddings, "http://search.example.com/v1"),
            credential: Some(Secret::new(PLANTED.to_owned())),
        })
        .await;

    assert_eq!(
        refused,
        Err(ProviderProfileError::InsecureCredentialTransport)
    );
    assert_eq!(setup.vault.references(), Vec::<String>::new());
}

#[tokio::test]
async fn the_service_embeds_with_the_prefix_of_each_kind_and_the_stored_credential() {
    let setup = setup().await;
    let server = FixtureServer::start().await.unwrap();
    let profile = create(
        &setup,
        SearchKind::Embeddings,
        server.base_url().as_str(),
        Some(PLANTED),
    )
    .await;
    for _call in 0..2 {
        server.respond(json_response(
            200,
            &json!({ "data": [{ "index": 0, "embedding": [0.6, 0.8] }] }),
        ));
    }
    let service = models(&setup.runtime, PLUGIN);

    let query = service
        .embed(
            profile.id.as_str(),
            InputKind::Query,
            vec!["hello".to_owned()],
        )
        .await
        .unwrap();
    let document = service
        .embed(
            profile.id.as_str(),
            InputKind::Document,
            vec!["a page".to_owned()],
        )
        .await
        .unwrap();

    assert_eq!(query, vec![vec![0.6, 0.8]]);
    assert_eq!(document, vec![vec![0.6, 0.8]]);
    assert_eq!(sent_body(&server, 0)["input"], json!(["Q: hello"]));
    assert_eq!(sent_body(&server, 1)["input"], json!(["D: a page"]));
    assert_eq!(
        server.requests()[0].header("authorization"),
        Some(format!("Bearer {PLANTED}").as_str())
    );
}

#[tokio::test]
async fn the_service_reranks_with_a_rerank_profile_and_refuses_what_does_not_fit() {
    let setup = setup().await;
    let server = FixtureServer::start().await.unwrap();
    let reranker = create(&setup, SearchKind::Rerank, server.base_url().as_str(), None).await;
    server.respond(json_response(
        200,
        &json!({ "results": [
            { "index": 1, "relevance_score": 0.75 },
            { "index": 0, "relevance_score": 0.25 },
        ] }),
    ));
    let service = models(&setup.runtime, PLUGIN);

    let scores = service
        .rerank(
            reranker.id.as_str(),
            "the question".to_owned(),
            vec!["unrelated".to_owned(), "answering".to_owned()],
        )
        .await
        .unwrap();
    let wrong_kind = service
        .embed(
            reranker.id.as_str(),
            InputKind::Query,
            vec!["text".to_owned()],
        )
        .await;
    let unknown = service
        .rerank(
            "0199e0f2-0000-7000-8000-000000000999",
            "q".to_owned(),
            vec!["text".to_owned()],
        )
        .await;
    let not_an_id = service
        .rerank("not an id", "q".to_owned(), vec!["text".to_owned()])
        .await;
    let none = service
        .rerank(reranker.id.as_str(), "q".to_owned(), Vec::new())
        .await;

    assert_eq!(scores, vec![0.25, 0.75]);
    assert_eq!(sent_body(&server, 0)["query"], json!("the question"));
    assert_eq!(wrong_kind, Err(ModelsError::WrongKind));
    assert_eq!(unknown, Err(ModelsError::ProfileNotFound));
    assert_eq!(not_an_id, Err(ModelsError::ProfileNotFound));
    assert_eq!(none, Err(ModelsError::InvalidInputs));
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn a_transient_failure_is_tried_again_and_a_refusal_reaches_the_plugin_with_its_code() {
    let setup = setup().await;
    let server = FixtureServer::start().await.unwrap();
    let profile = create(
        &setup,
        SearchKind::Embeddings,
        server.base_url().as_str(),
        None,
    )
    .await;
    server.respond(json_response(503, &json!({ "error": "loading the model" })));
    server.respond(json_response(
        200,
        &json!({ "data": [{ "index": 0, "embedding": [1.0] }] }),
    ));
    server.respond(json_response(
        401,
        &json!({ "error": { "message": "bad key" } }),
    ));
    let service = models(&setup.runtime, PLUGIN);

    let retried = service
        .embed(
            profile.id.as_str(),
            InputKind::Document,
            vec!["text".to_owned()],
        )
        .await;
    let refused = service
        .embed(
            profile.id.as_str(),
            InputKind::Document,
            vec!["text".to_owned()],
        )
        .await;

    assert_eq!(retried, Ok(vec![vec![1.0]]));
    assert_eq!(
        refused,
        Err(ModelsError::Refused {
            failure: "invalid_credential".to_owned()
        })
    );
    assert_eq!(server.requests().len(), 3);
}

#[tokio::test]
async fn profiles_list_by_kind_with_a_key_that_follows_the_model_and_the_document_prefix() {
    let setup = setup().await;
    let embeddings = create(
        &setup,
        SearchKind::Embeddings,
        "http://127.0.0.1:8080/v1",
        None,
    )
    .await;
    create(&setup, SearchKind::Rerank, "http://127.0.0.1:8081/v1", None).await;
    let service = models(&setup.runtime, PLUGIN);

    let listed = service
        .search_profiles(SearchModelKind::Embeddings)
        .await
        .unwrap();
    let moved = SearchProfileDraft {
        base_url: "http://127.0.0.1:9090/v1".to_owned(),
        ..draft(SearchKind::Embeddings, "http://127.0.0.1:8080/v1")
    };
    setup
        .runtime
        .update_search_profile(&embeddings.id, moved.clone(), CredentialChange::Keep)
        .await
        .unwrap();
    let after_move = service
        .search_profiles(SearchModelKind::Embeddings)
        .await
        .unwrap();
    let new_model = SearchProfileDraft {
        model: "another-model".to_owned(),
        ..moved
    };
    setup
        .runtime
        .update_search_profile(&embeddings.id, new_model, CredentialChange::Keep)
        .await
        .unwrap();
    let after_model = service
        .search_profiles(SearchModelKind::Embeddings)
        .await
        .unwrap();

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, embeddings.id.as_str());
    assert_eq!(listed[0].kind, SearchModelKind::Embeddings);
    assert_eq!(after_move[0].vector_key, listed[0].vector_key);
    assert_ne!(after_model[0].vector_key, listed[0].vector_key);
}

#[tokio::test]
async fn followers_hear_of_the_profiles_that_change_or_go() {
    let setup = setup().await;
    let profile = create(
        &setup,
        SearchKind::Embeddings,
        "http://127.0.0.1:8080/v1",
        None,
    )
    .await;
    let mut changes = models(&setup.runtime, PLUGIN).changes();

    setup
        .runtime
        .update_search_profile(
            &profile.id,
            draft(SearchKind::Embeddings, "http://127.0.0.1:8081/v1"),
            CredentialChange::Keep,
        )
        .await
        .unwrap();
    setup
        .runtime
        .delete_search_profile(&profile.id)
        .await
        .unwrap();

    let wait = Duration::from_secs(10);
    let first = tokio::time::timeout(wait, changes.next()).await.unwrap();
    let second = tokio::time::timeout(wait, changes.next()).await.unwrap();
    assert_eq!(first.as_deref(), Some(profile.id.as_str()));
    assert_eq!(second.as_deref(), Some(profile.id.as_str()));
}
