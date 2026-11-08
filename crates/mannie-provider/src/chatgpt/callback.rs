//! The browser's return from a ChatGPT sign-in (ADR-0062, decision 4): a
//! listener on the loopback interface that waits for one request to
//! `/auth/callback` with the sign-in's `state`, and answers it with a fixed
//! page.
//!
//! It follows the MCP plugin's listener (plugins/mcp/docs/adr/0002-sign-in-
//! to-remote-servers-with-oauth.md, decision 4), which the core cannot import,
//! with one difference: the first sign-in listens on port 1455, the port of
//! OpenAI's guide (ADR-0062, decision 4). Requests to other paths, or with another `state`, are answered and
//! otherwise ignored, so a page that guesses the port cannot end the sign-in.
//! The page's texts come from the UI's catalog (ADR-0019) and are escaped,
//! and the page loads nothing.

use std::io::ErrorKind;
use std::net::Ipv4Addr;
use std::time::Duration;

use mannie_types::provider::ChatGptCallbackPage;
use mannie_types::secret::Secret;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use super::CALLBACK_PATH;

/// The most bytes of a request that the listener reads.
const MAX_REQUEST_BYTES: usize = 16 * 1024;

/// How long one request may take to arrive.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Why the listener could not start.
#[derive(Debug, thiserror::Error)]
pub enum ListenError {
    /// Another program listens on the port, or the system reserved it.
    #[error("port {port} is in use or reserved")]
    PortUnavailable {
        /// The port.
        port: u16,
    },
    /// The listener failed.
    #[error("the sign-in's listener failed")]
    Failed(#[source] std::io::Error),
}

/// Why a sign-in ended without a code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackError {
    /// The browser did not come back in time.
    TimedOut,
    /// OpenAI returned an error, such as the owner's refusal.
    Refused,
    /// The listener failed.
    Failed,
}

/// The browser's return, still waiting for the page that answers it.
#[derive(Debug)]
pub struct Callback {
    /// The authorization code.
    pub code: Secret,
    /// The client that OpenAI issued, which the return of a first sign-in
    /// names.
    pub client_id: Option<String>,
    /// The scopes that the account granted, when the return names them.
    pub scope: Option<String>,
    /// The issuer, when the return names it (RFC 9207).
    pub issuer: Option<String>,
    stream: TcpStream,
}

impl Callback {
    /// Answers the browser with the page of a sign-in that worked or not.
    pub async fn answer(mut self, page: &ChatGptCallbackPage, signed_in: bool) {
        respond(&mut self.stream, 200, &page_html(page, signed_in)).await;
    }
}

/// A listener for one sign-in's return.
#[derive(Debug)]
pub struct CallbackListener {
    listener: TcpListener,
    port: u16,
}

impl CallbackListener {
    /// Listens on `127.0.0.1` and `port`, or on a free port for 0. The
    /// listener exists before the browser opens, so the return never meets a
    /// closed port.
    pub async fn bind(port: u16) -> Result<Self, ListenError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|error| match error.kind() {
                // Windows also refuses ports that Hyper-V or WinNAT reserved.
                ErrorKind::AddrInUse | ErrorKind::PermissionDenied => {
                    ListenError::PortUnavailable { port }
                }
                _other => ListenError::Failed(error),
            })?;
        let port = listener.local_addr().map_err(ListenError::Failed)?.port();
        Ok(Self { listener, port })
    }

    /// The port that the listener listens on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Waits at most `timeout` for the return with `state`.
    pub async fn wait(
        self,
        state: &str,
        page: &ChatGptCallbackPage,
        timeout: Duration,
    ) -> Result<Callback, CallbackError> {
        let waiting =
            async {
                loop {
                    let Ok((mut stream, _peer)) = self.listener.accept().await else {
                        return Err(CallbackError::Failed);
                    };
                    let target =
                        match tokio::time::timeout(REQUEST_TIMEOUT, request_target(&mut stream))
                            .await
                        {
                            Ok(Ok(Some(target))) => target,
                            _unreadable => continue,
                        };
                    let Some(query) = callback_query(&target) else {
                        respond(&mut stream, 404, "").await;
                        continue;
                    };
                    let parameters = parameters(query);
                    let find = |name: &str| {
                        parameters
                            .iter()
                            .find(|(key, _value)| key == name)
                            .map(|(_key, value)| value.clone())
                    };
                    if find("state").as_deref() != Some(state) {
                        respond(&mut stream, 400, &page_html(page, false)).await;
                        continue;
                    }
                    if find("error").is_some() {
                        respond(&mut stream, 200, &page_html(page, false)).await;
                        return Err(CallbackError::Refused);
                    }
                    let Some(code) = find("code").filter(|code| !code.is_empty()) else {
                        respond(&mut stream, 400, &page_html(page, false)).await;
                        continue;
                    };
                    return Ok(Callback {
                        code: Secret::new(code),
                        client_id: find("client_id").filter(|client| !client.is_empty()),
                        scope: find("scope"),
                        issuer: find("iss"),
                        stream,
                    });
                }
            };
        tokio::time::timeout(timeout, waiting)
            .await
            .unwrap_or(Err(CallbackError::TimedOut))
    }
}

/// The target of a request's first line, such as `/auth/callback?code=...`,
/// when the request is a GET.
async fn request_target(stream: &mut TcpStream) -> std::io::Result<Option<String>> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 2048];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut buffer).await?;
        if read == 0 || request.len() + read > MAX_REQUEST_BYTES {
            return Ok(None);
        }
        request.extend_from_slice(&buffer[..read]);
    }
    let text = String::from_utf8_lossy(&request);
    let first = text.lines().next().unwrap_or_default();
    let mut words = first.split(' ');
    let is_get = words.next() == Some("GET");
    Ok(words.next().filter(|_target| is_get).map(str::to_owned))
}

/// The query of a request to the return's path.
fn callback_query(target: &str) -> Option<&str> {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    (path == CALLBACK_PATH).then_some(query)
}

/// The parameters of a query, decoded.
fn parameters(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decoded(key), percent_decoded(value))
        })
        .collect()
}

/// A part of a query with its `+` and percent escapes decoded. A `%`
/// without two hexadecimal digits after it stays as it is.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        let escape = bytes
            .get(index + 1..index + 3)
            .filter(|digits| byte == b'%' && digits.iter().all(u8::is_ascii_hexdigit))
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| u8::from_str_radix(digits, 16).ok());
        match (byte, escape) {
            (b'%', Some(value)) => {
                decoded.push(value);
                index += 3;
            }
            (b'+', _none) => {
                decoded.push(b' ');
                index += 1;
            }
            (other, _none) => {
                decoded.push(other);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// The page that the browser shows.
fn page_html(page: &ChatGptCallbackPage, signed_in: bool) -> String {
    let message = if signed_in {
        &page.signed_in
    } else {
        &page.failed
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title></head><body><p>{}</p></body></html>",
        escaped(&page.title),
        escaped(message)
    )
}

/// A text with the characters that HTML reads as markup escaped.
fn escaped(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Writes an answer and closes the connection. A browser that left is not a
/// failure.
async fn respond(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        _other => "Not Found",
    };
    let answer = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    if let Err(error) = stream.write_all(answer.as_bytes()).await {
        tracing::debug!(%error, "the browser left before a sign-in's page");
    }
    if let Err(error) = stream.shutdown().await {
        tracing::debug!(%error, "the connection of a sign-in's page closed already");
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::time::Duration;

    use mannie_types::provider::ChatGptCallbackPage;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpStream;

    use super::{
        CallbackError, CallbackListener, ListenError, callback_query, escaped, parameters,
    };

    fn page() -> ChatGptCallbackPage {
        ChatGptCallbackPage {
            title: "Mannie <sign-in>".to_owned(),
            signed_in: "You are signed in.".to_owned(),
            failed: "The sign-in failed.".to_owned(),
        }
    }

    async fn request(port: u16, raw: &str) -> String {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        stream.write_all(raw.as_bytes()).await.unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).await.unwrap();
        answer
    }

    async fn get(port: u16, target: &str) -> String {
        request(
            port,
            &format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"),
        )
        .await
    }

    #[test]
    fn a_query_is_read_only_on_the_callback_path_and_decoded() {
        assert_eq!(callback_query("/auth/callback?code=a"), Some("code=a"));
        assert_eq!(callback_query("/auth/callback"), Some(""));
        assert_eq!(callback_query("/callback?code=a"), None);
        assert_eq!(
            parameters("scope=openid+email&client_id=oaiapp%5F1&x"),
            [
                ("scope".to_owned(), "openid email".to_owned()),
                ("client_id".to_owned(), "oaiapp_1".to_owned()),
                ("x".to_owned(), String::new()),
            ]
        );
        assert_eq!(
            escaped("<a href='x'>&</a>"),
            "&lt;a href=&#39;x&#39;&gt;&amp;&lt;/a&gt;"
        );
    }

    #[tokio::test]
    async fn the_return_with_the_state_ends_the_wait_with_its_code_and_client() {
        let listener = CallbackListener::bind(0).await.unwrap();
        let port = listener.port();
        let waiting = tokio::spawn(async move {
            listener
                .wait("state-1", &page(), Duration::from_secs(10))
                .await
        });

        let wrong_path = get(port, "/callback?state=state-1&code=c").await;
        let wrong_state = get(port, "/auth/callback?state=other&code=c").await;
        let not_get = request(
            port,
            "POST /auth/callback?state=state-1&code=c HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
        )
        .await;
        let answer = tokio::spawn(get(
            port,
            "/auth/callback?state=state-1&code=the-code&client_id=oaiapp_1&scope=openid+chatgpt.tokens.use.direct&iss=https%3A%2F%2Fauth.openai.com",
        ));
        let callback = waiting.await.unwrap().unwrap();
        assert_eq!(callback.code.expose(), "the-code");
        assert_eq!(callback.client_id.as_deref(), Some("oaiapp_1"));
        assert_eq!(
            callback.scope.as_deref(),
            Some("openid chatgpt.tokens.use.direct")
        );
        assert_eq!(callback.issuer.as_deref(), Some("https://auth.openai.com"));
        callback.answer(&page(), true).await;
        let page_text = answer.await.unwrap();

        assert!(wrong_path.starts_with("HTTP/1.1 404"), "{wrong_path}");
        assert!(wrong_state.starts_with("HTTP/1.1 400"), "{wrong_state}");
        assert!(
            not_get.is_empty() || !not_get.starts_with("HTTP/1.1 200"),
            "{not_get}"
        );
        assert!(page_text.starts_with("HTTP/1.1 200"), "{page_text}");
        assert!(page_text.contains("Content-Security-Policy: default-src 'none'"));
        assert!(page_text.contains("Mannie &lt;sign-in&gt;"));
        assert!(page_text.contains("You are signed in."));
    }

    #[tokio::test]
    async fn an_error_in_the_return_ends_the_sign_in_as_refused() {
        let listener = CallbackListener::bind(0).await.unwrap();
        let port = listener.port();
        let waiting = tokio::spawn(async move {
            listener
                .wait("state-1", &page(), Duration::from_secs(10))
                .await
        });

        let answer = get(port, "/auth/callback?state=state-1&error=access_denied").await;

        assert_eq!(waiting.await.unwrap().unwrap_err(), CallbackError::Refused);
        assert!(answer.contains("The sign-in failed."));
    }

    #[tokio::test]
    async fn an_oversized_request_is_ignored_and_the_wait_times_out() {
        let listener = CallbackListener::bind(0).await.unwrap();
        let port = listener.port();
        let waiting = tokio::spawn(async move {
            listener
                .wait("state-1", &page(), Duration::from_millis(500))
                .await
        });

        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        let oversized = format!(
            "GET /auth/callback?state=state-1&code={} HTTP/1.1\r\n\r\n",
            "a".repeat(20 * 1024)
        );
        // The listener stops reading once the request is too long, and the
        // connection may close under the write, which the test accepts.
        let _closed_early = stream.write_all(oversized.as_bytes()).await;

        assert_eq!(waiting.await.unwrap().unwrap_err(), CallbackError::TimedOut);
    }

    #[tokio::test]
    async fn a_busy_port_is_reported_as_unavailable() {
        let first = CallbackListener::bind(0).await.unwrap();

        let second = CallbackListener::bind(first.port()).await;

        assert!(
            matches!(second, Err(ListenError::PortUnavailable { port }) if port == first.port()),
            "{second:?}"
        );
    }
}
