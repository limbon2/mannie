//! Self-management providers (ADR-0081): what a plugin exposes so Mannie's
//! conversation agent can explain the plugin, inspect its secret-free
//! configuration and diagnostic state, and perform typed configuration changes
//! under the master's always-ask `Access::AppConfig` policy.
//!
//! A plugin contributes one [`PluginManagement`] entry under its own plugin ID
//! (for example `"ssh"`), and an optional feature may contribute an entry under
//! `<plugin-id>.<feature-id>` (for example `"workers.databases"`). IDs follow
//! [`IdRule::Owned`].
//!
//! Secret values never pass through [`ManagementProvider::guide`],
//! [`ManagementProvider::inspect`], or [`ManagementProvider::prepare_action`].
//! Only [`ManagementProvider::execute_action`] receives entered secrets in
//! [`ExecuteActionRequest::secrets`], wrapped in [`Secret`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::BoxFuture;
use crate::contracts::{ConfigFieldPreview, ConfigSecretField, Secret};
use crate::contributions::{ExtensionPoint, IdRule};
use crate::points::tools::is_tool_name;

/// The maximum byte length of each prose section in a [`ManagementGuide`].
pub const MAX_GUIDE_SECTION_BYTES: usize = 4 * 1024;

/// The maximum number of typed actions one management provider may declare.
pub const MAX_MANAGEMENT_ACTIONS: usize = 64;

/// The maximum number of preview fields on one [`PreparedAction`].
pub const MAX_ACTION_FIELDS: usize = 64;

/// The maximum number of secret inputs on one [`PreparedAction`].
pub const MAX_SECRET_FIELDS: usize = 32;

/// The maximum byte length of a secret field key.
pub const MAX_SECRET_KEY_BYTES: usize = 128;

/// The maximum character length of a [`PreparedAction::title`].
pub const MAX_TITLE_CHARS: usize = 256;

/// The extension point of plugins' self-management providers (ADR-0081).
#[derive(Debug)]
pub enum Management {}

impl ExtensionPoint for Management {
    const NAME: &'static str = "management";
    const MAJOR: u16 = 1;
    const IDS: IdRule = IdRule::Owned;
    type Entry = PluginManagement;

    fn id(entry: &PluginManagement) -> &str {
        &entry.id
    }
}

/// One management provider that a plugin or optional feature contributes while
/// it is on (ADR-0081).
#[derive(Debug, Clone)]
pub struct PluginManagement {
    /// The provider's ID: the plugin's ID, or `<plugin-id>.<feature-id>`.
    pub id: String,
    /// The provider serving guides, secret-free inspection and authorized
    /// configuration actions.
    pub provider: Arc<dyn ManagementProvider>,
}

impl PluginManagement {
    /// Builds a management contribution under `id`.
    pub fn new(id: impl Into<String>, provider: Arc<dyn ManagementProvider>) -> Self {
        Self {
            id: id.into(),
            provider,
        }
    }
}

/// What a plugin exposes for self-inspection and chat-driven configuration
/// (ADR-0081).
///
/// Every returned future owns its data so the core can cancel a call by
/// dropping it.
pub trait ManagementProvider: Send + Sync + fmt::Debug + 'static {
    /// Returns a static self-description of the plugin or feature, where it
    /// stores data and secrets, common troubleshooting steps, and its typed
    /// configuration actions.
    fn guide(&self) -> ManagementGuide;

    /// Returns a secret-free snapshot of the plugin's live configuration and
    /// diagnostic state. `query` is an optional plugin-specific selector (for
    /// example `"tools:<server-id>"` or `"collection:<id>"`).
    fn inspect(
        &self,
        query: Option<String>,
    ) -> BoxFuture<'static, Result<ManagementSnapshot, ManagementError>>;

    /// Validates a configuration action without side effects and builds the
    /// sanitized approval card preview (`fields`, `secret_fields`, and
    /// `canonical_arguments`). Never receives secret values.
    fn prepare_action(
        &self,
        request: PrepareActionRequest,
    ) -> BoxFuture<'static, Result<PreparedAction, ManagementError>>;

    /// Performs a configuration action after the owner explicitly approved it.
    /// Secrets arrive only in `request.secrets` and must never be echoed in the
    /// returned [`ActionResult`] or logged.
    fn execute_action(
        &self,
        request: ExecuteActionRequest,
    ) -> BoxFuture<'static, Result<ActionResult, ManagementError>>;
}

/// Static self-description of a plugin or optional feature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagementGuide {
    /// Concise explanation of what this plugin or feature does.
    pub summary: String,
    /// Where and how it stores configuration and credentials (SQLite tables,
    /// keyring families, files).
    pub storage: String,
    /// Common failure modes and how to diagnose them.
    pub troubleshooting: String,
    /// Typed configuration actions supported by this provider.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ManagementActionSpec>,
}

/// Schema and metadata for one typed configuration action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagementActionSpec {
    /// Lower-case `snake_case` action name (for example `"add_host"`).
    pub name: String,
    /// Plain description of what the action changes, for the model.
    pub description: String,
    /// Whether this action alters trust boundaries, credentials or safety
    /// controls.
    #[serde(default)]
    pub security_sensitive: bool,
    /// JSON Schema object describing the non-secret `arguments` object.
    pub parameters: serde_json::Value,
    /// Documented secret fields accepted or required by this action. Actions
    /// with dynamic secret keys (such as named env vars or HTTP headers) may
    /// also emit secret fields from [`ManagementProvider::prepare_action`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_fields: Vec<SecretFieldSpec>,
}

/// Documentation for one secret field of a [`ManagementActionSpec`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretFieldSpec {
    /// Key in the action's secret map (for example `"password"` or `"apiKey"`).
    pub key: String,
    /// Human-readable label shown beside the masked input.
    pub label: String,
    /// Brief description of the secret for the model and guide.
    pub description: String,
    /// Whether a value is required before the action can run.
    pub required: bool,
}

/// Secret-free diagnostic and configuration snapshot returned by
/// [`ManagementProvider::inspect`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagementSnapshot {
    /// Short plain-text summary of the inspected state.
    pub summary: String,
    /// Structured secret-free JSON data.
    pub data: serde_json::Value,
}

impl ManagementSnapshot {
    /// Builds a snapshot with a summary and structured data.
    pub fn new(summary: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            summary: summary.into(),
            data,
        }
    }
}

/// Request handed to [`ManagementProvider::prepare_action`]. Carries only
/// non-secret arguments and the keys (never values) of any throwaway secrets
/// supplied in chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareActionRequest {
    /// The action name.
    pub action: String,
    /// The non-secret JSON arguments object.
    pub arguments: serde_json::Value,
    /// Keys of throwaway secrets already supplied in chat, so `prepare_action`
    /// can mark matching [`ConfigSecretField::provided_in_chat`] entries
    /// without seeing their values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provided_secrets: Vec<String>,
}

impl PrepareActionRequest {
    /// Whether a throwaway value for `key` was supplied in chat.
    pub fn has_provided_secret(&self, key: &str) -> bool {
        self.provided_secrets.iter().any(|existing| existing == key)
    }
}

/// Sanitized approval preview produced by [`ManagementProvider::prepare_action`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedAction {
    /// Human-readable title shown on the approval card.
    pub title: String,
    /// Sanitized key-value rows shown on the approval card.
    pub fields: Vec<ConfigFieldPreview>,
    /// Secret inputs shown as masked password fields on the approval card.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_fields: Vec<ConfigSecretField>,
    /// Whether the approval card flags this action as security-sensitive.
    #[serde(default)]
    pub security_sensitive: bool,
    /// Validated, canonical non-secret JSON arguments covered by the approval
    /// digest and passed to [`ManagementProvider::execute_action`].
    pub canonical_arguments: serde_json::Value,
}

/// Request handed to [`ManagementProvider::execute_action`] after the owner
/// explicitly approved the prepared action.
///
/// This type intentionally never implements `Serialize` because it carries
/// [`Secret`] values.
#[derive(Debug)]
pub struct ExecuteActionRequest {
    /// The action name.
    pub action: String,
    /// The canonical non-secret JSON arguments approved by the owner.
    pub arguments: serde_json::Value,
    /// Owner-approved secrets, with values entered on the approval card taking
    /// precedence over throwaway chat values.
    pub secrets: BTreeMap<String, Secret>,
}

impl ExecuteActionRequest {
    /// Looks up a provided secret by key.
    pub fn secret(&self, key: &str) -> Option<&Secret> {
        self.secrets.get(key)
    }
}

/// Sanitized result returned by [`ManagementProvider::execute_action`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResult {
    /// Concise plain-English confirmation of what changed (never contains
    /// secret values).
    pub message: String,
    /// Optional structured secret-free result data (for example a created ID or
    /// scanned host-key fingerprint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl ActionResult {
    /// Builds an action result with only a message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            data: None,
        }
    }

    /// Attaches structured secret-free result data.
    #[must_use]
    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }
}

/// Error returned by a [`ManagementProvider`] operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagementError {
    /// Stable machine-readable code (for example `"unknown_action"`,
    /// `"invalid_arguments"`, `"not_found"`, `"conflict"`, `"failed"`).
    pub code: String,
    /// Plain-English explanation for the model and owner (never contains
    /// secrets).
    pub message: String,
    /// Optional field name when validation failed on a specific input field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

impl ManagementError {
    /// Builds a management error with a code and message.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            field: None,
        }
    }

    /// Builds an `unknown_action` error.
    pub fn unknown_action(action: &str) -> Self {
        Self::new("unknown_action", format!("unknown action {action:?}"))
    }

    /// Builds an `invalid_arguments` error.
    pub fn invalid_arguments(message: impl Into<String>) -> Self {
        Self::new("invalid_arguments", message)
    }

    /// Builds an `invalid_arguments` error naming a specific field.
    pub fn invalid_field(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: "invalid_arguments".to_owned(),
            message: message.into(),
            field: Some(field.into()),
        }
    }

    /// Builds a `not_found` error.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("not_found", message)
    }

    /// Builds a `conflict` error.
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new("conflict", message)
    }

    /// Builds a `failed` error.
    pub fn failed(message: impl Into<String>) -> Self {
        Self::new("failed", message)
    }
}

impl fmt::Display for ManagementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.field {
            Some(field) => write!(formatter, "{} ({field}): {}", self.code, self.message),
            None => write!(formatter, "{}: {}", self.code, self.message),
        }
    }
}

impl std::error::Error for ManagementError {}

/// Whether `key` is a valid secret field key: non-empty, bounded by
/// [`MAX_SECRET_KEY_BYTES`], and free of control or whitespace characters.
pub fn is_valid_secret_key(key: &str) -> bool {
    let fits = !key.is_empty() && key.len() <= MAX_SECRET_KEY_BYTES;

    if !fits {
        return false;
    }

    key.chars()
        .all(|character| !character.is_control() && !character.is_whitespace())
}

/// Checks that a [`ManagementGuide`] obeys the SDK's size, naming and schema
/// rules.
pub fn check_management_guide(guide: &ManagementGuide) -> Result<(), String> {
    for (name, text) in [
        ("summary", guide.summary.as_str()),
        ("storage", guide.storage.as_str()),
        ("troubleshooting", guide.troubleshooting.as_str()),
    ] {
        if text.trim().is_empty() {
            return Err(format!("the management guide {name} is empty"));
        }
        if text.len() > MAX_GUIDE_SECTION_BYTES {
            return Err(format!(
                "the management guide {name} exceeds {MAX_GUIDE_SECTION_BYTES} bytes"
            ));
        }
    }

    if guide.actions.len() > MAX_MANAGEMENT_ACTIONS {
        return Err(format!(
            "the management guide lists more than {MAX_MANAGEMENT_ACTIONS} actions"
        ));
    }

    let mut seen_actions = BTreeSet::new();

    for action in &guide.actions {
        if !is_tool_name(&action.name) {
            return Err(format!(
                "the management action {:?} is not a valid snake_case name",
                action.name
            ));
        }
        if !seen_actions.insert(action.name.as_str()) {
            return Err(format!(
                "the management action {:?} is listed more than once",
                action.name
            ));
        }
        if action.description.trim().is_empty() {
            return Err(format!(
                "the management action {:?} has an empty description",
                action.name
            ));
        }

        let is_object_schema = action
            .parameters
            .as_object()
            .and_then(|object| object.get("type"))
            .and_then(serde_json::Value::as_str)
            == Some("object");

        if !is_object_schema {
            return Err(format!(
                "the management action {:?} parameters schema must be a JSON object with type \"object\"",
                action.name
            ));
        }
        if action.secret_fields.len() > MAX_SECRET_FIELDS {
            return Err(format!(
                "the management action {:?} lists more than {MAX_SECRET_FIELDS} secret fields",
                action.name
            ));
        }

        let mut seen_secrets = BTreeSet::new();

        for secret in &action.secret_fields {
            if !is_valid_secret_key(&secret.key) {
                return Err(format!(
                    "the management action {:?} has an invalid secret key {:?}",
                    action.name, secret.key
                ));
            }
            if !seen_secrets.insert(secret.key.as_str()) {
                return Err(format!(
                    "the management action {:?} lists secret key {:?} more than once",
                    action.name, secret.key
                ));
            }
            if secret.label.trim().is_empty() {
                return Err(format!(
                    "the management action {:?} secret key {:?} has an empty label",
                    action.name, secret.key
                ));
            }
        }
    }

    Ok(())
}

/// Checks that a [`PreparedAction`] obeys the SDK's bounds and key rules before
/// it reaches the policy engine or approval card.
pub fn check_prepared_action(prepared: &PreparedAction) -> Result<(), String> {
    if prepared.title.trim().is_empty() || prepared.title.chars().count() > MAX_TITLE_CHARS {
        return Err(format!(
            "the prepared action title must be between 1 and {MAX_TITLE_CHARS} characters"
        ));
    }
    if prepared.fields.len() > MAX_ACTION_FIELDS {
        return Err(format!(
            "the prepared action lists more than {MAX_ACTION_FIELDS} preview fields"
        ));
    }
    if prepared.secret_fields.len() > MAX_SECRET_FIELDS {
        return Err(format!(
            "the prepared action lists more than {MAX_SECRET_FIELDS} secret fields"
        ));
    }

    let mut seen_secrets = BTreeSet::new();

    for secret in &prepared.secret_fields {
        if !is_valid_secret_key(&secret.key) {
            return Err(format!(
                "the prepared action has an invalid secret key {:?}",
                secret.key
            ));
        }
        if !seen_secrets.insert(secret.key.as_str()) {
            return Err(format!(
                "the prepared action lists secret key {:?} more than once",
                secret.key
            ));
        }
        if secret.label.trim().is_empty() {
            return Err(format!(
                "the prepared action secret key {:?} has an empty label",
                secret.key
            ));
        }
    }

    if !prepared.canonical_arguments.is_object() {
        return Err("the prepared action canonical arguments must be a JSON object".to_owned());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        ManagementActionSpec, ManagementGuide, PreparedAction, SecretFieldSpec,
        check_management_guide, check_prepared_action, is_valid_secret_key,
    };
    use crate::contracts::{ConfigFieldPreview, ConfigSecretField};

    fn valid_guide() -> ManagementGuide {
        ManagementGuide {
            summary: "Manages remote SSH hosts.".to_owned(),
            storage: "Stored in ssh_hosts; passwords in keyring under ssh/host/<id>.".to_owned(),
            troubleshooting: "Check pinned host key fingerprint and port.".to_owned(),
            actions: vec![ManagementActionSpec {
                name: "add_host".to_owned(),
                description: "Register a new SSH host.".to_owned(),
                security_sensitive: true,
                parameters: json!({
                    "type": "object",
                    "properties": { "name": { "type": "string" } },
                    "required": ["name"],
                    "additionalProperties": false
                }),
                secret_fields: vec![SecretFieldSpec {
                    key: "password".to_owned(),
                    label: "Password".to_owned(),
                    description: "SSH login password.".to_owned(),
                    required: true,
                }],
            }],
        }
    }

    #[test]
    fn valid_guide_and_prepared_action_pass_checks() {
        check_management_guide(&valid_guide()).unwrap();

        let prepared = PreparedAction {
            title: "Add SSH host prod".to_owned(),
            fields: vec![ConfigFieldPreview {
                label: "Name".to_owned(),
                value: "prod".to_owned(),
            }],
            secret_fields: vec![ConfigSecretField {
                key: "password".to_owned(),
                label: "Password".to_owned(),
                required: true,
                provided_in_chat: false,
            }],
            security_sensitive: true,
            canonical_arguments: json!({ "name": "prod" }),
        };
        check_prepared_action(&prepared).unwrap();
    }

    #[test]
    fn invalid_guides_and_secret_keys_are_refused() {
        assert!(is_valid_secret_key("var:GITHUB_TOKEN"));
        assert!(is_valid_secret_key("header:X-Api-Key"));
        assert!(!is_valid_secret_key(""));
        assert!(!is_valid_secret_key("has space"));
        assert!(!is_valid_secret_key("line\nbreak"));

        let mut bad_name = valid_guide();
        bad_name.actions[0].name = "AddHost".to_owned();
        assert!(check_management_guide(&bad_name).is_err());

        let mut duplicate_action = valid_guide();
        let clone = duplicate_action.actions[0].clone();
        duplicate_action.actions.push(clone);
        assert!(check_management_guide(&duplicate_action).is_err());

        let mut bad_schema = valid_guide();
        bad_schema.actions[0].parameters = json!({ "type": "string" });
        assert!(check_management_guide(&bad_schema).is_err());

        let bad_prepared = PreparedAction {
            title: "Add SSH host".to_owned(),
            fields: Vec::new(),
            secret_fields: Vec::new(),
            security_sensitive: false,
            canonical_arguments: json!("not an object"),
        };
        assert!(check_prepared_action(&bad_prepared).is_err());
    }
}
