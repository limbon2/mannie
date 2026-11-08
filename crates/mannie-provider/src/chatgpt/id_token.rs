//! Checking the ID token of a ChatGPT sign-in (ADR-0062, decision 3): its
//! RS256 signature against the issuer's published keys, and its issuer,
//! audience, nonce, subject and times, as OpenAI's guide asks (verified
//! 2026-11-08). Mannie keeps only the subject and the email address; the
//! token itself is dropped after the check.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use mannie_types::clock::UtcMillis;
use mannie_types::run::RunFailure;
use mannie_types::secret::Secret;
use ring::signature::{RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents};
use serde_json::Value;

use super::ChatGptEndpoints;
use crate::stream::{read_error_body, transport_failure};
use crate::transport::HttpClient;

/// How far the clocks of Mannie and OpenAI may differ, in milliseconds.
const LEEWAY_MILLIS: i64 = 5_000;

/// How long reading the issuer's keys may take.
const KEYS_TIMEOUT: Duration = Duration::from_secs(15);

/// What the ID token says about the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdClaims {
    /// The account's stable identifier at OpenAI, `sub`.
    pub subject: String,
    /// The account's email address, when the token names it.
    pub email: Option<String>,
}

/// Why an ID token failed its check.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdTokenError {
    /// The token is not a signed JWT.
    #[error("the ID token is not a signed JWT")]
    Malformed,
    /// The token is signed with another algorithm than RS256.
    #[error("the ID token is not signed with RS256")]
    UnsupportedAlgorithm,
    /// None of the issuer's keys matches the token.
    #[error("no key of the issuer matches the ID token")]
    UnknownKey,
    /// The signature does not verify.
    #[error("the ID token's signature does not verify")]
    BadSignature,
    /// A claim is missing or wrong.
    #[error("the ID token's claim {claim} is not valid")]
    InvalidClaim {
        /// The claim.
        claim: &'static str,
    },
    /// The issuer's keys could not be read.
    #[error("the issuer's keys could not be read: {failure}")]
    KeysUnavailable {
        /// Why.
        failure: RunFailure,
    },
}

/// What an ID token must say.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Expected<'a> {
    /// The issuer, `iss`.
    pub(crate) issuer: &'a str,
    /// The client that the token is for, `aud`.
    pub(crate) audience: &'a str,
    /// The sign-in's nonce.
    pub(crate) nonce: &'a str,
    /// The current time.
    pub(crate) now: UtcMillis,
}

/// Checks an ID token against the issuer's current keys.
pub async fn verify_id_token(
    client: &HttpClient,
    endpoints: &ChatGptEndpoints,
    id_token: &Secret,
    client_id: &str,
    nonce: &str,
) -> Result<IdClaims, IdTokenError> {
    let keys = issuer_keys(client, endpoints).await?;
    let expected = Expected {
        issuer: &endpoints.issuer,
        audience: client_id,
        nonce,
        now: client.clock().now(),
    };
    verify_with_keys(id_token.expose(), &keys, &expected)
}

/// Reads the issuer's published keys.
async fn issuer_keys(
    client: &HttpClient,
    endpoints: &ChatGptEndpoints,
) -> Result<Value, IdTokenError> {
    let unavailable = |failure| IdTokenError::KeysUnavailable { failure };
    let response = tokio::time::timeout(
        KEYS_TIMEOUT,
        client.get(endpoints.jwks.clone(), None).send(),
    )
    .await
    .map_err(|_elapsed| unavailable(RunFailure::TimedOut))?
    .map_err(|error| unavailable(transport_failure(&error).failure()))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(unavailable(RunFailure::BadRequest { status }));
    }
    let body = read_error_body(response, KEYS_TIMEOUT).await;
    serde_json::from_slice(&body).map_err(|_not_json| unavailable(RunFailure::InvalidResponse))
}

/// Checks an ID token against a key set (RFC 7517).
pub(crate) fn verify_with_keys(
    token: &str,
    keys: &Value,
    expected: &Expected<'_>,
) -> Result<IdClaims, IdTokenError> {
    let mut parts = token.split('.');
    let (Some(header), Some(payload), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(IdTokenError::Malformed);
    };
    let header = decoded_json(header)?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_not_base64| IdTokenError::Malformed)?;
    if signature.is_empty() {
        return Err(IdTokenError::Malformed);
    }
    if header.get("alg").and_then(Value::as_str) != Some("RS256") {
        return Err(IdTokenError::UnsupportedAlgorithm);
    }
    let kid = header.get("kid").and_then(Value::as_str);
    let candidates = signing_keys(keys, kid);
    if candidates.is_empty() {
        return Err(IdTokenError::UnknownKey);
    }
    // The signature covers the header and the payload as they were encoded.
    let signed = &token[..header_and_payload_length(token)];
    let verified = candidates.iter().any(|(modulus, exponent)| {
        RsaPublicKeyComponents {
            n: modulus.as_slice(),
            e: exponent.as_slice(),
        }
        .verify(&RSA_PKCS1_2048_8192_SHA256, signed.as_bytes(), &signature)
        .is_ok()
    });
    if !verified {
        return Err(IdTokenError::BadSignature);
    }
    claims(&decoded_json(payload)?, expected)
}

/// The length of `header.payload`, the part of a JWT that its signature
/// covers.
fn header_and_payload_length(token: &str) -> usize {
    token.rfind('.').unwrap_or(token.len())
}

/// A base64url part of a JWT, decoded as a JSON object.
fn decoded_json(part: &str) -> Result<Value, IdTokenError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(part)
        .map_err(|_not_base64| IdTokenError::Malformed)?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_not_json| IdTokenError::Malformed)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(IdTokenError::Malformed)
    }
}

/// The RSA signing keys of a key set that can have signed a token with `kid`:
/// their modulus and exponent.
fn signing_keys(keys: &Value, kid: Option<&str>) -> Vec<(Vec<u8>, Vec<u8>)> {
    let text = |key: &Value, name: &str| key.get(name).and_then(Value::as_str).map(str::to_owned);
    keys.get("keys")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|key| text(key, "kty").as_deref() == Some("RSA"))
        .filter(|key| text(key, "use").is_none_or(|usage| usage == "sig"))
        .filter(|key| text(key, "alg").is_none_or(|algorithm| algorithm == "RS256"))
        .filter(|key| kid.is_none_or(|kid| text(key, "kid").as_deref() == Some(kid)))
        .filter_map(|key| {
            let modulus = URL_SAFE_NO_PAD.decode(text(key, "n")?).ok()?;
            let exponent = URL_SAFE_NO_PAD.decode(text(key, "e")?).ok()?;
            Some((modulus, exponent))
        })
        .collect()
}

/// Checks the claims of a token whose signature verified.
fn claims(payload: &Value, expected: &Expected<'_>) -> Result<IdClaims, IdTokenError> {
    let invalid = |claim| IdTokenError::InvalidClaim { claim };
    let text = |name: &str| payload.get(name).and_then(Value::as_str);
    let seconds = |name: &str| {
        payload
            .get(name)
            .and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
            })
            .map(|seconds| seconds.saturating_mul(1_000))
    };
    if text("iss") != Some(expected.issuer) {
        return Err(invalid("iss"));
    }
    let audience = payload.get("aud");
    let names_audience = match audience {
        Some(Value::String(audience)) => audience == expected.audience,
        Some(Value::Array(audiences)) => audiences
            .iter()
            .any(|audience| audience.as_str() == Some(expected.audience)),
        _ => false,
    };
    if !names_audience {
        return Err(invalid("aud"));
    }
    let now = expected.now.as_millis();
    let expires = seconds("exp").ok_or(invalid("exp"))?;
    if expires.saturating_add(LEEWAY_MILLIS) <= now {
        return Err(invalid("exp"));
    }
    let issued = seconds("iat").ok_or(invalid("iat"))?;
    if issued > now.saturating_add(LEEWAY_MILLIS) {
        return Err(invalid("iat"));
    }
    if text("nonce") != Some(expected.nonce) {
        return Err(invalid("nonce"));
    }
    let subject = text("sub")
        .filter(|subject| !subject.is_empty())
        .ok_or(invalid("sub"))?;
    let session_is_valid = payload
        .get("sid")
        .is_none_or(|session| session.as_str().is_some_and(|session| !session.is_empty()));
    if !session_is_valid {
        return Err(invalid("sid"));
    }
    Ok(IdClaims {
        subject: subject.to_owned(),
        email: text("email").map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use mannie_types::clock::UtcMillis;
    use serde_json::{Value, json};

    use super::{Expected, IdClaims, IdTokenError, verify_with_keys};
    use crate::chatgpt::example_key::{ISSUER, NOW_MILLIS, key_set, signed_token, valid_claims};

    fn expected() -> Expected<'static> {
        Expected {
            issuer: ISSUER,
            audience: "oaiapp_1",
            nonce: "nonce-1",
            now: UtcMillis::from_millis(NOW_MILLIS),
        }
    }

    fn with(claims: &Value, name: &str, value: Value) -> Value {
        let mut claims = claims.clone();
        claims[name] = value;
        claims
    }

    #[test]
    fn a_valid_token_gives_the_subject_and_email() {
        let token = signed_token("key-1", &valid_claims("oaiapp_1", "nonce-1"));

        let claims = verify_with_keys(&token, &key_set("key-1"), &expected()).unwrap();

        assert_eq!(
            claims,
            IdClaims {
                subject: "user-123".to_owned(),
                email: Some("owner@example.com".to_owned()),
            }
        );
    }

    #[test]
    fn an_audience_list_that_names_the_client_passes() {
        let claims = with(
            &valid_claims("oaiapp_1", "nonce-1"),
            "aud",
            json!(["https://api.openai.com/v1", "oaiapp_1"]),
        );
        let token = signed_token("key-1", &claims);

        assert!(
            verify_with_keys(&token, &key_set("key-1"), &expected())
                .is_ok_and(|claims| claims.subject == "user-123")
        );
    }

    #[test]
    fn a_changed_payload_fails_the_signature() {
        let token = signed_token("key-1", &valid_claims("oaiapp_1", "nonce-1"));
        let mut parts: Vec<&str> = token.split('.').collect();
        let forged = URL_SAFE_NO_PAD.encode(
            with(
                &valid_claims("oaiapp_1", "nonce-1"),
                "sub",
                json!("someone-else"),
            )
            .to_string(),
        );
        parts[1] = &forged;

        let error = verify_with_keys(&parts.join("."), &key_set("key-1"), &expected()).unwrap_err();

        assert_eq!(error, IdTokenError::BadSignature);
    }

    #[test]
    fn an_unknown_key_or_another_algorithm_is_refused() {
        let token = signed_token("key-1", &valid_claims("oaiapp_1", "nonce-1"));
        let unsigned = format!(
            "{}.{}.",
            URL_SAFE_NO_PAD.encode(json!({ "alg": "none" }).to_string()),
            URL_SAFE_NO_PAD.encode(valid_claims("oaiapp_1", "nonce-1").to_string())
        );
        let hmac = format!(
            "{}.{}.c2lnbmF0dXJl",
            URL_SAFE_NO_PAD.encode(json!({ "alg": "HS256", "kid": "key-1" }).to_string()),
            URL_SAFE_NO_PAD.encode(valid_claims("oaiapp_1", "nonce-1").to_string())
        );

        assert_eq!(
            verify_with_keys(&token, &key_set("key-2"), &expected()).unwrap_err(),
            IdTokenError::UnknownKey
        );
        assert_eq!(
            verify_with_keys(&unsigned, &key_set("key-1"), &expected()).unwrap_err(),
            IdTokenError::Malformed
        );
        assert_eq!(
            verify_with_keys(&hmac, &key_set("key-1"), &expected()).unwrap_err(),
            IdTokenError::UnsupportedAlgorithm
        );
        assert_eq!(
            verify_with_keys("not-a-jwt", &key_set("key-1"), &expected()).unwrap_err(),
            IdTokenError::Malformed
        );
    }

    #[test]
    fn wrong_claims_are_refused_by_name() {
        let valid = valid_claims("oaiapp_1", "nonce-1");
        let now = NOW_MILLIS / 1_000;
        let cases = [
            (with(&valid, "iss", json!("https://evil.example")), "iss"),
            (with(&valid, "aud", json!("oaiapp_2")), "aud"),
            (with(&valid, "nonce", json!("nonce-2")), "nonce"),
            (with(&valid, "exp", json!(now - 60)), "exp"),
            (with(&valid, "iat", json!(now + 600)), "iat"),
            (with(&valid, "sub", json!("")), "sub"),
            (with(&valid, "sid", json!("")), "sid"),
        ];

        for (claims, claim) in cases {
            let token = signed_token("key-1", &claims);

            assert_eq!(
                verify_with_keys(&token, &key_set("key-1"), &expected()).unwrap_err(),
                IdTokenError::InvalidClaim { claim },
                "{claim}"
            );
        }
    }

    #[test]
    fn the_clocks_may_differ_by_five_seconds() {
        let now = NOW_MILLIS / 1_000;
        let just_expired = with(&valid_claims("oaiapp_1", "nonce-1"), "exp", json!(now - 2));
        let token = signed_token("key-1", &just_expired);

        assert!(
            verify_with_keys(&token, &key_set("key-1"), &expected())
                .is_ok_and(|claims| claims.email.is_some())
        );
    }
}
