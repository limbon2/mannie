//! The HTTP client of provider calls: reqwest over rustls with ring and the
//! operating system's trust store (ADR-0011, decision 2), no redirects, and
//! fixed timeouts and limits (docs/features/chat/providers.md, adapters).

use std::sync::Arc;
use std::time::Duration;

use mannie_types::clock::Clock;
use mannie_types::secret::Secret;
use reqwest::header::{AUTHORIZATION, HeaderValue};
use reqwest::{RequestBuilder, Url};
use zeroize::Zeroizing;

/// How long each phase of a call may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    /// Connecting to the server.
    pub connect: Duration,
    /// The response headers.
    pub headers: Duration,
    /// The first event. Local servers can take minutes to read a long prompt.
    pub first_event: Duration,
    /// The silence between two events.
    pub idle: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(15),
            headers: Duration::from_secs(120),
            first_event: Duration::from_secs(600),
            idle: Duration::from_secs(300),
        }
    }
}

/// The most bytes one response may have.
pub(crate) const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

/// The most bytes of an error body that Mannie reads.
pub(crate) const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

/// Why the HTTP client could not be built.
#[derive(Debug, thiserror::Error)]
#[error("the HTTP client could not be built")]
pub struct TransportError(#[source] reqwest::Error);

/// The HTTP client of provider calls, with the clock that their errors are
/// timed by. Cloning is cheap and shares the connection pool.
#[derive(Debug, Clone)]
pub struct HttpClient {
    client: reqwest::Client,
    clock: Arc<dyn Clock>,
    timeouts: Timeouts,
}

impl HttpClient {
    /// Builds the client with the standard timeouts.
    pub fn new(clock: Arc<dyn Clock>) -> Result<Self, TransportError> {
        Self::with_timeouts(clock, Timeouts::default())
    }

    /// Builds the client with other timeouts, for tests.
    pub fn with_timeouts(
        clock: Arc<dyn Clock>,
        timeouts: Timeouts,
    ) -> Result<Self, TransportError> {
        install_ring();
        let client = reqwest::Client::builder()
            .user_agent(concat!("Mannie/", env!("CARGO_PKG_VERSION")))
            // A redirect could carry the credential to another host, so none
            // is followed (docs/features/chat/providers.md, credentials).
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(timeouts.connect)
            .build()
            .map_err(TransportError)?;
        Ok(Self {
            client,
            clock,
            timeouts,
        })
    }

    /// The clock that the client's callers use.
    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    /// The timeouts of each phase of a call.
    pub fn timeouts(&self) -> Timeouts {
        self.timeouts
    }

    /// A POST of JSON to `url`, with the credential as a bearer token.
    pub(crate) fn post(
        &self,
        url: Url,
        credential: Option<&Secret>,
        body: Vec<u8>,
    ) -> RequestBuilder {
        let request = self
            .client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .body(body);
        authorize(request, credential)
    }

    /// A GET of `url`, with the credential as a bearer token.
    pub(crate) fn get(&self, url: Url, credential: Option<&Secret>) -> RequestBuilder {
        authorize(self.client.get(url), credential)
    }

    /// A POST of JSON to `url` that expects JSON back, with the credential as
    /// a bearer token, for the calls of search profiles (ADR-0047).
    pub(crate) fn post_json(
        &self,
        url: Url,
        credential: Option<&Secret>,
        body: Vec<u8>,
    ) -> RequestBuilder {
        let request = self
            .client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "application/json")
            .body(body);
        authorize(request, credential)
    }
}

/// Adds the credential as a bearer token, marked as sensitive so that the
/// HTTP stack never shows it in debug output.
fn authorize(request: RequestBuilder, credential: Option<&Secret>) -> RequestBuilder {
    let Some(credential) = credential else {
        return request;
    };
    let bearer = Zeroizing::new(format!("Bearer {}", credential.expose()));
    match HeaderValue::from_str(&bearer) {
        Ok(mut value) => {
            value.set_sensitive(true);
            request.header(AUTHORIZATION, value)
        }
        // A credential with characters that a header cannot carry was refused
        // when it was saved; the server answers 401 without one.
        Err(_invalid) => request,
    }
}

/// Installs ring as the process's crypto provider for rustls.
///
/// reqwest's `rustls-no-provider` feature takes the process-wide provider,
/// and building a client without one panics. ring is the only provider Mannie
/// uses (ADR-0008), so installing it once per process is the whole of this
/// state; a second call finds it installed and changes nothing.
pub(crate) fn install_ring() {
    if rustls::crypto::ring::default_provider()
        .install_default()
        .is_err()
    {
        tracing::trace!("the crypto provider was already installed");
    }
}

/// The URL of an endpoint below a base URL: `https://host/v1` and `responses`
/// give `https://host/v1/responses`, with or without a trailing slash on the
/// base.
pub(crate) fn endpoint(base: &Url, path: &[&str]) -> Url {
    let mut url = base.clone();
    if let Ok(mut segments) = url.path_segments_mut() {
        segments.pop_if_empty();
        segments.extend(path);
    }
    url
}

#[cfg(test)]
mod tests {
    use reqwest::Url;

    use super::endpoint;

    #[test]
    fn endpoints_extend_the_base_path() {
        let with_slash = Url::parse("https://api.openai.com/v1/").unwrap();
        let without_slash = Url::parse("http://127.0.0.1:8000/v1").unwrap();
        let root = Url::parse("http://localhost:11434").unwrap();

        assert_eq!(
            endpoint(&with_slash, &["responses"]).as_str(),
            "https://api.openai.com/v1/responses"
        );
        assert_eq!(
            endpoint(&without_slash, &["chat", "completions"]).as_str(),
            "http://127.0.0.1:8000/v1/chat/completions"
        );
        assert_eq!(
            endpoint(&root, &["models"]).as_str(),
            "http://localhost:11434/models"
        );
    }
}
