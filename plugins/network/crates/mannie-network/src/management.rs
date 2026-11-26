//! Self-management provider for the Network Client plugin (ADR-0075, ADR-0081).
//!
//! Exposes redacted snapshots of API collections, folders, requests,
//! environments, global variables, and cookies, plus typed configuration
//! actions with dynamic secret inputs for credentials and secret variables.
//!
//! Unlike the owner workbench IPC snapshot, every view returned from `inspect`
//! or `ActionResult` redacts all secret headers, secret parameters, secret
//! variables, authentication credentials, and cookie values.

use std::collections::BTreeMap;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{ConfigFieldPreview, ConfigSecretField, Secret};
use mannie_plugin_sdk::ipc::PluginError as _;
use mannie_plugin_sdk::points::management::{
    ActionResult, ExecuteActionRequest, ManagementActionSpec, ManagementError, ManagementGuide,
    ManagementProvider, ManagementSnapshot, PrepareActionRequest, PreparedAction,
    is_valid_secret_key,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::commands::{
    CollectionInput, CollectionRef, CookieRef, CreateRequest, EnvironmentInput, EnvironmentRef,
    FolderCreateInput, FolderRef, RenameFolder, RequestInput, RequestRef, SetActiveEnvironment,
    SetCollectionEnabled, UpdateCollection, UpdateRequest,
};
use crate::model::{
    AuthConfig, BodyConfig, CollectionView, CookieView, EnvironmentView, KeyValueEntry,
    NetworkError, NetworkStateSnapshot, RequestView, VariableEntry,
};
use crate::redact::{
    REDACTED, is_secret_header_name, is_secret_name, redact_auth, redact_body, redact_key_values,
    redact_variables,
};
use crate::state::NetworkManager;

const ACTION_CREATE_COLLECTION: &str = "create_collection";
const ACTION_UPDATE_COLLECTION: &str = "update_collection";
const ACTION_SET_COLLECTION_ENABLED: &str = "set_collection_enabled";
const ACTION_DELETE_COLLECTION: &str = "delete_collection";
const ACTION_CREATE_FOLDER: &str = "create_folder";
const ACTION_RENAME_FOLDER: &str = "rename_folder";
const ACTION_DELETE_FOLDER: &str = "delete_folder";
const ACTION_CREATE_REQUEST: &str = "create_request";
const ACTION_UPDATE_REQUEST: &str = "update_request";
const ACTION_DELETE_REQUEST: &str = "delete_request";
const ACTION_CREATE_ENVIRONMENT: &str = "create_environment";
const ACTION_UPDATE_ENVIRONMENT: &str = "update_environment";
const ACTION_DELETE_ENVIRONMENT: &str = "delete_environment";
const ACTION_SET_ACTIVE_ENVIRONMENT: &str = "set_active_environment";
const ACTION_SET_GLOBAL_VARIABLE: &str = "set_global_variable";
const ACTION_DELETE_GLOBAL_VARIABLE: &str = "delete_global_variable";
const ACTION_DELETE_COOKIE: &str = "delete_cookie";
const ACTION_CLEAR_COOKIES: &str = "clear_cookies";

const SECRET_AUTH_TOKEN: &str = "authToken";
const SECRET_AUTH_PASSWORD: &str = "authPassword";
const SECRET_AUTH_API_KEY: &str = "authApiKey";
const SECRET_GLOBAL_VALUE: &str = "value";

/// Management provider for the Network Client plugin.
#[derive(Debug, Clone)]
pub(crate) struct NetworkManagement {
    manager: NetworkManager,
}

impl NetworkManagement {
    /// Creates a management provider backed by the running network manager.
    pub(crate) fn new(manager: NetworkManager) -> Self {
        Self { manager }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetGlobalVariableArgs {
    key: String,
    #[serde(default)]
    value: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    secret: bool,
    #[serde(default)]
    description: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeleteGlobalVariableArgs {
    key: String,
}

const fn default_true() -> bool {
    true
}

fn map_network_error(error: NetworkError) -> ManagementError {
    match error {
        NetworkError::NameTaken | NetworkError::EnvironmentNameTaken => {
            ManagementError::conflict(error.to_string())
        }
        NetworkError::CollectionNotFound
        | NetworkError::FolderNotFound
        | NetworkError::RequestNotFound
        | NetworkError::EnvironmentNotFound
        | NetworkError::CookieNotFound
        | NetworkError::PreviewNotFound => ManagementError::not_found(error.to_string()),
        NetworkError::InvalidUrl => ManagementError::invalid_arguments(error.to_string()),
        other => ManagementError::new(other.code(), other.to_string()),
    }
}

fn validate_input<T: mannie_plugin_sdk::ipc::Validate>(input: &T) -> Result<(), ManagementError> {
    if let Err(refusal) = input.validate() {
        let message = format!("invalid value for {}", refusal.field);
        return Err(ManagementError::invalid_field(refusal.field, message));
    }
    Ok(())
}

fn is_template_reference(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with("{{") && trimmed.ends_with("}}") && trimmed.len() > 4
}

fn redact_cookie(cookie: &CookieView) -> CookieView {
    CookieView {
        id: cookie.id.clone(),
        name: cookie.name.clone(),
        value: REDACTED.to_owned(),
        domain: cookie.domain.clone(),
        host_only: cookie.host_only,
        path: cookie.path.clone(),
        secure: cookie.secure,
        http_only: cookie.http_only,
        expires_at: cookie.expires_at,
        enabled: cookie.enabled,
        created_at: cookie.created_at,
        updated_at: cookie.updated_at,
    }
}

fn redact_environment(env: &EnvironmentView) -> EnvironmentView {
    EnvironmentView {
        id: env.id.clone(),
        collection_id: env.collection_id.clone(),
        name: env.name.clone(),
        variables: redact_variables(&env.variables),
        created_at: env.created_at,
        updated_at: env.updated_at,
    }
}

fn redact_request(req: &RequestView) -> RequestView {
    RequestView {
        id: req.id.clone(),
        collection_id: req.collection_id.clone(),
        folder_id: req.folder_id.clone(),
        name: req.name.clone(),
        description: req.description.clone(),
        protocol: req.protocol,
        method: req.method,
        url_template: req.url_template.clone(),
        path_params: redact_key_values(&req.path_params, false),
        query_params: redact_key_values(&req.query_params, false),
        headers: redact_key_values(&req.headers, true),
        auth: redact_auth(&req.auth),
        body: redact_body(&req.body),
        timeout_seconds: req.timeout_seconds,
        use_cookies: req.use_cookies,
        sort_order: req.sort_order,
        created_at: req.created_at,
        updated_at: req.updated_at,
    }
}

fn redact_collection(col: &CollectionView) -> CollectionView {
    CollectionView {
        id: col.id.clone(),
        name: col.name.clone(),
        description: col.description.clone(),
        protocol: col.protocol,
        base_url: col.base_url.clone(),
        enabled: col.enabled,
        active_environment_id: col.active_environment_id.clone(),
        auto_cookies: col.auto_cookies,
        headers: redact_key_values(&col.headers, true),
        query_params: redact_key_values(&col.query_params, false),
        auth: redact_auth(&col.auth),
        variables: redact_variables(&col.variables),
        folders: col.folders.clone(),
        environments: col.environments.iter().map(redact_environment).collect(),
        requests: col.requests.iter().map(redact_request).collect(),
        created_at: col.created_at,
        updated_at: col.updated_at,
    }
}

fn auth_kind_label(auth: &AuthConfig) -> &'static str {
    match auth {
        AuthConfig::Inherit => "inherit",
        AuthConfig::None => "none",
        AuthConfig::Bearer { .. } => "bearer",
        AuthConfig::Basic { .. } => "basic",
        AuthConfig::ApiKey { .. } => "api_key",
    }
}

fn entry_is_secret(entry: &KeyValueEntry, header_rules: bool) -> bool {
    entry.secret
        || if header_rules {
            is_secret_header_name(&entry.key)
        } else {
            is_secret_name(&entry.key)
        }
}

fn var_is_secret(entry: &VariableEntry) -> bool {
    entry.secret || is_secret_name(&entry.key)
}

fn secret_entry_key(prefix: &str, name: &str) -> Result<String, ManagementError> {
    let key = format!("{prefix}:{}", name.trim());
    if !is_valid_secret_key(&key) {
        return Err(ManagementError::invalid_field(
            prefix,
            format!("secret key {name:?} contains unsupported whitespace or length"),
        ));
    }
    Ok(key)
}

fn collect_kv_secret_fields(
    entries: &mut [KeyValueEntry],
    existing: &[KeyValueEntry],
    prefix: &str,
    label_prefix: &str,
    header_rules: bool,
    provided_secrets: &[String],
    out: &mut Vec<ConfigSecretField>,
) -> Result<(), ManagementError> {
    for entry in entries.iter_mut() {
        if !entry_is_secret(entry, header_rules) {
            continue;
        }
        entry.secret = true;
        if is_template_reference(&entry.value) {
            continue;
        }
        if !entry.value.is_empty() {
            return Err(ManagementError::invalid_field(
                prefix,
                format!(
                    "secret {:?} must not carry a literal value in arguments; leave value empty and pass it in secrets or enter it on the approval card",
                    entry.key
                ),
            ));
        }

        let key = secret_entry_key(prefix, &entry.key)?;
        let already_stored = existing
            .iter()
            .any(|old| old.key.eq_ignore_ascii_case(&entry.key) && !old.value.is_empty());
        let provided = provided_secrets.iter().any(|item| item == &key);
        let label = if already_stored {
            format!(
                "{label_prefix} {} (leave empty to keep existing)",
                entry.key
            )
        } else {
            format!("{label_prefix} {}", entry.key)
        };
        out.push(ConfigSecretField {
            key,
            label,
            required: !already_stored,
            provided_in_chat: provided,
        });
    }
    Ok(())
}

fn collect_var_secret_fields(
    variables: &mut [VariableEntry],
    existing: &[VariableEntry],
    provided_secrets: &[String],
    out: &mut Vec<ConfigSecretField>,
) -> Result<(), ManagementError> {
    for entry in variables.iter_mut() {
        if !var_is_secret(entry) {
            continue;
        }
        entry.secret = true;
        if is_template_reference(&entry.value) {
            continue;
        }
        if !entry.value.is_empty() {
            return Err(ManagementError::invalid_field(
                "variables",
                format!(
                    "secret variable {:?} must not carry a literal value in arguments; leave value empty and pass it in secrets or enter it on the approval card",
                    entry.key
                ),
            ));
        }

        let key = secret_entry_key("var", &entry.key)?;
        let already_stored = existing
            .iter()
            .any(|old| old.key.eq_ignore_ascii_case(&entry.key) && !old.value.is_empty());
        let provided = provided_secrets.iter().any(|item| item == &key);
        let label = if already_stored {
            format!("Variable {} (leave empty to keep existing)", entry.key)
        } else {
            format!("Variable {}", entry.key)
        };
        out.push(ConfigSecretField {
            key,
            label,
            required: !already_stored,
            provided_in_chat: provided,
        });
    }
    Ok(())
}

fn collect_auth_secret_field(
    auth: &mut AuthConfig,
    existing: Option<&AuthConfig>,
    provided_secrets: &[String],
    out: &mut Vec<ConfigSecretField>,
) -> Result<(), ManagementError> {
    match auth {
        AuthConfig::Inherit | AuthConfig::None => Ok(()),
        AuthConfig::Bearer { token } => {
            if is_template_reference(token) {
                return Ok(());
            }
            if !token.is_empty() {
                return Err(ManagementError::invalid_field(
                    "auth",
                    "leave auth.token empty in arguments and provide `authToken` in secrets or on the approval card",
                ));
            }
            let has_existing =
                matches!(existing, Some(AuthConfig::Bearer { token }) if !token.is_empty());
            let provided = provided_secrets.iter().any(|k| k == SECRET_AUTH_TOKEN);
            out.push(ConfigSecretField {
                key: SECRET_AUTH_TOKEN.to_owned(),
                label: if has_existing {
                    "Bearer token (leave empty to keep existing)".to_owned()
                } else {
                    "Bearer token".to_owned()
                },
                required: !has_existing,
                provided_in_chat: provided,
            });
            Ok(())
        }
        AuthConfig::Basic { password, .. } => {
            if is_template_reference(password) {
                return Ok(());
            }
            if !password.is_empty() {
                return Err(ManagementError::invalid_field(
                    "auth",
                    "leave auth.password empty in arguments and provide `authPassword` in secrets or on the approval card",
                ));
            }
            let has_existing = matches!(existing, Some(AuthConfig::Basic { password, .. }) if !password.is_empty());
            let provided = provided_secrets.iter().any(|k| k == SECRET_AUTH_PASSWORD);
            out.push(ConfigSecretField {
                key: SECRET_AUTH_PASSWORD.to_owned(),
                label: if has_existing {
                    "Basic auth password (leave empty to keep existing)".to_owned()
                } else {
                    "Basic auth password".to_owned()
                },
                required: !has_existing,
                provided_in_chat: provided,
            });
            Ok(())
        }
        AuthConfig::ApiKey { value, .. } => {
            if is_template_reference(value) {
                return Ok(());
            }
            if !value.is_empty() {
                return Err(ManagementError::invalid_field(
                    "auth",
                    "leave auth.value empty in arguments and provide `authApiKey` in secrets or on the approval card",
                ));
            }
            let has_existing =
                matches!(existing, Some(AuthConfig::ApiKey { value, .. }) if !value.is_empty());
            let provided = provided_secrets.iter().any(|k| k == SECRET_AUTH_API_KEY);
            out.push(ConfigSecretField {
                key: SECRET_AUTH_API_KEY.to_owned(),
                label: if has_existing {
                    "API key value (leave empty to keep existing)".to_owned()
                } else {
                    "API key value".to_owned()
                },
                required: !has_existing,
                provided_in_chat: provided,
            });
            Ok(())
        }
    }
}

fn apply_kv_secrets(
    entries: &mut [KeyValueEntry],
    existing: &[KeyValueEntry],
    prefix: &str,
    header_rules: bool,
    secrets: &mut BTreeMap<String, Secret>,
) -> Result<(), ManagementError> {
    for entry in entries.iter_mut() {
        if !entry_is_secret(entry, header_rules) {
            continue;
        }
        entry.secret = true;
        if is_template_reference(&entry.value) {
            continue;
        }
        let key = secret_entry_key(prefix, &entry.key)?;
        if let Some(secret) = secrets.remove(&key) {
            entry.value.clear();
            entry.value.push_str(secret.expose());
        } else if let Some(old) = existing
            .iter()
            .find(|old| old.key.eq_ignore_ascii_case(&entry.key))
        {
            entry.value.clone_from(&old.value);
        }
    }
    Ok(())
}

fn apply_var_secrets(
    variables: &mut [VariableEntry],
    existing: &[VariableEntry],
    secrets: &mut BTreeMap<String, Secret>,
) -> Result<(), ManagementError> {
    for entry in variables.iter_mut() {
        if !var_is_secret(entry) {
            continue;
        }
        entry.secret = true;
        if is_template_reference(&entry.value) {
            continue;
        }
        let key = secret_entry_key("var", &entry.key)?;
        if let Some(secret) = secrets.remove(&key) {
            entry.value.clear();
            entry.value.push_str(secret.expose());
        } else if let Some(old) = existing
            .iter()
            .find(|old| old.key.eq_ignore_ascii_case(&entry.key))
        {
            entry.value.clone_from(&old.value);
        }
    }
    Ok(())
}

fn apply_auth_secret(
    auth: &mut AuthConfig,
    existing: Option<&AuthConfig>,
    secrets: &mut BTreeMap<String, Secret>,
) {
    match auth {
        AuthConfig::Inherit | AuthConfig::None => {}
        AuthConfig::Bearer { token } => {
            if !is_template_reference(token) {
                if let Some(secret) = secrets.remove(SECRET_AUTH_TOKEN) {
                    secret.expose().clone_into(token);
                } else if let Some(AuthConfig::Bearer { token: old }) = existing {
                    token.clone_from(old);
                }
            }
        }
        AuthConfig::Basic { password, .. } => {
            if !is_template_reference(password) {
                if let Some(secret) = secrets.remove(SECRET_AUTH_PASSWORD) {
                    secret.expose().clone_into(password);
                } else if let Some(AuthConfig::Basic { password: old, .. }) = existing {
                    password.clone_from(old);
                }
            }
        }
        AuthConfig::ApiKey { value, .. } => {
            if !is_template_reference(value) {
                if let Some(secret) = secrets.remove(SECRET_AUTH_API_KEY) {
                    secret.expose().clone_into(value);
                } else if let Some(AuthConfig::ApiKey { value: old, .. }) = existing {
                    value.clone_from(old);
                }
            }
        }
    }
}

fn obj_schema(props: &serde_json::Value, required: &[&str]) -> serde_json::Value {
    json!({
        "type": "object",
        "properties": props,
        "required": required,
        "additionalProperties": false
    })
}

fn kv_entry_schema() -> serde_json::Value {
    json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "key": { "type": "string" },
                "value": { "type": "string" },
                "enabled": { "type": "boolean" },
                "secret": { "type": "boolean" },
                "description": { "type": "string" }
            },
            "required": ["key", "value"],
            "additionalProperties": false
        }
    })
}

fn collection_actions() -> Vec<ManagementActionSpec> {
    let kv_schema = kv_entry_schema();
    let collection_input_schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "description": { "type": "string" },
            "protocol": { "type": "string", "enum": ["http"] },
            "baseUrl": { "type": "string" },
            "enabled": { "type": "boolean" },
            "autoCookies": { "type": "boolean" },
            "headers": kv_schema,
            "queryParams": kv_schema,
            "auth": { "type": "object" },
            "variables": kv_schema
        },
        "required": ["name"],
        "additionalProperties": false
    });

    vec![
        ManagementActionSpec {
            name: ACTION_CREATE_COLLECTION.to_owned(),
            description: "Create a new API collection. For secret headers, params, variables, or \
                auth credentials, leave their values empty in arguments and provide them under \
                `secrets` (`header:<key>`, `param:<key>`, `var:<key>`, `authToken`, \
                `authPassword`, `authApiKey`) or let the owner enter them on the approval card."
                .to_owned(),
            security_sensitive: true,
            parameters: collection_input_schema.clone(),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_UPDATE_COLLECTION.to_owned(),
            description: "Update an existing API collection. Existing secret values are kept \
                unless replaced via `secrets` or the approval card."
                .to_owned(),
            security_sensitive: true,
            parameters: obj_schema(
                &json!({
                    "collectionId": { "type": "string" },
                    "input": collection_input_schema
                }),
                &["collectionId", "input"],
            ),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_SET_COLLECTION_ENABLED.to_owned(),
            description: "Enable or disable a collection for agent tools.".to_owned(),
            security_sensitive: true,
            parameters: obj_schema(
                &json!({
                    "collectionId": { "type": "string" },
                    "enabled": { "type": "boolean" }
                }),
                &["collectionId", "enabled"],
            ),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_DELETE_COLLECTION.to_owned(),
            description: "Delete a collection, its folders, environments, requests, and stored \
                secrets."
                .to_owned(),
            security_sensitive: true,
            parameters: obj_schema(
                &json!({ "collectionId": { "type": "string" } }),
                &["collectionId"],
            ),
            secret_fields: Vec::new(),
        },
    ]
}

fn folder_actions() -> Vec<ManagementActionSpec> {
    vec![
        ManagementActionSpec {
            name: ACTION_CREATE_FOLDER.to_owned(),
            description: "Create a folder inside a collection.".to_owned(),
            security_sensitive: false,
            parameters: obj_schema(
                &json!({
                    "collectionId": { "type": "string" },
                    "parentId": { "type": ["string", "null"] },
                    "name": { "type": "string" }
                }),
                &["collectionId", "name"],
            ),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_RENAME_FOLDER.to_owned(),
            description: "Rename a folder.".to_owned(),
            security_sensitive: false,
            parameters: obj_schema(
                &json!({
                    "folderId": { "type": "string" },
                    "name": { "type": "string" }
                }),
                &["folderId", "name"],
            ),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_DELETE_FOLDER.to_owned(),
            description: "Delete a folder inside a collection.".to_owned(),
            security_sensitive: false,
            parameters: obj_schema(&json!({ "folderId": { "type": "string" } }), &["folderId"]),
            secret_fields: Vec::new(),
        },
    ]
}

fn request_actions() -> Vec<ManagementActionSpec> {
    let kv_schema = kv_entry_schema();
    let request_input_schema = json!({
        "type": "object",
        "properties": {
            "folderId": { "type": ["string", "null"] },
            "name": { "type": "string" },
            "description": { "type": "string" },
            "protocol": { "type": "string", "enum": ["http"] },
            "method": { "type": "string", "enum": ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"] },
            "urlTemplate": { "type": "string" },
            "pathParams": kv_schema,
            "queryParams": kv_schema,
            "headers": kv_schema,
            "auth": { "type": "object" },
            "body": { "type": "object" },
            "timeoutSeconds": { "type": "integer", "minimum": 1, "maximum": 300 },
            "useCookies": { "type": "boolean" }
        },
        "required": ["name"],
        "additionalProperties": false
    });

    vec![
        ManagementActionSpec {
            name: ACTION_CREATE_REQUEST.to_owned(),
            description: "Create a saved HTTP request in a collection.".to_owned(),
            security_sensitive: true,
            parameters: obj_schema(
                &json!({
                    "collectionId": { "type": "string" },
                    "input": request_input_schema
                }),
                &["collectionId", "input"],
            ),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_UPDATE_REQUEST.to_owned(),
            description: "Update an existing saved HTTP request.".to_owned(),
            security_sensitive: true,
            parameters: obj_schema(
                &json!({
                    "requestId": { "type": "string" },
                    "input": request_input_schema
                }),
                &["requestId", "input"],
            ),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_DELETE_REQUEST.to_owned(),
            description: "Delete a saved HTTP request and its secrets.".to_owned(),
            security_sensitive: true,
            parameters: obj_schema(
                &json!({ "requestId": { "type": "string" } }),
                &["requestId"],
            ),
            secret_fields: Vec::new(),
        },
    ]
}

fn environment_actions() -> Vec<ManagementActionSpec> {
    let kv_schema = kv_entry_schema();

    vec![
        ManagementActionSpec {
            name: ACTION_CREATE_ENVIRONMENT.to_owned(),
            description: "Create a named environment in a collection. Leave secret variable \
                values empty in arguments and pass them under `var:<key>`."
                .to_owned(),
            security_sensitive: true,
            parameters: obj_schema(
                &json!({
                    "collectionId": { "type": "string" },
                    "name": { "type": "string" },
                    "variables": kv_schema
                }),
                &["collectionId", "name"],
            ),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_UPDATE_ENVIRONMENT.to_owned(),
            description: "Update an existing environment and its variables.".to_owned(),
            security_sensitive: true,
            parameters: obj_schema(
                &json!({
                    "collectionId": { "type": "string" },
                    "environmentId": { "type": "string" },
                    "name": { "type": "string" },
                    "variables": kv_schema
                }),
                &["collectionId", "environmentId", "name"],
            ),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_DELETE_ENVIRONMENT.to_owned(),
            description: "Delete a collection environment and its secrets.".to_owned(),
            security_sensitive: true,
            parameters: obj_schema(
                &json!({ "environmentId": { "type": "string" } }),
                &["environmentId"],
            ),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_SET_ACTIVE_ENVIRONMENT.to_owned(),
            description: "Set or clear the active environment for a collection.".to_owned(),
            security_sensitive: true,
            parameters: obj_schema(
                &json!({
                    "collectionId": { "type": "string" },
                    "environmentId": { "type": ["string", "null"] }
                }),
                &["collectionId"],
            ),
            secret_fields: Vec::new(),
        },
    ]
}

fn global_and_cookie_actions() -> Vec<ManagementActionSpec> {
    vec![
        ManagementActionSpec {
            name: ACTION_SET_GLOBAL_VARIABLE.to_owned(),
            description: "Add or update one global variable. If secret is true (or key looks \
                like a secret), leave `value` empty and provide secret key `value`."
                .to_owned(),
            security_sensitive: true,
            parameters: obj_schema(
                &json!({
                    "key": { "type": "string" },
                    "value": { "type": "string" },
                    "enabled": { "type": "boolean" },
                    "secret": { "type": "boolean" },
                    "description": { "type": "string" }
                }),
                &["key"],
            ),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_DELETE_GLOBAL_VARIABLE.to_owned(),
            description: "Delete a global variable by key.".to_owned(),
            security_sensitive: true,
            parameters: obj_schema(&json!({ "key": { "type": "string" } }), &["key"]),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_DELETE_COOKIE.to_owned(),
            description: "Delete one stored cookie from the cookie jar.".to_owned(),
            security_sensitive: true,
            parameters: obj_schema(&json!({ "cookieId": { "type": "string" } }), &["cookieId"]),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_CLEAR_COOKIES.to_owned(),
            description: "Clear all cookies from the cookie jar.".to_owned(),
            security_sensitive: true,
            parameters: obj_schema(&json!({}), &[]),
            secret_fields: Vec::new(),
        },
    ]
}

fn guide_actions() -> Vec<ManagementActionSpec> {
    let mut actions = collection_actions();
    actions.extend(folder_actions());
    actions.extend(request_actions());
    actions.extend(environment_actions());
    actions.extend(global_and_cookie_actions());
    actions
}

fn find_collection<'a>(
    collections: &'a [CollectionView],
    id_or_name: &str,
) -> Option<&'a CollectionView> {
    collections
        .iter()
        .find(|col| col.id == id_or_name || col.name.eq_ignore_ascii_case(id_or_name))
}

fn find_request<'a>(
    collections: &'a [CollectionView],
    request_id: &str,
) -> Option<(&'a CollectionView, &'a RequestView)> {
    collections.iter().find_map(|col| {
        col.requests
            .iter()
            .find(|req| req.id == request_id)
            .map(|req| (col, req))
    })
}

fn find_environment<'a>(
    collections: &'a [CollectionView],
    env_id: &str,
) -> Option<(&'a CollectionView, &'a EnvironmentView)> {
    collections.iter().find_map(|col| {
        col.environments
            .iter()
            .find(|env| env.id == env_id)
            .map(|env| (col, env))
    })
}

fn find_folder<'a>(
    collections: &'a [CollectionView],
    folder_id: &str,
) -> Option<(&'a CollectionView, &'a crate::model::FolderView)> {
    collections.iter().find_map(|col| {
        col.folders
            .iter()
            .find(|folder| folder.id == folder_id)
            .map(|folder| (col, folder))
    })
}

fn collect_collection_secrets(
    input: &mut CollectionInput,
    existing: Option<&CollectionView>,
    provided: &[String],
) -> Result<Vec<ConfigSecretField>, ManagementError> {
    let old_headers = existing.map_or(&[][..], |col| col.headers.as_slice());
    let old_params = existing.map_or(&[][..], |col| col.query_params.as_slice());
    let old_vars = existing.map_or(&[][..], |col| col.variables.as_slice());
    let old_auth = existing.map(|col| &col.auth);

    let mut secret_fields = Vec::new();
    collect_kv_secret_fields(
        &mut input.headers,
        old_headers,
        "header",
        "Header",
        true,
        provided,
        &mut secret_fields,
    )?;
    collect_kv_secret_fields(
        &mut input.query_params,
        old_params,
        "param",
        "Query param",
        false,
        provided,
        &mut secret_fields,
    )?;
    collect_var_secret_fields(&mut input.variables, old_vars, provided, &mut secret_fields)?;
    collect_auth_secret_field(&mut input.auth, old_auth, provided, &mut secret_fields)?;
    Ok(secret_fields)
}

fn apply_collection_secrets(
    input: &mut CollectionInput,
    existing: Option<&CollectionView>,
    secrets: &mut BTreeMap<String, Secret>,
) -> Result<(), ManagementError> {
    let old_headers = existing.map_or(&[][..], |col| col.headers.as_slice());
    let old_params = existing.map_or(&[][..], |col| col.query_params.as_slice());
    let old_vars = existing.map_or(&[][..], |col| col.variables.as_slice());
    let old_auth = existing.map(|col| &col.auth);

    apply_kv_secrets(&mut input.headers, old_headers, "header", true, secrets)?;
    apply_kv_secrets(&mut input.query_params, old_params, "param", false, secrets)?;
    apply_var_secrets(&mut input.variables, old_vars, secrets)?;
    apply_auth_secret(&mut input.auth, old_auth, secrets);
    Ok(())
}

fn collect_request_secrets(
    input: &mut RequestInput,
    existing: Option<&RequestView>,
    provided: &[String],
) -> Result<Vec<ConfigSecretField>, ManagementError> {
    let old_headers = existing.map_or(&[][..], |req| req.headers.as_slice());
    let old_params = existing.map_or(&[][..], |req| req.query_params.as_slice());
    let old_paths = existing.map_or(&[][..], |req| req.path_params.as_slice());
    let old_form = existing.map_or(&[][..], |req| match &req.body {
        BodyConfig::FormUrlEncoded { entries } => entries.as_slice(),
        _ => &[],
    });
    let old_auth = existing.map(|req| &req.auth);

    let mut secret_fields = Vec::new();
    collect_kv_secret_fields(
        &mut input.headers,
        old_headers,
        "header",
        "Header",
        true,
        provided,
        &mut secret_fields,
    )?;
    collect_kv_secret_fields(
        &mut input.query_params,
        old_params,
        "param",
        "Query param",
        false,
        provided,
        &mut secret_fields,
    )?;
    collect_kv_secret_fields(
        &mut input.path_params,
        old_paths,
        "path",
        "Path param",
        false,
        provided,
        &mut secret_fields,
    )?;
    if let BodyConfig::FormUrlEncoded { entries } = &mut input.body {
        collect_kv_secret_fields(
            entries,
            old_form,
            "form",
            "Form field",
            false,
            provided,
            &mut secret_fields,
        )?;
    }
    collect_auth_secret_field(&mut input.auth, old_auth, provided, &mut secret_fields)?;
    Ok(secret_fields)
}

fn apply_request_secrets(
    input: &mut RequestInput,
    existing: Option<&RequestView>,
    secrets: &mut BTreeMap<String, Secret>,
) -> Result<(), ManagementError> {
    let old_headers = existing.map_or(&[][..], |req| req.headers.as_slice());
    let old_params = existing.map_or(&[][..], |req| req.query_params.as_slice());
    let old_paths = existing.map_or(&[][..], |req| req.path_params.as_slice());
    let old_form = existing.map_or(&[][..], |req| match &req.body {
        BodyConfig::FormUrlEncoded { entries } => entries.as_slice(),
        _ => &[],
    });
    let old_auth = existing.map(|req| &req.auth);

    apply_kv_secrets(&mut input.headers, old_headers, "header", true, secrets)?;
    apply_kv_secrets(&mut input.query_params, old_params, "param", false, secrets)?;
    apply_kv_secrets(&mut input.path_params, old_paths, "path", false, secrets)?;
    if let BodyConfig::FormUrlEncoded { entries } = &mut input.body {
        apply_kv_secrets(entries, old_form, "form", false, secrets)?;
    }
    apply_auth_secret(&mut input.auth, old_auth, secrets);
    Ok(())
}

fn collection_preview_fields(label: &str, input: &CollectionInput) -> Vec<ConfigFieldPreview> {
    vec![
        ConfigFieldPreview {
            label: label.to_owned(),
            value: input.name.trim().to_owned(),
        },
        ConfigFieldPreview {
            label: "Base URL".to_owned(),
            value: if input.base_url.is_empty() {
                "(none)".to_owned()
            } else {
                input.base_url.clone()
            },
        },
        ConfigFieldPreview {
            label: "Auth".to_owned(),
            value: auth_kind_label(&input.auth).to_owned(),
        },
    ]
}

fn request_preview_fields(col_name: &str, input: &RequestInput) -> Vec<ConfigFieldPreview> {
    vec![
        ConfigFieldPreview {
            label: "Collection".to_owned(),
            value: col_name.to_owned(),
        },
        ConfigFieldPreview {
            label: "Request".to_owned(),
            value: input.name.trim().to_owned(),
        },
        ConfigFieldPreview {
            label: "Endpoint".to_owned(),
            value: format!("{} {}", input.method.as_str(), input.url_template),
        },
    ]
}

fn prepare_collection_upsert(
    snapshot: &NetworkStateSnapshot,
    action: &str,
    arguments: serde_json::Value,
    provided: &[String],
) -> Result<PreparedAction, ManagementError> {
    match action {
        ACTION_CREATE_COLLECTION => {
            let mut input: CollectionInput = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            validate_input(&input)?;
            if find_collection(&snapshot.collections, input.name.trim()).is_some() {
                return Err(ManagementError::conflict(
                    "a collection with this name already exists",
                ));
            }
            let secret_fields = collect_collection_secrets(&mut input, None, provided)?;
            let fields = collection_preview_fields("Name", &input);
            let title = format!("Create API collection {}", input.name.trim());
            let canonical_arguments = serde_json::to_value(&input)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title,
                fields,
                secret_fields,
                security_sensitive: true,
                canonical_arguments,
            })
        }
        ACTION_UPDATE_COLLECTION => {
            let mut input: UpdateCollection = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            validate_input(&input)?;
            let existing = snapshot
                .collections
                .iter()
                .find(|col| col.id == input.collection_id)
                .ok_or_else(|| ManagementError::not_found("the collection was not found"))?;
            let secret_fields =
                collect_collection_secrets(&mut input.input, Some(existing), provided)?;
            let fields = collection_preview_fields("Collection", &input.input);
            let canonical_arguments = serde_json::to_value(&input)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title: format!("Update API collection {}", existing.name),
                fields,
                secret_fields,
                security_sensitive: true,
                canonical_arguments,
            })
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

fn prepare_collection_state_action(
    snapshot: &NetworkStateSnapshot,
    action: &str,
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    match action {
        ACTION_SET_COLLECTION_ENABLED => {
            let input: SetCollectionEnabled = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            validate_input(&input)?;
            let existing = snapshot
                .collections
                .iter()
                .find(|col| col.id == input.collection_id)
                .ok_or_else(|| ManagementError::not_found("the collection was not found"))?;
            let verb = if input.enabled { "Enable" } else { "Disable" };
            let canonical_arguments = serde_json::to_value(&input)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title: format!("{verb} API collection {}", existing.name),
                fields: vec![
                    ConfigFieldPreview {
                        label: "Collection".to_owned(),
                        value: existing.name.clone(),
                    },
                    ConfigFieldPreview {
                        label: "Enabled".to_owned(),
                        value: if input.enabled { "Yes" } else { "No" }.to_owned(),
                    },
                ],
                secret_fields: Vec::new(),
                security_sensitive: true,
                canonical_arguments,
            })
        }
        ACTION_DELETE_COLLECTION => {
            let input: CollectionRef = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            validate_input(&input)?;
            let existing = snapshot
                .collections
                .iter()
                .find(|col| col.id == input.collection_id)
                .ok_or_else(|| ManagementError::not_found("the collection was not found"))?;
            let canonical_arguments = serde_json::to_value(&input)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title: format!("Delete API collection {}", existing.name),
                fields: vec![
                    ConfigFieldPreview {
                        label: "Collection".to_owned(),
                        value: existing.name.clone(),
                    },
                    ConfigFieldPreview {
                        label: "Requests".to_owned(),
                        value: existing.requests.len().to_string(),
                    },
                ],
                secret_fields: Vec::new(),
                security_sensitive: true,
                canonical_arguments,
            })
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

fn prepare_folder_action(
    snapshot: &NetworkStateSnapshot,
    action: &str,
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    match action {
        ACTION_CREATE_FOLDER => {
            let input: FolderCreateInput = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            validate_input(&input)?;
            let col = snapshot
                .collections
                .iter()
                .find(|col| col.id == input.collection_id)
                .ok_or_else(|| ManagementError::not_found("the collection was not found"))?;
            let canonical_arguments = serde_json::to_value(&input)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title: format!("Create folder {} in {}", input.name.trim(), col.name),
                fields: vec![
                    ConfigFieldPreview {
                        label: "Collection".to_owned(),
                        value: col.name.clone(),
                    },
                    ConfigFieldPreview {
                        label: "Folder".to_owned(),
                        value: input.name.trim().to_owned(),
                    },
                ],
                secret_fields: Vec::new(),
                security_sensitive: false,
                canonical_arguments,
            })
        }
        ACTION_RENAME_FOLDER => {
            let input: RenameFolder = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            validate_input(&input)?;
            let (col, folder) = find_folder(&snapshot.collections, &input.folder_id)
                .ok_or_else(|| ManagementError::not_found("the folder was not found"))?;
            let canonical_arguments = serde_json::to_value(&input)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title: format!("Rename folder {}", folder.name),
                fields: vec![
                    ConfigFieldPreview {
                        label: "Collection".to_owned(),
                        value: col.name.clone(),
                    },
                    ConfigFieldPreview {
                        label: "Current name".to_owned(),
                        value: folder.name.clone(),
                    },
                    ConfigFieldPreview {
                        label: "New name".to_owned(),
                        value: input.name.trim().to_owned(),
                    },
                ],
                secret_fields: Vec::new(),
                security_sensitive: false,
                canonical_arguments,
            })
        }
        ACTION_DELETE_FOLDER => {
            let input: FolderRef = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            validate_input(&input)?;
            let (col, folder) = find_folder(&snapshot.collections, &input.folder_id)
                .ok_or_else(|| ManagementError::not_found("the folder was not found"))?;
            let canonical_arguments = serde_json::to_value(&input)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title: format!("Delete folder {}", folder.name),
                fields: vec![
                    ConfigFieldPreview {
                        label: "Collection".to_owned(),
                        value: col.name.clone(),
                    },
                    ConfigFieldPreview {
                        label: "Folder".to_owned(),
                        value: folder.name.clone(),
                    },
                ],
                secret_fields: Vec::new(),
                security_sensitive: false,
                canonical_arguments,
            })
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

fn prepare_request_action(
    snapshot: &NetworkStateSnapshot,
    action: &str,
    arguments: serde_json::Value,
    provided: &[String],
) -> Result<PreparedAction, ManagementError> {
    match action {
        ACTION_CREATE_REQUEST => {
            let mut input: CreateRequest = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            validate_input(&input)?;
            let col = snapshot
                .collections
                .iter()
                .find(|col| col.id == input.collection_id)
                .ok_or_else(|| ManagementError::not_found("the collection was not found"))?;
            let secret_fields = collect_request_secrets(&mut input.input, None, provided)?;
            let fields = request_preview_fields(&col.name, &input.input);
            let title = format!("Create request {}", input.input.name.trim());
            let canonical_arguments = serde_json::to_value(&input)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title,
                fields,
                secret_fields,
                security_sensitive: true,
                canonical_arguments,
            })
        }
        ACTION_UPDATE_REQUEST => {
            let mut input: UpdateRequest = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            validate_input(&input)?;
            let (col, existing) = find_request(&snapshot.collections, &input.request_id)
                .ok_or_else(|| ManagementError::not_found("the request was not found"))?;
            let secret_fields =
                collect_request_secrets(&mut input.input, Some(existing), provided)?;
            let fields = request_preview_fields(&col.name, &input.input);
            let canonical_arguments = serde_json::to_value(&input)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title: format!("Update request {}", existing.name),
                fields,
                secret_fields,
                security_sensitive: true,
                canonical_arguments,
            })
        }
        ACTION_DELETE_REQUEST => {
            let input: RequestRef = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            validate_input(&input)?;
            let (col, req) = find_request(&snapshot.collections, &input.request_id)
                .ok_or_else(|| ManagementError::not_found("the request was not found"))?;
            let canonical_arguments = serde_json::to_value(&input)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title: format!("Delete request {}", req.name),
                fields: vec![
                    ConfigFieldPreview {
                        label: "Collection".to_owned(),
                        value: col.name.clone(),
                    },
                    ConfigFieldPreview {
                        label: "Request".to_owned(),
                        value: req.name.clone(),
                    },
                ],
                secret_fields: Vec::new(),
                security_sensitive: true,
                canonical_arguments,
            })
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

fn prepare_save_environment(
    snapshot: &NetworkStateSnapshot,
    action: &str,
    arguments: serde_json::Value,
    provided: &[String],
) -> Result<PreparedAction, ManagementError> {
    let mut input: EnvironmentInput = serde_json::from_value(arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    if action == ACTION_CREATE_ENVIRONMENT {
        input.environment_id = None;
    } else if input.environment_id.is_none() {
        return Err(ManagementError::invalid_field(
            "environmentId",
            "environmentId is required for update_environment",
        ));
    }
    validate_input(&input)?;
    let col = snapshot
        .collections
        .iter()
        .find(|col| col.id == input.collection_id)
        .ok_or_else(|| ManagementError::not_found("the collection was not found"))?;
    let existing_vars: &[VariableEntry] = match &input.environment_id {
        Some(env_id) => {
            let existing = col
                .environments
                .iter()
                .find(|env| env.id == *env_id)
                .ok_or_else(|| ManagementError::not_found("the environment was not found"))?;
            &existing.variables
        }
        None => &[],
    };

    let mut secret_fields = Vec::new();
    collect_var_secret_fields(
        &mut input.variables,
        existing_vars,
        provided,
        &mut secret_fields,
    )?;

    let verb = if action == ACTION_CREATE_ENVIRONMENT {
        "Create"
    } else {
        "Update"
    };
    let canonical_arguments =
        serde_json::to_value(&input).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title: format!("{verb} environment {}", input.name.trim()),
        fields: vec![
            ConfigFieldPreview {
                label: "Collection".to_owned(),
                value: col.name.clone(),
            },
            ConfigFieldPreview {
                label: "Environment".to_owned(),
                value: input.name.trim().to_owned(),
            },
            ConfigFieldPreview {
                label: "Variables".to_owned(),
                value: input.variables.len().to_string(),
            },
        ],
        secret_fields,
        security_sensitive: true,
        canonical_arguments,
    })
}

fn prepare_environment_ref_action(
    snapshot: &NetworkStateSnapshot,
    action: &str,
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    match action {
        ACTION_DELETE_ENVIRONMENT => {
            let input: EnvironmentRef = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            validate_input(&input)?;
            let (col, env) = find_environment(&snapshot.collections, &input.environment_id)
                .ok_or_else(|| ManagementError::not_found("the environment was not found"))?;
            let canonical_arguments = serde_json::to_value(&input)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title: format!("Delete environment {}", env.name),
                fields: vec![
                    ConfigFieldPreview {
                        label: "Collection".to_owned(),
                        value: col.name.clone(),
                    },
                    ConfigFieldPreview {
                        label: "Environment".to_owned(),
                        value: env.name.clone(),
                    },
                ],
                secret_fields: Vec::new(),
                security_sensitive: true,
                canonical_arguments,
            })
        }
        ACTION_SET_ACTIVE_ENVIRONMENT => {
            let input: SetActiveEnvironment = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            validate_input(&input)?;
            let col = snapshot
                .collections
                .iter()
                .find(|col| col.id == input.collection_id)
                .ok_or_else(|| ManagementError::not_found("the collection was not found"))?;
            let env_label = match &input.environment_id {
                Some(env_id) => {
                    let env = col
                        .environments
                        .iter()
                        .find(|e| e.id == *env_id)
                        .ok_or_else(|| {
                            ManagementError::not_found("the environment was not found")
                        })?;
                    env.name.clone()
                }
                None => "None (cleared)".to_owned(),
            };
            let canonical_arguments = serde_json::to_value(&input)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title: format!("Set active environment for {}", col.name),
                fields: vec![
                    ConfigFieldPreview {
                        label: "Collection".to_owned(),
                        value: col.name.clone(),
                    },
                    ConfigFieldPreview {
                        label: "Active environment".to_owned(),
                        value: env_label,
                    },
                ],
                secret_fields: Vec::new(),
                security_sensitive: true,
                canonical_arguments,
            })
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

fn global_variable_secret_field(
    snapshot: &NetworkStateSnapshot,
    key: &str,
    value: &str,
    provided: &[String],
) -> Result<Option<ConfigSecretField>, ManagementError> {
    if is_template_reference(value) {
        return Ok(None);
    }
    if !value.is_empty() {
        return Err(ManagementError::invalid_field(
            "value",
            "leave value empty for a secret global variable and provide it under secrets.value or on the approval card",
        ));
    }

    let has_existing = snapshot
        .global_variables
        .iter()
        .any(|existing| existing.key.eq_ignore_ascii_case(key) && !existing.value.is_empty());
    let provided_in_chat = provided.iter().any(|k| k == SECRET_GLOBAL_VALUE);
    let label = if has_existing {
        format!("Value for {key} (leave empty to keep existing)")
    } else {
        format!("Value for {key}")
    };

    Ok(Some(ConfigSecretField {
        key: SECRET_GLOBAL_VALUE.to_owned(),
        label,
        required: !has_existing,
        provided_in_chat,
    }))
}

fn prepare_set_global_variable(
    snapshot: &NetworkStateSnapshot,
    arguments: serde_json::Value,
    provided: &[String],
) -> Result<PreparedAction, ManagementError> {
    let mut args: SetGlobalVariableArgs = serde_json::from_value(arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    let key_trimmed = args.key.trim().to_owned();
    if key_trimmed.is_empty() {
        return Err(ManagementError::invalid_field(
            "key",
            "variable key cannot be empty",
        ));
    }
    key_trimmed.clone_into(&mut args.key);
    let is_secret = args.secret || is_secret_name(&key_trimmed);
    args.secret = is_secret;

    let mut secret_fields = Vec::new();
    if is_secret
        && let Some(field) =
            global_variable_secret_field(snapshot, &key_trimmed, &args.value, provided)?
    {
        secret_fields.push(field);
    }

    let probe = VariableEntry {
        key: args.key.clone(),
        value: args.value.clone(),
        enabled: args.enabled,
        secret: args.secret,
        description: args.description.clone(),
    };
    if !crate::model::are_valid_variables(&[probe]) {
        return Err(ManagementError::invalid_arguments(
            "global variable key, value, or description is invalid",
        ));
    }

    let display_val = if is_secret {
        REDACTED.to_owned()
    } else {
        args.value.clone()
    };
    let canonical_arguments =
        serde_json::to_value(&args).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title: format!("Set global variable {key_trimmed}"),
        fields: vec![
            ConfigFieldPreview {
                label: "Key".to_owned(),
                value: key_trimmed,
            },
            ConfigFieldPreview {
                label: "Value".to_owned(),
                value: display_val,
            },
            ConfigFieldPreview {
                label: "Secret".to_owned(),
                value: if is_secret { "Yes" } else { "No" }.to_owned(),
            },
        ],
        secret_fields,
        security_sensitive: true,
        canonical_arguments,
    })
}

fn prepare_delete_global_or_cookie(
    snapshot: &NetworkStateSnapshot,
    action: &str,
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    match action {
        ACTION_DELETE_GLOBAL_VARIABLE => {
            let args: DeleteGlobalVariableArgs = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            let exists = snapshot
                .global_variables
                .iter()
                .any(|var| var.key.eq_ignore_ascii_case(args.key.trim()));
            if !exists {
                return Err(ManagementError::not_found(format!(
                    "no global variable named {:?}",
                    args.key
                )));
            }
            let canonical_arguments = serde_json::to_value(&args)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title: format!("Delete global variable {}", args.key.trim()),
                fields: vec![ConfigFieldPreview {
                    label: "Key".to_owned(),
                    value: args.key.trim().to_owned(),
                }],
                secret_fields: Vec::new(),
                security_sensitive: true,
                canonical_arguments,
            })
        }
        ACTION_DELETE_COOKIE => {
            let input: CookieRef = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            validate_input(&input)?;
            let cookie = snapshot
                .cookies
                .iter()
                .find(|c| c.id == input.cookie_id)
                .ok_or_else(|| ManagementError::not_found("the cookie was not found"))?;
            let canonical_arguments = serde_json::to_value(&input)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title: format!("Delete cookie {}", cookie.name),
                fields: vec![
                    ConfigFieldPreview {
                        label: "Cookie".to_owned(),
                        value: cookie.name.clone(),
                    },
                    ConfigFieldPreview {
                        label: "Domain".to_owned(),
                        value: cookie.domain.clone(),
                    },
                ],
                secret_fields: Vec::new(),
                security_sensitive: true,
                canonical_arguments,
            })
        }
        ACTION_CLEAR_COOKIES => {
            let is_empty_obj = arguments.as_object().is_some_and(serde_json::Map::is_empty);
            if !is_empty_obj {
                return Err(ManagementError::invalid_arguments(
                    "clear_cookies takes no arguments",
                ));
            }
            Ok(PreparedAction {
                title: "Clear all Network Client cookies".to_owned(),
                fields: vec![ConfigFieldPreview {
                    label: "Stored cookies".to_owned(),
                    value: snapshot.cookies.len().to_string(),
                }],
                secret_fields: Vec::new(),
                security_sensitive: true,
                canonical_arguments: json!({}),
            })
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

async fn execute_collection_action(
    manager: &NetworkManager,
    mut request: ExecuteActionRequest,
) -> Result<ActionResult, ManagementError> {
    match request.action.as_str() {
        ACTION_CREATE_COLLECTION => {
            let mut input: CollectionInput = serde_json::from_value(request.arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            apply_collection_secrets(&mut input, None, &mut request.secrets)?;
            let created = manager
                .create_collection(input)
                .await
                .map_err(map_network_error)?;
            let redacted = redact_collection(&created);
            let data = serde_json::to_value(&redacted)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!("Created collection {}.", redacted.name)).with_data(data))
        }
        ACTION_UPDATE_COLLECTION => {
            let mut input: UpdateCollection = serde_json::from_value(request.arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            let snapshot = manager.snapshot();
            let existing = snapshot
                .collections
                .iter()
                .find(|col| col.id == input.collection_id)
                .ok_or_else(|| ManagementError::not_found("the collection was not found"))?;
            apply_collection_secrets(&mut input.input, Some(existing), &mut request.secrets)?;
            let updated = manager
                .update_collection(&input.collection_id, input.input)
                .await
                .map_err(map_network_error)?;
            let redacted = redact_collection(&updated);
            let data = serde_json::to_value(&redacted)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!("Updated collection {}.", redacted.name)).with_data(data))
        }
        ACTION_SET_COLLECTION_ENABLED => {
            let input: SetCollectionEnabled = serde_json::from_value(request.arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            manager
                .set_collection_enabled(&input.collection_id, input.enabled)
                .await
                .map_err(map_network_error)?;
            let state = if input.enabled { "enabled" } else { "disabled" };
            Ok(
                ActionResult::new(format!("Collection is now {state}.")).with_data(json!({
                    "collectionId": input.collection_id,
                    "enabled": input.enabled,
                })),
            )
        }
        ACTION_DELETE_COLLECTION => {
            let input: CollectionRef = serde_json::from_value(request.arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            manager
                .delete_collection(&input.collection_id)
                .await
                .map_err(map_network_error)?;
            Ok(ActionResult::new("Deleted collection.")
                .with_data(json!({ "collectionId": input.collection_id })))
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

async fn execute_folder_action(
    manager: &NetworkManager,
    action: &str,
    arguments: serde_json::Value,
) -> Result<ActionResult, ManagementError> {
    match action {
        ACTION_CREATE_FOLDER => {
            let input: FolderCreateInput = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            let folder = manager
                .create_folder(input)
                .await
                .map_err(map_network_error)?;
            let data = serde_json::to_value(&folder)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!("Created folder {}.", folder.name)).with_data(data))
        }
        ACTION_RENAME_FOLDER => {
            let input: RenameFolder = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            manager
                .rename_folder(&input.folder_id, &input.name)
                .await
                .map_err(map_network_error)?;
            Ok(
                ActionResult::new(format!("Renamed folder to {}.", input.name.trim())).with_data(
                    json!({
                        "folderId": input.folder_id,
                        "name": input.name.trim(),
                    }),
                ),
            )
        }
        ACTION_DELETE_FOLDER => {
            let input: FolderRef = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            manager
                .delete_folder(&input.folder_id)
                .await
                .map_err(map_network_error)?;
            Ok(ActionResult::new("Deleted folder.")
                .with_data(json!({ "folderId": input.folder_id })))
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

async fn execute_request_action(
    manager: &NetworkManager,
    mut request: ExecuteActionRequest,
) -> Result<ActionResult, ManagementError> {
    match request.action.as_str() {
        ACTION_CREATE_REQUEST => {
            let mut input: CreateRequest = serde_json::from_value(request.arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            apply_request_secrets(&mut input.input, None, &mut request.secrets)?;
            let created = manager
                .create_request(&input.collection_id, input.input)
                .await
                .map_err(map_network_error)?;
            let redacted = redact_request(&created);
            let data = serde_json::to_value(&redacted)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!("Created request {}.", redacted.name)).with_data(data))
        }
        ACTION_UPDATE_REQUEST => {
            let mut input: UpdateRequest = serde_json::from_value(request.arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            let snapshot = manager.snapshot();
            let (_col, existing) = find_request(&snapshot.collections, &input.request_id)
                .ok_or_else(|| ManagementError::not_found("the request was not found"))?;
            apply_request_secrets(&mut input.input, Some(existing), &mut request.secrets)?;
            let updated = manager
                .update_request(&input.request_id, input.input)
                .await
                .map_err(map_network_error)?;
            let redacted = redact_request(&updated);
            let data = serde_json::to_value(&redacted)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!("Updated request {}.", redacted.name)).with_data(data))
        }
        ACTION_DELETE_REQUEST => {
            let input: RequestRef = serde_json::from_value(request.arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            manager
                .delete_request(&input.request_id)
                .await
                .map_err(map_network_error)?;
            Ok(ActionResult::new("Deleted request.")
                .with_data(json!({ "requestId": input.request_id })))
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

async fn execute_environment_action(
    manager: &NetworkManager,
    mut request: ExecuteActionRequest,
) -> Result<ActionResult, ManagementError> {
    match request.action.as_str() {
        ACTION_CREATE_ENVIRONMENT | ACTION_UPDATE_ENVIRONMENT => {
            let mut input: EnvironmentInput = serde_json::from_value(request.arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            let snapshot = manager.snapshot();
            let existing_vars: &[VariableEntry] = match &input.environment_id {
                Some(env_id) => find_environment(&snapshot.collections, env_id)
                    .map_or(&[], |(_col, env)| env.variables.as_slice()),
                None => &[],
            };
            apply_var_secrets(&mut input.variables, existing_vars, &mut request.secrets)?;
            let saved = manager
                .save_environment(input)
                .await
                .map_err(map_network_error)?;
            let redacted = redact_environment(&saved);
            let data = serde_json::to_value(&redacted)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!("Saved environment {}.", redacted.name)).with_data(data))
        }
        ACTION_DELETE_ENVIRONMENT => {
            let input: EnvironmentRef = serde_json::from_value(request.arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            manager
                .delete_environment(&input.environment_id)
                .await
                .map_err(map_network_error)?;
            Ok(ActionResult::new("Deleted environment.")
                .with_data(json!({ "environmentId": input.environment_id })))
        }
        ACTION_SET_ACTIVE_ENVIRONMENT => {
            let input: SetActiveEnvironment = serde_json::from_value(request.arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            manager
                .set_active_environment(&input.collection_id, input.environment_id.as_deref())
                .await
                .map_err(map_network_error)?;
            Ok(
                ActionResult::new("Updated active environment.").with_data(json!({
                    "collectionId": input.collection_id,
                    "environmentId": input.environment_id,
                })),
            )
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

async fn execute_global_or_cookie_action(
    manager: &NetworkManager,
    mut request: ExecuteActionRequest,
) -> Result<ActionResult, ManagementError> {
    match request.action.as_str() {
        ACTION_SET_GLOBAL_VARIABLE => {
            let args: SetGlobalVariableArgs = serde_json::from_value(request.arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            let mut vars = manager.snapshot().global_variables;
            let is_secret = args.secret || is_secret_name(&args.key);
            let existing_value = vars
                .iter()
                .find(|v| v.key.eq_ignore_ascii_case(&args.key))
                .map(|v| v.value.clone());

            let value = if is_secret && !is_template_reference(&args.value) {
                if let Some(secret) = request.secrets.remove(SECRET_GLOBAL_VALUE) {
                    secret.expose().to_owned()
                } else if let Some(old) = existing_value {
                    old
                } else {
                    return Err(ManagementError::invalid_field(
                        SECRET_GLOBAL_VALUE,
                        "a secret value is required",
                    ));
                }
            } else {
                args.value
            };

            let entry = VariableEntry {
                key: args.key.clone(),
                value,
                enabled: args.enabled,
                secret: is_secret,
                description: args.description,
            };
            if let Some(slot) = vars
                .iter_mut()
                .find(|v| v.key.eq_ignore_ascii_case(&args.key))
            {
                *slot = entry;
            } else {
                vars.push(entry);
            }

            let saved = manager
                .save_global_variables(vars)
                .await
                .map_err(map_network_error)?;
            let redacted = redact_variables(&saved);
            Ok(
                ActionResult::new(format!("Saved global variable {}.", args.key))
                    .with_data(json!({ "globalVariables": redacted })),
            )
        }
        ACTION_DELETE_GLOBAL_VARIABLE => {
            let args: DeleteGlobalVariableArgs = serde_json::from_value(request.arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            let mut vars = manager.snapshot().global_variables;
            let before = vars.len();
            vars.retain(|v| !v.key.eq_ignore_ascii_case(args.key.trim()));
            if vars.len() == before {
                return Err(ManagementError::not_found(format!(
                    "no global variable named {:?}",
                    args.key
                )));
            }
            let saved = manager
                .save_global_variables(vars)
                .await
                .map_err(map_network_error)?;
            let redacted = redact_variables(&saved);
            Ok(
                ActionResult::new(format!("Deleted global variable {}.", args.key.trim()))
                    .with_data(json!({ "globalVariables": redacted })),
            )
        }
        ACTION_DELETE_COOKIE => {
            let input: CookieRef = serde_json::from_value(request.arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            manager
                .delete_cookie(&input.cookie_id)
                .await
                .map_err(map_network_error)?;
            Ok(ActionResult::new("Deleted cookie.")
                .with_data(json!({ "cookieId": input.cookie_id })))
        }
        ACTION_CLEAR_COOKIES => {
            manager.clear_cookies().await.map_err(map_network_error)?;
            Ok(ActionResult::new("Cleared all cookies.").with_data(json!({ "ok": true })))
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

impl ManagementProvider for NetworkManagement {
    fn guide(&self) -> ManagementGuide {
        ManagementGuide {
            summary: "Manages Postman-style API collections, folders, saved HTTP requests, \
                environments, global variables, and a cookie jar, and exposes redacted agent \
                tools (`network_list_collections`, `network_list_requests`, `network_get_request`, \
                `network_send_request`) under per-endpoint approvals."
                .to_owned(),
            storage: "Stores collection, folder, request, environment, variable, and cookie \
                metadata in SQLite (`network_*` tables). Stores secret headers, secret params, \
                secret variables, auth credentials, and cookie values only in the master's OS \
                credential store under `network/*` families."
                .to_owned(),
            troubleshooting: "Use `mannie_inspect` with target `network` to list collections, \
                global variables, and cookies (all secret values redacted), or query \
                `collection:<id-or-name>`, `environment:<id>`, or `request:<id>`. Credentials and \
                cookies are refused over plain HTTP to non-local hosts. If a collection is \
                disabled, agents cannot list or run its requests until enabled."
                .to_owned(),
            actions: guide_actions(),
        }
    }

    fn inspect(
        &self,
        query: Option<String>,
    ) -> BoxFuture<'static, Result<ManagementSnapshot, ManagementError>> {
        let manager = self.manager.clone();

        Box::pin(async move {
            let snapshot = manager.snapshot();
            let trimmed = query.as_deref().map(str::trim).filter(|q| !q.is_empty());

            if let Some(selector) = trimmed {
                if let Some(key) = selector.strip_prefix("collection:") {
                    let col = find_collection(&snapshot.collections, key).ok_or_else(|| {
                        ManagementError::not_found(format!("no collection matching {key:?}"))
                    })?;
                    let redacted = redact_collection(col);
                    let summary = format!(
                        "Collection {} ({} request(s), {} environment(s)).",
                        redacted.name,
                        redacted.requests.len(),
                        redacted.environments.len()
                    );
                    let data = serde_json::to_value(&redacted)
                        .map_err(|error| ManagementError::failed(error.to_string()))?;
                    return Ok(ManagementSnapshot::new(summary, data));
                }
                if let Some(env_id) = selector.strip_prefix("environment:") {
                    let (col, env) =
                        find_environment(&snapshot.collections, env_id).ok_or_else(|| {
                            ManagementError::not_found(format!("no environment with ID {env_id:?}"))
                        })?;
                    let redacted = redact_environment(env);
                    let summary = format!(
                        "Environment {} in collection {} ({} variable(s)).",
                        redacted.name,
                        col.name,
                        redacted.variables.len()
                    );
                    let data = serde_json::to_value(&redacted)
                        .map_err(|error| ManagementError::failed(error.to_string()))?;
                    return Ok(ManagementSnapshot::new(summary, data));
                }
                if let Some(req_id) = selector.strip_prefix("request:") {
                    let (col, req) =
                        find_request(&snapshot.collections, req_id).ok_or_else(|| {
                            ManagementError::not_found(format!("no request with ID {req_id:?}"))
                        })?;
                    let redacted = redact_request(req);
                    let summary = format!(
                        "Request {} {} ({}) in collection {}.",
                        redacted.method.as_str(),
                        redacted.url_template,
                        redacted.name,
                        col.name
                    );
                    let data = serde_json::to_value(&redacted)
                        .map_err(|error| ManagementError::failed(error.to_string()))?;
                    return Ok(ManagementSnapshot::new(summary, data));
                }
                return Err(ManagementError::invalid_field(
                    "query",
                    format!(
                        "unsupported network inspect query {selector:?}; omit query or use \"collection:<id-or-name>\", \"environment:<id>\", or \"request:<id>\""
                    ),
                ));
            }

            let collections_summary: Vec<serde_json::Value> = snapshot
                .collections
                .iter()
                .map(|col| {
                    json!({
                        "id": col.id,
                        "name": col.name,
                        "description": col.description,
                        "baseUrl": col.base_url,
                        "enabled": col.enabled,
                        "activeEnvironmentId": col.active_environment_id,
                        "autoCookies": col.auto_cookies,
                        "authKind": auth_kind_label(&col.auth),
                        "folderCount": col.folders.len(),
                        "environmentCount": col.environments.len(),
                        "requestCount": col.requests.len(),
                    })
                })
                .collect();
            let global_variables = redact_variables(&snapshot.global_variables);
            let cookies: Vec<CookieView> = snapshot.cookies.iter().map(redact_cookie).collect();

            let summary = format!(
                "{} collection(s), {} global variable(s), {} cookie(s).",
                collections_summary.len(),
                global_variables.len(),
                cookies.len()
            );
            let data = json!({
                "collections": collections_summary,
                "globalVariables": global_variables,
                "cookies": cookies,
                "activeWorkbenchRequests": manager.active_run_count(),
            });

            Ok(ManagementSnapshot::new(summary, data))
        })
    }

    fn prepare_action(
        &self,
        request: PrepareActionRequest,
    ) -> BoxFuture<'static, Result<PreparedAction, ManagementError>> {
        let manager = self.manager.clone();

        Box::pin(async move {
            let snapshot = manager.snapshot();
            let provided = &request.provided_secrets;

            match request.action.as_str() {
                ACTION_CREATE_COLLECTION | ACTION_UPDATE_COLLECTION => prepare_collection_upsert(
                    &snapshot,
                    &request.action,
                    request.arguments,
                    provided,
                ),
                ACTION_SET_COLLECTION_ENABLED | ACTION_DELETE_COLLECTION => {
                    prepare_collection_state_action(&snapshot, &request.action, request.arguments)
                }
                ACTION_CREATE_FOLDER | ACTION_RENAME_FOLDER | ACTION_DELETE_FOLDER => {
                    prepare_folder_action(&snapshot, &request.action, request.arguments)
                }
                ACTION_CREATE_REQUEST | ACTION_UPDATE_REQUEST | ACTION_DELETE_REQUEST => {
                    prepare_request_action(&snapshot, &request.action, request.arguments, provided)
                }
                ACTION_CREATE_ENVIRONMENT | ACTION_UPDATE_ENVIRONMENT => prepare_save_environment(
                    &snapshot,
                    &request.action,
                    request.arguments,
                    provided,
                ),
                ACTION_DELETE_ENVIRONMENT | ACTION_SET_ACTIVE_ENVIRONMENT => {
                    prepare_environment_ref_action(&snapshot, &request.action, request.arguments)
                }
                ACTION_SET_GLOBAL_VARIABLE => {
                    prepare_set_global_variable(&snapshot, request.arguments, provided)
                }
                ACTION_DELETE_GLOBAL_VARIABLE | ACTION_DELETE_COOKIE | ACTION_CLEAR_COOKIES => {
                    prepare_delete_global_or_cookie(&snapshot, &request.action, request.arguments)
                }
                other => Err(ManagementError::unknown_action(other)),
            }
        })
    }

    fn execute_action(
        &self,
        request: ExecuteActionRequest,
    ) -> BoxFuture<'static, Result<ActionResult, ManagementError>> {
        let manager = self.manager.clone();

        Box::pin(async move {
            match request.action.as_str() {
                ACTION_CREATE_COLLECTION
                | ACTION_UPDATE_COLLECTION
                | ACTION_SET_COLLECTION_ENABLED
                | ACTION_DELETE_COLLECTION => execute_collection_action(&manager, request).await,
                ACTION_CREATE_FOLDER | ACTION_RENAME_FOLDER | ACTION_DELETE_FOLDER => {
                    execute_folder_action(&manager, &request.action, request.arguments).await
                }
                ACTION_CREATE_REQUEST | ACTION_UPDATE_REQUEST | ACTION_DELETE_REQUEST => {
                    execute_request_action(&manager, request).await
                }
                ACTION_CREATE_ENVIRONMENT
                | ACTION_UPDATE_ENVIRONMENT
                | ACTION_DELETE_ENVIRONMENT
                | ACTION_SET_ACTIVE_ENVIRONMENT => {
                    execute_environment_action(&manager, request).await
                }
                ACTION_SET_GLOBAL_VARIABLE
                | ACTION_DELETE_GLOBAL_VARIABLE
                | ACTION_DELETE_COOKIE
                | ACTION_CLEAR_COOKIES => execute_global_or_cookie_action(&manager, request).await,
                other => Err(ManagementError::unknown_action(other)),
            }
        })
    }
}
