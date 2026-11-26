//! Self-management provider for the MCP plugin (ADR-0081).
//!
//! Exposes registered MCP servers, per-server tool listings, redacted server
//! logs, program resolution diagnostics, and typed actions with dynamic secret
//! fields (`var:<NAME>`, `header:<NAME>`, `clientSecret`) under the master's
//! always-ask `Access::AppConfig` policy.

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{ConfigFieldPreview, ConfigSecretField, Secret};
use mannie_plugin_sdk::ipc::{PluginError as _, Validate as _};
use mannie_plugin_sdk::points::management::{
    ActionResult, ExecuteActionRequest, ManagementActionSpec, ManagementError, ManagementGuide,
    ManagementProvider, ManagementSnapshot, PrepareActionRequest, PreparedAction, SecretFieldSpec,
    is_valid_secret_key,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::commands::{
    MAX_ENABLED_TOOLS, McpError, NamedValue, SecretValue, ServerInput, ServerListUpdate, ServerRef,
    ServerTransport, SetServerEnabled, SetToolEnabled, UpdateServer, ValueRules,
};
use crate::rules::{DEFAULT_TIMEOUT_SECONDS, is_client_secret, is_program};
use crate::servers::Servers;
use crate::store::{ServerRecord, StoredEndpoint};

const ACTION_ADD_SERVER: &str = "add_server";
const ACTION_UPDATE_SERVER: &str = "update_server";
const ACTION_REMOVE_SERVER: &str = "remove_server";
const ACTION_SET_SERVER_ENABLED: &str = "set_server_enabled";
const ACTION_RESTART_SERVER: &str = "restart_server";
const ACTION_SET_TOOL_ENABLED: &str = "set_tool_enabled";
const ACTION_ACKNOWLEDGE_TOOLS: &str = "acknowledge_tools";
const SECRET_CLIENT_SECRET: &str = "clientSecret";

/// Management provider for the MCP plugin.
#[derive(Debug, Clone)]
pub(crate) struct McpManagement {
    servers: Servers,
}

impl McpManagement {
    /// Creates a management provider backed by the running MCP servers registry.
    pub(crate) fn new(servers: Servers) -> Self {
        Self { servers }
    }
}

fn default_true() -> bool {
    true
}

fn default_timeout() -> u32 {
    DEFAULT_TIMEOUT_SECONDS
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "mirrors the non-secret configuration flags of ServerInput"
)]
struct AddServerArgs {
    name: String,
    #[serde(default)]
    prefix: String,
    transport: ServerTransport,
    #[serde(default)]
    program: String,
    #[serde(default)]
    arguments: Vec<String>,
    #[serde(default)]
    folder: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    plain: Vec<NamedValue>,
    #[serde(default)]
    secret_names: Vec<String>,
    #[serde(default)]
    oauth: bool,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    has_client_secret: bool,
    #[serde(default = "default_true")]
    images: bool,
    #[serde(default = "default_true")]
    instructions: bool,
    #[serde(default = "default_timeout")]
    timeout_seconds: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateServerPatch {
    server_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    prefix: Option<String>,
    #[serde(default)]
    program: Option<String>,
    #[serde(default)]
    arguments: Option<Vec<String>>,
    #[serde(default)]
    folder: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    plain: Option<Vec<NamedValue>>,
    #[serde(default)]
    secret_names: Option<Vec<String>>,
    #[serde(default)]
    oauth: Option<bool>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    has_client_secret: Option<bool>,
    #[serde(default)]
    forget_client_secret: bool,
    #[serde(default)]
    images: Option<bool>,
    #[serde(default)]
    instructions: Option<bool>,
    #[serde(default)]
    timeout_seconds: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "mirrors the non-secret configuration flags of UpdateServer"
)]
struct CanonicalUpdateServer {
    server_id: String,
    name: String,
    prefix: String,
    transport: ServerTransport,
    program: String,
    arguments: Vec<String>,
    folder: String,
    url: String,
    plain: Vec<NamedValue>,
    secret_names: Vec<String>,
    oauth: bool,
    client_id: String,
    forget_client_secret: bool,
    images: bool,
    instructions: bool,
    timeout_seconds: u32,
}

fn secret_key(transport: ServerTransport, name: &str) -> String {
    match transport {
        ServerTransport::Stdio => format!("var:{name}"),
        ServerTransport::Http => format!("header:{name}"),
    }
}

fn secret_label(transport: ServerTransport, name: &str, optional: bool) -> String {
    let kind = match transport {
        ServerTransport::Stdio => "Environment variable",
        ServerTransport::Http => "HTTP header",
    };

    if optional {
        format!("{kind} {name} (leave empty to keep existing)")
    } else {
        format!("{kind} {name}")
    }
}

fn map_mcp_error(error: McpError) -> ManagementError {
    match error {
        McpError::NameTaken
        | McpError::PrefixTaken
        | McpError::TransportChanged
        | McpError::TooManyTools { .. } => ManagementError::conflict(error.to_string()),
        McpError::ServerNotFound | McpError::ToolNotFound | McpError::ProgramNotFound => {
            ManagementError::not_found(error.to_string())
        }
        other => ManagementError::new(other.code(), other.to_string()),
    }
}

fn server_id_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "serverId": {
                "type": "string",
                "description": "UUID of the registered MCP server."
            }
        },
        "required": ["serverId"],
        "additionalProperties": false
    })
}

fn server_properties_schema() -> serde_json::Value {
    json!({
        "name": {
            "type": "string",
            "description": "Unique display name of the MCP server."
        },
        "prefix": {
            "type": "string",
            "description": "Optional tool name prefix after mcp_ (derived from name if omitted)."
        },
        "program": {
            "type": "string",
            "description": "Executable name on PATH or absolute path (stdio transport only)."
        },
        "arguments": {
            "type": "array",
            "items": { "type": "string" },
            "description": "Program arguments (stdio transport only)."
        },
        "folder": {
            "type": "string",
            "description": "Optional absolute working directory (stdio transport only)."
        },
        "url": {
            "type": "string",
            "description": "Streamable HTTP endpoint URL (http transport only)."
        },
        "plain": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "value": { "type": "string" }
                },
                "required": ["name", "value"],
                "additionalProperties": false
            },
            "description": "Non-secret environment variables (stdio) or HTTP headers (http)."
        },
        "secretNames": {
            "type": "array",
            "items": { "type": "string" },
            "description": "Names of secret environment variables (stdio) or HTTP headers (http). Supply values in `secrets` under `var:<NAME>` or `header:<NAME>`, or leave for the owner to enter on the approval card."
        },
        "oauth": {
            "type": "boolean",
            "description": "Whether an http server signs in with OAuth."
        },
        "clientId": {
            "type": "string",
            "description": "Optional registered OAuth client ID (http + oauth only)."
        },
        "hasClientSecret": {
            "type": "boolean",
            "description": "Set true when providing an OAuth client secret under secret key `clientSecret`."
        },
        "images": {
            "type": "boolean",
            "description": "Whether tool result images reach the model (defaults to true)."
        },
        "instructions": {
            "type": "boolean",
            "description": "Whether server instructions reach the model (defaults to true)."
        },
        "timeoutSeconds": {
            "type": "integer",
            "minimum": 10,
            "maximum": 3600,
            "description": "Per-call timeout in seconds."
        }
    })
}

fn server_crud_actions() -> Vec<ManagementActionSpec> {
    let mut add_props = server_properties_schema();
    if let Some(map) = add_props.as_object_mut() {
        map.insert(
            "transport".to_owned(),
            json!({
                "type": "string",
                "enum": ["stdio", "http"],
                "description": "Server transport: `stdio` for a local program or `http` for Streamable HTTP."
            }),
        );
    }

    let mut update_props = server_properties_schema();
    if let Some(map) = update_props.as_object_mut() {
        map.insert(
            "serverId".to_owned(),
            json!({
                "type": "string",
                "description": "UUID of the MCP server to update."
            }),
        );
        map.insert(
            "forgetClientSecret".to_owned(),
            json!({
                "type": "boolean",
                "description": "Set true to delete a stored OAuth client secret."
            }),
        );
    }

    vec![
        ManagementActionSpec {
            name: ACTION_ADD_SERVER.to_owned(),
            description: "Register a new stdio or Streamable HTTP MCP server and connect to it. \
                Declare secret env var or header names in `secretNames`; `prepare_action` emits \
                matching secret inputs (`var:<NAME>` for stdio, `header:<NAME>` for http, and \
                `clientSecret` when OAuth uses a client secret)."
                .to_owned(),
            security_sensitive: true,
            parameters: json!({
                "type": "object",
                "properties": add_props,
                "required": ["name", "transport"],
                "additionalProperties": false
            }),
            secret_fields: vec![SecretFieldSpec {
                key: SECRET_CLIENT_SECRET.to_owned(),
                label: "OAuth client secret".to_owned(),
                description: "Optional OAuth client secret when oauth is true and clientId is set."
                    .to_owned(),
                required: false,
            }],
        },
        ManagementActionSpec {
            name: ACTION_UPDATE_SERVER.to_owned(),
            description: "Update an existing MCP server (transport cannot change). Omitted fields \
                keep their current values; existing secrets remain stored unless replaced."
                .to_owned(),
            security_sensitive: true,
            parameters: json!({
                "type": "object",
                "properties": update_props,
                "required": ["serverId"],
                "additionalProperties": false
            }),
            secret_fields: vec![SecretFieldSpec {
                key: SECRET_CLIENT_SECRET.to_owned(),
                label: "OAuth client secret".to_owned(),
                description: "Optional replacement OAuth client secret.".to_owned(),
                required: false,
            }],
        },
        ManagementActionSpec {
            name: ACTION_REMOVE_SERVER.to_owned(),
            description: "Stop and remove a registered MCP server and delete its stored secrets."
                .to_owned(),
            security_sensitive: true,
            parameters: server_id_schema(),
            secret_fields: Vec::new(),
        },
    ]
}

fn server_runtime_actions() -> Vec<ManagementActionSpec> {
    vec![
        ManagementActionSpec {
            name: ACTION_SET_SERVER_ENABLED.to_owned(),
            description: "Turn a registered MCP server on or off.".to_owned(),
            security_sensitive: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "serverId": {
                        "type": "string",
                        "description": "UUID of the MCP server."
                    },
                    "enabled": {
                        "type": "boolean",
                        "description": "Whether the server should be enabled and connected."
                    }
                },
                "required": ["serverId", "enabled"],
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_RESTART_SERVER.to_owned(),
            description: "Restart an enabled MCP server connection (also clears crash backoff)."
                .to_owned(),
            security_sensitive: false,
            parameters: server_id_schema(),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_SET_TOOL_ENABLED.to_owned(),
            description: "Enable or disable a specific tool exposed by an MCP server (subject to \
                the global enabled-tool limit)."
                .to_owned(),
            security_sensitive: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "serverId": {
                        "type": "string",
                        "description": "UUID of the MCP server."
                    },
                    "tool": {
                        "type": "string",
                        "description": "Original tool name reported by the server."
                    },
                    "enabled": {
                        "type": "boolean",
                        "description": "Whether agents may use this tool."
                    }
                },
                "required": ["serverId", "tool", "enabled"],
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_ACKNOWLEDGE_TOOLS.to_owned(),
            description: "Clear new-tool markers on an MCP server after reviewing newly added \
                tools."
                .to_owned(),
            security_sensitive: true,
            parameters: server_id_schema(),
            secret_fields: Vec::new(),
        },
    ]
}

fn guide_actions() -> Vec<ManagementActionSpec> {
    let mut actions = server_crud_actions();
    actions.extend(server_runtime_actions());
    actions
}

fn validate_secret_keys(
    transport: ServerTransport,
    secret_names: &[String],
) -> Result<(), ManagementError> {
    for name in secret_names {
        let key = secret_key(transport, name);
        if !is_valid_secret_key(&key) {
            return Err(ManagementError::invalid_field(
                "secretNames",
                format!("secret name {name:?} produces an invalid secret key"),
            ));
        }
    }
    Ok(())
}

fn validate_server_shape(input: &ServerInput) -> Result<(), ManagementError> {
    if let Err(refusal) = input.validate() {
        let message = format!("invalid value for {}", refusal.field);
        return Err(ManagementError::invalid_field(refusal.field, message));
    }
    Ok(())
}

struct ServerPreview<'a> {
    name: &'a str,
    prefix: &'a str,
    transport: ServerTransport,
    program: &'a str,
    arguments: &'a [String],
    folder: &'a str,
    url: &'a str,
    plain: &'a [NamedValue],
    secret_names: &'a [String],
    oauth: bool,
    timeout_seconds: u32,
}

fn server_preview_fields(preview: &ServerPreview<'_>) -> Vec<ConfigFieldPreview> {
    let mut fields = vec![
        ConfigFieldPreview {
            label: "Name".to_owned(),
            value: preview.name.to_owned(),
        },
        ConfigFieldPreview {
            label: "Transport".to_owned(),
            value: match preview.transport {
                ServerTransport::Stdio => "stdio".to_owned(),
                ServerTransport::Http => "http".to_owned(),
            },
        },
    ];

    if !preview.prefix.is_empty() {
        fields.push(ConfigFieldPreview {
            label: "Prefix".to_owned(),
            value: preview.prefix.to_owned(),
        });
    }

    match preview.transport {
        ServerTransport::Stdio => {
            fields.push(ConfigFieldPreview {
                label: "Program".to_owned(),
                value: preview.program.to_owned(),
            });
            if !preview.arguments.is_empty() {
                fields.push(ConfigFieldPreview {
                    label: "Arguments".to_owned(),
                    value: preview.arguments.join(" "),
                });
            }
            if !preview.folder.is_empty() {
                fields.push(ConfigFieldPreview {
                    label: "Working folder".to_owned(),
                    value: preview.folder.to_owned(),
                });
            }
        }
        ServerTransport::Http => {
            fields.push(ConfigFieldPreview {
                label: "URL".to_owned(),
                value: preview.url.to_owned(),
            });
            if preview.oauth {
                fields.push(ConfigFieldPreview {
                    label: "Authentication".to_owned(),
                    value: "OAuth".to_owned(),
                });
            }
        }
    }

    if !preview.plain.is_empty() {
        let names: Vec<&str> = preview
            .plain
            .iter()
            .map(|item| item.name.as_str())
            .collect();
        let label = match preview.transport {
            ServerTransport::Stdio => "Plain variables",
            ServerTransport::Http => "Plain headers",
        };
        fields.push(ConfigFieldPreview {
            label: label.to_owned(),
            value: names.join(", "),
        });
    }

    if !preview.secret_names.is_empty() {
        let label = match preview.transport {
            ServerTransport::Stdio => "Secret variables",
            ServerTransport::Http => "Secret headers",
        };
        fields.push(ConfigFieldPreview {
            label: label.to_owned(),
            value: preview.secret_names.join(", "),
        });
    }

    fields.push(ConfigFieldPreview {
        label: "Timeout".to_owned(),
        value: format!("{}s", preview.timeout_seconds),
    });

    fields
}

fn prepare_add_server(
    servers: &Servers,
    request: PrepareActionRequest,
) -> Result<PreparedAction, ManagementError> {
    let provided_secrets = request.provided_secrets.clone();
    let args: AddServerArgs = serde_json::from_value(request.arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;

    validate_secret_keys(args.transport, &args.secret_names)?;

    let probe = ServerInput {
        name: args.name.clone(),
        prefix: args.prefix.clone(),
        transport: args.transport,
        program: args.program.clone(),
        arguments: args.arguments.clone(),
        folder: args.folder.clone(),
        url: args.url.clone(),
        plain: args.plain.clone(),
        secrets: args
            .secret_names
            .iter()
            .map(|name| SecretValue {
                name: name.clone(),
                value: Some(Secret::new("placeholder".to_owned())),
            })
            .collect(),
        oauth: args.oauth,
        client_id: args.client_id.clone(),
        client_secret: (args.has_client_secret
            || provided_secrets.iter().any(|k| k == SECRET_CLIENT_SECRET))
        .then(|| Secret::new("placeholder".to_owned())),
        forget_client_secret: false,
        images: args.images,
        instructions: args.instructions,
        timeout_seconds: args.timeout_seconds,
    };
    validate_server_shape(&probe)?;

    let name_taken = servers
        .names()
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&args.name));
    if name_taken {
        return Err(ManagementError::conflict(format!(
            "an MCP server named {:?} already exists",
            args.name
        )));
    }
    if !args.prefix.is_empty() && servers.by_prefix(&args.prefix).is_some() {
        return Err(ManagementError::conflict(format!(
            "another MCP server already uses prefix {:?}",
            args.prefix
        )));
    }

    let mut secret_fields: Vec<ConfigSecretField> = args
        .secret_names
        .iter()
        .map(|name| {
            let key = secret_key(args.transport, name);
            let provided = provided_secrets.iter().any(|item| item == &key);
            ConfigSecretField {
                key,
                label: secret_label(args.transport, name, false),
                required: true,
                provided_in_chat: provided,
            }
        })
        .collect();

    let wants_client_secret = args.transport == ServerTransport::Http
        && args.oauth
        && !args.client_id.is_empty()
        && (args.has_client_secret || provided_secrets.iter().any(|k| k == SECRET_CLIENT_SECRET));
    if wants_client_secret {
        let provided = provided_secrets.iter().any(|k| k == SECRET_CLIENT_SECRET);
        secret_fields.push(ConfigSecretField {
            key: SECRET_CLIENT_SECRET.to_owned(),
            label: "OAuth client secret".to_owned(),
            required: args.has_client_secret,
            provided_in_chat: provided,
        });
    }

    let fields = server_preview_fields(&ServerPreview {
        name: &args.name,
        prefix: &args.prefix,
        transport: args.transport,
        program: &args.program,
        arguments: &args.arguments,
        folder: &args.folder,
        url: &args.url,
        plain: &args.plain,
        secret_names: &args.secret_names,
        oauth: args.oauth,
        timeout_seconds: args.timeout_seconds,
    });

    let title = format!("Add MCP server {}", args.name);
    let canonical_arguments =
        serde_json::to_value(&args).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title,
        fields,
        secret_fields,
        security_sensitive: true,
        canonical_arguments,
    })
}

fn current_endpoint_fields(
    record: &ServerRecord,
) -> (ServerTransport, String, Vec<String>, String, String) {
    match &record.endpoint {
        StoredEndpoint::Program {
            program,
            arguments,
            folder,
        } => (
            ServerTransport::Stdio,
            program.clone(),
            arguments.clone(),
            folder.clone().unwrap_or_default(),
            String::new(),
        ),
        StoredEndpoint::Url { url } => (
            ServerTransport::Http,
            String::new(),
            Vec::new(),
            String::new(),
            url.clone(),
        ),
    }
}

fn merge_server_update(current: &ServerRecord, patch: UpdateServerPatch) -> CanonicalUpdateServer {
    let (transport, cur_program, cur_args, cur_folder, cur_url) = current_endpoint_fields(current);
    let cur_plain: Vec<NamedValue> = current
        .plain
        .iter()
        .map(|(name, value)| NamedValue {
            name: name.clone(),
            value: value.clone(),
        })
        .collect();

    CanonicalUpdateServer {
        server_id: current.id.clone(),
        name: patch.name.unwrap_or_else(|| current.name.clone()),
        prefix: patch.prefix.unwrap_or_else(|| current.prefix.clone()),
        transport,
        program: patch.program.unwrap_or(cur_program),
        arguments: patch.arguments.unwrap_or(cur_args),
        folder: patch.folder.unwrap_or(cur_folder),
        url: patch.url.unwrap_or(cur_url),
        plain: patch.plain.unwrap_or(cur_plain),
        secret_names: patch
            .secret_names
            .unwrap_or_else(|| current.secret_names.clone()),
        oauth: patch.oauth.unwrap_or(current.oauth),
        client_id: patch
            .client_id
            .unwrap_or_else(|| current.client_id.clone().unwrap_or_default()),
        forget_client_secret: patch.forget_client_secret,
        images: patch.images.unwrap_or(current.images),
        instructions: patch.instructions.unwrap_or(current.instructions),
        timeout_seconds: patch.timeout_seconds.unwrap_or(current.timeout_seconds),
    }
}

fn update_secret_fields(
    current: &ServerRecord,
    canonical: &CanonicalUpdateServer,
    has_client_secret_flag: Option<bool>,
    provided_secrets: &[String],
) -> Vec<ConfigSecretField> {
    let mut secret_fields: Vec<ConfigSecretField> = canonical
        .secret_names
        .iter()
        .map(|secret_name| {
            let key = secret_key(canonical.transport, secret_name);
            let provided = provided_secrets.iter().any(|item| item == &key);
            let existed = current
                .secret_names
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(secret_name));
            let is_missing = current
                .missing_secrets
                .iter()
                .any(|missing| missing.eq_ignore_ascii_case(secret_name));
            let required = !existed || is_missing;
            ConfigSecretField {
                key,
                label: secret_label(canonical.transport, secret_name, !required),
                required,
                provided_in_chat: provided,
            }
        })
        .collect();

    let can_have_client_secret = canonical.transport == ServerTransport::Http
        && canonical.oauth
        && !canonical.client_id.is_empty()
        && !canonical.forget_client_secret;
    if can_have_client_secret {
        let provided = provided_secrets.iter().any(|k| k == SECRET_CLIENT_SECRET);
        let optional = current.client_secret_set || !has_client_secret_flag.unwrap_or(false);
        let label = if optional {
            "OAuth client secret (leave empty to keep existing)".to_owned()
        } else {
            "OAuth client secret".to_owned()
        };
        secret_fields.push(ConfigSecretField {
            key: SECRET_CLIENT_SECRET.to_owned(),
            label,
            required: !optional,
            provided_in_chat: provided,
        });
    }

    secret_fields
}

fn prepare_update_server(
    servers: &Servers,
    request: PrepareActionRequest,
) -> Result<PreparedAction, ManagementError> {
    let provided_secrets = request.provided_secrets.clone();
    let patch: UpdateServerPatch = serde_json::from_value(request.arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;

    let server_ref = ServerRef {
        server_id: patch.server_id.clone(),
    };
    if let Err(refusal) = server_ref.validate() {
        return Err(ManagementError::invalid_field(
            refusal.field,
            "serverId must be a valid UUID",
        ));
    }

    let current = servers
        .record(&patch.server_id)
        .ok_or_else(|| ManagementError::not_found("the MCP server is not registered"))?;
    let has_client_secret_flag = patch.has_client_secret;
    let canonical = merge_server_update(&current, patch);

    validate_secret_keys(canonical.transport, &canonical.secret_names)?;

    let has_client_secret = has_client_secret_flag.unwrap_or(false)
        || provided_secrets.iter().any(|k| k == SECRET_CLIENT_SECRET);
    let probe = ServerInput {
        name: canonical.name.clone(),
        prefix: canonical.prefix.clone(),
        transport: canonical.transport,
        program: canonical.program.clone(),
        arguments: canonical.arguments.clone(),
        folder: canonical.folder.clone(),
        url: canonical.url.clone(),
        plain: canonical.plain.clone(),
        secrets: canonical
            .secret_names
            .iter()
            .map(|secret_name| SecretValue {
                name: secret_name.clone(),
                value: None,
            })
            .collect(),
        oauth: canonical.oauth,
        client_id: canonical.client_id.clone(),
        client_secret: has_client_secret.then(|| Secret::new("placeholder".to_owned())),
        forget_client_secret: canonical.forget_client_secret,
        images: canonical.images,
        instructions: canonical.instructions,
        timeout_seconds: canonical.timeout_seconds,
    };
    validate_server_shape(&probe)?;

    if !canonical.prefix.is_empty()
        && let Some(other) = servers.by_prefix(&canonical.prefix)
        && other.id != current.id
    {
        return Err(ManagementError::conflict(format!(
            "another MCP server already uses prefix {:?}",
            canonical.prefix
        )));
    }

    let secret_fields = update_secret_fields(
        &current,
        &canonical,
        has_client_secret_flag,
        &provided_secrets,
    );
    let fields = server_preview_fields(&ServerPreview {
        name: &canonical.name,
        prefix: &canonical.prefix,
        transport: canonical.transport,
        program: &canonical.program,
        arguments: &canonical.arguments,
        folder: &canonical.folder,
        url: &canonical.url,
        plain: &canonical.plain,
        secret_names: &canonical.secret_names,
        oauth: canonical.oauth,
        timeout_seconds: canonical.timeout_seconds,
    });
    let title = format!("Update MCP server {}", canonical.name);
    let canonical_arguments = serde_json::to_value(&canonical)
        .map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title,
        fields,
        secret_fields,
        security_sensitive: true,
        canonical_arguments,
    })
}

fn parse_server_ref(arguments: serde_json::Value) -> Result<ServerRef, ManagementError> {
    let server_ref: ServerRef = serde_json::from_value(arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    if let Err(refusal) = server_ref.validate() {
        return Err(ManagementError::invalid_field(
            refusal.field,
            "serverId must be a valid UUID",
        ));
    }
    Ok(server_ref)
}

fn prepare_simple_server_action(
    servers: &Servers,
    action: &str,
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    let server_ref = parse_server_ref(arguments)?;
    let record = servers
        .record(&server_ref.server_id)
        .ok_or_else(|| ManagementError::not_found("the MCP server is not registered"))?;

    let (title, security_sensitive) = match action {
        ACTION_REMOVE_SERVER => (format!("Remove MCP server {}", record.name), true),
        ACTION_RESTART_SERVER => (format!("Restart MCP server {}", record.name), false),
        ACTION_ACKNOWLEDGE_TOOLS => (
            format!("Acknowledge new tools for MCP server {}", record.name),
            true,
        ),
        _ => return Err(ManagementError::unknown_action(action)),
    };

    let fields = vec![
        ConfigFieldPreview {
            label: "Server".to_owned(),
            value: record.name,
        },
        ConfigFieldPreview {
            label: "Prefix".to_owned(),
            value: record.prefix,
        },
    ];

    let canonical_arguments = serde_json::to_value(&server_ref)
        .map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title,
        fields,
        secret_fields: Vec::new(),
        security_sensitive,
        canonical_arguments,
    })
}

fn prepare_set_server_enabled(
    servers: &Servers,
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    let input: SetServerEnabled = serde_json::from_value(arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    if let Err(refusal) = input.validate() {
        return Err(ManagementError::invalid_field(
            refusal.field,
            "serverId must be a valid UUID",
        ));
    }

    let record = servers
        .record(&input.server_id)
        .ok_or_else(|| ManagementError::not_found("the MCP server is not registered"))?;

    let verb = if input.enabled { "Enable" } else { "Disable" };
    let fields = vec![
        ConfigFieldPreview {
            label: "Server".to_owned(),
            value: record.name.clone(),
        },
        ConfigFieldPreview {
            label: "State".to_owned(),
            value: if input.enabled { "Enabled" } else { "Disabled" }.to_owned(),
        },
    ];

    let canonical_arguments =
        serde_json::to_value(&input).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title: format!("{verb} MCP server {}", record.name),
        fields,
        secret_fields: Vec::new(),
        security_sensitive: true,
        canonical_arguments,
    })
}

fn prepare_set_tool_enabled(
    servers: &Servers,
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    let input: SetToolEnabled = serde_json::from_value(arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    if let Err(refusal) = input.validate() {
        let message = format!("invalid {}", refusal.field);
        return Err(ManagementError::invalid_field(refusal.field, message));
    }

    let (record, _tool) = servers
        .tool_of(&input.server_id, &input.tool)
        .ok_or_else(|| ManagementError::not_found("the MCP server or tool was not found"))?;

    let verb = if input.enabled { "Enable" } else { "Disable" };
    let fields = vec![
        ConfigFieldPreview {
            label: "Server".to_owned(),
            value: record.name,
        },
        ConfigFieldPreview {
            label: "Tool".to_owned(),
            value: input.tool.clone(),
        },
        ConfigFieldPreview {
            label: "State".to_owned(),
            value: if input.enabled { "Enabled" } else { "Disabled" }.to_owned(),
        },
    ];

    let canonical_arguments =
        serde_json::to_value(&input).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title: format!("{verb} MCP tool {}", input.tool),
        fields,
        secret_fields: Vec::new(),
        security_sensitive: true,
        canonical_arguments,
    })
}

fn collect_secret_values(
    transport: ServerTransport,
    secret_names: &[String],
    secrets: &mut std::collections::BTreeMap<String, Secret>,
    require_all: bool,
) -> Result<Vec<SecretValue>, ManagementError> {
    let rules = ValueRules::of(transport);
    let mut values = Vec::with_capacity(secret_names.len());

    for name in secret_names {
        let key = secret_key(transport, name);
        let value = secrets.remove(&key);
        match (&value, require_all) {
            (None, true) => {
                return Err(ManagementError::invalid_field(
                    key,
                    format!("a value is required for secret {name:?}"),
                ));
            }
            (Some(secret), _) => {
                let valid = !secret.is_empty() && (rules.is_value)(secret.expose());
                if !valid {
                    return Err(ManagementError::invalid_field(
                        key,
                        format!("invalid value for secret {name:?}"),
                    ));
                }
            }
            (None, false) => {}
        }
        values.push(SecretValue {
            name: name.clone(),
            value,
        });
    }

    Ok(values)
}

async fn execute_add_server(
    servers: &Servers,
    mut request: ExecuteActionRequest,
) -> Result<ActionResult, ManagementError> {
    let args: AddServerArgs = serde_json::from_value(request.arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;

    let secrets = collect_secret_values(
        args.transport,
        &args.secret_names,
        &mut request.secrets,
        true,
    )?;
    let client_secret = request.secrets.remove(SECRET_CLIENT_SECRET);
    if let Some(secret) = &client_secret
        && !is_client_secret(secret.expose())
    {
        return Err(ManagementError::invalid_field(
            SECRET_CLIENT_SECRET,
            "invalid OAuth client secret",
        ));
    }

    let input = ServerInput {
        name: args.name,
        prefix: args.prefix,
        transport: args.transport,
        program: args.program,
        arguments: args.arguments,
        folder: args.folder,
        url: args.url,
        plain: args.plain,
        secrets,
        oauth: args.oauth,
        client_id: args.client_id,
        client_secret,
        forget_client_secret: false,
        images: args.images,
        instructions: args.instructions,
        timeout_seconds: args.timeout_seconds,
    };
    validate_server_shape(&input)?;

    let view = servers.add(input).await.map_err(map_mcp_error)?;
    let message = format!("Added MCP server {} (prefix {}).", view.name, view.prefix);
    let data =
        serde_json::to_value(&view).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(ActionResult::new(message).with_data(data))
}

async fn execute_update_server(
    servers: &Servers,
    mut request: ExecuteActionRequest,
) -> Result<ActionResult, ManagementError> {
    let args: CanonicalUpdateServer = serde_json::from_value(request.arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;

    let secrets = collect_secret_values(
        args.transport,
        &args.secret_names,
        &mut request.secrets,
        false,
    )?;
    let client_secret = request.secrets.remove(SECRET_CLIENT_SECRET);
    if let Some(secret) = &client_secret
        && !is_client_secret(secret.expose())
    {
        return Err(ManagementError::invalid_field(
            SECRET_CLIENT_SECRET,
            "invalid OAuth client secret",
        ));
    }

    let input = ServerInput {
        name: args.name,
        prefix: args.prefix,
        transport: args.transport,
        program: args.program,
        arguments: args.arguments,
        folder: args.folder,
        url: args.url,
        plain: args.plain,
        secrets,
        oauth: args.oauth,
        client_id: args.client_id,
        client_secret,
        forget_client_secret: args.forget_client_secret,
        images: args.images,
        instructions: args.instructions,
        timeout_seconds: args.timeout_seconds,
    };
    validate_server_shape(&input)?;

    let view = servers
        .update(UpdateServer {
            server_id: args.server_id,
            server: input,
        })
        .await
        .map_err(map_mcp_error)?;

    let message = format!("Updated MCP server {}.", view.name);
    let data =
        serde_json::to_value(&view).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(ActionResult::new(message).with_data(data))
}

async fn execute_server_runtime_action(
    servers: &Servers,
    action: &str,
    arguments: serde_json::Value,
) -> Result<ActionResult, ManagementError> {
    match action {
        ACTION_REMOVE_SERVER => {
            let server_ref = parse_server_ref(arguments)?;
            servers
                .remove(&server_ref.server_id)
                .await
                .map_err(map_mcp_error)?;

            Ok(
                ActionResult::new("Removed MCP server and deleted its stored secrets.")
                    .with_data(json!({ "serverId": server_ref.server_id })),
            )
        }
        ACTION_SET_SERVER_ENABLED => {
            let input: SetServerEnabled = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            let view = servers
                .set_enabled(&input.server_id, input.enabled)
                .await
                .map_err(map_mcp_error)?;
            let state = if view.enabled { "enabled" } else { "disabled" };
            let data = serde_json::to_value(&view)
                .map_err(|error| ManagementError::failed(error.to_string()))?;

            Ok(
                ActionResult::new(format!("MCP server {} is now {state}.", view.name))
                    .with_data(data),
            )
        }
        ACTION_RESTART_SERVER => {
            let server_ref = parse_server_ref(arguments)?;
            servers
                .restart(&server_ref.server_id)
                .await
                .map_err(map_mcp_error)?;

            Ok(ActionResult::new("Restarted MCP server.")
                .with_data(json!({ "serverId": server_ref.server_id })))
        }
        ACTION_SET_TOOL_ENABLED => {
            let input: SetToolEnabled = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            servers
                .set_tool_enabled(&input.server_id, &input.tool, input.enabled)
                .await
                .map_err(map_mcp_error)?;
            let state = if input.enabled { "enabled" } else { "disabled" };

            Ok(
                ActionResult::new(format!("MCP tool {} is now {state}.", input.tool)).with_data(
                    json!({
                        "serverId": input.server_id,
                        "tool": input.tool,
                        "enabled": input.enabled,
                    }),
                ),
            )
        }
        ACTION_ACKNOWLEDGE_TOOLS => {
            let server_ref = parse_server_ref(arguments)?;
            servers
                .acknowledge_tools(&server_ref.server_id)
                .await
                .map_err(map_mcp_error)?;

            Ok(ActionResult::new("Acknowledged new MCP tools.")
                .with_data(json!({ "serverId": server_ref.server_id })))
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

impl ManagementProvider for McpManagement {
    fn guide(&self) -> ManagementGuide {
        ManagementGuide {
            summary: "Connects Mannie to Model Context Protocol (MCP) tool servers, both local \
                stdio programs managed by the core and remote Streamable HTTP URLs, with per-tool \
                enable switches, new-tool review, resource mentions, and OAuth support."
                .to_owned(),
            storage: "Stores server configurations and tool states in the `mcp_servers` and \
                `mcp_tools` SQLite tables. Stores secret environment variables, secret HTTP \
                headers, and OAuth client secrets/tokens only in the master's OS credential store \
                under keyed families `mcp/variable/*`, `mcp/header/*`, and `mcp/oauth/*`."
                .to_owned(),
            troubleshooting: "Use `mannie_inspect` with target `mcp` to list servers, statuses, \
                and warnings; use query `tools:<server-id>` to inspect a server's discovered \
                tools, `log:<server-id>` for its redacted stderr and connection events, or \
                `locate:<program>` to check whether a stdio program resolves on PATH. Remote \
                servers must speak Streamable HTTP (SSE-only 2024 servers are refused). OAuth \
                browser sign-in must be started by the owner in Settings."
                .to_owned(),
            actions: guide_actions(),
        }
    }

    fn inspect(
        &self,
        query: Option<String>,
    ) -> BoxFuture<'static, Result<ManagementSnapshot, ManagementError>> {
        let servers = self.servers.clone();

        Box::pin(async move {
            let trimmed = query.as_deref().map(str::trim).filter(|q| !q.is_empty());

            if let Some(selector) = trimmed {
                if let Some(server_id) = selector.strip_prefix("tools:") {
                    let tools = servers.tools(server_id).map_err(map_mcp_error)?;
                    let summary = format!("{} tool(s) listed for MCP server.", tools.len());
                    return Ok(ManagementSnapshot::new(
                        summary,
                        json!({ "serverId": server_id, "tools": tools }),
                    ));
                }
                if let Some(server_id) = selector.strip_prefix("log:") {
                    let log = servers.log(server_id).map_err(map_mcp_error)?;
                    let summary = format!(
                        "{} log entry/entries for MCP server (truncated: {}).",
                        log.entries.len(),
                        log.cut
                    );
                    return Ok(ManagementSnapshot::new(
                        summary,
                        json!({
                            "serverId": server_id,
                            "entries": log.entries,
                            "cut": log.cut,
                        }),
                    ));
                }
                if let Some(program) = selector.strip_prefix("locate:") {
                    if !is_program(program) {
                        return Err(ManagementError::invalid_field(
                            "query",
                            "program name is invalid",
                        ));
                    }
                    let located = servers.locate(program).await.map_err(map_mcp_error)?;
                    let summary = format!("Resolved program {program:?} to {}.", located.path);
                    return Ok(ManagementSnapshot::new(
                        summary,
                        json!({
                            "program": program,
                            "path": located.path,
                            "inProject": located.in_project,
                        }),
                    ));
                }
                return Err(ManagementError::invalid_field(
                    "query",
                    format!(
                        "unsupported mcp inspect query {selector:?}; omit query or use \"tools:<id>\", \"log:<id>\", or \"locate:<program>\""
                    ),
                ));
            }

            let ServerListUpdate::Servers {
                servers: views,
                enabled_tools,
            } = servers.list_update();
            let summary = format!(
                "{} registered MCP server(s), {enabled_tools}/{MAX_ENABLED_TOOLS} tools enabled.",
                views.len()
            );
            let data = json!({
                "servers": views,
                "enabledTools": enabled_tools,
                "maxEnabledTools": MAX_ENABLED_TOOLS,
            });

            Ok(ManagementSnapshot::new(summary, data))
        })
    }

    fn prepare_action(
        &self,
        request: PrepareActionRequest,
    ) -> BoxFuture<'static, Result<PreparedAction, ManagementError>> {
        let servers = self.servers.clone();

        Box::pin(async move {
            match request.action.as_str() {
                ACTION_ADD_SERVER => prepare_add_server(&servers, request),
                ACTION_UPDATE_SERVER => prepare_update_server(&servers, request),
                ACTION_REMOVE_SERVER | ACTION_RESTART_SERVER | ACTION_ACKNOWLEDGE_TOOLS => {
                    prepare_simple_server_action(&servers, &request.action, request.arguments)
                }
                ACTION_SET_SERVER_ENABLED => {
                    prepare_set_server_enabled(&servers, request.arguments)
                }
                ACTION_SET_TOOL_ENABLED => prepare_set_tool_enabled(&servers, request.arguments),
                other => Err(ManagementError::unknown_action(other)),
            }
        })
    }

    fn execute_action(
        &self,
        request: ExecuteActionRequest,
    ) -> BoxFuture<'static, Result<ActionResult, ManagementError>> {
        let servers = self.servers.clone();

        Box::pin(async move {
            match request.action.as_str() {
                ACTION_ADD_SERVER => execute_add_server(&servers, request).await,
                ACTION_UPDATE_SERVER => execute_update_server(&servers, request).await,
                ACTION_REMOVE_SERVER
                | ACTION_SET_SERVER_ENABLED
                | ACTION_RESTART_SERVER
                | ACTION_SET_TOOL_ENABLED
                | ACTION_ACKNOWLEDGE_TOOLS => {
                    execute_server_runtime_action(&servers, &request.action, request.arguments)
                        .await
                }
                other => Err(ManagementError::unknown_action(other)),
            }
        })
    }
}
