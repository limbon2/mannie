//! The ChatGPT sign-in over real HTTP (ADR-0062): the token endpoint's code
//! exchange, refresh and revocation, the issuer's keys for the ID token, and
//! the plan's catalog of models.

#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use mannie_provider::chatgpt::{
    AuthorizationRequest, ChatGptEndpoints, IdTokenError, TokenError, authorization, exchange_code,
    refresh_tokens, revoke_refresh_token, verify_id_token,
};
use mannie_provider::testing::{
    Chunking, FixtureResponse, FixtureServer, RecordedRequest, example_key_set, signed_id_token,
};
use mannie_provider::{HttpClient, ProviderError, list_plan_models};
use mannie_types::clock::{ManualClock, UtcMillis};
use mannie_types::run::RunFailure;
use mannie_types::secret::Secret;
use reqwest::Url;
use serde_json::json;

/// 2026-11-08T00:00:00Z.
const NOW_MILLIS: i64 = 1_794_096_000_000;

fn client() -> HttpClient {
    HttpClient::new(Arc::new(ManualClock::new(UtcMillis::from_millis(
        NOW_MILLIS,
    ))))
    .unwrap()
}

/// The endpoints of the fixture server, which answers every path in turn.
fn endpoints(server: &FixtureServer) -> ChatGptEndpoints {
    let mut issuer = server.base_url();
    issuer.set_path("/");
    ChatGptEndpoints::at(&issuer, &server.base_url())
}

/// The fields of a recorded form.
fn form(request: &RecordedRequest) -> Vec<(String, String)> {
    url::form_urlencoded::parse(&request.body)
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

fn field<'a>(fields: &'a [(String, String)], name: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(key, _value)| key == name)
        .map(|(_key, value)| value.as_str())
}

fn tokens_answer() -> FixtureResponse {
    FixtureResponse::json(
        200,
        json!({
            "access_token": "planted-access-token-0123456789",
            "refresh_token": "planted-refresh-token-0123456789",
            "id_token": "planted-id-token",
            "expires_in": 3600,
            "scope": "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct",
            "token_type": "Bearer"
        })
        .to_string(),
    )
}

#[tokio::test]
async fn a_code_is_redeemed_with_its_verifier_redirect_and_resource() {
    let server = FixtureServer::start().await.unwrap();
    let endpoints = endpoints(&server);
    let authorization = authorization(&AuthorizationRequest {
        endpoints: &endpoints,
        host_id: "urn:uuid:9f0c1d6e-3a52-4b8e-9d4c-2f1e8a7b6c5d",
        port: 1455,
        client_id: None,
        login_hint: None,
    })
    .unwrap();
    server.respond(tokens_answer());

    let tokens = exchange_code(
        &client(),
        &endpoints,
        "oaiapp_1",
        &Secret::new("the-code".to_owned()),
        &authorization,
    )
    .await
    .unwrap();

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/api/accounts/oauth/token");
    assert_eq!(
        request.header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(request.header("authorization"), None);
    let fields = form(request);
    assert_eq!(field(&fields, "grant_type"), Some("authorization_code"));
    assert_eq!(field(&fields, "client_id"), Some("oaiapp_1"));
    assert_eq!(field(&fields, "code"), Some("the-code"));
    assert_eq!(
        field(&fields, "code_verifier"),
        Some(authorization.verifier.expose())
    );
    assert_eq!(
        field(&fields, "redirect_uri"),
        Some("http://127.0.0.1:1455/auth/callback")
    );
    assert_eq!(
        field(&fields, "resource"),
        Some("https://api.openai.com/v1")
    );
    assert_eq!(
        tokens.access_token.expose(),
        "planted-access-token-0123456789"
    );
    assert!(tokens.grants("chatgpt.tokens.use.direct"));
}

#[tokio::test]
async fn a_refresh_posts_its_form_with_the_client_and_resource() {
    let server = FixtureServer::start().await.unwrap();
    server.respond(tokens_answer());

    let tokens = refresh_tokens(
        &client(),
        &endpoints(&server),
        "oaiapp_1",
        &Secret::new("old-refresh-token".to_owned()),
    )
    .await
    .unwrap();

    let fields = form(&server.requests()[0]);
    assert_eq!(field(&fields, "grant_type"), Some("refresh_token"));
    assert_eq!(field(&fields, "client_id"), Some("oaiapp_1"));
    assert_eq!(field(&fields, "refresh_token"), Some("old-refresh-token"));
    assert_eq!(
        field(&fields, "resource"),
        Some("https://api.openai.com/v1")
    );
    assert_eq!(field(&fields, "scope"), None);
    assert_eq!(
        tokens.refresh_token.as_ref().map(Secret::expose),
        Some("planted-refresh-token-0123456789")
    );
}

#[tokio::test]
async fn a_refresh_without_a_new_refresh_token_names_none() {
    let server = FixtureServer::start().await.unwrap();
    server.respond(FixtureResponse::json(
        200,
        r#"{"access_token":"new-access","expires_in":3600}"#,
    ));

    let tokens = refresh_tokens(
        &client(),
        &endpoints(&server),
        "oaiapp_1",
        &Secret::new("old-refresh-token".to_owned()),
    )
    .await
    .unwrap();

    assert!(tokens.refresh_token.is_none());
    assert_eq!(tokens.access_token.expose(), "new-access");
}

#[tokio::test]
async fn terminal_codes_are_terminal_in_the_oauth_and_the_openai_shape() {
    for code in [
        "invalid_grant",
        "invalid_refresh_token",
        "token_expired",
        "refresh_token_expired",
        "refresh_token_invalidated",
        "refresh_token_reused",
    ] {
        for body in [
            json!({ "error": code, "error_description": "gone" }),
            json!({ "error": { "code": code, "message": "gone" } }),
        ] {
            let server = FixtureServer::start().await.unwrap();
            server.respond(FixtureResponse::json(400, body.to_string()));

            let error = refresh_tokens(
                &client(),
                &endpoints(&server),
                "oaiapp_1",
                &Secret::new("old".to_owned()),
            )
            .await
            .unwrap_err();

            assert_eq!(
                error,
                TokenError::Terminal {
                    code: code.to_owned()
                },
                "{body}"
            );
        }
    }
}

#[tokio::test]
async fn server_errors_and_rate_limits_of_a_refresh_are_transient() {
    let server = FixtureServer::start().await.unwrap();
    server.respond(FixtureResponse::json(503, r#"{"error":"unavailable"}"#));
    let mut limited = FixtureResponse::json(429, r#"{"error":"slow_down"}"#);
    limited
        .headers
        .push(("retry-after".to_owned(), "20".to_owned()));
    server.respond(limited);
    server.respond(FixtureResponse::json(400, r#"{"error":"invalid_client"}"#));
    server.respond(FixtureResponse::json(400, r#"{"error":"invalid_request"}"#));
    let refresh = || async {
        refresh_tokens(
            &client(),
            &endpoints(&server),
            "oaiapp_1",
            &Secret::new("old".to_owned()),
        )
        .await
        .unwrap_err()
    };

    assert_eq!(
        refresh().await,
        TokenError::Transient {
            cause: RunFailure::ServerError { status: 503 },
            retry_after: None
        }
    );
    assert_eq!(
        refresh().await,
        TokenError::Transient {
            cause: RunFailure::RateLimited {
                retry_at: Some(UtcMillis::from_millis(NOW_MILLIS + 20_000))
            },
            retry_after: Some(Duration::from_secs(20))
        }
    );
    assert_eq!(refresh().await, TokenError::InvalidClient);
    assert_eq!(
        refresh().await,
        TokenError::Refused {
            status: 400,
            code: Some("invalid_request".to_owned())
        }
    );
}

#[tokio::test]
async fn an_unreachable_token_endpoint_is_transient() {
    let server = FixtureServer::start().await.unwrap();
    let endpoints = endpoints(&server);
    drop(server);

    let error = refresh_tokens(
        &client(),
        &endpoints,
        "oaiapp_1",
        &Secret::new("old".to_owned()),
    )
    .await
    .unwrap_err();

    assert!(matches!(error, TokenError::Transient { .. }), "{error:?}");
}

#[tokio::test]
async fn a_token_endpoint_redirect_is_refused() {
    let server = FixtureServer::start().await.unwrap();
    let mut redirect = FixtureResponse::json(302, "");
    redirect.headers.push((
        "location".to_owned(),
        "https://elsewhere.example/".to_owned(),
    ));
    server.respond(redirect);

    let error = refresh_tokens(
        &client(),
        &endpoints(&server),
        "oaiapp_1",
        &Secret::new("old".to_owned()),
    )
    .await
    .unwrap_err();

    assert_eq!(
        error,
        TokenError::Refused {
            status: 302,
            code: None
        }
    );
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn revocation_posts_the_refresh_token_and_an_empty_answer_confirms_it() {
    let server = FixtureServer::start().await.unwrap();
    server.respond(FixtureResponse::json(200, ""));

    revoke_refresh_token(
        &client(),
        &endpoints(&server),
        "oaiapp_1",
        &Secret::new("the-refresh-token".to_owned()),
    )
    .await
    .unwrap();

    let request = &server.requests()[0];
    assert_eq!(request.path, "/api/accounts/oauth/revoke");
    let fields = form(request);
    assert_eq!(field(&fields, "token"), Some("the-refresh-token"));
    assert_eq!(field(&fields, "token_type_hint"), Some("refresh_token"));
    assert_eq!(field(&fields, "client_id"), Some("oaiapp_1"));
}

#[tokio::test]
async fn an_id_token_is_checked_against_the_issuers_keys() {
    let server = FixtureServer::start().await.unwrap();
    let endpoints = endpoints(&server);
    let now = NOW_MILLIS / 1_000;
    let claims = json!({
        "iss": endpoints.issuer,
        "aud": "oaiapp_1",
        "sub": "user-123",
        "email": "owner@example.com",
        "nonce": "nonce-1",
        "iat": now,
        "exp": now + 3_600
    });
    let token = Secret::new(signed_id_token("key-1", &claims));
    server.respond(FixtureResponse::json(
        200,
        example_key_set("key-1").to_string(),
    ));
    server.respond(FixtureResponse::json(
        200,
        example_key_set("key-2").to_string(),
    ));
    server.respond(FixtureResponse::json(503, "{}"));

    let claims = verify_id_token(&client(), &endpoints, &token, "oaiapp_1", "nonce-1")
        .await
        .unwrap();
    let unknown = verify_id_token(&client(), &endpoints, &token, "oaiapp_1", "nonce-1")
        .await
        .unwrap_err();
    let unavailable = verify_id_token(&client(), &endpoints, &token, "oaiapp_1", "nonce-1")
        .await
        .unwrap_err();

    assert_eq!(claims.subject, "user-123");
    assert_eq!(claims.email.as_deref(), Some("owner@example.com"));
    assert_eq!(server.requests()[0].path, "/.well-known/jwks.json");
    assert_eq!(unknown, IdTokenError::UnknownKey);
    assert_eq!(
        unavailable,
        IdTokenError::KeysUnavailable {
            failure: RunFailure::BadRequest { status: 503 }
        }
    );
}

#[tokio::test]
async fn the_plan_catalog_is_read_with_the_token_and_refusals_are_classified() {
    let server = FixtureServer::start().await.unwrap();
    server.respond(FixtureResponse::json(
        200,
        json!({
            "models": [
                { "slug": "gpt-6.1-sol", "display_name": "GPT-6.1 Sol", "visibility": "list" },
                { "slug": "hidden", "display_name": "Hidden", "visibility": "hide" }
            ]
        })
        .to_string(),
    ));
    server.respond(FixtureResponse::json(
        401,
        r#"{"error":{"code":"subscription_sharing_invalid_user","message":"no"}}"#,
    ));
    server.respond(FixtureResponse::json(
        429,
        r#"{"error":{"code":"subscription_sharing_usage_limit_exceeded","message":"limit"}}"#,
    ));
    let token = Secret::new("planted-access-token-0123456789".to_owned());
    let api: Url = server.base_url();

    let models = list_plan_models(&client(), &api, &token).await.unwrap();
    let refused = list_plan_models(&client(), &api, &token).await.unwrap_err();
    let limited = list_plan_models(&client(), &api, &token).await.unwrap_err();

    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "gpt-6.1-sol");
    assert_eq!(models[0].display_name.as_deref(), Some("GPT-6.1 Sol"));
    let request = &server.requests()[0];
    assert_eq!(request.path, "/v1/models");
    assert_eq!(
        request.header("authorization"),
        Some("Bearer planted-access-token-0123456789")
    );
    assert_eq!(
        refused,
        ProviderError::Permanent(RunFailure::InvalidCredential)
    );
    assert_eq!(
        limited,
        ProviderError::Permanent(RunFailure::ChatGptUsageLimitReached)
    );
}

#[tokio::test]
async fn a_plan_catalog_longer_than_an_error_body_is_read_whole() {
    let server = FixtureServer::start().await.unwrap();
    // Fields that Mannie does not use make the catalog several times longer
    // than the 64 KiB kept of an error body.
    let instructions = "You are a coding agent. ".repeat(4_000);
    let catalog = json!({
        "models": [
            { "slug": "gpt-6.1-sol", "display_name": "GPT-6.1 Sol", "visibility": "list", "base_instructions": instructions },
            { "slug": "hidden", "display_name": "Hidden", "visibility": "hide", "base_instructions": instructions },
            { "slug": "gpt-6-mini", "display_name": "GPT-6 mini", "visibility": "list", "base_instructions": instructions }
        ]
    })
    .to_string();
    assert!(catalog.len() > 4 * 64 * 1024);
    let mut answer = FixtureResponse::json(200, catalog);
    answer.chunking = Chunking::Random {
        seed: 7,
        max: 16 * 1024,
    };
    server.respond(answer);
    let token = Secret::new("planted-access-token-0123456789".to_owned());

    let models = list_plan_models(&client(), &server.base_url(), &token)
        .await
        .unwrap();

    let names: Vec<(&str, Option<&str>)> = models
        .iter()
        .map(|model| (model.id.as_str(), model.display_name.as_deref()))
        .collect();
    assert_eq!(
        names,
        [
            ("gpt-6.1-sol", Some("GPT-6.1 Sol")),
            ("gpt-6-mini", Some("GPT-6 mini"))
        ]
    );
}

#[tokio::test]
async fn a_plan_catalog_that_is_not_json_or_too_long_is_refused() {
    let server = FixtureServer::start().await.unwrap();
    let mut page = FixtureResponse::json(200, "<!doctype html><title>Moved</title>");
    page.headers = vec![("content-type".to_owned(), "text/html".to_owned())];
    server.respond(page);
    // One byte more than the 16 MiB that Mannie reads of a list of models.
    let padding = "a".repeat(16 * 1024 * 1024 - r#"{"models":[""]}"#.len() + 1);
    server.respond(FixtureResponse::json(
        200,
        format!(r#"{{"models":["{padding}"]}}"#),
    ));
    let token = Secret::new("planted-access-token-0123456789".to_owned());
    let api: Url = server.base_url();

    let not_json = list_plan_models(&client(), &api, &token).await.unwrap_err();
    let too_long = list_plan_models(&client(), &api, &token).await.unwrap_err();

    assert_eq!(
        not_json,
        ProviderError::Permanent(RunFailure::InvalidResponse)
    );
    assert_eq!(
        too_long,
        ProviderError::Permanent(RunFailure::InvalidResponse)
    );
}
