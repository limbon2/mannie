//! The adapters of search profiles over real HTTP (ADR-0047): what an
//! embedding and a rerank request carry, how their answers are read, how a
//! refusal is classified as the chat adapters classify it, and the test of a
//! search profile. The answers are constructed after the shapes that
//! llama.cpp's server, vLLM and Text Embeddings Inference document.

#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use mannie_provider::testing::{FixtureResponse, FixtureServer};
use mannie_provider::{
    HttpClient, ProviderError, SearchTarget, SearchTestInput, Timeouts, embed, rerank,
    test_search_profile,
};
use mannie_types::clock::{ManualClock, UtcMillis};
use mannie_types::provider::{
    CheckNote, CheckOutcome, RerankProbe, SearchCheckKind, SearchKind, ServerKind, SkipReason,
};
use mannie_types::run::RunFailure;
use mannie_types::secret::Secret;
use serde_json::{Value, json};

const PLANTED: &str = "sk-planted-search-secret-0123456789";

fn client() -> HttpClient {
    let timeouts = Timeouts {
        connect: Duration::from_secs(5),
        headers: Duration::from_secs(5),
        first_event: Duration::from_secs(5),
        idle: Duration::from_secs(5),
    };
    HttpClient::with_timeouts(
        Arc::new(ManualClock::new(UtcMillis::from_millis(0))),
        timeouts,
    )
    .unwrap()
}

fn target(server: &FixtureServer, credential: Option<&str>) -> SearchTarget {
    SearchTarget {
        base_url: server.base_url(),
        model: "an-embedding-model".to_owned(),
        credential: credential.map(|text| Secret::new(text.to_owned())),
    }
}

fn texts(items: &[&str]) -> Vec<String> {
    items.iter().map(|&item| item.to_owned()).collect()
}

fn json_response(status: u16, body: &Value) -> FixtureResponse {
    FixtureResponse::json(status, serde_json::to_vec(body).unwrap())
}

fn sent_body(server: &FixtureServer, request: usize) -> Value {
    serde_json::from_slice(&server.requests()[request].body).unwrap()
}

#[tokio::test]
async fn an_embedding_request_names_the_model_and_its_inputs_and_reads_the_vectors_in_order() {
    let server = FixtureServer::start().await.unwrap();
    server.respond(json_response(
        200,
        &json!({ "object": "list", "data": [
            { "object": "embedding", "index": 1, "embedding": [0.0, 1.0] },
            { "object": "embedding", "index": 0, "embedding": [1.0, 0.0] },
        ], "model": "an-embedding-model" }),
    ));

    let vectors = embed(
        &client(),
        &target(&server, Some(PLANTED)),
        &texts(&["first", "second"]),
    )
    .await
    .unwrap();

    assert_eq!(vectors, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
    let requests = server.requests();
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/v1/embeddings");
    assert_eq!(
        requests[0].header("authorization"),
        Some(format!("Bearer {PLANTED}").as_str())
    );
    assert_eq!(
        sent_body(&server, 0),
        json!({ "model": "an-embedding-model", "input": ["first", "second"], "encoding_format": "float" })
    );
}

#[tokio::test]
async fn a_rerank_request_sends_the_documents_and_reads_the_scores_in_their_order() {
    let server = FixtureServer::start().await.unwrap();
    // Jina's shape, sorted by score, as llama.cpp's server answers.
    server.respond(json_response(
        200,
        &json!({ "model": "a-reranker", "object": "list", "results": [
            { "index": 1, "relevance_score": 0.9 },
            { "index": 0, "relevance_score": 0.1 },
        ] }),
    ));

    let scores = rerank(
        &client(),
        &target(&server, None),
        "the question",
        &texts(&["unrelated", "answering"]),
    )
    .await
    .unwrap();

    assert_eq!(scores, vec![0.1, 0.9]);
    assert_eq!(server.requests()[0].path, "/v1/rerank");
    assert_eq!(server.requests()[0].header("authorization"), None);
    assert_eq!(
        sent_body(&server, 0),
        json!({ "model": "an-embedding-model", "query": "the question", "documents": ["unrelated", "answering"], "top_n": 2 })
    );
}

#[tokio::test]
async fn refusals_are_classified_as_the_chat_adapters_classify_them() {
    let cases = [
        (
            401,
            json!({ "error": { "message": "bad key" } }),
            ProviderError::Permanent(RunFailure::InvalidCredential),
        ),
        (
            404,
            json!({ "error": { "message": "model not found" } }),
            ProviderError::Permanent(RunFailure::ModelNotFound {
                model: "an-embedding-model".to_owned(),
            }),
        ),
        (
            503,
            json!({ "error": "loading" }),
            ProviderError::Transient {
                cause: RunFailure::ServerError { status: 503 },
                retry_after: None,
            },
        ),
        (
            400,
            json!({ "error": { "message": "input is too long" } }),
            ProviderError::ContextOverflow,
        ),
    ];
    for (status, body, expected) in cases {
        let server = FixtureServer::start().await.unwrap();
        server.respond(json_response(status, &body));

        let error = embed(&client(), &target(&server, None), &texts(&["text"]))
            .await
            .unwrap_err();

        assert_eq!(error, expected, "status {status}");
    }
}

#[tokio::test]
async fn an_answer_that_is_not_the_shape_of_the_api_is_an_invalid_response() {
    for body in [
        json!({ "data": [] }),
        json!({ "results": [{ "index": 0, "relevance_score": 0.5 }] }),
        json!("ready"),
    ] {
        let server = FixtureServer::start().await.unwrap();
        server.respond(json_response(200, &body));
        server.respond(json_response(200, &body));

        let embedded = embed(&client(), &target(&server, None), &texts(&["text"])).await;
        let reranked = rerank(&client(), &target(&server, None), "q", &texts(&["a", "b"])).await;

        assert_eq!(
            embedded,
            Err(ProviderError::Permanent(RunFailure::InvalidResponse)),
            "{body}"
        );
        assert_eq!(
            reranked,
            Err(ProviderError::Permanent(RunFailure::InvalidResponse)),
            "{body}"
        );
    }
}

#[tokio::test]
async fn too_many_inputs_never_leave_mannie() {
    let server = FixtureServer::start().await.unwrap();
    let too_many: Vec<String> = (0..65).map(|number| number.to_string()).collect();
    let too_long = vec!["x".repeat(32 * 1024 + 1)];

    let many = embed(&client(), &target(&server, None), &too_many).await;
    let long = rerank(&client(), &target(&server, None), "q", &too_long).await;
    let none = embed(&client(), &target(&server, None), &[]).await;

    for outcome in [many.map(|_| ()), long.map(|_| ()), none.map(|_| ())] {
        assert_eq!(
            outcome,
            Err(ProviderError::Permanent(RunFailure::BadRequest {
                status: 413
            }))
        );
    }
    assert_eq!(server.requests().len(), 0);
}

/// The answers of the server and model checks: the list of models, then the
/// four probes of the server's kind, which this server does not know.
fn respond_to_the_server_checks(server: &FixtureServer, model: &str) {
    server.respond(json_response(200, &json!({ "data": [{ "id": model }] })));
    for _probe in 0..4 {
        server.respond(json_response(404, &json!({ "error": "not found" })));
    }
}

#[tokio::test]
async fn the_test_of_an_embedding_profile_reports_the_dimensions() {
    let server = FixtureServer::start().await.unwrap();
    respond_to_the_server_checks(&server, "an-embedding-model");
    server.respond(json_response(
        200,
        &json!({ "data": [{ "index": 0, "embedding": [0.25, 0.5, 0.75] }] }),
    ));

    let report = test_search_profile(
        &client(),
        SearchTestInput {
            kind: SearchKind::Embeddings,
            target: target(&server, None),
            credential_check: CheckOutcome::Skipped {
                reason: SkipReason::NoCredential,
            },
        },
    )
    .await;

    let checks: Vec<(SearchCheckKind, CheckOutcome)> = report
        .checks
        .iter()
        .map(|check| (check.check, check.outcome.clone()))
        .collect();
    assert_eq!(
        checks,
        [
            (
                SearchCheckKind::Credential,
                CheckOutcome::Skipped {
                    reason: SkipReason::NoCredential
                }
            ),
            (SearchCheckKind::Server, CheckOutcome::Passed),
            (SearchCheckKind::Model, CheckOutcome::Passed),
            (SearchCheckKind::Embed, CheckOutcome::Passed),
        ]
    );
    assert_eq!(report.dimensions, Some(3));
    assert_eq!(report.server, Some(ServerKind::Other));
}

#[tokio::test]
async fn the_test_of_a_rerank_profile_reports_both_scores_and_warns_of_a_wrong_order() {
    let server = FixtureServer::start().await.unwrap();
    respond_to_the_server_checks(&server, "a-reranker");
    server.respond(json_response(
        200,
        &json!([{ "index": 0, "score": 0.2 }, { "index": 1, "score": 0.6 }]),
    ));
    let mut rerank_target = target(&server, None);
    rerank_target.model = "a-reranker".to_owned();

    let report = test_search_profile(
        &client(),
        SearchTestInput {
            kind: SearchKind::Rerank,
            target: rerank_target,
            credential_check: CheckOutcome::Passed,
        },
    )
    .await;

    assert_eq!(
        report
            .checks
            .last()
            .map(|check| (check.check, check.outcome.clone())),
        Some((
            SearchCheckKind::Rerank,
            CheckOutcome::Warning {
                note: CheckNote::UnexpectedOrder
            }
        ))
    );
    assert_eq!(
        report.scores,
        Some(RerankProbe {
            answering: 0.2,
            unrelated: 0.6
        })
    );
}

#[tokio::test]
async fn a_server_that_refuses_the_credential_fails_the_test_before_it_embeds() {
    let server = FixtureServer::start().await.unwrap();
    server.respond(json_response(401, &json!({ "error": "unauthorized" })));

    let report = test_search_profile(
        &client(),
        SearchTestInput {
            kind: SearchKind::Embeddings,
            target: target(&server, Some(PLANTED)),
            credential_check: CheckOutcome::Passed,
        },
    )
    .await;

    let outcomes: Vec<CheckOutcome> = report
        .checks
        .iter()
        .map(|check| check.outcome.clone())
        .collect();
    assert_eq!(
        outcomes,
        [
            CheckOutcome::Passed,
            CheckOutcome::Failed {
                failure: RunFailure::InvalidCredential
            },
            CheckOutcome::Skipped {
                reason: SkipReason::EarlierCheckFailed
            },
            CheckOutcome::Skipped {
                reason: SkipReason::EarlierCheckFailed
            },
        ]
    );
    assert_eq!(server.requests().len(), 1);
    assert_eq!(report.dimensions, None);
}
