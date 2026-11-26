//! Self-management provider for the Proxmox plugin (ADR-0081).
//!
//! Exposes registered Proxmox VE servers, fail-closed VM allowlists, TLS
//! certificate pinning status, and typed actions for managing servers,
//! certificates, and VM allowlists through the master's always-ask
//! `Access::AppConfig` policy.

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{ConfigFieldPreview, ConfigSecretField};
use mannie_plugin_sdk::ipc::{PluginError as _, Validate as _};
use mannie_plugin_sdk::points::management::{
    ActionResult, ExecuteActionRequest, ManagementActionSpec, ManagementError, ManagementGuide,
    ManagementProvider, ManagementSnapshot, PrepareActionRequest, PreparedAction, SecretFieldSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::model::{
    AddServer, DEFAULT_PORT, MAX_ALLOWED_VMS_PER_SERVER, MAX_NAME_CHARS, MAX_NOTE_CHARS,
    MAX_USERNAME_CHARS, PinCertificate, ProxmoxError, ServerRef, ServerStatus, SetAllowedVms,
    UpdateServer, canonical_endpoint, format_fingerprint_colon, is_valid_note, is_valid_password,
    is_valid_server_name, is_valid_username, normalize_host, normalize_sha256_hex,
};
use crate::state::ProxmoxManager;

const ACTION_ADD_SERVER: &str = "add_server";
const ACTION_UPDATE_SERVER: &str = "update_server";
const ACTION_REMOVE_SERVER: &str = "remove_server";
const ACTION_INSPECT_CERTIFICATE: &str = "inspect_certificate";
const ACTION_PIN_CERTIFICATE: &str = "pin_certificate";
const ACTION_CLEAR_CERTIFICATE_PIN: &str = "clear_certificate_pin";
const ACTION_TEST_CONNECTION: &str = "test_connection";
const ACTION_DISCOVER_VMS: &str = "discover_vms";
const ACTION_SET_ALLOWED_VMS: &str = "set_allowed_vms";
const SECRET_PASSWORD: &str = "password";

/// Management provider for the Proxmox plugin.
#[derive(Debug, Clone)]
pub(crate) struct ProxmoxManagement {
    manager: ProxmoxManager,
}

impl ProxmoxManagement {
    /// Creates a management provider backed by the running Proxmox manager.
    pub(crate) fn new(manager: ProxmoxManager) -> Self {
        Self { manager }
    }
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddServerArgs {
    name: String,
    host: String,
    #[serde(default = "default_port")]
    port: u16,
    username: String,
    #[serde(default)]
    note: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateServerPatch {
    server_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CanonicalUpdateServer {
    server_id: String,
    name: String,
    host: String,
    port: u16,
    username: String,
    note: String,
}

fn map_proxmox_error(error: ProxmoxError) -> ManagementError {
    match error {
        ProxmoxError::NameTaken | ProxmoxError::PinMismatch => {
            ManagementError::conflict(error.to_string())
        }
        ProxmoxError::ServerNotFound | ProxmoxError::VmNotFound => {
            ManagementError::not_found(error.to_string())
        }
        ProxmoxError::InvalidRequest { ref detail } => ManagementError::invalid_arguments(detail),
        other => ManagementError::new(other.code(), other.to_string()),
    }
}

fn status_label(status: ServerStatus) -> &'static str {
    match status {
        ServerStatus::Ready => "ready",
        ServerStatus::NoVmsAllowed => "no_vms_allowed",
        ServerStatus::PasswordMissing => "password_missing",
    }
}

fn validate_server_fields(
    name: &str,
    host: &str,
    port: u16,
    username: &str,
    note: &str,
) -> Result<String, ManagementError> {
    if !is_valid_server_name(name) {
        return Err(ManagementError::invalid_field(
            "name",
            format!("server name must be 1 to {MAX_NAME_CHARS} printable characters"),
        ));
    }

    let Some(canonical_host) = normalize_host(host) else {
        return Err(ManagementError::invalid_field(
            "host",
            "host must be a valid hostname, IPv4, or IPv6 address without scheme, port, or path",
        ));
    };

    if port == 0 {
        return Err(ManagementError::invalid_field(
            "port",
            "port must be between 1 and 65535",
        ));
    }

    if !is_valid_username(username) {
        return Err(ManagementError::invalid_field(
            "username",
            format!(
                "username must be user@realm (3 to {MAX_USERNAME_CHARS} characters, e.g. root@pam or api@pve)"
            ),
        ));
    }

    if !is_valid_note(note) {
        return Err(ManagementError::invalid_field(
            "note",
            format!("note must be at most {MAX_NOTE_CHARS} characters without NUL bytes"),
        ));
    }

    Ok(canonical_host)
}

fn server_id_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "serverId": {
                "type": "string",
                "description": "UUID of the registered Proxmox VE server."
            }
        },
        "required": ["serverId"],
        "additionalProperties": false
    })
}

fn server_crud_actions() -> Vec<ManagementActionSpec> {
    vec![
        ManagementActionSpec {
            name: ACTION_ADD_SERVER.to_owned(),
            description: "Register a Proxmox VE server with zero allowed VMs by default and store \
                its account password in the OS credential store."
                .to_owned(),
            security_sensitive: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Unique display name for the server."
                    },
                    "host": {
                        "type": "string",
                        "description": "Hostname, IPv4, or IPv6 address (without https:// or port)."
                    },
                    "port": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 65535,
                        "default": DEFAULT_PORT,
                        "description": "HTTPS API port (defaults to 8006)."
                    },
                    "username": {
                        "type": "string",
                        "description": "Proxmox username with realm (for example `user@pve` or `root@pam`)."
                    },
                    "note": {
                        "type": "string",
                        "description": "Optional note describing the server."
                    }
                },
                "required": ["name", "host", "username"],
                "additionalProperties": false
            }),
            secret_fields: vec![SecretFieldSpec {
                key: SECRET_PASSWORD.to_owned(),
                label: "Password".to_owned(),
                description: "Proxmox account password stored in the OS credential store."
                    .to_owned(),
                required: true,
            }],
        },
        ManagementActionSpec {
            name: ACTION_UPDATE_SERVER.to_owned(),
            description: "Update a Proxmox VE server's settings and optionally replace its \
                password. Changing host or port clears its pinned certificate and VM allowlist."
                .to_owned(),
            security_sensitive: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "serverId": {
                        "type": "string",
                        "description": "UUID of the Proxmox server to update."
                    },
                    "name": { "type": "string" },
                    "host": { "type": "string" },
                    "port": { "type": "integer", "minimum": 1, "maximum": 65535 },
                    "username": { "type": "string" },
                    "note": { "type": "string" }
                },
                "required": ["serverId"],
                "additionalProperties": false
            }),
            secret_fields: vec![SecretFieldSpec {
                key: SECRET_PASSWORD.to_owned(),
                label: "New password (optional)".to_owned(),
                description: "Replacement password; leave empty to keep the stored password."
                    .to_owned(),
                required: false,
            }],
        },
        ManagementActionSpec {
            name: ACTION_REMOVE_SERVER.to_owned(),
            description: "Remove a registered Proxmox server, delete its stored password, and \
                close live VNC sessions."
                .to_owned(),
            security_sensitive: true,
            parameters: server_id_schema(),
            secret_fields: Vec::new(),
        },
    ]
}

fn tls_and_vm_actions() -> Vec<ManagementActionSpec> {
    vec![
        ManagementActionSpec {
            name: ACTION_INSPECT_CERTIFICATE.to_owned(),
            description: "Perform a TLS-only handshake to inspect the server's presented leaf \
                certificate fingerprint without sending credentials."
                .to_owned(),
            security_sensitive: false,
            parameters: server_id_schema(),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_PIN_CERTIFICATE.to_owned(),
            description: "Pin a verified leaf TLS certificate SHA-256 fingerprint for a server."
                .to_owned(),
            security_sensitive: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "serverId": {
                        "type": "string",
                        "description": "UUID of the Proxmox server."
                    },
                    "sha256": {
                        "type": "string",
                        "description": "64-hex-digit SHA-256 digest (or colon-separated fingerprint) from inspect_certificate."
                    }
                },
                "required": ["serverId", "sha256"],
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_CLEAR_CERTIFICATE_PIN.to_owned(),
            description: "Clear a server's pinned leaf certificate and return it to OS trust \
                store verification."
                .to_owned(),
            security_sensitive: true,
            parameters: server_id_schema(),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_TEST_CONNECTION.to_owned(),
            description: "Test TLS trust, Proxmox ticket login, and QEMU VM visibility.".to_owned(),
            security_sensitive: false,
            parameters: server_id_schema(),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_DISCOVER_VMS.to_owned(),
            description: "Query all QEMU VMs visible to the server's Proxmox account so the owner \
                can choose which VMs to allow."
                .to_owned(),
            security_sensitive: false,
            parameters: server_id_schema(),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_SET_ALLOWED_VMS.to_owned(),
            description: "Replace the server's explicit QEMU VM allowlist (fail-closed) and close \
                live VNC sessions for any removed VM."
                .to_owned(),
            security_sensitive: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "serverId": {
                        "type": "string",
                        "description": "UUID of the Proxmox server."
                    },
                    "vms": {
                        "type": "array",
                        "maxItems": MAX_ALLOWED_VMS_PER_SERVER,
                        "items": {
                            "type": "object",
                            "properties": {
                                "vmid": { "type": "integer", "minimum": 1 },
                                "node": { "type": "string" },
                                "name": { "type": "string" }
                            },
                            "required": ["vmid", "node", "name"],
                            "additionalProperties": false
                        },
                        "description": "Complete list of VMs allowed for Mannie on this server."
                    }
                },
                "required": ["serverId", "vms"],
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
    ]
}

fn guide_actions() -> Vec<ManagementActionSpec> {
    let mut actions = server_crud_actions();
    actions.extend(tls_and_vm_actions());
    actions
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

fn prepare_add_server(
    manager: &ProxmoxManager,
    request: PrepareActionRequest,
) -> Result<PreparedAction, ManagementError> {
    let provided_in_chat = request.has_provided_secret(SECRET_PASSWORD);
    let args: AddServerArgs = serde_json::from_value(request.arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;

    let trimmed_name = args.name.trim().to_owned();
    let trimmed_user = args.username.trim().to_owned();
    let trimmed_note = args.note.trim().to_owned();
    let host = validate_server_fields(
        &trimmed_name,
        &args.host,
        args.port,
        &trimmed_user,
        &trimmed_note,
    )?;

    if manager.find_server(&trimmed_name).is_some() {
        return Err(ManagementError::conflict(format!(
            "a Proxmox server named {trimmed_name:?} already exists"
        )));
    }

    let canonical = AddServerArgs {
        name: trimmed_name,
        host,
        port: args.port,
        username: trimmed_user,
        note: trimmed_note,
    };

    let mut fields = vec![
        ConfigFieldPreview {
            label: "Name".to_owned(),
            value: canonical.name.clone(),
        },
        ConfigFieldPreview {
            label: "Endpoint".to_owned(),
            value: canonical_endpoint(&canonical.host, canonical.port),
        },
        ConfigFieldPreview {
            label: "Username".to_owned(),
            value: canonical.username.clone(),
        },
    ];

    if !canonical.note.is_empty() {
        fields.push(ConfigFieldPreview {
            label: "Note".to_owned(),
            value: canonical.note.clone(),
        });
    }

    let canonical_arguments = serde_json::to_value(&canonical)
        .map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title: format!("Add Proxmox server {}", canonical.name),
        fields,
        secret_fields: vec![ConfigSecretField {
            key: SECRET_PASSWORD.to_owned(),
            label: "Password".to_owned(),
            required: true,
            provided_in_chat,
        }],
        security_sensitive: true,
        canonical_arguments,
    })
}

fn prepare_update_server(
    manager: &ProxmoxManager,
    request: PrepareActionRequest,
) -> Result<PreparedAction, ManagementError> {
    let provided_in_chat = request.has_provided_secret(SECRET_PASSWORD);
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

    let existing = manager
        .find_server(&patch.server_id)
        .ok_or_else(|| ManagementError::not_found("the Proxmox server was not found"))?;

    let name = patch.name.map_or_else(
        || existing.server.name.clone(),
        |value| value.trim().to_owned(),
    );
    let raw_host = patch.host.unwrap_or_else(|| existing.server.host.clone());
    let port = patch.port.unwrap_or(existing.server.port);
    let username = patch.username.map_or_else(
        || existing.server.username.clone(),
        |value| value.trim().to_owned(),
    );
    let note = patch.note.map_or_else(
        || existing.server.note.clone(),
        |value| value.trim().to_owned(),
    );

    let host = validate_server_fields(&name, &raw_host, port, &username, &note)?;

    if let Some(other) = manager.find_server(&name)
        && other.server.id != existing.server.id
    {
        return Err(ManagementError::conflict(format!(
            "another Proxmox server is already named {name:?}"
        )));
    }

    let endpoint_changed = host != existing.server.host || port != existing.server.port;
    let canonical = CanonicalUpdateServer {
        server_id: existing.server.id,
        name,
        host,
        port,
        username,
        note,
    };

    let mut fields = vec![
        ConfigFieldPreview {
            label: "Name".to_owned(),
            value: canonical.name.clone(),
        },
        ConfigFieldPreview {
            label: "Endpoint".to_owned(),
            value: canonical_endpoint(&canonical.host, canonical.port),
        },
        ConfigFieldPreview {
            label: "Username".to_owned(),
            value: canonical.username.clone(),
        },
    ];

    if !canonical.note.is_empty() {
        fields.push(ConfigFieldPreview {
            label: "Note".to_owned(),
            value: canonical.note.clone(),
        });
    }

    if endpoint_changed {
        fields.push(ConfigFieldPreview {
            label: "Reset on endpoint change".to_owned(),
            value: "Pinned certificate and VM allowlist will be cleared".to_owned(),
        });
    }

    let canonical_arguments = serde_json::to_value(&canonical)
        .map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title: format!("Update Proxmox server {}", canonical.name),
        fields,
        secret_fields: vec![ConfigSecretField {
            key: SECRET_PASSWORD.to_owned(),
            label: "New password (leave empty to keep existing)".to_owned(),
            required: false,
            provided_in_chat,
        }],
        security_sensitive: true,
        canonical_arguments,
    })
}

fn prepare_server_ref_action(
    manager: &ProxmoxManager,
    action: &str,
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    let server_ref = parse_server_ref(arguments)?;
    let state = manager
        .find_server(&server_ref.server_id)
        .ok_or_else(|| ManagementError::not_found("the Proxmox server was not found"))?;

    let (title, security_sensitive) = match action {
        ACTION_REMOVE_SERVER => (format!("Remove Proxmox server {}", state.server.name), true),
        ACTION_INSPECT_CERTIFICATE => (
            format!("Inspect TLS certificate for {}", state.server.name),
            false,
        ),
        ACTION_CLEAR_CERTIFICATE_PIN => (
            format!("Clear pinned TLS certificate for {}", state.server.name),
            true,
        ),
        ACTION_TEST_CONNECTION => (
            format!("Test Proxmox connection to {}", state.server.name),
            false,
        ),
        ACTION_DISCOVER_VMS => (format!("Discover QEMU VMs on {}", state.server.name), false),
        _ => return Err(ManagementError::unknown_action(action)),
    };

    let fields = vec![
        ConfigFieldPreview {
            label: "Server".to_owned(),
            value: state.server.name.clone(),
        },
        ConfigFieldPreview {
            label: "Endpoint".to_owned(),
            value: state.server.endpoint(),
        },
        ConfigFieldPreview {
            label: "Allowed VMs".to_owned(),
            value: state.allowed_vms.len().to_string(),
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

fn prepare_pin_certificate(
    manager: &ProxmoxManager,
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    let input: PinCertificate = serde_json::from_value(arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    if let Err(refusal) = input.validate() {
        let message = format!("invalid {}", refusal.field);
        return Err(ManagementError::invalid_field(refusal.field, message));
    }

    let state = manager
        .find_server(&input.server_id)
        .ok_or_else(|| ManagementError::not_found("the Proxmox server was not found"))?;
    let hex = normalize_sha256_hex(&input.sha256)
        .ok_or_else(|| ManagementError::invalid_field("sha256", "invalid SHA-256 fingerprint"))?;

    let canonical = PinCertificate {
        server_id: state.server.id.clone(),
        sha256: hex.clone(),
    };
    let fields = vec![
        ConfigFieldPreview {
            label: "Server".to_owned(),
            value: state.server.name.clone(),
        },
        ConfigFieldPreview {
            label: "Endpoint".to_owned(),
            value: state.server.endpoint(),
        },
        ConfigFieldPreview {
            label: "SHA-256 fingerprint".to_owned(),
            value: format_fingerprint_colon(&hex),
        },
    ];

    let canonical_arguments = serde_json::to_value(&canonical)
        .map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title: format!("Pin TLS certificate for {}", state.server.name),
        fields,
        secret_fields: Vec::new(),
        security_sensitive: true,
        canonical_arguments,
    })
}

fn prepare_set_allowed_vms(
    manager: &ProxmoxManager,
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    let input: SetAllowedVms = serde_json::from_value(arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    if let Err(refusal) = input.validate() {
        let message = format!("invalid {}", refusal.field);
        return Err(ManagementError::invalid_field(refusal.field, message));
    }

    let state = manager
        .find_server(&input.server_id)
        .ok_or_else(|| ManagementError::not_found("the Proxmox server was not found"))?;

    let vm_summary = if input.vms.is_empty() {
        "None (all VMs disallowed)".to_owned()
    } else {
        input
            .vms
            .iter()
            .map(|vm| format!("{} ({} on {})", vm.vmid, vm.name, vm.node))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let fields = vec![
        ConfigFieldPreview {
            label: "Server".to_owned(),
            value: state.server.name.clone(),
        },
        ConfigFieldPreview {
            label: "Endpoint".to_owned(),
            value: state.server.endpoint(),
        },
        ConfigFieldPreview {
            label: "Allowed VMs".to_owned(),
            value: vm_summary,
        },
    ];

    let canonical_arguments =
        serde_json::to_value(&input).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title: format!("Set allowed VMs for {}", state.server.name),
        fields,
        secret_fields: Vec::new(),
        security_sensitive: true,
        canonical_arguments,
    })
}

async fn execute_add_server(
    manager: &ProxmoxManager,
    mut request: ExecuteActionRequest,
) -> Result<ActionResult, ManagementError> {
    let args: AddServerArgs = serde_json::from_value(request.arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;

    let password = request.secrets.remove(SECRET_PASSWORD).ok_or_else(|| {
        ManagementError::invalid_field(
            SECRET_PASSWORD,
            "a password is required to add a Proxmox server",
        )
    })?;

    if !is_valid_password(password.expose()) {
        return Err(ManagementError::invalid_field(
            SECRET_PASSWORD,
            "password must be non-empty and contain no NUL bytes",
        ));
    }

    let state = manager
        .add_server(AddServer {
            name: args.name,
            host: args.host,
            port: args.port,
            username: args.username,
            password,
            note: args.note,
        })
        .await
        .map_err(map_proxmox_error)?;

    let view = state.view();
    let message = format!(
        "Added Proxmox server {} ({}) with zero allowed VMs.",
        view.name, view.endpoint
    );
    let data =
        serde_json::to_value(&view).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(ActionResult::new(message).with_data(data))
}

async fn execute_update_server(
    manager: &ProxmoxManager,
    mut request: ExecuteActionRequest,
) -> Result<ActionResult, ManagementError> {
    let args: CanonicalUpdateServer = serde_json::from_value(request.arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;

    let password = request.secrets.remove(SECRET_PASSWORD);
    if let Some(secret) = &password
        && !is_valid_password(secret.expose())
    {
        return Err(ManagementError::invalid_field(
            SECRET_PASSWORD,
            "password must be non-empty and contain no NUL bytes",
        ));
    }

    let state = manager
        .update_server(UpdateServer {
            server_id: args.server_id,
            name: args.name,
            host: args.host,
            port: args.port,
            username: args.username,
            password,
            note: args.note,
        })
        .await
        .map_err(map_proxmox_error)?;

    let view = state.view();
    let message = format!("Updated Proxmox server {} ({}).", view.name, view.endpoint);
    let data =
        serde_json::to_value(&view).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(ActionResult::new(message).with_data(data))
}

async fn execute_certificate_or_remove_action(
    manager: &ProxmoxManager,
    action: &str,
    arguments: serde_json::Value,
) -> Result<ActionResult, ManagementError> {
    match action {
        ACTION_REMOVE_SERVER => {
            let server_ref = parse_server_ref(arguments)?;
            manager
                .remove_server(&server_ref.server_id)
                .await
                .map_err(map_proxmox_error)?;

            Ok(
                ActionResult::new("Removed Proxmox server and deleted its stored password.")
                    .with_data(json!({ "serverId": server_ref.server_id })),
            )
        }
        ACTION_INSPECT_CERTIFICATE => {
            let server_ref = parse_server_ref(arguments)?;
            let info = manager
                .inspect_certificate(&server_ref.server_id)
                .await
                .map_err(map_proxmox_error)?;
            let message = format!(
                "Inspected leaf TLS certificate: {} (valid now: {}, OS trusted: {}).",
                info.fingerprint, info.valid_now, info.os_trusted
            );
            let data = serde_json::to_value(&info)
                .map_err(|error| ManagementError::failed(error.to_string()))?;

            Ok(ActionResult::new(message).with_data(data))
        }
        ACTION_PIN_CERTIFICATE => {
            let input: PinCertificate = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            let state = manager
                .pin_certificate(input)
                .await
                .map_err(map_proxmox_error)?;
            let view = state.view();
            let data = serde_json::to_value(&view)
                .map_err(|error| ManagementError::failed(error.to_string()))?;

            Ok(
                ActionResult::new(format!("Pinned leaf TLS certificate for {}.", view.name))
                    .with_data(data),
            )
        }
        ACTION_CLEAR_CERTIFICATE_PIN => {
            let server_ref = parse_server_ref(arguments)?;
            let state = manager
                .clear_certificate_pin(&server_ref.server_id)
                .await
                .map_err(map_proxmox_error)?;
            let view = state.view();
            let data = serde_json::to_value(&view)
                .map_err(|error| ManagementError::failed(error.to_string()))?;

            Ok(
                ActionResult::new(format!("Cleared pinned TLS certificate for {}.", view.name))
                    .with_data(data),
            )
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

async fn execute_connection_or_vm_action(
    manager: &ProxmoxManager,
    action: &str,
    arguments: serde_json::Value,
) -> Result<ActionResult, ManagementError> {
    match action {
        ACTION_TEST_CONNECTION => {
            let server_ref = parse_server_ref(arguments)?;
            let result = manager
                .test_connection(&server_ref.server_id)
                .await
                .map_err(map_proxmox_error)?;
            let message = format!(
                "Connected to {} ({} visible VM(s), {} allowed).",
                result.endpoint, result.visible_vm_count, result.allowed_vm_count
            );
            let data = serde_json::to_value(&result)
                .map_err(|error| ManagementError::failed(error.to_string()))?;

            Ok(ActionResult::new(message).with_data(data))
        }
        ACTION_DISCOVER_VMS => {
            let server_ref = parse_server_ref(arguments)?;
            let vms = manager
                .discover_server_vms(&server_ref.server_id)
                .await
                .map_err(map_proxmox_error)?;
            let message = format!("Discovered {} QEMU VM(s).", vms.len());

            Ok(ActionResult::new(message).with_data(json!({
                "serverId": server_ref.server_id,
                "vms": vms,
            })))
        }
        ACTION_SET_ALLOWED_VMS => {
            let input: SetAllowedVms = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            let state = manager
                .set_allowed_vms(input)
                .await
                .map_err(map_proxmox_error)?;
            let view = state.view();
            let message = format!(
                "Updated VM allowlist for {} ({} allowed VM(s)).",
                view.name,
                view.allowed_vms.len()
            );
            let data = serde_json::to_value(&view)
                .map_err(|error| ManagementError::failed(error.to_string()))?;

            Ok(ActionResult::new(message).with_data(data))
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

impl ManagementProvider for ProxmoxManagement {
    fn guide(&self) -> ManagementGuide {
        ManagementGuide {
            summary: "Manages Proxmox VE servers, fail-closed QEMU VM allowlists, leaf TLS \
                certificate pinning, VM lifecycle/configuration tools, and live VNC console \
                sessions under the master's policy and journal."
                .to_owned(),
            storage: "Stores server metadata and certificate pins in `proxmox_servers` and \
                explicitly allowed VMs in `proxmox_allowed_vms` in SQLite. Stores each server's \
                account password only in the master's OS credential store under \
                `proxmox/server/<server-id>`."
                .to_owned(),
            troubleshooting: "New servers start with zero allowed VMs (`no_vms_allowed`) until \
                you run `discover_vms` and `set_allowed_vms`. Self-signed Proxmox certificates \
                fail OS trust (`certificate_untrusted`) until you run `inspect_certificate` and \
                `pin_certificate` with the verified SHA-256 fingerprint. Two-factor Proxmox \
                accounts are unsupported; use a non-2FA user@realm account. Changing host or port \
                clears both the pinned certificate and VM allowlist."
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
            let trimmed = query.as_deref().map(str::trim).filter(|q| !q.is_empty());

            if let Some(selector) = trimmed {
                let Some(key) = selector.strip_prefix("server:") else {
                    return Err(ManagementError::invalid_field(
                        "query",
                        format!(
                            "unsupported proxmox inspect query {selector:?}; omit query or use \"server:<id-or-name>\""
                        ),
                    ));
                };

                let state = manager.find_server(key).ok_or_else(|| {
                    ManagementError::not_found(format!("no Proxmox server matching {key:?}"))
                })?;
                let view = state.view();
                let summary = format!(
                    "Proxmox server {} ({}, status: {}, {} allowed VM(s)).",
                    view.name,
                    view.endpoint,
                    status_label(view.status),
                    view.allowed_vms.len()
                );
                let data = serde_json::to_value(&view)
                    .map_err(|error| ManagementError::failed(error.to_string()))?;

                return Ok(ManagementSnapshot::new(summary, data));
            }

            let views: Vec<_> = manager
                .states()
                .iter()
                .map(crate::model::ServerState::view)
                .collect();
            let active_vnc = manager.vnc().active_count();
            let ready = views
                .iter()
                .filter(|view| view.status == ServerStatus::Ready)
                .count();
            let summary = format!(
                "{} registered Proxmox server(s) ({ready} ready, {active_vnc} active VNC session(s)).",
                views.len()
            );
            let data = json!({
                "servers": views,
                "count": views.len(),
                "readyCount": ready,
                "activeVncSessions": active_vnc,
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
            match request.action.as_str() {
                ACTION_ADD_SERVER => prepare_add_server(&manager, request),
                ACTION_UPDATE_SERVER => prepare_update_server(&manager, request),
                ACTION_REMOVE_SERVER
                | ACTION_INSPECT_CERTIFICATE
                | ACTION_CLEAR_CERTIFICATE_PIN
                | ACTION_TEST_CONNECTION
                | ACTION_DISCOVER_VMS => {
                    prepare_server_ref_action(&manager, &request.action, request.arguments)
                }
                ACTION_PIN_CERTIFICATE => prepare_pin_certificate(&manager, request.arguments),
                ACTION_SET_ALLOWED_VMS => prepare_set_allowed_vms(&manager, request.arguments),
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
                ACTION_ADD_SERVER => execute_add_server(&manager, request).await,
                ACTION_UPDATE_SERVER => execute_update_server(&manager, request).await,
                ACTION_REMOVE_SERVER
                | ACTION_INSPECT_CERTIFICATE
                | ACTION_PIN_CERTIFICATE
                | ACTION_CLEAR_CERTIFICATE_PIN => {
                    execute_certificate_or_remove_action(
                        &manager,
                        &request.action,
                        request.arguments,
                    )
                    .await
                }
                ACTION_TEST_CONNECTION | ACTION_DISCOVER_VMS | ACTION_SET_ALLOWED_VMS => {
                    execute_connection_or_vm_action(&manager, &request.action, request.arguments)
                        .await
                }
                other => Err(ManagementError::unknown_action(other)),
            }
        })
    }
}
