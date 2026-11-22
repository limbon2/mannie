//! HTTP execution, transport safety, RFC 6265 cookie jar matching and
//! `Set-Cookie` parsing (ADR-0075, decision 6).

use std::time::{Duration, Instant};

use base64::Engine as _;
use mannie_plugin_sdk::contracts::{UtcMillis, credential_transport_allowed};
use reqwest::header::{COOKIE, HeaderMap, HeaderName, HeaderValue};
use tokio::sync::oneshot;
use url::Url;

use crate::model::{
    CookieView, HttpMethod, ManualResponseView, NetworkError, Protocol, ResponseCookieView,
    ResponseHeaderView,
};
use crate::template::ResolvedHttpRequest;

/// Builds the plugin's shared `reqwest::Client` using `ring`, the OS trust
/// store, system proxy settings and automatic redirects disabled so secret
/// headers and cookies never forward across origins.
pub(crate) fn build_http_client() -> Result<reqwest::Client, reqwest::Error> {
    if rustls::crypto::ring::default_provider()
        .install_default()
        .is_err()
    {
        tracing::trace!("the crypto provider was already installed");
    }

    reqwest::Client::builder()
        .user_agent(concat!("Mannie/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .build()
}

/// A parsed `Set-Cookie` header scoped to the request URL.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "RFC 6265 parsed Set-Cookie flags (host_only, secure, http_only) plus expiry removal flag"
)]
pub(crate) struct ParsedSetCookie {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) domain: String,
    pub(crate) host_only: bool,
    pub(crate) path: String,
    pub(crate) secure: bool,
    pub(crate) http_only: bool,
    pub(crate) expires_at: Option<UtcMillis>,
    /// True when `Max-Age <= 0` or `Expires` is in the past, requesting cookie
    /// deletion.
    pub(crate) remove: bool,
}

/// Outcome of an HTTP exchange before owner or agent formatting.
#[derive(Debug, Clone)]
pub(crate) struct HttpExchange {
    pub(crate) status: u16,
    pub(crate) status_text: String,
    pub(crate) duration_ms: u32,
    pub(crate) total_bytes: usize,
    pub(crate) truncated: bool,
    pub(crate) content_type: Option<String>,
    pub(crate) body_bytes: Vec<u8>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) set_cookies: Vec<ParsedSetCookie>,
}

impl HttpExchange {
    /// Converts this exchange into the unredacted owner workbench view.
    pub(crate) fn into_owner_view(
        self,
        protocol: Protocol,
        method: HttpMethod,
        url: String,
    ) -> ManualResponseView {
        let (body, is_binary) = match String::from_utf8(self.body_bytes) {
            Ok(text) => (text, false),
            Err(error) => (
                base64::engine::general_purpose::STANDARD.encode(error.into_bytes()),
                true,
            ),
        };
        let headers = self
            .headers
            .into_iter()
            .map(|(name, value)| ResponseHeaderView { name, value })
            .collect();
        let cookies = self
            .set_cookies
            .into_iter()
            .map(|cookie| ResponseCookieView {
                name: cookie.name,
                value: cookie.value,
                domain: cookie.domain,
                host_only: cookie.host_only,
                path: cookie.path,
                secure: cookie.secure,
                http_only: cookie.http_only,
                expires_at: cookie.expires_at,
            })
            .collect();
        ManualResponseView {
            protocol,
            method,
            url,
            status: self.status,
            status_text: self.status_text,
            duration_ms: self.duration_ms,
            bytes: u32::try_from(self.total_bytes).unwrap_or(u32::MAX),
            truncated: self.truncated,
            content_type: self.content_type,
            body,
            is_binary,
            headers,
            cookies,
        }
    }
}

/// Selects enabled, unexpired cookies matching `url` per RFC 6265 domain, path
/// and `Secure` rules.
pub(crate) fn matching_cookies<'a>(
    cookies: &'a [CookieView],
    url: &Url,
    now: UtcMillis,
) -> Vec<&'a CookieView> {
    let Some(host) = url.host_str() else {
        return Vec::new();
    };
    let host_lower = host.to_ascii_lowercase();
    let is_https = url.scheme() == "https";
    let req_path = url.path();

    cookies
        .iter()
        .filter(|cookie| {
            let active = cookie.enabled
                && cookie
                    .expires_at
                    .is_none_or(|exp| exp.as_millis() > now.as_millis());
            let scheme_ok = !cookie.secure || is_https;
            let domain_ok = if cookie.host_only {
                host_only_matches(&host_lower, &cookie.domain)
            } else {
                domain_matches(&host_lower, &cookie.domain)
            };
            let path_ok = path_matches(req_path, &cookie.path);
            active && scheme_ok && domain_ok && path_ok
        })
        .collect()
}

/// RFC 6265 section 5.4 host-only matching: exact host equality only.
pub(crate) fn host_only_matches(request_host_lower: &str, cookie_domain: &str) -> bool {
    let normalized = cookie_domain.trim().to_ascii_lowercase();
    !normalized.is_empty() && request_host_lower == normalized
}

/// RFC 6265 section 5.1.3 domain matching.
pub(crate) fn domain_matches(request_host_lower: &str, cookie_domain: &str) -> bool {
    let normalized = cookie_domain
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();

    if normalized.is_empty() {
        return false;
    }
    if request_host_lower == normalized {
        return true;
    }
    // IP literals match only by exact equality, never suffix matching.
    if request_host_lower.parse::<std::net::IpAddr>().is_ok() {
        return false;
    }

    request_host_lower.ends_with(&format!(".{normalized}"))
}

/// RFC 6265 section 5.1.4 path matching.
pub(crate) fn path_matches(request_path: &str, cookie_path: &str) -> bool {
    let cpath = if cookie_path.is_empty() {
        "/"
    } else {
        cookie_path
    };

    if request_path == cpath {
        return true;
    }
    if let Some(rest) = request_path.strip_prefix(cpath) {
        return cpath.ends_with('/') || rest.starts_with('/');
    }
    false
}

/// Parses one `Set-Cookie` header value scoped to `request_url`. Refuses a
/// `Domain` attribute that does not domain-match the request host so a remote
/// server cannot plant cookies for unrelated domains.
pub(crate) fn parse_set_cookie(
    header_value: &str,
    request_url: &Url,
    now: UtcMillis,
) -> Option<ParsedSetCookie> {
    let request_host = request_url.host_str()?.to_ascii_lowercase();
    let mut parts = header_value.split(';');
    let pair = parts.next()?.trim();
    let (raw_name, raw_value) = pair.split_once('=')?;
    let name = raw_name.trim();

    if name.is_empty() || name.chars().any(char::is_control) {
        return None;
    }

    let value = raw_value.trim().to_owned();
    let mut domain = request_host.clone();
    let mut host_only = true;
    let mut path = default_cookie_path(request_url.path());
    let mut secure = false;
    let mut http_only = false;
    let mut expires_at: Option<UtcMillis> = None;
    let mut remove = false;
    let mut has_max_age = false;

    for attr in parts {
        let trimmed = attr.trim();

        if trimmed.is_empty() {
            continue;
        }

        if let Some((attr_name, attr_val)) = trimmed.split_once('=') {
            let key = attr_name.trim().to_ascii_lowercase();
            let val = attr_val.trim();

            match key.as_str() {
                "domain" => {
                    let candidate = val.trim_start_matches('.').to_ascii_lowercase();

                    if !domain_matches(&request_host, &candidate) {
                        return None;
                    }

                    domain = candidate;
                    host_only = false;
                }
                "path" => {
                    if val.starts_with('/') {
                        val.clone_into(&mut path);
                    }
                }
                "max-age" => {
                    if let Ok(seconds) = val.parse::<i64>() {
                        has_max_age = true;

                        if seconds <= 0 {
                            remove = true;
                            expires_at = Some(now);
                        } else {
                            let ms = seconds.saturating_mul(1_000);
                            expires_at =
                                Some(UtcMillis::from_millis(now.as_millis().saturating_add(ms)));
                        }
                    }
                }
                "expires" if !has_max_age => {
                    if let Some(parsed) = parse_http_date(val) {
                        if parsed.as_millis() <= now.as_millis() {
                            remove = true;
                        }

                        expires_at = Some(parsed);
                    }
                }
                _other => {}
            }
        } else {
            let flag = trimmed.to_ascii_lowercase();

            if flag == "secure" {
                secure = true;
            } else if flag == "httponly" {
                http_only = true;
            }
        }
    }

    Some(ParsedSetCookie {
        name: name.to_owned(),
        value,
        domain,
        host_only,
        path,
        secure,
        http_only,
        expires_at,
        remove,
    })
}

fn default_cookie_path(request_path: &str) -> String {
    if !request_path.starts_with('/') || request_path == "/" {
        return "/".to_owned();
    }

    match request_path.rfind('/') {
        Some(0) | None => "/".to_owned(),
        Some(idx) => request_path[..idx].to_owned(),
    }
}

/// Parses RFC 7231 IMF-fixdate (`Sun, 06 Nov 1994 08:49:37 GMT`) into UTC
/// milliseconds without pulling an extra date crate.
fn parse_http_date(value: &str) -> Option<UtcMillis> {
    let trimmed = value.trim();
    let without_day = trimmed
        .split_once(',')
        .map_or(trimmed, |(_dow, rest)| rest.trim());
    let tokens: Vec<&str> = without_day.split_whitespace().collect();

    if tokens.len() < 4 {
        return None;
    }

    let day: i64 = tokens[0].parse().ok()?;
    let month: i64 = match tokens[1].to_ascii_lowercase().as_str() {
        "jan" => 1,
        "feb" => 2,
        "mar" => 3,
        "apr" => 4,
        "may" => 5,
        "jun" => 6,
        "jul" => 7,
        "aug" => 8,
        "sep" => 9,
        "oct" => 10,
        "nov" => 11,
        "dec" => 12,
        _other => return None,
    };
    let year: i64 = tokens[2].parse().ok()?;
    let hms: Vec<&str> = tokens[3].split(':').collect();

    if hms.len() != 3 {
        return None;
    }

    let hour: i64 = hms[0].parse().ok()?;
    let minute: i64 = hms[1].parse().ok()?;
    let second: i64 = hms[2].parse().ok()?;
    let valid_range = (1..=31).contains(&day)
        && (1970..=9999).contains(&year)
        && (0..=23).contains(&hour)
        && (0..=59).contains(&minute)
        && (0..=60).contains(&second);

    if !valid_range {
        return None;
    }

    // Howard Hinnant's civil-to-days algorithm relative to 1970-01-01.
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hour * 3_600 + minute * 60 + second;
    Some(UtcMillis::from_millis(secs.saturating_mul(1_000)))
}

/// Executes an HTTP request with transport safety, cookie attachment, bounded
/// download and optional cancellation.
#[expect(
    clippy::too_many_arguments,
    reason = "HTTP execution takes client, method, resolved request, cookies, clock, cap and cancel channel"
)]
pub(crate) async fn execute_http(
    client: &reqwest::Client,
    method: HttpMethod,
    resolved: &ResolvedHttpRequest,
    cookies: &[CookieView],
    use_cookies: bool,
    now: UtcMillis,
    max_response_bytes: usize,
    cancel: Option<oneshot::Receiver<()>>,
) -> Result<HttpExchange, NetworkError> {
    let attached = if use_cookies {
        matching_cookies(cookies, &resolved.real_url, now)
    } else {
        Vec::new()
    };
    let sends_secrets = resolved.uses_credentials || !attached.is_empty();

    if sends_secrets && !credential_transport_allowed(&resolved.real_url) {
        return Err(NetworkError::InsecureCredentialTransport);
    }

    let builder = build_request_builder(client, method, resolved, &attached)?;
    let started = Instant::now();
    let send_fut = send_and_read_exchange(
        builder,
        &resolved.real_url,
        now,
        max_response_bytes,
        started,
    );

    match cancel {
        Some(cancel_rx) => {
            tokio::select! {
                biased;
                _canceled = cancel_rx => Err(NetworkError::RequestCanceled),
                result = send_fut => result,
            }
        }
        None => send_fut.await,
    }
}

fn build_request_builder(
    client: &reqwest::Client,
    method: HttpMethod,
    resolved: &ResolvedHttpRequest,
    attached: &[&CookieView],
) -> Result<reqwest::RequestBuilder, NetworkError> {
    let mut header_map = HeaderMap::new();
    let mut has_explicit_content_type = false;
    let mut has_explicit_cookie = false;

    for (name, value, is_secret) in &resolved.headers {
        let Ok(header_name) = HeaderName::from_bytes(name.as_bytes()) else {
            return Err(NetworkError::RequestFailed {
                detail: format!("invalid header name \"{name}\""),
            });
        };
        let Ok(mut header_value) = HeaderValue::from_str(value) else {
            return Err(NetworkError::RequestFailed {
                detail: format!("invalid header value for \"{name}\""),
            });
        };

        if *is_secret {
            header_value.set_sensitive(true);
        }
        if header_name == reqwest::header::CONTENT_TYPE {
            has_explicit_content_type = true;
        }
        if header_name == COOKIE {
            has_explicit_cookie = true;
        }

        header_map.insert(header_name, header_value);
    }

    if !attached.is_empty() {
        let jar_header = attached
            .iter()
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ");
        let combined = if has_explicit_cookie {
            let existing = header_map
                .get(COOKIE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            format!("{existing}; {jar_header}")
        } else {
            jar_header
        };
        if let Ok(mut value) = HeaderValue::from_str(&combined) {
            value.set_sensitive(true);
            header_map.insert(COOKIE, value);
        }
    }

    let mut builder = client
        .request(method.to_reqwest(), resolved.real_url.clone())
        .timeout(Duration::from_secs(u64::from(resolved.timeout_seconds)));

    if let Some((bytes, default_content_type)) = &resolved.body {
        if !has_explicit_content_type
            && let Some(ct) = default_content_type
            && let Ok(val) = HeaderValue::from_str(ct)
        {
            header_map.insert(reqwest::header::CONTENT_TYPE, val);
        }

        builder = builder.body(bytes.clone());
    }

    Ok(builder.headers(header_map))
}

async fn send_and_read_exchange(
    builder: reqwest::RequestBuilder,
    request_url: &Url,
    now: UtcMillis,
    max_response_bytes: usize,
    started: Instant,
) -> Result<HttpExchange, NetworkError> {
    let mut response = builder.send().await.map_err(|err| map_send_error(&err))?;
    let status = response.status();
    let status_code = status.as_u16();
    let status_text = status.canonical_reason().unwrap_or("").to_owned();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let mut headers = Vec::new();
    let mut set_cookies = Vec::new();

    for (name, value) in response.headers() {
        let val_str = String::from_utf8_lossy(value.as_bytes()).into_owned();

        if name == reqwest::header::SET_COOKIE
            && let Some(parsed) = parse_set_cookie(&val_str, request_url, now)
        {
            set_cookies.push(parsed);
        }

        headers.push((name.as_str().to_owned(), val_str));
    }

    let mut body_bytes = Vec::new();
    let mut total_bytes = 0usize;
    let mut truncated = false;

    while let Some(chunk) = response.chunk().await.map_err(|err| map_body_error(&err))? {
        total_bytes = total_bytes.saturating_add(chunk.len());

        if body_bytes.len() < max_response_bytes {
            let remaining = max_response_bytes - body_bytes.len();
            let take = chunk.len().min(remaining);
            body_bytes.extend_from_slice(&chunk[..take]);
            if take < chunk.len() {
                truncated = true;
                break;
            }
        } else {
            truncated = true;
            break;
        }
    }

    let duration_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
    Ok(HttpExchange {
        status: status_code,
        status_text,
        duration_ms,
        total_bytes,
        truncated,
        content_type,
        body_bytes,
        headers,
        set_cookies,
    })
}

/// Classifies an error returned by `reqwest::RequestBuilder::send`.
/// Connection failures occur before HTTP bytes leave the host; timeouts and
/// mid-flight disconnects leave the remote server's outcome unknown.
fn map_send_error(error: &reqwest::Error) -> NetworkError {
    if error.is_timeout() {
        return NetworkError::RequestTimedOut;
    }

    // Never include the URL (which may hold secret query parameters) in error
    // messages (T-114).
    if error.is_connect() || error.is_builder() {
        return NetworkError::RequestFailed {
            detail: "could not connect to the remote host".to_owned(),
        };
    }

    let detail = if error.is_body() {
        "failed while sending or reading the HTTP body".to_owned()
    } else if error.is_decode() {
        "failed to decode the response".to_owned()
    } else {
        "connection closed before a complete response arrived".to_owned()
    };
    NetworkError::ResponseLost { detail }
}

/// Classifies an error while reading response body chunks after the server has
/// already responded; the remote server already received the request.
fn map_body_error(error: &reqwest::Error) -> NetworkError {
    if error.is_timeout() {
        return NetworkError::RequestTimedOut;
    }

    let detail = if error.is_decode() {
        "failed to decode the response body".to_owned()
    } else {
        "connection closed while reading the response body".to_owned()
    };
    NetworkError::ResponseLost { detail }
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::UtcMillis;
    use url::Url;

    use crate::model::CookieView;

    use super::{domain_matches, matching_cookies, parse_set_cookie, path_matches};

    #[test]
    fn cookie_domain_and_path_matching_follow_rfc_6265() {
        assert!(domain_matches("api.example.com", "api.example.com"));
        assert!(domain_matches("api.example.com", ".example.com"));
        assert!(domain_matches("api.example.com", "example.com"));
        assert!(!domain_matches("notexample.com", "example.com"));
        assert!(!domain_matches("127.0.0.1", "0.0.1"));
        assert!(domain_matches("127.0.0.1", "127.0.0.1"));

        assert!(path_matches("/v1/orders", "/"));
        assert!(path_matches("/v1/orders", "/v1"));
        assert!(path_matches("/v1/orders", "/v1/"));
        assert!(!path_matches("/v10/orders", "/v1"));
    }

    #[test]
    fn set_cookie_parsing_scopes_domain_and_handles_expiry() {
        let now = UtcMillis::from_millis(1_700_000_000_000);
        let url = Url::parse("https://api.example.com/v1/login").unwrap();

        let parsed = parse_set_cookie(
            "session=tok_12345678; Path=/v1; Domain=example.com; Max-Age=3600; Secure; HttpOnly",
            &url,
            now,
        )
        .unwrap();
        assert_eq!(parsed.name, "session");
        assert_eq!(parsed.value, "tok_12345678");
        assert_eq!(parsed.domain, "example.com");
        assert!(!parsed.host_only);
        assert_eq!(parsed.path, "/v1");
        assert!(parsed.secure);
        assert!(parsed.http_only);
        assert!(!parsed.remove);
        assert_eq!(
            parsed.expires_at,
            Some(UtcMillis::from_millis(1_700_003_600_000))
        );

        // Omitting Domain produces a host-only cookie (RFC 6265 section 5.4).
        let host_only = parse_set_cookie("host_sid=abc; Secure", &url, now).unwrap();
        assert!(host_only.host_only);
        assert_eq!(host_only.domain, "api.example.com");

        // Unrelated domain must be rejected.
        assert!(parse_set_cookie("evil=1; Domain=other.com", &url, now).is_none());

        // Max-Age=0 marks removal.
        let cleared = parse_set_cookie("session=; Max-Age=0", &url, now).unwrap();
        assert!(cleared.remove);

        // Secure cookie is withheld on plain HTTP.
        let cookie = CookieView {
            id: "c1".to_owned(),
            name: "session".to_owned(),
            value: "secret".to_owned(),
            domain: "example.com".to_owned(),
            host_only: false,
            path: "/".to_owned(),
            secure: true,
            http_only: true,
            expires_at: None,
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        let http_url = Url::parse("http://api.example.com/v1/me").unwrap();
        assert_eq!(
            matching_cookies(std::slice::from_ref(&cookie), &http_url, now),
            Vec::<&CookieView>::new()
        );
        assert_eq!(matching_cookies(&[cookie], &url, now).len(), 1);

        // Host-only cookie must NOT match subdomains.
        let host_cookie = CookieView {
            id: "c2".to_owned(),
            name: "host_sid".to_owned(),
            value: "parent_only".to_owned(),
            domain: "example.com".to_owned(),
            host_only: true,
            path: "/".to_owned(),
            secure: true,
            http_only: true,
            expires_at: None,
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        let parent_url = Url::parse("https://example.com/v1/me").unwrap();
        let sub_url = Url::parse("https://sub.example.com/v1/me").unwrap();
        assert_eq!(
            matching_cookies(std::slice::from_ref(&host_cookie), &parent_url, now).len(),
            1
        );
        assert_eq!(matching_cookies(&[host_cookie], &sub_url, now).len(), 0);
    }
}
