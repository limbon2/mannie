//! Secret detection and redaction for agent outputs, approvals, transcripts,
//! archives, logs and error messages (ADR-0075, decision 5).
//!
//! Structured secret fields and cookies are always replaced with `[redacted]`
//! regardless of length. Free-form text (response bodies, URLs and transport
//! error messages) additionally scrubs every known secret or cookie value of
//! at least [`MIN_SCRUB_BYTES`] bytes, along with its URL-encoded and
//! Basic-auth base64 forms.

use std::collections::BTreeSet;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use mannie_plugin_sdk::contracts::looks_like_secret;
use url::form_urlencoded;

use crate::model::{AuthConfig, BodyConfig, KeyValueEntry, VariableEntry};

/// The fixed redaction placeholder shown to agents and recorded in approvals,
/// transcripts, archives and logs.
pub(crate) const REDACTED: &str = "[redacted]";

/// Shortest raw secret length scrubbed from arbitrary response bodies and error
/// strings, preventing false-positive masking of short tokens like `"1"` or
/// `"ok"`. Structured secret fields and cookies are always replaced regardless
/// of length.
pub(crate) const MIN_SCRUB_BYTES: usize = 8;

/// Splits camel-case identifiers (`apiKey` -> `api_Key`) so `looks_like_secret`
/// detects secret words inside camel-case names.
pub(crate) fn camel_case_apart(name: &str) -> String {
    let mut apart = String::with_capacity(name.len() + 4);
    let mut after_lower = false;

    for ch in name.chars() {
        if after_lower && ch.is_ascii_uppercase() {
            apart.push('_');
        }

        after_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        apart.push(ch);
    }
    apart
}

/// Whether a header name always carries credentials or cookies.
pub(crate) fn is_secret_header_name(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    let explicit = matches!(
        lower.as_str(),
        "authorization"
            | "proxy-authorization"
            | "cookie"
            | "set-cookie"
            | "x-api-key"
            | "x-auth-token"
            | "x-access-token"
            | "x-csrf-token"
            | "x-xsrf-token"
            | "api-key"
            | "apikey"
    );
    explicit || looks_like_secret(&camel_case_apart(&lower))
}

/// Whether a variable, query parameter, path parameter or form field name
/// implies a secret value.
pub(crate) fn is_secret_name(name: &str) -> bool {
    looks_like_secret(&camel_case_apart(name.trim()))
}

/// Returns a copy of `entries` with every secret entry's value replaced by
/// `[redacted]`.
pub(crate) fn redact_key_values(
    entries: &[KeyValueEntry],
    header_rules: bool,
) -> Vec<KeyValueEntry> {
    entries
        .iter()
        .map(|entry| {
            let secret = entry.secret
                || if header_rules {
                    is_secret_header_name(&entry.key)
                } else {
                    is_secret_name(&entry.key)
                };
            let value = if secret {
                REDACTED.to_owned()
            } else {
                entry.value.clone()
            };
            KeyValueEntry {
                key: entry.key.clone(),
                value,
                enabled: entry.enabled,
                secret,
                description: entry.description.clone(),
            }
        })
        .collect()
}

/// Returns a copy of `variables` with every secret variable's value replaced by
/// `[redacted]`.
pub(crate) fn redact_variables(variables: &[VariableEntry]) -> Vec<VariableEntry> {
    variables
        .iter()
        .map(|entry| {
            let secret = entry.secret || is_secret_name(&entry.key);
            let value = if secret {
                REDACTED.to_owned()
            } else {
                entry.value.clone()
            };
            VariableEntry {
                key: entry.key.clone(),
                value,
                enabled: entry.enabled,
                secret,
                description: entry.description.clone(),
            }
        })
        .collect()
}

/// Returns a copy of `auth` with every credential field replaced by
/// `[redacted]`.
pub(crate) fn redact_auth(auth: &AuthConfig) -> AuthConfig {
    match auth {
        AuthConfig::Inherit => AuthConfig::Inherit,
        AuthConfig::None => AuthConfig::None,
        AuthConfig::Bearer { .. } => AuthConfig::Bearer {
            token: REDACTED.to_owned(),
        },
        AuthConfig::Basic { username, .. } => AuthConfig::Basic {
            username: username.clone(),
            password: REDACTED.to_owned(),
        },
        AuthConfig::ApiKey { name, location, .. } => AuthConfig::ApiKey {
            name: name.clone(),
            value: REDACTED.to_owned(),
            location: *location,
        },
    }
}

/// Returns a copy of `body` with any secret form fields replaced by
/// `[redacted]`.
pub(crate) fn redact_body(body: &BodyConfig) -> BodyConfig {
    match body {
        BodyConfig::None => BodyConfig::None,
        BodyConfig::Json { content } => BodyConfig::Json {
            content: content.clone(),
        },
        BodyConfig::Text {
            content,
            content_type,
        } => BodyConfig::Text {
            content: content.clone(),
            content_type: content_type.clone(),
        },
        BodyConfig::FormUrlEncoded { entries } => BodyConfig::FormUrlEncoded {
            entries: redact_key_values(entries, false),
        },
    }
}

/// Scrubs known secret and cookie values from free-form text (response bodies,
/// headers, URLs and error text) before returning anything to an agent.
#[derive(Debug, Default, Clone)]
pub(crate) struct Redactor {
    needles: Vec<String>,
}

impl Redactor {
    /// Builds a redactor from raw secret values and optional Basic-auth
    /// `(username, password)` pairs.
    pub(crate) fn new<'a>(
        secrets: impl IntoIterator<Item = &'a str>,
        basic_pairs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Self {
        let mut set = BTreeSet::new();

        for secret in secrets {
            Self::insert_forms(&mut set, secret);
        }

        for (user, password) in basic_pairs {
            Self::insert_forms(&mut set, password);

            if !password.is_empty() {
                let encoded = BASE64.encode(format!("{user}:{password}"));
                if encoded.len() >= MIN_SCRUB_BYTES {
                    set.insert(encoded);
                }
            }
        }

        let mut needles: Vec<String> = set.into_iter().collect();
        needles.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
        Self { needles }
    }

    /// Adds a newly discovered secret (such as a `Set-Cookie` value returned by
    /// the server) to this redactor.
    pub(crate) fn add_secret(&mut self, secret: &str) {
        let mut set: BTreeSet<String> = self.needles.iter().cloned().collect();
        Self::insert_forms(&mut set, secret);
        let mut needles: Vec<String> = set.into_iter().collect();
        needles.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
        self.needles = needles;
    }

    fn insert_forms(set: &mut BTreeSet<String>, value: &str) {
        let trimmed = value.trim();

        if trimmed.len() < MIN_SCRUB_BYTES {
            return;
        }

        set.insert(trimmed.to_owned());

        let url_encoded: String = form_urlencoded::byte_serialize(trimmed.as_bytes()).collect();
        if url_encoded.len() >= MIN_SCRUB_BYTES {
            set.insert(url_encoded);
        }

        let percent_space = percent_encode_rfc3986(trimmed);
        if percent_space.len() >= MIN_SCRUB_BYTES {
            set.insert(percent_space);
        }
    }

    /// Replaces every occurrence of every registered secret needle with
    /// `[redacted]`.
    pub(crate) fn scrub(&self, text: &str) -> String {
        let mut cleaned = text.to_owned();

        for needle in &self.needles {
            if cleaned.contains(needle.as_str()) {
                cleaned = cleaned.replace(needle.as_str(), REDACTED);
            }
        }
        cleaned
    }
}

/// RFC 3986 percent-encoding of `value` (uppercase hex, keeping unreserved
/// characters).
pub(crate) fn percent_encode_rfc3986(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(value.len());

    for byte in value.bytes() {
        let unreserved = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');

        if unreserved {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{REDACTED, Redactor, is_secret_header_name, is_secret_name};

    #[test]
    fn secret_headers_and_names_are_detected() {
        for header in [
            "Authorization",
            "cookie",
            "Set-Cookie",
            "X-Api-Key",
            "X-Auth-Token",
            "Proxy-Authorization",
            "X-Custom-Secret",
        ] {
            assert!(is_secret_header_name(header), "{header}");
        }
        assert!(!is_secret_header_name("Accept"));
        assert!(!is_secret_header_name("Content-Type"));
        assert!(is_secret_name("apiKey"));
        assert!(is_secret_name("accessToken"));
        assert!(is_secret_name("db_password"));
        assert!(!is_secret_name("pageSize"));
    }

    #[test]
    fn redactor_scrubs_raw_url_encoded_and_basic_auth_forms() {
        let token = "sk_live_abc+123/xyz";
        let redactor = Redactor::new([token, "short"], [("alice", "super_secret_pw")]);

        let raw = format!("echoed {token} and short");
        assert_eq!(redactor.scrub(&raw), format!("echoed {REDACTED} and short"));

        let encoded = "https://api.example.com?k=sk_live_abc%2B123%2Fxyz";
        assert_eq!(
            redactor.scrub(encoded),
            format!("https://api.example.com?k={REDACTED}")
        );

        let basic_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            "alice:super_secret_pw",
        );
        assert_eq!(
            redactor.scrub(&format!("Authorization: Basic {basic_b64}")),
            format!("Authorization: Basic {REDACTED}")
        );
    }
}
