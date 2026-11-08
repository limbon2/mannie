//! The OAuth requests of a ChatGPT sign-in: the authorization URL with PKCE,
//! and the token endpoint's code exchange, refresh and revocation.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use mannie_types::run::RunFailure;
use mannie_types::secret::Secret;
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;
use zeroize::Zeroizing;

use super::{CALLBACK_PATH, ChatGptEndpoints, RESOURCE, RandomUnavailable, SCOPES, random_bytes};
use crate::errors::retry_after;
use crate::provider::ProviderError;
use crate::stream::{read_error_body, transport_failure};
use crate::transport::HttpClient;

/// How long a request to the token or revocation endpoint may take in all.
const TOKEN_TIMEOUT: Duration = Duration::from_secs(30);

/// The client that a first sign-in names, which registers Mannie. It is the
/// entry point of the registration, never a client to keep.
const REGISTRATION_CLIENT: &str = "dynamic_agent_client";

/// The name that a first sign-in suggests for Mannie's registration.
const AGENT_NAME: &str = "Mannie";

/// The error codes of a refresh token that is gone for good, after which the
/// owner signs in again (verified 2026-11-08,
/// <https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions>).
const TERMINAL_CODES: &[&str] = &[
    "invalid_grant",
    "invalid_refresh_token",
    "token_expired",
    "refresh_token_expired",
    "refresh_token_invalidated",
    "refresh_token_reused",
];

/// The most characters of an error code that Mannie keeps.
const MAX_CODE_CHARACTERS: usize = 64;

/// What a sign-in's authorization URL is made from.
#[derive(Debug, Clone, Copy)]
pub struct AuthorizationRequest<'a> {
    /// Where Mannie signs in.
    pub endpoints: &'a ChatGptEndpoints,
    /// This installation's host ID.
    pub host_id: &'a str,
    /// The loopback port of the browser's return.
    pub port: u16,
    /// The client that OpenAI issued at an earlier sign-in, or `None` to
    /// register Mannie.
    pub client_id: Option<&'a str>,
    /// The email address of the account to sign in to again, if known.
    pub login_hint: Option<&'a str>,
}

/// A sign-in's authorization: the URL that the browser opens, and what the
/// browser's return is checked against and redeemed with.
#[derive(Debug, Clone)]
pub struct Authorization {
    /// The URL that the browser opens.
    pub url: Url,
    /// The `state` that the return must carry.
    pub state: String,
    /// The `nonce` that the ID token must carry.
    pub nonce: String,
    /// The PKCE verifier that redeems the code.
    pub verifier: Secret,
    /// The redirect URI, which the code exchange repeats.
    pub redirect_uri: String,
}

/// The redirect URI of a sign-in whose listener has `port`. Only the port may
/// differ between sign-ins.
pub fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}{CALLBACK_PATH}")
}

/// A random value for `state`, `nonce` or a PKCE verifier: 32 random bytes,
/// base64url-encoded into 43 characters.
fn random_value() -> Result<String, RandomUnavailable> {
    Ok(URL_SAFE_NO_PAD.encode(random_bytes::<32>()?))
}

/// Makes a sign-in's authorization: a fresh `state`, `nonce` and PKCE
/// verifier, and the URL with OpenAI's parameters. A first sign-in names the
/// registration client and Mannie's name; a later one names the issued
/// client and, when known, the account.
pub fn authorization(
    request: &AuthorizationRequest<'_>,
) -> Result<Authorization, RandomUnavailable> {
    let state = random_value()?;
    let nonce = random_value()?;
    let verifier = Zeroizing::new(random_value()?);
    let challenge = URL_SAFE_NO_PAD.encode(ring::digest::digest(
        &ring::digest::SHA256,
        verifier.as_bytes(),
    ));
    let redirect_uri = redirect_uri(request.port);
    let mut url = request.endpoints.authorize.clone();
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair(
            "client_id",
            request.client_id.unwrap_or(REGISTRATION_CLIENT),
        );
        query.append_pair("redirect_uri", &redirect_uri);
        query.append_pair("scope", SCOPES);
        query.append_pair("resource", RESOURCE);
        query.append_pair("state", &state);
        query.append_pair("nonce", &nonce);
        query.append_pair("code_challenge", &challenge);
        query.append_pair("code_challenge_method", "S256");
        query.append_pair("ext_agent_host_id", request.host_id);
        if request.client_id.is_none() {
            query.append_pair("agent_name_hint", AGENT_NAME);
        }
        if let Some(hint) = request.login_hint {
            query.append_pair("login_hint", hint);
        }
    }
    Ok(Authorization {
        url,
        state,
        nonce,
        verifier: Secret::new(verifier.to_string()),
        redirect_uri,
    })
}

/// What the token endpoint returned. The tokens are secrets from the moment
/// they are read.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenSet {
    /// The access token, which calls send as a bearer token.
    pub access_token: Secret,
    /// The new refresh token, when the endpoint sent one.
    pub refresh_token: Option<Secret>,
    /// The ID token, which names the account.
    pub id_token: Option<Secret>,
    /// How long the access token lasts, in seconds.
    pub expires_in: Option<u64>,
    /// The scopes that the account granted, separated by spaces.
    pub scope: Option<String>,
}

impl TokenSet {
    /// Whether the account granted `scope`.
    pub fn grants(&self, scope: &str) -> bool {
        self.scope
            .as_deref()
            .is_some_and(|scopes| scopes.split_whitespace().any(|granted| granted == scope))
    }
}

/// Why a request to the token or revocation endpoint failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    /// The grant is gone for good, such as an expired, revoked or reused
    /// refresh token: the owner signs in again.
    #[error("the token endpoint ended the grant ({code})")]
    Terminal {
        /// The endpoint's error code.
        code: String,
    },
    /// The endpoint refused Mannie's client.
    #[error("the token endpoint refused the client")]
    InvalidClient,
    /// The endpoint refused the request in another way.
    #[error("the token endpoint refused the request with status {status}")]
    Refused {
        /// The HTTP status.
        status: u16,
        /// The endpoint's error code, cleaned, if it named one.
        code: Option<String>,
    },
    /// The endpoint could not be reached, did not answer in time, or failed
    /// for now. The tokens stay.
    #[error("the token endpoint failed for now: {cause}")]
    Transient {
        /// Why.
        cause: RunFailure,
        /// How long the endpoint asked to wait.
        retry_after: Option<Duration>,
    },
    /// The answer has no usable tokens.
    #[error("the token endpoint's answer has no usable tokens")]
    InvalidResponse,
}

/// Redeems the code of a sign-in.
pub async fn exchange_code(
    client: &HttpClient,
    endpoints: &ChatGptEndpoints,
    client_id: &str,
    code: &Secret,
    authorization: &Authorization,
) -> Result<TokenSet, TokenError> {
    let form = [
        ("grant_type", "authorization_code"),
        ("client_id", client_id),
        ("code", code.expose()),
        ("code_verifier", authorization.verifier.expose()),
        ("redirect_uri", authorization.redirect_uri.as_str()),
        ("resource", RESOURCE),
    ];
    let body = post(client, endpoints.token.clone(), &form).await?;
    token_set(&body)
}

/// Refreshes the tokens. The answer's refresh token replaces the one sent,
/// which stops working.
pub async fn refresh_tokens(
    client: &HttpClient,
    endpoints: &ChatGptEndpoints,
    client_id: &str,
    refresh_token: &Secret,
) -> Result<TokenSet, TokenError> {
    // Without `scope`, the refresh keeps the grant as it is.
    let form = [
        ("grant_type", "refresh_token"),
        ("client_id", client_id),
        ("refresh_token", refresh_token.expose()),
        ("resource", RESOURCE),
    ];
    let body = post(client, endpoints.token.clone(), &form).await?;
    token_set(&body)
}

/// Revokes a refresh token, which ends the session at OpenAI. An empty answer
/// with status 200 confirms it, also for a token that was no longer valid.
pub async fn revoke_refresh_token(
    client: &HttpClient,
    endpoints: &ChatGptEndpoints,
    client_id: &str,
    refresh_token: &Secret,
) -> Result<(), TokenError> {
    let form = [
        ("token", refresh_token.expose()),
        ("token_type_hint", "refresh_token"),
        ("client_id", client_id),
    ];
    post(client, endpoints.revoke.clone(), &form).await?;
    Ok(())
}

/// The tokens in an answer.
fn token_set(body: &[u8]) -> Result<TokenSet, TokenError> {
    let tokens: TokenSet =
        serde_json::from_slice(body).map_err(|_not_tokens| TokenError::InvalidResponse)?;
    if tokens.access_token.expose().is_empty() {
        return Err(TokenError::InvalidResponse);
    }
    Ok(tokens)
}

/// POSTs a form and returns the body of a successful answer. The body can
/// hold tokens, so it is cleared when dropped.
async fn post(
    client: &HttpClient,
    url: Url,
    form: &[(&str, &str)],
) -> Result<Zeroizing<Vec<u8>>, TokenError> {
    let body = Zeroizing::new(
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish(),
    );
    let sending = client.post_form(url, &body).send();
    let response = tokio::time::timeout(TOKEN_TIMEOUT, sending)
        .await
        .map_err(|_elapsed| TokenError::Transient {
            cause: RunFailure::TimedOut,
            retry_after: None,
        })?
        .map_err(|error| match transport_failure(&error) {
            ProviderError::Transient { cause, retry_after } => {
                TokenError::Transient { cause, retry_after }
            }
            ProviderError::ContextOverflow | ProviderError::Permanent(_) => {
                TokenError::InvalidResponse
            }
        })?;
    let status = response.status().as_u16();
    let wait = retry_after(response.headers(), client.clock().now());
    let body = Zeroizing::new(read_error_body(response, TOKEN_TIMEOUT).await);
    if (200..300).contains(&status) {
        return Ok(body);
    }
    let code = error_code(&body);
    tracing::debug!(
        status,
        code = code.as_deref(),
        "the token endpoint refused a request"
    );
    Err(classify(status, code, wait, client))
}

/// The class of a refused request: a terminal code ends the grant whatever
/// the status; rate limits and server errors are transient.
fn classify(
    status: u16,
    code: Option<String>,
    wait: Option<Duration>,
    client: &HttpClient,
) -> TokenError {
    if let Some(code) = code.as_deref() {
        if TERMINAL_CODES.contains(&code) {
            return TokenError::Terminal {
                code: code.to_owned(),
            };
        }
        if code == "invalid_client" {
            return TokenError::InvalidClient;
        }
    }
    match status {
        408 => TokenError::Transient {
            cause: RunFailure::TimedOut,
            retry_after: wait,
        },
        429 => TokenError::Transient {
            cause: RunFailure::RateLimited {
                retry_at: wait.map(|wait| client.clock().now().saturating_add(wait)),
            },
            retry_after: wait,
        },
        500..=599 => TokenError::Transient {
            cause: RunFailure::ServerError { status },
            retry_after: wait,
        },
        _ => TokenError::Refused { status, code },
    }
}

/// The error code of an answer, in OAuth's shape (`{"error": "..."}`) or
/// OpenAI's (`{"error": {"code": "..."}}`), cleaned to letters, digits, `_`,
/// `.` and `-` for logs.
fn error_code(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let error = value.get("error")?;
    let code = error
        .as_str()
        .or_else(|| error.get("code").and_then(Value::as_str))?;
    let cleaned: String = code
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-')
        })
        .take(MAX_CODE_CHARACTERS)
        .collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

#[cfg(test)]
mod tests {
    use mannie_types::secret::Secret;
    use reqwest::Url;

    use super::{AuthorizationRequest, TokenError, TokenSet, authorization, error_code, token_set};
    use crate::chatgpt::{ChatGptEndpoints, SCOPES};

    fn query(url: &Url, name: &str) -> Option<String> {
        url.query_pairs()
            .find(|(key, _value)| key == name)
            .map(|(_key, value)| value.into_owned())
    }

    #[test]
    fn a_first_sign_in_registers_mannie_on_the_fixed_port() {
        let endpoints = ChatGptEndpoints::openai();
        let request = AuthorizationRequest {
            endpoints: &endpoints,
            host_id: "urn:uuid:9f0c1d6e-3a52-4b8e-9d4c-2f1e8a7b6c5d",
            port: 1455,
            client_id: None,
            login_hint: None,
        };

        let authorization = authorization(&request).unwrap();
        let url = &authorization.url;

        assert!(
            url.as_str()
                .starts_with("https://auth.openai.com/api/accounts/authorize?")
        );
        assert_eq!(
            query(url, "client_id").as_deref(),
            Some("dynamic_agent_client")
        );
        assert_eq!(query(url, "agent_name_hint").as_deref(), Some("Mannie"));
        assert_eq!(
            query(url, "redirect_uri").as_deref(),
            Some("http://127.0.0.1:1455/auth/callback")
        );
        assert_eq!(query(url, "scope").as_deref(), Some(SCOPES));
        assert_eq!(
            query(url, "resource").as_deref(),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(query(url, "response_type").as_deref(), Some("code"));
        assert_eq!(query(url, "code_challenge_method").as_deref(), Some("S256"));
        assert_eq!(
            query(url, "ext_agent_host_id").as_deref(),
            Some(request.host_id)
        );
        assert_eq!(
            query(url, "state").as_deref(),
            Some(authorization.state.as_str())
        );
        assert_eq!(
            query(url, "nonce").as_deref(),
            Some(authorization.nonce.as_str())
        );
        assert_eq!(query(url, "login_hint"), None);
        assert_eq!(
            authorization.redirect_uri,
            "http://127.0.0.1:1455/auth/callback"
        );
    }

    #[test]
    fn a_later_sign_in_names_its_client_and_account() {
        let endpoints = ChatGptEndpoints::openai();
        let request = AuthorizationRequest {
            endpoints: &endpoints,
            host_id: "urn:uuid:9f0c1d6e-3a52-4b8e-9d4c-2f1e8a7b6c5d",
            port: 54_321,
            client_id: Some("oaiapp_123"),
            login_hint: Some("owner@example.com"),
        };

        let authorization = authorization(&request).unwrap();
        let url = &authorization.url;

        assert_eq!(query(url, "client_id").as_deref(), Some("oaiapp_123"));
        assert_eq!(query(url, "agent_name_hint"), None);
        assert_eq!(
            query(url, "login_hint").as_deref(),
            Some("owner@example.com")
        );
        assert_eq!(
            query(url, "redirect_uri").as_deref(),
            Some("http://127.0.0.1:54321/auth/callback")
        );
    }

    #[test]
    fn the_challenge_is_the_s256_of_the_verifier_and_values_are_fresh() {
        use base64::Engine as _;

        let endpoints = ChatGptEndpoints::openai();
        let request = AuthorizationRequest {
            endpoints: &endpoints,
            host_id: "urn:uuid:9f0c1d6e-3a52-4b8e-9d4c-2f1e8a7b6c5d",
            port: 1455,
            client_id: None,
            login_hint: None,
        };

        let first = authorization(&request).unwrap();
        let second = authorization(&request).unwrap();
        let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            ring::digest::digest(&ring::digest::SHA256, first.verifier.expose().as_bytes()),
        );

        assert_eq!(query(&first.url, "code_challenge"), Some(expected));
        assert_eq!(first.verifier.expose().len(), 43);
        assert_ne!(first.state, second.state);
        assert_ne!(first.nonce, second.nonce);
        assert_ne!(first.verifier.expose(), second.verifier.expose());
        assert!(!first.url.as_str().contains(first.verifier.expose()));
    }

    #[test]
    fn a_token_answer_becomes_secrets_and_its_scopes() {
        let body = br#"{"access_token":"at-1","refresh_token":"rt-1","id_token":"it-1","expires_in":3600,"scope":"openid chatgpt.tokens.use.direct offline_access","token_type":"Bearer","earliest_refresh_at":0}"#;

        let tokens: TokenSet = token_set(body).unwrap();

        assert_eq!(tokens.access_token.expose(), "at-1");
        assert_eq!(
            tokens.refresh_token.as_ref().map(Secret::expose),
            Some("rt-1")
        );
        assert_eq!(tokens.expires_in, Some(3600));
        assert!(tokens.grants("chatgpt.tokens.use.direct"));
        assert!(!tokens.grants("chatgpt.tokens"));
        assert_eq!(
            token_set(br#"{"access_token":""}"#).unwrap_err(),
            TokenError::InvalidResponse
        );
        assert_eq!(
            token_set(br#"{"refresh_token":"rt"}"#).unwrap_err(),
            TokenError::InvalidResponse
        );
        assert!(!format!("{tokens:?}").contains("at-1"));
    }

    #[test]
    fn error_codes_are_read_in_both_shapes_and_cleaned() {
        assert_eq!(
            error_code(br#"{"error":"invalid_grant","error_description":"x"}"#).as_deref(),
            Some("invalid_grant")
        );
        assert_eq!(
            error_code(br#"{"error":{"code":"refresh_token_reused","message":"x"}}"#).as_deref(),
            Some("refresh_token_reused")
        );
        assert_eq!(
            error_code(br#"{"error":"bad\ncode<script>"}"#).as_deref(),
            Some("badcodescript")
        );
        assert_eq!(error_code(b"not json"), None);
    }
}
