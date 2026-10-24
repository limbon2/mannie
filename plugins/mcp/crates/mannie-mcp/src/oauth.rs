//! Signing in to remote servers with OAuth (plugins/mcp/docs/adr/0002-sign-
//! in-to-remote-servers-with-oauth.md), through rmcp's `auth`: the
//! authorization code flow with PKCE S256 and `state`, discovery of the
//! authorization server through the server's protected resource metadata,
//! the `resource` of RFC 8707, and the `iss` of RFC 9207, which rmcp checks
//! before it redeems the code (docs/roadmap/m12-validation.md, section 4).
//!
//! rmcp's own HTTP client of OAuth follows no redirect to another origin,
//! bounds each response, and reaches a private or loopback address only when
//! the server's URL names one of the same class (T-61). The plugin refuses
//! an authorization server that does not name S256 among its PKCE methods,
//! which rmcp only warns about, and endpoints that a credential may not
//! travel to (ADR-0043, decision 8).
//!
//! The client is the one that the owner registered, or one that the
//! authorization server registers for Mannie as a native application. The
//! tokens and the registration go to the server's OAuth state in the
//! credential store, through [`VaultStore`], and rmcp refreshes them on its
//! own. A sign-in opens no browser: the settings open the page that
//! [`begin`] returns, and the browser comes back to the loopback listener of
//! `callback.rs`.

use std::net::Ipv4Addr;

use async_trait::async_trait;
use mannie_plugin_sdk::contracts::{Secret, Url, credential_transport_allowed};
use rmcp::transport::auth::{
    AuthClient, AuthError, AuthorizationManager, AuthorizationMetadata, AuthorizationRequest,
    AuthorizationSession, CredentialStore, OAuthClientConfig, StoredCredentials,
};
use tokio::net::TcpListener;

use crate::callback::{self, CALLBACK_PATH, CallbackError};
use crate::commands::{LogEvent, McpError, ServerFailure, SignInPage, SignInStarted};
use crate::servers::Servers;
use crate::store::{ServerRecord, StoredEndpoint};

/// The name under which Mannie registers itself with an authorization
/// server.
const CLIENT_NAME: &str = "Mannie";

/// The client of a connection with the server's tokens, and the values
/// that the server's output must not show.
#[derive(Debug)]
pub(crate) struct Authorized {
    pub(crate) client: AuthClient<reqwest::Client>,
    pub(crate) secrets: Vec<Secret>,
}

/// Why a connection has no authorized client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClientError {
    /// No tokens are stored, or the authorization server changed.
    SignInNeeded,
    /// The authorization server could not be reached.
    Unreachable,
    /// The credential store could not be used.
    CredentialStoreUnavailable,
}

/// rmcp's store of credentials over a server's OAuth state in the
/// credential store.
struct VaultStore {
    servers: Servers,
    server_id: String,
}

fn store_error(reason: &str) -> AuthError {
    AuthError::CredentialStoreError(reason.to_owned())
}

#[async_trait]
impl CredentialStore for VaultStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        let record = self
            .servers
            .record(&self.server_id)
            .ok_or_else(|| store_error("the server was removed"))?;
        let bundle = self
            .servers
            .oauth_bundle(&record)
            .await
            .map_err(|_error| store_error("the credential store is not available"))?;
        Ok(bundle
            .and_then(|bundle| bundle.credentials)
            .and_then(|text| serde_json::from_str(text.expose()).ok()))
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let text = serde_json::to_string(&credentials)
            .map_err(|_error| store_error("the credentials do not serialize"))?;
        let text = Secret::new(text);
        // A refresh's tokens stay out of the server's output too.
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(text.expose()) {
            self.servers.redact_too(&self.server_id, tokens_of(&value));
        }
        self.replace(Some(text)).await
    }

    async fn clear(&self) -> Result<(), AuthError> {
        self.replace(None).await
    }
}

impl VaultStore {
    async fn replace(&self, credentials: Option<Secret>) -> Result<(), AuthError> {
        let record = self
            .servers
            .record(&self.server_id)
            .ok_or_else(|| store_error("the server was removed"))?;
        let mut bundle = self
            .servers
            .oauth_bundle(&record)
            .await
            .map_err(|_error| store_error("the credential store is not available"))?
            .unwrap_or_default();
        bundle.credentials = credentials;
        self.servers
            .save_oauth(&self.server_id, &bundle, record.oauth_parts)
            .await
            .map_err(|_error| store_error("the credential store is not available"))
    }
}

/// The URL of a server that signs in with OAuth.
fn oauth_url(record: &ServerRecord) -> Option<&str> {
    match &record.endpoint {
        StoredEndpoint::Url { url } if record.oauth => Some(url),
        _other => None,
    }
}

/// The authorized client of a connection to a server that signed in.
pub(crate) async fn authorized_client(
    servers: &Servers,
    record: &ServerRecord,
) -> Result<Authorized, ClientError> {
    let url = oauth_url(record).ok_or(ClientError::SignInNeeded)?;
    if !record.signed_in {
        return Err(ClientError::SignInNeeded);
    }
    let bundle = servers
        .oauth_bundle(record)
        .await
        .map_err(|_error| ClientError::CredentialStoreUnavailable)?
        .ok_or(ClientError::SignInNeeded)?;
    let credentials = bundle
        .credentials
        .as_ref()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text.expose()).ok())
        .ok_or(ClientError::SignInNeeded)?;
    let mut manager = AuthorizationManager::new(url)
        .await
        .map_err(|_error| ClientError::Unreachable)?;
    manager.set_credential_store(VaultStore {
        servers: servers.clone(),
        server_id: record.id.clone(),
    });
    let initialized = manager
        .initialize_from_store()
        .await
        .map_err(|error| match error {
            AuthError::CredentialStoreError(_reason) => ClientError::CredentialStoreUnavailable,
            _other => ClientError::Unreachable,
        })?;
    if !initialized {
        return Err(ClientError::SignInNeeded);
    }
    let client_id = credentials
        .get("client_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if let Some(client_secret) = &bundle.client_secret {
        // rmcp configures only the client ID from its store, and a
        // confidential client refreshes with its secret.
        let config = OAuthClientConfig::new(client_id, format!("http://127.0.0.1{CALLBACK_PATH}"))
            .with_client_secret(client_secret.expose());
        manager
            .configure_client(config)
            .map_err(|_error| ClientError::SignInNeeded)?;
    }
    let mut secrets = tokens_of(&credentials);
    secrets.extend(bundle.client_secret);
    Ok(Authorized {
        client: AuthClient::new(servers.http().clone(), manager),
        secrets,
    })
}

/// The tokens of stored credentials, as rmcp writes them.
fn tokens_of(credentials: &serde_json::Value) -> Vec<Secret> {
    ["access_token", "refresh_token"]
        .iter()
        .filter_map(|field| {
            credentials
                .pointer(&format!("/token_response/{field}"))
                .and_then(serde_json::Value::as_str)
                .map(|token| Secret::new(token.to_owned()))
        })
        .collect()
}

/// Whether an authorization server offers what a sign-in needs: PKCE with
/// S256, and endpoints that a credential may travel to.
fn is_supported(metadata: &AuthorizationMetadata) -> bool {
    let has_s256 = metadata
        .code_challenge_methods_supported
        .as_ref()
        .is_some_and(|methods| methods.iter().any(|method| method == "S256"));
    let allows =
        |endpoint: &str| Url::parse(endpoint).is_ok_and(|url| credential_transport_allowed(&url));
    let endpoints_allowed = allows(&metadata.authorization_endpoint)
        && allows(&metadata.token_endpoint)
        && metadata.registration_endpoint.as_deref().is_none_or(allows);
    has_s256 && endpoints_allowed
}

/// Starts a sign-in and returns the authorization server's page, which the
/// settings open. A task waits for the browser's return, redeems the code,
/// stores the tokens and connects to the server.
pub(crate) async fn begin(
    servers: &Servers,
    server_id: &str,
    page: SignInPage,
) -> Result<SignInStarted, McpError> {
    let record = servers.record(server_id).ok_or(McpError::ServerNotFound)?;
    let url = oauth_url(&record).ok_or(McpError::NoSignIn)?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|_error| McpError::AuthorizationFailed)?;
    let port = listener
        .local_addr()
        .map_err(|_error| McpError::AuthorizationFailed)?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
    let mut manager = AuthorizationManager::new(url)
        .await
        .map_err(|error| authorization_failed(servers, server_id, &error))?;
    manager.set_credential_store(VaultStore {
        servers: servers.clone(),
        server_id: server_id.to_owned(),
    });
    let resolution = manager
        .resolve_metadata()
        .await
        .map_err(|error| authorization_failed(servers, server_id, &error))?;
    if !is_supported(&resolution.metadata) {
        servers.log_event(
            server_id,
            LogEvent::Failed {
                failure: ServerFailure::AuthorizationUnsupported,
            },
        );
        return Err(McpError::AuthorizationUnsupported);
    }
    manager.set_metadata(resolution.metadata);
    let mut request = AuthorizationRequest::new(redirect_uri).with_client_name(CLIENT_NAME);
    if let Some(client_id) = &record.client_id {
        request = request.with_preregistered_client(client_id.clone());
        let bundle = servers.oauth_bundle(&record).await?.unwrap_or_default();
        if let Some(client_secret) = &bundle.client_secret {
            request = request.with_client_secret(client_secret.expose());
        }
    }
    let session =
        AuthorizationSession::new(manager, request)
            .await
            .map_err(|(_manager, error)| {
                if matches!(error, AuthError::RegistrationFailed(_)) && record.client_id.is_none() {
                    return McpError::ClientIdNeeded;
                }
                authorization_failed(servers, server_id, &error)
            })?;
    let page_url = session.get_authorization_url().to_owned();
    let parsed = Url::parse(&page_url).map_err(|_error| McpError::AuthorizationFailed)?;
    if !credential_transport_allowed(&parsed) {
        return Err(McpError::AuthorizationUnsupported);
    }
    let state = parsed
        .query_pairs()
        .find(|(name, _value)| name == "state")
        .map(|(_name, value)| value.into_owned())
        .ok_or(McpError::AuthorizationFailed)?;
    let host = parsed.host_str().unwrap_or_default().to_owned();
    let finishing = tokio::spawn(finish(
        servers.clone(),
        server_id.to_owned(),
        session,
        listener,
        state,
        page,
    ));
    servers.replace_sign_in(server_id, finishing.abort_handle());
    tracing::info!(server = %server_id, "a sign-in to an MCP server started");
    Ok(SignInStarted {
        url: page_url,
        host,
    })
}

/// Waits for the browser's return, redeems the code and connects.
async fn finish(
    servers: Servers,
    server_id: String,
    session: AuthorizationSession,
    listener: TcpListener,
    state: String,
    page: SignInPage,
) {
    let redirect = match callback::wait(listener, &state, &page).await {
        Ok(redirect) => redirect,
        Err(error) => {
            tracing::info!(server = %server_id, ?error, "a sign-in to an MCP server ended without a code");
            if error != CallbackError::TimedOut {
                servers.log_event(
                    &server_id,
                    LogEvent::Failed {
                        failure: ServerFailure::SignInFailed,
                    },
                );
            }
            return;
        }
    };
    let redeemed = session
        .handle_callback_with_issuer(&redirect.code, &state, redirect.issuer.as_deref())
        .await;
    let signed_in = redeemed.is_ok();
    redirect.answer(&page, signed_in).await;
    if let Err(error) = redeemed {
        tracing::warn!(server = %server_id, error = %error_kind(&error), "a sign-in to an MCP server failed");
        servers.log_event(
            &server_id,
            LogEvent::Failed {
                failure: ServerFailure::SignInFailed,
            },
        );
        return;
    }
    servers.log_event(&server_id, LogEvent::SignedIn);
    tracing::info!(server = %server_id, "the owner signed in to an MCP server");
    if let Err(error) = servers.restart(&server_id).await {
        tracing::debug!(server = %server_id, ?error, "an MCP server was not restarted after its sign-in");
    }
}

/// What a failure of OAuth means for the owner, logged without its text,
/// which may quote a server's answer.
fn authorization_failed(servers: &Servers, server_id: &str, error: &AuthError) -> McpError {
    tracing::warn!(server = %server_id, error = %error_kind(error), "an authorization server failed");
    servers.log_event(
        server_id,
        LogEvent::Failed {
            failure: ServerFailure::SignInFailed,
        },
    );
    McpError::AuthorizationFailed
}

/// The kind of an error of OAuth, for the log.
fn error_kind(error: &AuthError) -> &'static str {
    match error {
        AuthError::AuthorizationRequired => "authorization_required",
        AuthError::AuthorizationFailed(_) => "authorization_failed",
        AuthError::TokenExchangeFailed(_) => "token_exchange_failed",
        AuthError::TokenRefreshFailed(_) | AuthError::TokenRefreshRejected(_) => {
            "token_refresh_failed"
        }
        AuthError::CredentialStoreError(_) => "credential_store",
        AuthError::HttpError(_) => "http",
        AuthError::MetadataError(_) => "metadata",
        AuthError::PkceUnsupported => "pkce_unsupported",
        AuthError::RegistrationFailed(_) => "registration_failed",
        AuthError::AuthorizationServerMismatch { .. }
        | AuthError::AuthorizationServerMissingIssuer { .. } => "issuer_mismatch",
        _other => "oauth",
    }
}

#[cfg(test)]
mod tests {
    use rmcp::transport::auth::AuthorizationMetadata;

    use super::is_supported;

    fn metadata(methods: Option<&[&str]>, token_endpoint: &str) -> AuthorizationMetadata {
        serde_json::from_value(serde_json::json!({
            "authorization_endpoint": "https://auth.example.com/authorize",
            "token_endpoint": token_endpoint,
            "issuer": "https://auth.example.com",
            "code_challenge_methods_supported": methods,
        }))
        .unwrap()
    }

    #[test]
    fn a_sign_in_needs_s256_and_endpoints_that_a_credential_may_travel_to() {
        assert!(is_supported(&metadata(
            Some(&["S256"]),
            "https://auth.example.com/token"
        )));
        assert!(!is_supported(&metadata(
            Some(&["plain"]),
            "https://auth.example.com/token"
        )));
        assert!(!is_supported(&metadata(
            None,
            "https://auth.example.com/token"
        )));
        assert!(!is_supported(&metadata(
            Some(&["S256"]),
            "http://auth.example.com/token"
        )));
        assert!(is_supported(&metadata(
            Some(&["S256"]),
            "http://127.0.0.1:9000/token"
        )));
    }
}
