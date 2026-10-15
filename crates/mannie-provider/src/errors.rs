//! Classifying failures into transient, context overflow and permanent
//! (ADR-0011, decision 3), and reading how long a provider asks to wait.

use std::time::Duration;

use mannie_types::clock::UtcMillis;
use mannie_types::run::RunFailure;
use reqwest::header::HeaderMap;
use serde_json::Value;
use time::{Date, Month, PrimitiveDateTime, Time};

use crate::provider::ProviderError;
use crate::quirks::Quirks;

/// Phrases that servers use when a request is longer than the model's
/// context, in lower case. A server's quirk profile adds its own.
const OVERFLOW_PHRASES: &[&str] = &[
    "context_length_exceeded",
    "maximum context length",
    "context length exceeded",
    "exceeds the context",
    "exceeds the maximum context",
    "longer than the model's context",
    "prompt is too long",
    "input is too long",
    "reduce the length of the messages",
];

/// Phrases that servers use when they refuse a request because of its tools,
/// in lower case (docs/features/system-tools/tool-calls.md, adapters): vLLM
/// without its tool options, Ollama with a model without a tool template,
/// OpenRouter without an endpoint that supports tools, and llama.cpp without
/// its chat templates.
const TOOLS_UNSUPPORTED_PHRASES: &[&str] = &[
    "does not support tools",
    "enable-auto-tool-choice",
    "tool choice requires",
    "support tool use",
    "tools param requires",
    "tool calling is not supported",
    "tools are not supported",
];

/// What a server's error document says: OpenAI's `{"error": {...}}`, a flat
/// object with `message` and `code`, or a plain string.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ErrorDetails {
    /// The error code, when it is a string.
    pub(crate) code: Option<String>,
    /// The error type.
    pub(crate) kind: Option<String>,
    /// The message, for the log and for recognizing a context overflow.
    pub(crate) message: Option<String>,
}

impl ErrorDetails {
    /// Reads the details from an error body. Anything that is not JSON counts
    /// as the message.
    pub(crate) fn from_body(body: &[u8]) -> Self {
        match serde_json::from_slice::<Value>(body) {
            Ok(value) => Self::from_value(&value),
            Err(_not_json) => {
                let text = String::from_utf8_lossy(body).trim().to_owned();
                Self {
                    message: (!text.is_empty()).then_some(text),
                    ..Self::default()
                }
            }
        }
    }

    /// Reads the details from an error object in a stream or a body.
    pub(crate) fn from_value(value: &Value) -> Self {
        let error = value.get("error").unwrap_or(value);
        if let Some(message) = error.as_str() {
            return Self {
                message: Some(message.to_owned()),
                ..Self::default()
            };
        }
        let text = |key: &str| error.get(key).and_then(Value::as_str).map(str::to_owned);
        Self {
            code: text("code"),
            kind: text("type"),
            message: text("message"),
        }
    }

    fn mentions(&self, phrases: &[&str]) -> bool {
        let fields = [&self.code, &self.kind, &self.message];
        fields.into_iter().flatten().any(|field| {
            let lower = field.to_lowercase();
            phrases.iter().any(|phrase| lower.contains(phrase))
        })
    }

    fn has_code(&self, codes: &[&str]) -> bool {
        let fields = [&self.code, &self.kind];
        fields
            .into_iter()
            .flatten()
            .any(|field| codes.contains(&field.as_str()))
    }
}

/// What classifying a failure needs to know.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Context<'a> {
    /// The model of the call, for the message about an unknown model.
    pub(crate) model: &'a str,
    /// The server's quirks.
    pub(crate) quirks: &'a Quirks,
    /// The current time, to read `Retry-After` dates.
    pub(crate) now: UtcMillis,
}

impl Context<'_> {
    fn is_context_overflow(&self, details: &ErrorDetails) -> bool {
        details.mentions(OVERFLOW_PHRASES) || details.mentions(self.quirks.overflow_phrases)
    }

    fn rate_limited(&self, retry_after: Option<Duration>) -> ProviderError {
        ProviderError::Transient {
            cause: RunFailure::RateLimited {
                retry_at: retry_after.map(|wait| self.now.saturating_add(wait)),
            },
            retry_after,
        }
    }
}

/// Classifies an HTTP response that did not succeed.
pub(crate) fn classify_status(
    status: u16,
    headers: &HeaderMap,
    details: &ErrorDetails,
    context: Context<'_>,
) -> ProviderError {
    let retry_after = retry_after(headers, context.now);
    let transient = |cause| ProviderError::Transient { cause, retry_after };
    match status {
        300..=399 => ProviderError::Permanent(RunFailure::RedirectRefused),
        400..=499 if details.mentions(TOOLS_UNSUPPORTED_PHRASES) => {
            ProviderError::Permanent(RunFailure::ToolsUnsupported)
        }
        401 => ProviderError::Permanent(RunFailure::InvalidCredential),
        402 => ProviderError::Permanent(RunFailure::QuotaExceeded),
        403 => ProviderError::Permanent(RunFailure::PermissionDenied),
        404 => {
            let names_model =
                details.has_code(&["model_not_found"]) || details.mentions(&["model"]);
            if names_model {
                ProviderError::Permanent(RunFailure::ModelNotFound {
                    model: context.model.to_owned(),
                })
            } else {
                ProviderError::Permanent(RunFailure::EndpointNotFound)
            }
        }
        408 => transient(RunFailure::TimedOut),
        429 if details.has_code(&["insufficient_quota"]) => {
            ProviderError::Permanent(RunFailure::QuotaExceeded)
        }
        429 => context.rate_limited(retry_after),
        400 | 413 | 422 if context.is_context_overflow(details) => ProviderError::ContextOverflow,
        400..=499 if details.has_code(&["content_filter", "content_policy_violation"]) => {
            ProviderError::Permanent(RunFailure::ContentFiltered)
        }
        409 | 425 | 500..=599 => transient(RunFailure::ServerError { status }),
        _ => ProviderError::Permanent(RunFailure::BadRequest { status }),
    }
}

/// Classifies an error that arrives inside a stream, where no HTTP status
/// tells its class.
pub(crate) fn classify_stream_error(details: &ErrorDetails, context: Context<'_>) -> ProviderError {
    if context.is_context_overflow(details) {
        return ProviderError::ContextOverflow;
    }
    if details.mentions(TOOLS_UNSUPPORTED_PHRASES) {
        return ProviderError::Permanent(RunFailure::ToolsUnsupported);
    }
    if details.has_code(&["insufficient_quota"]) {
        return ProviderError::Permanent(RunFailure::QuotaExceeded);
    }
    if details.has_code(&["rate_limit_exceeded", "rate_limit_error"]) {
        return context.rate_limited(None);
    }
    if details.has_code(&["content_filter", "content_policy_violation"]) {
        return ProviderError::Permanent(RunFailure::ContentFiltered);
    }
    if details.has_code(&["invalid_api_key", "authentication_error"]) {
        return ProviderError::Permanent(RunFailure::InvalidCredential);
    }
    // A server that fails in the middle of a reply usually recovers: the
    // runtime tries again.
    ProviderError::Transient {
        cause: RunFailure::ServerError { status: 500 },
        retry_after: None,
    }
}

/// How long the provider asks to wait: `retry-after-ms` in milliseconds, or
/// `Retry-After` in seconds or as an HTTP date.
pub(crate) fn retry_after(headers: &HeaderMap, now: UtcMillis) -> Option<Duration> {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
    };
    if let Some(millis) = header("retry-after-ms").and_then(|text| text.parse::<f64>().ok())
        && millis.is_finite()
        && millis >= 0.0
    {
        return Some(Duration::from_secs_f64(millis / 1000.0));
    }
    let value = header("retry-after")?;
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let at = parse_http_date(value)?;
    let wait = at.as_millis().saturating_sub(now.as_millis());
    Some(Duration::from_millis(u64::try_from(wait).unwrap_or(0)))
}

/// Reads an HTTP date in its standard form, IMF-fixdate:
/// `Sun, 06 Nov 1994 08:49:37 GMT`.
pub(crate) fn parse_http_date(text: &str) -> Option<UtcMillis> {
    let rest = text.split_once(", ")?.1;
    let mut parts = rest.split(' ');
    let day: u8 = parts.next()?.parse().ok()?;
    let month = match parts.next()? {
        "Jan" => Month::January,
        "Feb" => Month::February,
        "Mar" => Month::March,
        "Apr" => Month::April,
        "May" => Month::May,
        "Jun" => Month::June,
        "Jul" => Month::July,
        "Aug" => Month::August,
        "Sep" => Month::September,
        "Oct" => Month::October,
        "Nov" => Month::November,
        "Dec" => Month::December,
        _ => return None,
    };
    let year: i32 = parts.next()?.parse().ok()?;
    let mut clock = parts.next()?.split(':');
    let hour: u8 = clock.next()?.parse().ok()?;
    let minute: u8 = clock.next()?.parse().ok()?;
    let second: u8 = clock.next()?.parse().ok()?;
    if parts.next()? != "GMT" {
        return None;
    }
    let date = Date::from_calendar_date(year, month, day).ok()?;
    let time = Time::from_hms(hour, minute, second).ok()?;
    let seconds = PrimitiveDateTime::new(date, time)
        .assume_utc()
        .unix_timestamp();
    Some(UtcMillis::from_millis(seconds.checked_mul(1000)?))
}

#[cfg(test)]
mod tests;
