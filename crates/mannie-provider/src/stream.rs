//! The part of a streamed call that both adapters share: sending the request,
//! checking the response, reading the body with its timeouts and limits, and
//! parsing server-sent events. Each adapter supplies a [`Translator`] that
//! turns the events of its API into normalized events.

use std::collections::VecDeque;
use std::pin::Pin;

use futures_util::{Stream, StreamExt};
use mannie_types::run::RunFailure;
use mannie_types::secret::Secret;
use reqwest::RequestBuilder;
use reqwest::header::CONTENT_TYPE;

use crate::errors::{self, Context, ErrorDetails};
use crate::provider::{EventStream, ProviderError, ProviderEvent};
use crate::quirks::Quirks;
use crate::sse::{SseEvent, SseParser};
use crate::transport::{HttpClient, MAX_ERROR_BODY_BYTES, MAX_RESPONSE_BYTES};

/// Whether a stream goes on after an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    /// More events follow.
    Continue,
    /// The translator emitted the end of the call; nothing more is read.
    Finished,
}

/// Turns the events of one API into normalized events.
pub(crate) trait Translator: Send + 'static {
    /// Handles one event.
    fn event(
        &mut self,
        event: SseEvent,
        out: &mut Vec<ProviderEvent>,
    ) -> Result<Flow, ProviderError>;

    /// The stream ended without the translator finishing it. Either it
    /// finishes the call now, or the stream was cut.
    fn end(&mut self, out: &mut Vec<ProviderEvent>) -> Result<(), ProviderError>;
}

/// What a call needs besides its request.
pub(crate) struct Call {
    /// The request, ready to send.
    pub(crate) request: RequestBuilder,
    /// The model, for error messages.
    pub(crate) model: String,
    /// The server's quirks, for classifying errors.
    pub(crate) quirks: Quirks,
    /// The credential of the call, so that a server that repeats it in an
    /// error message does not put it into the log.
    pub(crate) credential: Option<Secret>,
}

type Body = Pin<Box<dyn Stream<Item = reqwest::Result<bytes::Bytes>> + Send>>;

/// The state of a call between items.
struct Driver<T> {
    client: HttpClient,
    request: Option<RequestBuilder>,
    model: String,
    quirks: Quirks,
    credential: Option<Secret>,
    body: Option<Body>,
    parser: SseParser,
    translator: T,
    queue: VecDeque<Result<ProviderEvent, ProviderError>>,
    received: usize,
    seen_event: bool,
    stopped: bool,
}

/// Streams a call through `translator`.
pub(crate) fn drive<T: Translator>(client: HttpClient, call: Call, translator: T) -> EventStream {
    let driver = Driver {
        client,
        request: Some(call.request),
        model: call.model,
        quirks: call.quirks,
        credential: call.credential,
        body: None,
        parser: SseParser::new(),
        translator,
        queue: VecDeque::new(),
        received: 0,
        seen_event: false,
        stopped: false,
    };
    Box::pin(futures_util::stream::unfold(
        driver,
        |mut driver| async move { driver.next().await.map(|item| (item, driver)) },
    ))
}

impl<T: Translator> Driver<T> {
    async fn next(&mut self) -> Option<Result<ProviderEvent, ProviderError>> {
        loop {
            if let Some(item) = self.queue.pop_front() {
                return Some(item);
            }
            if self.stopped {
                // Dropping the body closes the connection.
                self.body = None;
                return None;
            }
            if let Some(request) = self.request.take() {
                let opened = open(
                    &self.client,
                    &self.model,
                    &self.quirks,
                    self.credential.as_ref(),
                    request,
                )
                .await;
                match opened {
                    Ok(body) => self.body = Some(body),
                    Err(error) => self.fail(error),
                }
                continue;
            }
            self.read().await;
        }
    }

    fn fail(&mut self, error: ProviderError) {
        self.queue.push_back(Err(error));
        self.stopped = true;
    }

    /// Reads the next chunk of the body and translates the events it
    /// completes.
    async fn read(&mut self) {
        let Some(body) = self.body.as_mut() else {
            self.stopped = true;
            return;
        };
        let wait = if self.seen_event {
            self.client.timeouts().idle
        } else {
            self.client.timeouts().first_event
        };
        let chunk = match tokio::time::timeout(wait, body.next()).await {
            Err(_elapsed) => return self.fail(timed_out()),
            Ok(None) => return self.end(),
            Ok(Some(Err(error))) => {
                tracing::debug!(error = %error, "the provider's stream broke");
                return self.fail(broken_stream());
            }
            Ok(Some(Ok(chunk))) => chunk,
        };
        self.received = self.received.saturating_add(chunk.len());
        if self.received > MAX_RESPONSE_BYTES {
            return self.fail(ProviderError::Permanent(RunFailure::InvalidResponse));
        }
        let events = match self.parser.push(&chunk) {
            Ok(events) => events,
            Err(error) => {
                tracing::debug!(%error, "the provider's stream is not valid");
                return self.fail(ProviderError::Permanent(RunFailure::InvalidResponse));
            }
        };
        let mut out = Vec::new();
        for event in events {
            self.seen_event = true;
            match self.translator.event(event, &mut out) {
                Ok(Flow::Continue) => {}
                Ok(Flow::Finished) => {
                    self.queue.extend(out.drain(..).map(Ok));
                    self.stopped = true;
                    return;
                }
                Err(error) => {
                    self.queue.extend(out.drain(..).map(Ok));
                    return self.fail(error);
                }
            }
        }
        self.queue.extend(out.into_iter().map(Ok));
    }

    /// The body ended before the translator finished the call.
    fn end(&mut self) {
        if self.parser.has_partial_event() {
            return self.fail(broken_stream());
        }
        let mut out = Vec::new();
        let ended = self.translator.end(&mut out);
        self.queue.extend(out.into_iter().map(Ok));
        match ended {
            Ok(()) => self.stopped = true,
            Err(error) => self.fail(error),
        }
    }
}

/// Sends the request and checks the response before its body is read. It
/// borrows the driver's fields one by one, so that the stream stays `Send`.
async fn open(
    client: &HttpClient,
    model: &str,
    quirks: &Quirks,
    credential: Option<&Secret>,
    request: RequestBuilder,
) -> Result<Body, ProviderError> {
    let response = tokio::time::timeout(client.timeouts().headers, request.send())
        .await
        .map_err(|_elapsed| timed_out())?
        .map_err(|error| transport_failure(&error))?;
    let status = response.status();
    if !status.is_success() {
        let headers = response.headers().clone();
        let body = read_error_body(response, client.timeouts().idle).await;
        let details = ErrorDetails::from_body(&body);
        let context = Context {
            model,
            quirks,
            now: client.clock().now(),
        };
        let error = errors::classify_status(status.as_u16(), &headers, &details, context);
        tracing::debug!(
            status = status.as_u16(),
            code = details.code.as_deref(),
            message = loggable(details.message.as_deref(), credential).as_deref(),
            "the provider refused the call"
        );
        return Err(error);
    }
    let is_event_stream = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .trim()
                .to_ascii_lowercase()
                .starts_with("text/event-stream")
        });
    if !is_event_stream {
        tracing::debug!("the provider answered without an event stream");
        return Err(ProviderError::Permanent(RunFailure::InvalidResponse));
    }
    Ok(Box::pin(response.bytes_stream()))
}

/// A provider's message as the log may show it: without the credential, in
/// case the server repeats it (security architecture, rules for handling
/// secrets).
pub(crate) fn loggable(message: Option<&str>, credential: Option<&Secret>) -> Option<String> {
    let message = message?;
    let secret = credential
        .map(Secret::expose)
        .filter(|secret| !secret.is_empty());
    Some(match secret {
        Some(secret) => message.replace(secret, "[REDACTED]"),
        None => message.to_owned(),
    })
}

/// Reads at most the first 64 KiB of an error body.
pub(crate) async fn read_error_body(
    response: reqwest::Response,
    wait: std::time::Duration,
) -> Vec<u8> {
    let mut body = Vec::new();
    let mut chunks = response.bytes_stream();
    let reading = async {
        while let Some(Ok(chunk)) = chunks.next().await {
            let room = MAX_ERROR_BODY_BYTES.saturating_sub(body.len());
            body.extend_from_slice(&chunk[..chunk.len().min(room)]);
            if body.len() >= MAX_ERROR_BODY_BYTES {
                break;
            }
        }
    };
    if tokio::time::timeout(wait, reading).await.is_err() {
        tracing::debug!("the provider's error body did not arrive in time");
    }
    body
}

pub(crate) fn timed_out() -> ProviderError {
    ProviderError::Transient {
        cause: RunFailure::TimedOut,
        retry_after: None,
    }
}

pub(crate) fn broken_stream() -> ProviderError {
    ProviderError::Transient {
        cause: RunFailure::BrokenStream,
        retry_after: None,
    }
}

/// Classifies a failure of the HTTP stack before a response arrived.
pub(crate) fn transport_failure(error: &reqwest::Error) -> ProviderError {
    if error.is_redirect() {
        return ProviderError::Permanent(RunFailure::RedirectRefused);
    }
    if error.is_timeout() {
        return timed_out();
    }
    if error.is_builder() {
        return ProviderError::Permanent(RunFailure::InvalidResponse);
    }
    tracing::debug!(%error, "the provider could not be reached");
    ProviderError::Transient {
        cause: RunFailure::ConnectionFailed,
        retry_after: None,
    }
}
