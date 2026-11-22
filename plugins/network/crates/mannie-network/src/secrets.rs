//! Chunked OS credential-store storage for Network Client secrets and cookies
//! (ADR-0075, decision 5).
//!
//! Windows Credential Manager limits a single credential value to roughly
//! 1,280 UTF-16 code units (R36). Every secret payload is split into numbered
//! chunks of at most 1,000 UTF-16 code units under `<family>/<owner-id>-<part>`
//! and counted in the corresponding SQLite row's `secret_parts` column.

use std::collections::BTreeMap;
use std::sync::Arc;

use mannie_plugin_sdk::contracts::Secret;
use mannie_plugin_sdk::services::secrets::{SecretsApi, SecretsError};

use crate::model::{AuthConfig, BodyConfig, KeyValueEntry, VariableEntry};
use crate::redact::{is_secret_header_name, is_secret_name};

/// Maximum UTF-16 code units per credential-store entry.
const PART_UNITS: usize = 1_000;

/// Maximum chunks per entity, matching SQLite CHECK constraints.
pub(crate) const MAX_PARTS: u32 = 1_000;

/// Keyed secret families declared in `plugins/network/plugin.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    /// Collection-level secret variables, headers, query params and auth.
    Collection,
    /// Environment-level secret variables.
    Environment,
    /// Cookie-jar values.
    Cookie,
    /// Saved-request secret headers, query/path params, form fields and auth.
    Request,
    /// Global secret variables.
    Global,
}

impl Family {
    const fn name(self) -> &'static str {
        match self {
            Self::Collection => "collection",
            Self::Environment => "environment",
            Self::Cookie => "cookie",
            Self::Request => "request",
            Self::Global => "global",
        }
    }
}

fn entry_key(family: Family, owner_id: &str, part: u32) -> String {
    format!("{}/{owner_id}-{part}", family.name())
}

fn split_utf16(text: &str) -> Vec<Secret> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut units = 0;

    for ch in text.chars() {
        let width = ch.len_utf16();

        if units + width > PART_UNITS {
            parts.push(Secret::new(std::mem::take(&mut current)));
            units = 0;
        }

        current.push(ch);
        units += width;
    }
    if !current.is_empty() {
        parts.push(Secret::new(current));
    }

    parts
}

/// Manages chunked secret maps inside the plugin's bound OS credential store.
#[derive(Debug, Clone)]
pub(crate) struct Vault {
    secrets: Arc<dyn SecretsApi>,
}

impl Vault {
    /// Wraps the host-provided `secrets` service API.
    pub(crate) fn new(secrets: Arc<dyn SecretsApi>) -> Self {
        Self { secrets }
    }

    /// Saves a named map of secrets for `(family, owner_id)` and returns the
    /// number of chunks written. An empty map writes 0 chunks.
    pub(crate) async fn save_map(
        &self,
        family: Family,
        owner_id: &str,
        values: &BTreeMap<String, String>,
    ) -> Result<u32, SecretsError> {
        if values.is_empty() {
            return Ok(0);
        }

        let encoded = Secret::new(serde_json::to_string(values).unwrap_or_default());
        self.save_text(family, owner_id, &encoded).await
    }

    /// Loads a named map of secrets of `parts` chunks. Missing chunks yield an
    /// empty map rather than failing startup.
    pub(crate) async fn load_map(
        &self,
        family: Family,
        owner_id: &str,
        parts: u32,
    ) -> Result<BTreeMap<String, String>, SecretsError> {
        if parts == 0 {
            return Ok(BTreeMap::new());
        }

        let Some(secret) = self.load_text(family, owner_id, parts).await? else {
            return Ok(BTreeMap::new());
        };
        let parsed =
            serde_json::from_str::<BTreeMap<String, String>>(secret.expose()).unwrap_or_default();
        Ok(parsed)
    }

    /// Writes a secret string in UTF-16 bounded chunks and returns the chunk
    /// count.
    pub(crate) async fn save_text(
        &self,
        family: Family,
        owner_id: &str,
        text: &Secret,
    ) -> Result<u32, SecretsError> {
        let parts = split_utf16(text.expose());
        let count = u32::try_from(parts.len()).unwrap_or(u32::MAX);

        if count > MAX_PARTS {
            return Err(SecretsError::Unavailable);
        }

        for (index, part) in (0..count).zip(parts) {
            self.secrets
                .save(&entry_key(family, owner_id, index), part)
                .await?;
        }
        Ok(count)
    }

    /// Reads `parts` chunks back into a single `Secret`, or `None` if any chunk
    /// is missing.
    pub(crate) async fn load_text(
        &self,
        family: Family,
        owner_id: &str,
        parts: u32,
    ) -> Result<Option<Secret>, SecretsError> {
        let mut combined = String::new();

        for index in 0..parts.min(MAX_PARTS) {
            let Some(part) = self
                .secrets
                .load(&entry_key(family, owner_id, index))
                .await?
            else {
                return Ok(None);
            };
            combined.push_str(part.expose());
        }
        Ok(Some(Secret::new(combined)))
    }

    /// Deletes chunks from `first` up to `previous_count`.
    pub(crate) async fn delete_from(
        &self,
        family: Family,
        owner_id: &str,
        first: u32,
        previous_count: u32,
    ) -> Result<(), SecretsError> {
        for index in first..previous_count.min(MAX_PARTS) {
            self.secrets
                .delete(&entry_key(family, owner_id, index))
                .await?;
        }
        Ok(())
    }
}

/// Strips secret values from a variable list for SQLite storage, recording the
/// extracted secrets in `secrets` under `<prefix>:<index>:<key>`.
pub(crate) fn strip_variables(
    prefix: &str,
    variables: &[VariableEntry],
    secrets: &mut BTreeMap<String, String>,
) -> Vec<VariableEntry> {
    variables
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let is_secret = entry.secret || is_secret_name(&entry.key);

            if is_secret {
                if !entry.value.is_empty() {
                    secrets.insert(
                        format!("{prefix}:{index}:{}", entry.key),
                        entry.value.clone(),
                    );
                }

                VariableEntry {
                    key: entry.key.clone(),
                    value: String::new(),
                    enabled: entry.enabled,
                    secret: true,
                    description: entry.description.clone(),
                }
            } else {
                entry.clone()
            }
        })
        .collect()
}

/// Hydrates secret variable values from `secrets` for the owner workbench or
/// in-memory request execution.
pub(crate) fn hydrate_variables(
    prefix: &str,
    variables: &[VariableEntry],
    secrets: &BTreeMap<String, String>,
) -> Vec<VariableEntry> {
    variables
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let is_secret = entry.secret || is_secret_name(&entry.key);

            if is_secret {
                let value = secrets
                    .get(&format!("{prefix}:{index}:{}", entry.key))
                    .cloned()
                    .unwrap_or_default();
                VariableEntry {
                    key: entry.key.clone(),
                    value,
                    enabled: entry.enabled,
                    secret: true,
                    description: entry.description.clone(),
                }
            } else {
                entry.clone()
            }
        })
        .collect()
}

/// Strips secret values from key-value entries (headers, query/path params or
/// form fields) for SQLite storage.
pub(crate) fn strip_key_values(
    prefix: &str,
    entries: &[KeyValueEntry],
    header_rules: bool,
    secrets: &mut BTreeMap<String, String>,
) -> Vec<KeyValueEntry> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let is_secret = entry.secret
                || if header_rules {
                    is_secret_header_name(&entry.key)
                } else {
                    is_secret_name(&entry.key)
                };

            if is_secret {
                if !entry.value.is_empty() {
                    secrets.insert(
                        format!("{prefix}:{index}:{}", entry.key),
                        entry.value.clone(),
                    );
                }

                KeyValueEntry {
                    key: entry.key.clone(),
                    value: String::new(),
                    enabled: entry.enabled,
                    secret: true,
                    description: entry.description.clone(),
                }
            } else {
                entry.clone()
            }
        })
        .collect()
}

/// Hydrates secret key-value entries from `secrets`.
pub(crate) fn hydrate_key_values(
    prefix: &str,
    entries: &[KeyValueEntry],
    header_rules: bool,
    secrets: &BTreeMap<String, String>,
) -> Vec<KeyValueEntry> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let is_secret = entry.secret
                || if header_rules {
                    is_secret_header_name(&entry.key)
                } else {
                    is_secret_name(&entry.key)
                };

            if is_secret {
                let value = secrets
                    .get(&format!("{prefix}:{index}:{}", entry.key))
                    .cloned()
                    .unwrap_or_default();
                KeyValueEntry {
                    key: entry.key.clone(),
                    value,
                    enabled: entry.enabled,
                    secret: true,
                    description: entry.description.clone(),
                }
            } else {
                entry.clone()
            }
        })
        .collect()
}

/// Strips credential fields from `auth` for SQLite storage.
pub(crate) fn strip_auth(auth: &AuthConfig, secrets: &mut BTreeMap<String, String>) -> AuthConfig {
    match auth {
        AuthConfig::Inherit => AuthConfig::Inherit,
        AuthConfig::None => AuthConfig::None,
        AuthConfig::Bearer { token } => {
            if !token.is_empty() {
                secrets.insert("auth:bearer".to_owned(), token.clone());
            }

            AuthConfig::Bearer {
                token: String::new(),
            }
        }
        AuthConfig::Basic { username, password } => {
            if !password.is_empty() {
                secrets.insert("auth:basic_password".to_owned(), password.clone());
            }

            AuthConfig::Basic {
                username: username.clone(),
                password: String::new(),
            }
        }
        AuthConfig::ApiKey {
            name,
            value,
            location,
        } => {
            if !value.is_empty() {
                secrets.insert("auth:api_key".to_owned(), value.clone());
            }

            AuthConfig::ApiKey {
                name: name.clone(),
                value: String::new(),
                location: *location,
            }
        }
    }
}

/// Hydrates credential fields of `auth` from `secrets`.
pub(crate) fn hydrate_auth(auth: &AuthConfig, secrets: &BTreeMap<String, String>) -> AuthConfig {
    match auth {
        AuthConfig::Inherit => AuthConfig::Inherit,
        AuthConfig::None => AuthConfig::None,
        AuthConfig::Bearer { .. } => AuthConfig::Bearer {
            token: secrets.get("auth:bearer").cloned().unwrap_or_default(),
        },
        AuthConfig::Basic { username, .. } => AuthConfig::Basic {
            username: username.clone(),
            password: secrets
                .get("auth:basic_password")
                .cloned()
                .unwrap_or_default(),
        },
        AuthConfig::ApiKey { name, location, .. } => AuthConfig::ApiKey {
            name: name.clone(),
            value: secrets.get("auth:api_key").cloned().unwrap_or_default(),
            location: *location,
        },
    }
}

/// Strips secret form entries from `body` for SQLite storage.
pub(crate) fn strip_body(body: &BodyConfig, secrets: &mut BTreeMap<String, String>) -> BodyConfig {
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
            entries: strip_key_values("form", entries, false, secrets),
        },
    }
}

/// Hydrates secret form entries of `body` from `secrets`.
pub(crate) fn hydrate_body(body: &BodyConfig, secrets: &BTreeMap<String, String>) -> BodyConfig {
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
            entries: hydrate_key_values("form", entries, false, secrets),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::model::{AuthConfig, VariableEntry};

    use super::{
        PART_UNITS, hydrate_auth, hydrate_variables, split_utf16, strip_auth, strip_variables,
    };

    #[test]
    fn long_secrets_split_within_windows_utf16_limit() {
        let text = format!("{}é{}", "x".repeat(PART_UNITS - 1), "😀".repeat(600));
        let chunks = split_utf16(&text);
        let reassembled: String = chunks
            .iter()
            .map(mannie_plugin_sdk::contracts::Secret::expose)
            .collect();

        assert_eq!(reassembled, text);
        assert!(chunks.len() >= 3);
        for chunk in &chunks {
            assert!(chunk.expose().encode_utf16().count() <= PART_UNITS);
        }
    }

    #[test]
    fn strip_and_hydrate_keep_secrets_out_of_sqlite_structs() {
        let vars = vec![
            VariableEntry {
                key: "base_url".to_owned(),
                value: "https://api.example.com".to_owned(),
                enabled: true,
                secret: false,
                description: String::new(),
            },
            VariableEntry {
                key: "api_token".to_owned(),
                value: "secret-value-12345".to_owned(),
                enabled: true,
                secret: false,
                description: String::new(),
            },
        ];
        let auth = AuthConfig::Bearer {
            token: "bearer-secret-9999".to_owned(),
        };
        let mut secrets = BTreeMap::new();

        let stripped_vars = strip_variables("var", &vars, &mut secrets);
        let stripped_auth = strip_auth(&auth, &mut secrets);

        assert_eq!(stripped_vars[0].value, "https://api.example.com");
        assert!(stripped_vars[1].secret);
        assert_eq!(stripped_vars[1].value, "");
        assert_eq!(
            stripped_auth,
            AuthConfig::Bearer {
                token: String::new()
            }
        );

        let hydrated_vars = hydrate_variables("var", &stripped_vars, &secrets);
        let hydrated_auth = hydrate_auth(&stripped_auth, &secrets);
        assert_eq!(hydrated_vars[1].value, "secret-value-12345");
        assert_eq!(hydrated_auth, auth);
    }
}
