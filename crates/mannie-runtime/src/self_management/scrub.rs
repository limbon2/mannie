//! Pre-persistence secret scrubbing for `mannie_configure` tool calls
//! (ADR-0081, T-129).
//!
//! When the owner intentionally types a throwaway credential in chat and the
//! model passes it in `mannie_configure`'s top-level `secrets` object, this
//! module extracts each secret value into `Zeroizing` [`Secret`] storage and
//! replaces the value in the JSON string with `"[redacted]"` BEFORE
//! `Change::Delta` is written to SQLite or published to any subscriber.
//!
//! As defense in depth, it also scrubs any unexpected secret-named string
//! fields that a model mistakenly puts inside `arguments`, marking them so
//! `prepare` refuses the call with clear guidance instead of leaking the value
//! to disk.

use std::collections::BTreeMap;

use mannie_types::secret::Secret;
use serde_json::{Map, Value};

/// Marker written in place of scrubbed secret values in persisted tool-call
/// arguments.
pub(crate) const REDACTED_VALUE: &str = "[redacted]";

/// Internal marker written only when a model mistakenly put a raw secret value
/// inside `arguments` instead of omitting it or using top-level `secrets`.
const MISPLACED_SECRET_MARKER: &str = "[redacted:misplaced_secret]";

/// Normalized exact key names that must never hold raw secret strings inside
/// `mannie_configure`'s non-secret `arguments` object.
const MISPLACED_SECRET_KEYS: [&str; 13] = [
    "password",
    "passphrase",
    "apikey",
    "clientsecret",
    "secret",
    "secretvalue",
    "accesstoken",
    "refreshtoken",
    "bearertoken",
    "token",
    "credential",
    "privatekey",
    "pairingcode",
];

/// Result of scrubbing raw `mannie_configure` tool-call arguments.
#[derive(Debug)]
pub(crate) struct ScrubbedConfigure {
    /// Sanitized JSON text safe to persist in SQLite `parts.text` and emit on
    /// transcript streams.
    pub(crate) stored_arguments: String,
    /// Extracted throwaway secrets from top-level `"secrets"`, held only in
    /// zeroizing memory for the lifetime of this single tool call.
    pub(crate) secrets: BTreeMap<String, Secret>,
}

/// Extracts top-level `"secrets"` values into zeroizing [`Secret`]s and scrubs
/// any raw secret values from the JSON text before persistence.
pub(crate) fn scrub_configure_arguments(raw_arguments: &str) -> ScrubbedConfigure {
    let Ok(mut parsed) = serde_json::from_str::<Value>(raw_arguments) else {
        return ScrubbedConfigure {
            stored_arguments: raw_arguments.to_owned(),
            secrets: BTreeMap::new(),
        };
    };

    let Some(root) = parsed.as_object_mut() else {
        return ScrubbedConfigure {
            stored_arguments: raw_arguments.to_owned(),
            secrets: BTreeMap::new(),
        };
    };

    let mut secrets = BTreeMap::new();
    let mut changed = false;

    if let Some(Value::Object(secret_map)) = root.get_mut("secrets") {
        extract_top_level_secrets(secret_map, &mut secrets, &mut changed);
    }

    if let Some(arguments) = root.get_mut("arguments") {
        scrub_misplaced_secrets(arguments, &mut changed);
    }

    let stored_arguments = if changed {
        serde_json::to_string(&parsed).unwrap_or_else(|_| raw_arguments.to_owned())
    } else {
        raw_arguments.to_owned()
    };

    ScrubbedConfigure {
        stored_arguments,
        secrets,
    }
}

/// Checks whether a parsed `arguments` JSON value contains any field that was
/// scrubbed by defense-in-depth because the model mistakenly placed a secret
/// inside `arguments` instead of omitting it or using top-level `secrets`.
pub(crate) fn has_misplaced_secret_marker(value: &Value) -> Option<String> {
    find_misplaced_marker(value, "")
}

fn extract_top_level_secrets(
    secret_map: &mut Map<String, Value>,
    secrets: &mut BTreeMap<String, Secret>,
    changed: &mut bool,
) {
    for (key, value) in secret_map.iter_mut() {
        match value {
            Value::String(raw) => {
                if !raw.is_empty() && raw != REDACTED_VALUE {
                    let owned = std::mem::take(raw);
                    secrets.insert(key.clone(), Secret::new(owned));
                }
                *value = Value::String(REDACTED_VALUE.to_owned());
                *changed = true;
            }
            Value::Null => {}
            _ => {
                *value = Value::String(REDACTED_VALUE.to_owned());
                *changed = true;
            }
        }
    }
}

fn scrub_misplaced_secrets(value: &mut Value, changed: &mut bool) {
    match value {
        Value::Object(map) => {
            for (key, entry) in map.iter_mut() {
                if is_misplaced_secret_entry(key, entry) {
                    *entry = Value::String(MISPLACED_SECRET_MARKER.to_owned());
                    *changed = true;
                } else {
                    scrub_misplaced_secrets(entry, changed);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                scrub_misplaced_secrets(item, changed);
            }
        }
        _ => {}
    }
}

fn is_misplaced_secret_entry(key: &str, value: &Value) -> bool {
    let Value::String(text) = value else {
        return false;
    };
    if text.is_empty() || text == REDACTED_VALUE || text == MISPLACED_SECRET_MARKER {
        return false;
    }

    let normalized: String = key
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect();
    MISPLACED_SECRET_KEYS.contains(&normalized.as_str())
}

fn find_misplaced_marker(value: &Value, prefix: &str) -> Option<String> {
    match value {
        Value::Object(map) => {
            for (key, entry) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                if entry.as_str() == Some(MISPLACED_SECRET_MARKER) {
                    return Some(path);
                }
                if let Some(found) = find_misplaced_marker(entry, &path) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let path = format!("{prefix}[{index}]");
                if let Some(found) = find_misplaced_marker(item, &path) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{REDACTED_VALUE, has_misplaced_secret_marker, scrub_configure_arguments};

    #[test]
    fn extracts_top_level_secrets_and_redacts_stored_json() {
        let planted = "throwaway-ssh-password-774411";
        let raw = json!({
            "activity": "Add lab SSH host",
            "target": "ssh",
            "action": "add_host",
            "arguments": {
                "name": "lab",
                "address": "192.168.1.50",
                "port": 22,
                "user": "ubuntu"
            },
            "secrets": {
                "password": planted
            }
        })
        .to_string();

        let scrubbed = scrub_configure_arguments(&raw);

        assert!(
            !scrubbed.stored_arguments.contains(planted),
            "{}",
            scrubbed.stored_arguments
        );
        let parsed: Value = serde_json::from_str(&scrubbed.stored_arguments).unwrap();
        assert_eq!(parsed["activity"], "Add lab SSH host");
        assert_eq!(parsed["secrets"]["password"], REDACTED_VALUE);
        assert_eq!(scrubbed.secrets["password"].expose(), planted);
        assert_eq!(has_misplaced_secret_marker(&parsed["arguments"]), None);
    }

    #[test]
    fn scrubs_misplaced_secrets_inside_arguments_and_flags_them() {
        let planted = "sk-misplaced-api-key-991122";
        let raw = json!({
            "target": "providers",
            "action": "create",
            "arguments": {
                "name": "Test",
                "apiKey": planted,
                "hasCredential": true,
                "secretVariableNames": ["GITHUB_TOKEN"]
            }
        })
        .to_string();

        let scrubbed = scrub_configure_arguments(&raw);

        assert!(
            !scrubbed.stored_arguments.contains(planted),
            "{}",
            scrubbed.stored_arguments
        );
        assert!(scrubbed.secrets.is_empty());
        let parsed: Value = serde_json::from_str(&scrubbed.stored_arguments).unwrap();
        assert_eq!(parsed["arguments"]["hasCredential"], true);
        assert_eq!(
            parsed["arguments"]["secretVariableNames"],
            json!(["GITHUB_TOKEN"])
        );
        assert_eq!(
            has_misplaced_secret_marker(&parsed["arguments"]).as_deref(),
            Some("apiKey")
        );
    }

    #[test]
    fn non_json_passes_through_unchanged() {
        let raw = "{broken json";
        let scrubbed = scrub_configure_arguments(raw);
        assert_eq!(scrubbed.stored_arguments, raw);
        assert!(scrubbed.secrets.is_empty());
    }
}
