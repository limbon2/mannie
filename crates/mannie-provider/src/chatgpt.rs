//! Signing in with ChatGPT, so that profiles call OpenAI on the owner's
//! ChatGPT plan (ADR-0062).
//!
//! OpenAI's sign-in is OAuth 2.0 with PKCE for a public client, as
//! <https://developers.openai.com/siwc/token-sharing-open-source/sign-in>
//! describes it (verified 2026-11-08). The first sign-in registers Mannie
//! through the authorization request itself, and the browser's return names
//! the client that OpenAI issued; later sign-ins reuse that client. Access
//! tokens last an hour, and every refresh replaces the refresh token.
//!
//! The flow is written on the shared HTTP client instead of an OAuth library:
//! its registration through `dynamic_agent_client`, `ext_agent_host_id` and
//! `resource` are OpenAI's own, and the oauth2 crate belongs to the MCP plugin
//! (ADR-0062, decision 3).

mod callback;
#[cfg(any(test, feature = "testing"))]
pub(crate) mod example_key;
mod id_token;
mod oauth;

use reqwest::Url;
use ring::rand::SecureRandom as _;

pub use callback::{Callback, CallbackError, CallbackListener, ListenError};
pub use id_token::{IdClaims, IdTokenError, verify_id_token};
pub use oauth::{
    Authorization, AuthorizationRequest, TokenError, TokenSet, authorization, exchange_code,
    redirect_uri, refresh_tokens, revoke_refresh_token,
};

/// The issuer of OpenAI's sign-in, as its ID tokens name it.
pub const OPENAI_ISSUER: &str = "https://auth.openai.com";

/// The base URL of OpenAI's API, which profiles on the ChatGPT plan call.
pub const OPENAI_API_BASE: &str = "https://api.openai.com/v1";

/// The resource that the tokens are for.
pub const RESOURCE: &str = "https://api.openai.com/v1";

/// The loopback port of the first sign-in, which registers Mannie: the port
/// of OpenAI's guide, `http://127.0.0.1:1455/auth/callback`. Later sign-ins
/// may use any free port; only the port may vary.
pub const REGISTRATION_PORT: u16 = 1455;

/// The path of the browser's return.
pub const CALLBACK_PATH: &str = "/auth/callback";

/// The scope that allows Mannie to bill the ChatGPT plan.
pub const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";

/// The scopes that a sign-in asks for: the account's identity, a refresh
/// token, and the use of the ChatGPT plan.
pub const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";

/// Where Mannie signs in and calls the ChatGPT plan: OpenAI's servers, or a
/// validation build's mock server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatGptEndpoints {
    /// The issuer that ID tokens must name.
    pub issuer: String,
    /// The authorization endpoint, which the browser opens.
    pub authorize: Url,
    /// The token endpoint.
    pub token: Url,
    /// The revocation endpoint.
    pub revoke: Url,
    /// The issuer's signing keys.
    pub jwks: Url,
    /// The base URL of the Responses API and the catalog of models.
    pub api_base: Url,
    /// The loopback port of the first sign-in's return: 1455 for OpenAI, as
    /// its guide gives it, and 0, any free port, for tests and validation
    /// builds that run beside each other.
    pub registration_port: u16,
}

impl ChatGptEndpoints {
    /// OpenAI's endpoints, as its discovery document named them on
    /// 2026-11-08.
    ///
    /// # Panics
    ///
    /// Never: the constants are valid URLs.
    pub fn openai() -> Self {
        let issuer = Url::parse(OPENAI_ISSUER).expect("the issuer is a valid URL");
        let api_base = Url::parse(OPENAI_API_BASE).expect("the API base is a valid URL");
        Self::at(&issuer, &api_base)
    }

    /// The endpoints of an authorization server at `issuer`, on OpenAI's
    /// paths, and of the API at `api_base`.
    pub fn at(issuer: &Url, api_base: &Url) -> Self {
        let below = |path: &str| {
            let mut url = issuer.clone();
            url.set_path(path);
            url.set_query(None);
            url.set_fragment(None);
            url
        };
        Self {
            issuer: issuer.as_str().trim_end_matches('/').to_owned(),
            authorize: below("/api/accounts/authorize"),
            token: below("/api/accounts/oauth/token"),
            revoke: below("/api/accounts/oauth/revoke"),
            jwks: below("/.well-known/jwks.json"),
            api_base: api_base.clone(),
            registration_port: REGISTRATION_PORT,
        }
    }
}

/// The operating system gave no random bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the operating system gave no random bytes")]
pub struct RandomUnavailable;

/// Random bytes from the operating system, through ring.
fn random_bytes<const N: usize>() -> Result<[u8; N], RandomUnavailable> {
    let mut bytes = [0_u8; N];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_unspecified| RandomUnavailable)?;
    Ok(bytes)
}

/// A new ID of this installation for OpenAI's `ext_agent_host_id`: `urn:uuid:`
/// and a random UUID of version 4, one of the forms that OpenAI accepts. It
/// identifies nothing about the owner. The IDs that Mannie makes for itself
/// are UUIDv7; this one follows OpenAI's format.
pub fn new_host_id() -> Result<String, RandomUnavailable> {
    let uuid = uuid::Builder::from_random_bytes(random_bytes::<16>()?).into_uuid();
    Ok(format!("urn:uuid:{}", uuid.hyphenated()))
}

/// A host ID as Mannie stores it: `urn:uuid:` and a hyphenated UUID.
pub fn is_host_id(text: &str) -> bool {
    text.strip_prefix("urn:uuid:")
        .and_then(|uuid| uuid::Uuid::try_parse(uuid).ok())
        .is_some_and(|uuid| uuid.get_version_num() == 4)
}

#[cfg(test)]
mod tests {
    use reqwest::Url;

    use super::{ChatGptEndpoints, is_host_id, new_host_id};

    #[test]
    fn openai_endpoints_are_the_documented_ones() {
        let endpoints = ChatGptEndpoints::openai();

        assert_eq!(endpoints.issuer, "https://auth.openai.com");
        assert_eq!(
            endpoints.authorize.as_str(),
            "https://auth.openai.com/api/accounts/authorize"
        );
        assert_eq!(
            endpoints.token.as_str(),
            "https://auth.openai.com/api/accounts/oauth/token"
        );
        assert_eq!(
            endpoints.revoke.as_str(),
            "https://auth.openai.com/api/accounts/oauth/revoke"
        );
        assert_eq!(
            endpoints.jwks.as_str(),
            "https://auth.openai.com/.well-known/jwks.json"
        );
        assert_eq!(endpoints.api_base.as_str(), "https://api.openai.com/v1");
        assert_eq!(endpoints.registration_port, 1455);
    }

    #[test]
    fn endpoints_of_another_server_keep_its_origin() {
        let issuer = Url::parse("http://127.0.0.1:4000/").unwrap();
        let api = Url::parse("http://127.0.0.1:4000/v1").unwrap();

        let endpoints = ChatGptEndpoints::at(&issuer, &api);

        assert_eq!(endpoints.issuer, "http://127.0.0.1:4000");
        assert_eq!(
            endpoints.token.as_str(),
            "http://127.0.0.1:4000/api/accounts/oauth/token"
        );
    }

    #[test]
    fn host_ids_are_random_version_4_uuids() {
        let first = new_host_id().unwrap();
        let second = new_host_id().unwrap();

        assert!(is_host_id(&first), "{first}");
        assert!(first.starts_with("urn:uuid:"));
        assert_ne!(first, second);
        assert!(!is_host_id("urn:uuid:0192f3a4-5b6c-7d8e-9f00-112233445566"));
        assert!(!is_host_id("did:key:z6Mk"));
    }
}
