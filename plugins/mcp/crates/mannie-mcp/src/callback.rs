//! The browser's return from a sign-in (plugins/mcp/docs/adr/0002-sign-in-
//! to-remote-servers-with-oauth.md): a listener on a random loopback port
//! that waits at most [`SIGN_IN_TIMEOUT`] for one request to `/callback`
//! with the sign-in's `state`, and answers it with a fixed page.
//!
//! Requests to other paths, or with another `state`, are answered and
//! otherwise ignored, so a page that guesses the port cannot end the
//! sign-in. The page's texts come from the UI's catalog (ADR-0019) and are
//! escaped, and the page loads nothing.

use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use crate::commands::SignInPage;

/// How long a sign-in waits for the browser.
pub(crate) const SIGN_IN_TIMEOUT: Duration = Duration::from_mins(5);

/// The path of the redirect.
pub(crate) const CALLBACK_PATH: &str = "/callback";

/// The most bytes of a request that the listener reads.
const MAX_REQUEST_BYTES: usize = 16 * 1024;

/// How long one request may take to arrive.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The redirect that ended a sign-in, still waiting for its answer.
#[derive(Debug)]
pub(crate) struct Redirect {
    /// The authorization code.
    pub(crate) code: String,
    /// The authorization server's `iss`, which RFC 9207 checks.
    pub(crate) issuer: Option<String>,
    stream: TcpStream,
}

impl Redirect {
    /// Answers the browser with the page of a sign-in that worked or not.
    pub(crate) async fn answer(mut self, page: &SignInPage, signed_in: bool) {
        respond(&mut self.stream, 200, &page_html(page, signed_in)).await;
    }
}

/// Why a sign-in ended without a code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallbackError {
    /// The browser did not return in time.
    TimedOut,
    /// The authorization server returned an error, such as the owner's
    /// refusal.
    Refused,
    /// The listener failed.
    Failed,
}

/// Waits for the redirect with `state`.
pub(crate) async fn wait(
    listener: TcpListener,
    state: &str,
    page: &SignInPage,
) -> Result<Redirect, CallbackError> {
    let waiting = async {
        loop {
            let Ok((mut stream, _peer)) = listener.accept().await else {
                return Err(CallbackError::Failed);
            };
            let target =
                match tokio::time::timeout(REQUEST_TIMEOUT, request_target(&mut stream)).await {
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
            let Some(code) = find("code") else {
                respond(&mut stream, 400, &page_html(page, false)).await;
                continue;
            };
            return Ok(Redirect {
                code,
                issuer: find("iss"),
                stream,
            });
        }
    };
    tokio::time::timeout(SIGN_IN_TIMEOUT, waiting)
        .await
        .unwrap_or(Err(CallbackError::TimedOut))
}

/// The target of a request's first line, such as `/callback?code=...`.
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

/// The query of a request to the callback's path.
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
fn page_html(page: &SignInPage, signed_in: bool) -> String {
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

/// Writes an answer and closes the connection. A browser that left is not
/// a failure.
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

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::{TcpListener, TcpStream};

    use super::{CallbackError, callback_query, escaped, parameters, wait};
    use crate::commands::SignInPage;

    fn page() -> SignInPage {
        SignInPage {
            title: "Mannie <sign-in>".to_owned(),
            signed_in: "You are signed in.".to_owned(),
            failed: "The sign-in failed.".to_owned(),
        }
    }

    async fn request(port: u16, target: &str) -> String {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        stream
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).await.unwrap();
        answer
    }

    #[test]
    fn a_query_is_read_only_on_the_callback_path_and_decoded() {
        assert_eq!(callback_query("/callback?code=a"), Some("code=a"));
        assert_eq!(callback_query("/favicon.ico"), None);
        assert_eq!(
            parameters("code=a%2Fb+c&state=x&iss=https%3A%2F%2Fauth"),
            [
                ("code".to_owned(), "a/b c".to_owned()),
                ("state".to_owned(), "x".to_owned()),
                ("iss".to_owned(), "https://auth".to_owned()),
            ]
        );
        assert_eq!(
            parameters("bad=%zz&end=%4"),
            [
                ("bad".to_owned(), "%zz".to_owned()),
                ("end".to_owned(), "%4".to_owned()),
            ]
        );
        assert_eq!(escaped("<b>\"&'"), "&lt;b&gt;&quot;&amp;&#39;");
    }

    #[tokio::test]
    async fn only_the_sign_ins_state_ends_the_wait_and_the_page_is_escaped() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let page = page();
        let waiting = tokio::spawn(async move { wait(listener, "expected", &page).await });

        let other = request(port, "/callback?code=stolen&state=guess").await;
        let favicon = request(port, "/favicon.ico").await;
        let right = tokio::spawn(request(
            port,
            "/callback?code=c0de&state=expected&iss=https%3A%2F%2Fauth",
        ));
        let redirect = waiting.await.unwrap().unwrap();
        redirect.answer(&super::tests::page(), true).await;
        let answer = right.await.unwrap();

        assert!(other.starts_with("HTTP/1.1 400"));
        assert!(favicon.starts_with("HTTP/1.1 404"));
        assert!(answer.starts_with("HTTP/1.1 200"));
        assert!(answer.contains("<title>Mannie &lt;sign-in&gt;</title>"));
        assert!(answer.contains("You are signed in."));
        assert!(answer.contains("Content-Security-Policy: default-src 'none'"));
    }

    #[tokio::test]
    async fn a_refusal_ends_the_wait() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiting = tokio::spawn(async move { wait(listener, "expected", &page()).await });

        let answer = request(port, "/callback?error=access_denied&state=expected").await;

        assert_eq!(waiting.await.unwrap().unwrap_err(), CallbackError::Refused);
        assert!(answer.contains("The sign-in failed."));
    }
}
