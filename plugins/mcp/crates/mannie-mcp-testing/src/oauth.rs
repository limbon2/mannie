//! A fake authorization server on the remote server's own origin, as many
//! remote MCP servers sign in: protected resource metadata (RFC 9728),
//! authorization server metadata (RFC 8414), dynamic client registration,
//! an authorization endpoint that redirects at once, as a browser whose
//! owner agreed would, with the `iss` of RFC 9207, and a token endpoint that
//! checks PKCE with S256 and refreshes tokens. The endpoint of the MCP
//! server takes only the access tokens that it issued.
//!
//! [`sign_in_through`] plays the browser: it opens the authorization page
//! and follows its redirect to the client's loopback listener.

use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt::Write as _;
use std::sync::{Mutex, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt as _, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Request, Response, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;

/// An answer of the server.
pub(crate) type Answer = Response<BoxBody<Bytes, Infallible>>;

/// How the fake authorization server answers.
#[derive(Debug, Clone)]
pub struct OAuthOptions {
    /// Whether its metadata names S256 among its PKCE methods, without which
    /// a client must not sign in.
    pub s256: bool,
    /// The seconds that an access token lasts.
    pub expires_in: u64,
}

impl Default for OAuthOptions {
    fn default() -> Self {
        Self {
            s256: true,
            expires_in: 3600,
        }
    }
}

/// The client ID that registration gives.
pub const REGISTERED_CLIENT: &str = "registered-client";

/// A code that the authorization endpoint gave, with what redeeming it
/// must match.
#[derive(Debug, Clone)]
struct Grant {
    challenge: String,
    client_id: String,
    redirect_uri: String,
}

#[derive(Debug, Default)]
struct Issued {
    codes: HashMap<String, Grant>,
    access: Vec<String>,
    refresh: Vec<String>,
    counter: u64,
}

/// The fake authorization server.
#[derive(Debug)]
pub(crate) struct Authorization {
    origin: String,
    options: OAuthOptions,
    issued: Mutex<Issued>,
}

impl Authorization {
    pub(crate) fn new(origin: String, options: OAuthOptions) -> Self {
        Self {
            origin,
            options,
            issued: Mutex::new(Issued::default()),
        }
    }

    fn issued(&self) -> std::sync::MutexGuard<'_, Issued> {
        self.issued.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The access tokens that it issued.
    pub(crate) fn access_tokens(&self) -> Vec<String> {
        self.issued().access.clone()
    }

    /// Whether a path belongs to the authorization server.
    pub(crate) fn handles(path: &str) -> bool {
        path.starts_with("/.well-known/") || matches!(path, "/authorize" | "/token" | "/register")
    }

    /// Whether a request carries an access token that it issued.
    pub(crate) fn takes(&self, request: &Request<Incoming>) -> bool {
        let token = request
            .headers()
            .get(hyper::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        token.is_some_and(|token| self.issued().access.iter().any(|issued| issued == token))
    }

    /// The answer to a request without an access token that it issued.
    pub(crate) fn challenge(&self) -> Answer {
        let mut response = text(StatusCode::UNAUTHORIZED, "sign in first");
        let challenge = format!(
            "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource/mcp\"",
            self.origin
        );
        if let Ok(value) = hyper::header::HeaderValue::from_str(&challenge) {
            response
                .headers_mut()
                .insert(hyper::header::WWW_AUTHENTICATE, value);
        }
        response
    }

    /// The answer to a request of the authorization server.
    pub(crate) async fn answer(&self, request: Request<Incoming>) -> Answer {
        let path = request.uri().path().to_owned();
        let query = request.uri().query().unwrap_or("").to_owned();
        if path.starts_with("/.well-known/oauth-protected-resource") {
            return self.protected_resource();
        }
        let is_metadata = path.starts_with("/.well-known/oauth-authorization-server")
            || path.starts_with("/.well-known/openid-configuration");
        if is_metadata {
            return self.metadata();
        }
        match path.as_str() {
            "/register" => Self::register(request).await,
            "/authorize" => self.authorize(&query),
            "/token" => self.token(request).await,
            _other => text(StatusCode::NOT_FOUND, "not found"),
        }
    }

    fn protected_resource(&self) -> Answer {
        let origin = &self.origin;
        json_answer(
            StatusCode::OK,
            &json!({
                "resource": format!("{origin}/mcp"),
                "authorization_servers": [origin],
                "scopes_supported": ["tools"],
            }),
        )
    }

    fn metadata(&self) -> Answer {
        let origin = &self.origin;
        let methods = if self.options.s256 { "S256" } else { "plain" };
        json_answer(
            StatusCode::OK,
            &json!({
                "issuer": origin,
                "authorization_endpoint": format!("{origin}/authorize"),
                "token_endpoint": format!("{origin}/token"),
                "registration_endpoint": format!("{origin}/register"),
                "code_challenge_methods_supported": [methods],
                "response_types_supported": ["code"],
                "grant_types_supported": ["authorization_code", "refresh_token"],
                "token_endpoint_auth_methods_supported": ["none"],
                "scopes_supported": ["tools"],
                "authorization_response_iss_parameter_supported": true,
            }),
        )
    }

    async fn register(request: Request<Incoming>) -> Answer {
        let body = body_of(request).await;
        let asked: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        json_answer(
            StatusCode::CREATED,
            &json!({
                "client_id": REGISTERED_CLIENT,
                "client_name": asked["client_name"],
                "redirect_uris": asked["redirect_uris"],
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "none",
            }),
        )
    }

    fn authorize(&self, query: &str) -> Answer {
        let parameters = form(query);
        let find = |name: &str| {
            parameters
                .iter()
                .find(|(key, _value)| key == name)
                .map(|(_key, value)| value.clone())
                .unwrap_or_default()
        };
        let redirect_uri = find("redirect_uri");
        let is_valid = find("response_type") == "code" && find("code_challenge_method") == "S256";
        if !is_valid || redirect_uri.is_empty() {
            return text(StatusCode::BAD_REQUEST, "invalid request");
        }
        let code = {
            let mut issued = self.issued();
            issued.counter += 1;
            let code = format!("code-{}", issued.counter);
            issued.codes.insert(
                code.clone(),
                Grant {
                    challenge: find("code_challenge"),
                    client_id: find("client_id"),
                    redirect_uri: redirect_uri.clone(),
                },
            );
            code
        };
        let location = format!(
            "{redirect_uri}?code={code}&state={}&iss={}",
            encoded(&find("state")),
            encoded(&self.origin)
        );
        let mut response = text(StatusCode::FOUND, "");
        if let Ok(value) = hyper::header::HeaderValue::from_str(&location) {
            response
                .headers_mut()
                .insert(hyper::header::LOCATION, value);
        }
        response
    }

    async fn token(&self, request: Request<Incoming>) -> Answer {
        let body = body_of(request).await;
        let parameters = form(&String::from_utf8_lossy(&body));
        let find = |name: &str| {
            parameters
                .iter()
                .find(|(key, _value)| key == name)
                .map(|(_key, value)| value.clone())
                .unwrap_or_default()
        };
        let granted = match find("grant_type").as_str() {
            "authorization_code" => self.redeem(
                &find("code"),
                &find("code_verifier"),
                &find("redirect_uri"),
                &find("client_id"),
            ),
            "refresh_token" => self.refresh(&find("refresh_token")),
            _other => false,
        };
        if !granted {
            return json_answer(
                StatusCode::BAD_REQUEST,
                &json!({ "error": "invalid_grant" }),
            );
        }
        let (access, refresh) = {
            let mut issued = self.issued();
            issued.counter += 1;
            let access = format!("planted-access-token-{}", issued.counter);
            let refresh = format!("planted-refresh-token-{}", issued.counter);
            issued.access.push(access.clone());
            issued.refresh.push(refresh.clone());
            (access, refresh)
        };
        json_answer(
            StatusCode::OK,
            &json!({
                "access_token": access,
                "token_type": "Bearer",
                "expires_in": self.options.expires_in,
                "refresh_token": refresh,
                "scope": "tools",
            }),
        )
    }

    /// Whether a code redeems: it was given, once, to the same client and
    /// redirect, and the verifier's S256 is its challenge.
    fn redeem(&self, code: &str, verifier: &str, redirect_uri: &str, client_id: &str) -> bool {
        let Some(grant) = self.issued().codes.remove(code) else {
            return false;
        };
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let matches_grant = grant.redirect_uri == redirect_uri && grant.client_id == client_id;
        matches_grant && grant.challenge == challenge
    }

    fn refresh(&self, token: &str) -> bool {
        self.issued().refresh.iter().any(|issued| issued == token)
    }
}

/// A request's body.
async fn body_of(request: Request<Incoming>) -> Bytes {
    request
        .into_body()
        .collect()
        .await
        .map(http_body_util::Collected::to_bytes)
        .unwrap_or_default()
}

/// An answer of text.
pub(crate) fn text(status: StatusCode, body: &'static str) -> Answer {
    let mut response = Response::new(Full::new(Bytes::from_static(body.as_bytes())).boxed());
    *response.status_mut() = status;
    response
}

/// An answer of JSON.
fn json_answer(status: StatusCode, value: &Value) -> Answer {
    let mut response = Response::new(Full::new(Bytes::from(value.to_string())).boxed());
    *response.status_mut() = status;
    response.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("application/json"),
    );
    response
}

/// The pairs of a query or a form, decoded.
fn form(text: &str) -> Vec<(String, String)> {
    text.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decoded(key), decoded(value))
        })
        .collect()
}

/// A part of a query with its `+` and percent escapes decoded.
fn decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escape = bytes
            .get(index + 1..index + 3)
            .filter(|digits| bytes[index] == b'%' && digits.iter().all(u8::is_ascii_hexdigit))
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| u8::from_str_radix(digits, 16).ok());
        if let Some(value) = escape {
            decoded.push(value);
            index += 3;
            continue;
        }
        decoded.push(if bytes[index] == b'+' {
            b' '
        } else {
            bytes[index]
        });
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// A text with every byte but the unreserved ones percent-encoded.
fn encoded(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            let _written = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

/// What a request of the browser got.
#[derive(Debug, Clone)]
pub struct Browsed {
    /// The status.
    pub status: u16,
    /// The `Location` of a redirect.
    pub location: Option<String>,
    /// The body.
    pub body: String,
}

/// Opens a page on a loopback address, as a browser does, without
/// following a redirect.
pub async fn browse(url: &str) -> std::io::Result<Browsed> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| std::io::Error::other("the browser of the tests opens http only"))?;
    let (authority, path) = rest
        .split_once('/')
        .map_or((rest, "/".to_owned()), |(authority, path)| {
            (authority, format!("/{path}"))
        });
    let mut stream = TcpStream::connect(authority).await?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await?;
    let answer = String::from_utf8_lossy(&answer).into_owned();
    let (head, body) = answer
        .split_once("\r\n\r\n")
        .unwrap_or((answer.as_str(), ""));
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|status| status.parse().ok())
        .unwrap_or(0);
    let location = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _value)| name.eq_ignore_ascii_case("location"))
        .map(|(_name, value)| value.trim().to_owned());
    Ok(Browsed {
        status,
        location,
        body: body.to_owned(),
    })
}

/// Opens the authorization page of a sign-in and follows its redirect back
/// to the client, as the owner's browser does. Returns the client's page.
pub async fn sign_in_through(authorization_page: &str) -> std::io::Result<Browsed> {
    let page = browse(authorization_page).await?;
    let location = page.location.ok_or_else(|| {
        std::io::Error::other(format!("the authorization page answered {}", page.status))
    })?;
    browse(&location).await
}

#[cfg(test)]
mod tests {
    use super::{decoded, encoded, form};

    #[test]
    fn queries_cross_their_encoding_both_ways() {
        let text = "http://127.0.0.1:9/a b?c=d&é";

        assert_eq!(decoded(&encoded(text)), text);
        assert_eq!(
            form("a=1+2&b=%2F"),
            [
                ("a".to_owned(), "1 2".to_owned()),
                ("b".to_owned(), "/".to_owned())
            ]
        );
    }
}
