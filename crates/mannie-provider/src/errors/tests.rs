use std::time::Duration;

use mannie_types::clock::UtcMillis;
use mannie_types::provider::{ApiInterface, ServerKind};
use mannie_types::run::RunFailure;
use reqwest::header::{HeaderMap, HeaderValue};

use super::{
    Context, ErrorDetails, classify_status, classify_stream_error, parse_http_date, retry_after,
};
use crate::provider::ProviderError;
use crate::quirks::quirks_for;

/// 2026-10-13T12:00:00Z
const NOON: i64 = 1_791_892_800_000;

fn context_for(server: ServerKind) -> (crate::quirks::Quirks, UtcMillis) {
    (
        quirks_for(ApiInterface::ChatCompletions, Some(server)),
        UtcMillis::from_millis(NOON),
    )
}

fn classify(status: u16, body: &str, headers: &[(&'static str, &str)]) -> ProviderError {
    let (quirks, now) = context_for(ServerKind::Other);
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.insert(*name, HeaderValue::from_str(value).unwrap());
    }
    let context = Context {
        model: "qwen3",
        quirks: &quirks,
        now,
    };
    classify_status(
        status,
        &map,
        &ErrorDetails::from_body(body.as_bytes()),
        context,
    )
}

fn transient(cause: RunFailure, retry_after: Option<Duration>) -> ProviderError {
    ProviderError::Transient { cause, retry_after }
}

#[test]
fn statuses_fall_into_their_classes() {
    let cases = [
        (
            401,
            "{}",
            ProviderError::Permanent(RunFailure::InvalidCredential),
        ),
        (
            402,
            "{}",
            ProviderError::Permanent(RunFailure::QuotaExceeded),
        ),
        (
            403,
            "{}",
            ProviderError::Permanent(RunFailure::PermissionDenied),
        ),
        (
            404,
            r#"{"error":{"message":"The model `qwen3` does not exist","code":"model_not_found"}}"#,
            ProviderError::Permanent(RunFailure::ModelNotFound {
                model: "qwen3".to_owned(),
            }),
        ),
        (
            404,
            "Not Found",
            ProviderError::Permanent(RunFailure::EndpointNotFound),
        ),
        (408, "", transient(RunFailure::TimedOut, None)),
        (
            409,
            "",
            transient(RunFailure::ServerError { status: 409 }, None),
        ),
        (
            425,
            "",
            transient(RunFailure::ServerError { status: 425 }, None),
        ),
        (
            500,
            "",
            transient(RunFailure::ServerError { status: 500 }, None),
        ),
        (
            503,
            "",
            transient(RunFailure::ServerError { status: 503 }, None),
        ),
        (
            301,
            "",
            ProviderError::Permanent(RunFailure::RedirectRefused),
        ),
        (
            400,
            r#"{"error":{"message":"bad"}}"#,
            ProviderError::Permanent(RunFailure::BadRequest { status: 400 }),
        ),
        (
            418,
            "",
            ProviderError::Permanent(RunFailure::BadRequest { status: 418 }),
        ),
    ];
    for (status, body, expected) in cases {
        assert_eq!(classify(status, body, &[]), expected, "{status} {body}");
    }
}

#[test]
fn a_rate_limit_is_transient_but_an_exhausted_quota_is_not() {
    let limited = classify(
        429,
        r#"{"error":{"code":"rate_limit_exceeded"}}"#,
        &[("retry-after", "7")],
    );
    let quota = classify(
        429,
        r#"{"error":{"type":"insufficient_quota","code":"insufficient_quota"}}"#,
        &[],
    );

    assert_eq!(
        limited,
        ProviderError::Transient {
            cause: RunFailure::RateLimited {
                retry_at: Some(UtcMillis::from_millis(NOON + 7_000))
            },
            retry_after: Some(Duration::from_secs(7)),
        }
    );
    assert_eq!(quota, ProviderError::Permanent(RunFailure::QuotaExceeded));
}

#[test]
fn context_overflows_are_recognized_by_code_and_by_wording() {
    let openai = classify(
        400,
        r#"{"error":{"message":"too long","code":"context_length_exceeded"}}"#,
        &[],
    );
    let vllm = classify(
        400,
        r#"{"object":"error","message":"This model's maximum context length is 32768 tokens. However, you requested 40000 tokens.","type":"BadRequestError","code":400}"#,
        &[],
    );
    let lm_studio = classify(
        400,
        r#"{"error":"Trying to keep the first 9000 tokens when context the overflows. However, the model is loaded with context length of only 8192 tokens"}"#,
        &[],
    );

    assert_eq!(openai, ProviderError::ContextOverflow);
    assert_eq!(vllm, ProviderError::ContextOverflow);
    assert_eq!(lm_studio, ProviderError::ContextOverflow);
}

#[test]
fn content_filters_are_permanent() {
    let filtered = classify(
        400,
        r#"{"error":{"code":"content_filter","message":"blocked"}}"#,
        &[],
    );

    assert_eq!(
        filtered,
        ProviderError::Permanent(RunFailure::ContentFiltered)
    );
}

#[test]
fn errors_inside_a_stream_are_classified_without_a_status() {
    let (quirks, now) = context_for(ServerKind::OpenAi);
    let context = Context {
        model: "gpt-5",
        quirks: &quirks,
        now,
    };
    let classify = |json: &str| {
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        classify_stream_error(&ErrorDetails::from_value(&value), context)
    };

    assert_eq!(
        classify(r#"{"code":"context_length_exceeded","message":"x"}"#),
        ProviderError::ContextOverflow
    );
    assert_eq!(
        classify(r#"{"error":{"code":"insufficient_quota"}}"#),
        ProviderError::Permanent(RunFailure::QuotaExceeded)
    );
    assert!(matches!(
        classify(r#"{"code":"rate_limit_exceeded"}"#),
        ProviderError::Transient {
            cause: RunFailure::RateLimited { .. },
            ..
        }
    ));
    assert_eq!(
        classify(r#"{"error":{"message":"overloaded"}}"#),
        transient(RunFailure::ServerError { status: 500 }, None)
    );
}

#[test]
fn retry_after_is_read_in_seconds_milliseconds_and_dates() {
    let now = UtcMillis::from_millis(NOON);
    let headers = |pairs: &[(&'static str, &str)]| {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    };

    assert_eq!(
        retry_after(&headers(&[("retry-after", "12")]), now),
        Some(Duration::from_secs(12))
    );
    assert_eq!(
        retry_after(
            &headers(&[("retry-after-ms", "1500"), ("retry-after", "9")]),
            now
        ),
        Some(Duration::from_millis(1_500))
    );
    assert_eq!(
        retry_after(
            &headers(&[("retry-after", "Tue, 13 Oct 2026 12:00:30 GMT")]),
            now
        ),
        Some(Duration::from_secs(30))
    );
    assert_eq!(
        retry_after(
            &headers(&[("retry-after", "Tue, 13 Oct 2026 11:00:00 GMT")]),
            now
        ),
        Some(Duration::ZERO),
        "a date in the past means now"
    );
    assert_eq!(retry_after(&headers(&[("retry-after", "soon")]), now), None);
    assert_eq!(retry_after(&headers(&[]), now), None);
}

#[test]
fn http_dates_parse_only_in_their_standard_form() {
    assert_eq!(
        parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"),
        Some(UtcMillis::from_millis(784_111_777_000))
    );
    assert_eq!(parse_http_date("Sunday, 06-Nov-94 08:49:37 GMT"), None);
    assert_eq!(parse_http_date("Sun, 06 Nov 1994 08:49:37 PST"), None);
    assert_eq!(parse_http_date("Sun, 31 Feb 1994 08:49:37 GMT"), None);
}

#[test]
fn error_details_come_from_every_shape_servers_use() {
    let nested = ErrorDetails::from_body(br#"{"error":{"message":"m","type":"t","code":"c"}}"#);
    let flat = ErrorDetails::from_body(br#"{"message":"m","code":400}"#);
    let string = ErrorDetails::from_body(br#"{"error":"m"}"#);
    let text = ErrorDetails::from_body(b"upstream timeout");

    assert_eq!(nested.code.as_deref(), Some("c"));
    assert_eq!(nested.kind.as_deref(), Some("t"));
    assert_eq!(flat.message.as_deref(), Some("m"));
    assert_eq!(flat.code, None, "numeric codes are not error codes");
    assert_eq!(string.message.as_deref(), Some("m"));
    assert_eq!(text.message.as_deref(), Some("upstream timeout"));
}
