//! Self-management provider for the SSH plugin (ADR-0081).
//!
//! Exposes registered SSH host metadata (never passwords), host-key pinning
//! diagnostics, and typed actions for adding, updating, removing, scanning,
//! trusting, and testing SSH hosts through the master's always-ask
//! `Access::AppConfig` approval flow.

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{ConfigFieldPreview, ConfigSecretField};
use mannie_plugin_sdk::ipc::{PluginError as _, Validate as _};
use mannie_plugin_sdk::points::management::{
    ActionResult, ExecuteActionRequest, ManagementActionSpec, ManagementError, ManagementGuide,
    ManagementProvider, ManagementSnapshot, PrepareActionRequest, PreparedAction, SecretFieldSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::commands::{
    AddHost, HostRef, HostStatus, KeyComparison, SshError, TrustHostKey, UpdateHost,
};
use crate::hosts::Hosts;
use crate::rules::{
    DEFAULT_PORT, MAX_NAME_CHARS, MAX_NOTE_CHARS, MAX_USERNAME_CHARS, endpoint, is_host_name,
    is_note, is_password, is_port, is_username, stored_address,
};

const ACTION_ADD_HOST: &str = "add_host";
const ACTION_UPDATE_HOST: &str = "update_host";
const ACTION_REMOVE_HOST: &str = "remove_host";
const ACTION_SCAN_HOST_KEY: &str = "scan_host_key";
const ACTION_TRUST_HOST_KEY: &str = "trust_host_key";
const ACTION_TEST_HOST: &str = "test_host";
const SECRET_PASSWORD: &str = "password";

/// Management provider for the SSH plugin.
#[derive(Debug, Clone)]
pub(crate) struct SshManagement {
    hosts: Hosts,
}

impl SshManagement {
    /// Creates a management provider backed by the running SSH hosts state.
    pub(crate) fn new(hosts: Hosts) -> Self {
        Self { hosts }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddHostArgs {
    name: String,
    address: String,
    #[serde(default = "default_port")]
    port: u16,
    username: String,
    #[serde(default)]
    note: String,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateHostPatch {
    host_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    address: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CanonicalUpdateHost {
    host_id: String,
    name: String,
    address: String,
    port: u16,
    username: String,
    note: String,
}

fn map_ssh_error(error: SshError) -> ManagementError {
    match error {
        SshError::NameTaken | SshError::ScanOutdated => {
            ManagementError::conflict(error.to_string())
        }
        SshError::HostNotFound => ManagementError::not_found(error.to_string()),
        other => ManagementError::new(other.code(), other.to_string()),
    }
}

fn validate_host_fields(
    name: &str,
    address: &str,
    port: u16,
    username: &str,
    note: &str,
) -> Result<String, ManagementError> {
    if !is_host_name(name) {
        return Err(ManagementError::invalid_field(
            "name",
            format!(
                "host name must be 1 to {MAX_NAME_CHARS} ASCII letters, digits, '.', '_' or '-', starting with a letter or digit"
            ),
        ));
    }

    let Some(canonical_address) = stored_address(address) else {
        return Err(ManagementError::invalid_field(
            "address",
            "address must be a valid hostname, IPv4 address, or IPv6 address without scheme, user, or port",
        ));
    };

    if !is_port(port) {
        return Err(ManagementError::invalid_field(
            "port",
            "port must be between 1 and 65535",
        ));
    }

    if !is_username(username) {
        return Err(ManagementError::invalid_field(
            "username",
            format!(
                "username must be 1 to {MAX_USERNAME_CHARS} visible characters without whitespace or control characters"
            ),
        ));
    }

    if !is_note(note) {
        return Err(ManagementError::invalid_field(
            "note",
            format!(
                "note must be at most {MAX_NOTE_CHARS} characters without control characters other than line breaks"
            ),
        ));
    }

    Ok(canonical_address)
}

fn status_label(status: HostStatus) -> &'static str {
    match status {
        HostStatus::Ready => "ready",
        HostStatus::KeyNotChecked => "key_not_checked",
        HostStatus::KeyChanged => "key_changed",
        HostStatus::PasswordMissing => "password_missing",
    }
}

fn comparison_label(comparison: KeyComparison) -> &'static str {
    match comparison {
        KeyComparison::NotPinned => "not_pinned",
        KeyComparison::Same => "same",
        KeyComparison::Changed => "changed",
    }
}

fn host_id_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "hostId": {
                "type": "string",
                "description": "UUID of the registered SSH host."
            }
        },
        "required": ["hostId"],
        "additionalProperties": false
    })
}

fn add_host_action_spec() -> ManagementActionSpec {
    ManagementActionSpec {
        name: ACTION_ADD_HOST.to_owned(),
        description: "Register a new SSH host with its login password stored in the OS \
            credential store. After adding, run `scan_host_key` and `trust_host_key` before \
            running commands."
            .to_owned(),
        security_sensitive: true,
        parameters: json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Unique host name (1-64 ASCII letters, digits, '.', '_' or '-', starting with letter or digit)."
                },
                "address": {
                    "type": "string",
                    "description": "Hostname, IPv4, or IPv6 address (no scheme, username, or port)."
                },
                "port": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 65535,
                    "default": DEFAULT_PORT,
                    "description": "SSH server port (defaults to 22)."
                },
                "username": {
                    "type": "string",
                    "description": "Login username on the remote host."
                },
                "note": {
                    "type": "string",
                    "description": "Optional note describing the host for agents."
                }
            },
            "required": ["name", "address", "username"],
            "additionalProperties": false
        }),
        secret_fields: vec![SecretFieldSpec {
            key: SECRET_PASSWORD.to_owned(),
            label: "Password".to_owned(),
            description: "SSH login password stored in the OS credential store.".to_owned(),
            required: true,
        }],
    }
}

fn host_crud_actions() -> Vec<ManagementActionSpec> {
    vec![
        add_host_action_spec(),
        ManagementActionSpec {
            name: ACTION_UPDATE_HOST.to_owned(),
            description: "Update an existing SSH host's fields and optionally replace its stored \
                password. Omitted fields keep their current values; changing address or port \
                clears the pinned host key."
                .to_owned(),
            security_sensitive: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "hostId": {
                        "type": "string",
                        "description": "UUID of the SSH host to update."
                    },
                    "name": {
                        "type": "string",
                        "description": "New unique host name."
                    },
                    "address": {
                        "type": "string",
                        "description": "New hostname, IPv4, or IPv6 address."
                    },
                    "port": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 65535,
                        "description": "New SSH server port."
                    },
                    "username": {
                        "type": "string",
                        "description": "New login username."
                    },
                    "note": {
                        "type": "string",
                        "description": "Updated note about the host."
                    }
                },
                "required": ["hostId"],
                "additionalProperties": false
            }),
            secret_fields: vec![SecretFieldSpec {
                key: SECRET_PASSWORD.to_owned(),
                label: "New password (optional)".to_owned(),
                description: "Replacement SSH password; leave empty to keep the stored password."
                    .to_owned(),
                required: false,
            }],
        },
        ManagementActionSpec {
            name: ACTION_REMOVE_HOST.to_owned(),
            description: "Delete a registered SSH host and its stored password.".to_owned(),
            security_sensitive: true,
            parameters: host_id_schema(),
            secret_fields: Vec::new(),
        },
    ]
}

fn host_verification_actions() -> Vec<ManagementActionSpec> {
    vec![
        ManagementActionSpec {
            name: ACTION_SCAN_HOST_KEY.to_owned(),
            description: "Connect to a registered SSH host without sending a password and return \
                the host key algorithm, SHA-256 fingerprint, and comparison with any pinned key."
                .to_owned(),
            security_sensitive: false,
            parameters: host_id_schema(),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_TRUST_HOST_KEY.to_owned(),
            description: "Pin the SHA-256 host key fingerprint observed by the most recent \
                `scan_host_key` call for a host."
                .to_owned(),
            security_sensitive: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "hostId": {
                        "type": "string",
                        "description": "UUID of the SSH host."
                    },
                    "fingerprint": {
                        "type": "string",
                        "description": "Exact SHA256:... fingerprint reported by scan_host_key."
                    }
                },
                "required": ["hostId", "fingerprint"],
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_TEST_HOST.to_owned(),
            description: "Test SSH authentication to a registered host with a pinned key without \
                running any remote command."
                .to_owned(),
            security_sensitive: false,
            parameters: host_id_schema(),
            secret_fields: Vec::new(),
        },
    ]
}

fn guide_actions() -> Vec<ManagementActionSpec> {
    let mut actions = host_crud_actions();
    actions.extend(host_verification_actions());
    actions
}

fn prepare_add_host(
    hosts: &Hosts,
    request: PrepareActionRequest,
) -> Result<PreparedAction, ManagementError> {
    let provided_in_chat = request.has_provided_secret(SECRET_PASSWORD);
    let args: AddHostArgs = serde_json::from_value(request.arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;

    let address = validate_host_fields(
        &args.name,
        &args.address,
        args.port,
        &args.username,
        &args.note,
    )?;

    if hosts.by_name(&args.name).is_some() {
        return Err(ManagementError::conflict(format!(
            "an SSH host named {:?} already exists",
            args.name
        )));
    }

    let canonical = AddHostArgs {
        name: args.name,
        address,
        port: args.port,
        username: args.username,
        note: args.note,
    };

    let mut fields = vec![
        ConfigFieldPreview {
            label: "Name".to_owned(),
            value: canonical.name.clone(),
        },
        ConfigFieldPreview {
            label: "Endpoint".to_owned(),
            value: endpoint(&canonical.username, &canonical.address, canonical.port),
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
        title: format!("Add SSH host {}", canonical.name),
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

fn prepare_update_host(
    hosts: &Hosts,
    request: PrepareActionRequest,
) -> Result<PreparedAction, ManagementError> {
    let provided_in_chat = request.has_provided_secret(SECRET_PASSWORD);
    let patch: UpdateHostPatch = serde_json::from_value(request.arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;

    let host_ref = HostRef {
        host_id: patch.host_id.clone(),
    };
    if let Err(refusal) = host_ref.validate() {
        return Err(ManagementError::invalid_field(
            refusal.field,
            "hostId must be a valid UUID",
        ));
    }

    let current = hosts
        .by_id(&patch.host_id)
        .ok_or_else(|| ManagementError::not_found("the SSH host is not registered"))?;

    let name = patch.name.unwrap_or_else(|| current.name.clone());
    let raw_address = patch.address.unwrap_or_else(|| current.address.clone());
    let port = patch.port.unwrap_or(current.port);
    let username = patch.username.unwrap_or_else(|| current.username.clone());
    let note = patch.note.unwrap_or_else(|| current.note.clone());

    let address = validate_host_fields(&name, &raw_address, port, &username, &note)?;

    if let Some(other) = hosts.by_name(&name)
        && other.id != current.id
    {
        return Err(ManagementError::conflict(format!(
            "another SSH host is already named {name:?}"
        )));
    }

    let moved = current.address != address || current.port != port;
    let canonical = CanonicalUpdateHost {
        host_id: current.id,
        name,
        address,
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
            value: endpoint(&canonical.username, &canonical.address, canonical.port),
        },
    ];

    if !canonical.note.is_empty() {
        fields.push(ConfigFieldPreview {
            label: "Note".to_owned(),
            value: canonical.note.clone(),
        });
    }

    if moved {
        fields.push(ConfigFieldPreview {
            label: "Pinned host key".to_owned(),
            value: "Will be cleared because the address or port changed".to_owned(),
        });
    }

    let canonical_arguments = serde_json::to_value(&canonical)
        .map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title: format!("Update SSH host {}", canonical.name),
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

fn parse_host_ref(arguments: serde_json::Value) -> Result<HostRef, ManagementError> {
    let host_ref: HostRef = serde_json::from_value(arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;

    if let Err(refusal) = host_ref.validate() {
        return Err(ManagementError::invalid_field(
            refusal.field,
            "hostId must be a valid UUID",
        ));
    }

    Ok(host_ref)
}

fn prepare_host_action(
    hosts: &Hosts,
    action: &str,
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    let host_ref = parse_host_ref(arguments)?;
    let current = hosts
        .by_id(&host_ref.host_id)
        .ok_or_else(|| ManagementError::not_found("the SSH host is not registered"))?;

    let (title, security_sensitive) = match action {
        ACTION_REMOVE_HOST => (format!("Remove SSH host {}", current.name), true),
        ACTION_SCAN_HOST_KEY => (format!("Scan SSH host key for {}", current.name), false),
        ACTION_TEST_HOST => (format!("Test SSH connection to {}", current.name), false),
        _ => return Err(ManagementError::unknown_action(action)),
    };

    let fields = vec![
        ConfigFieldPreview {
            label: "Name".to_owned(),
            value: current.name.clone(),
        },
        ConfigFieldPreview {
            label: "Endpoint".to_owned(),
            value: current.endpoint(),
        },
        ConfigFieldPreview {
            label: "Status".to_owned(),
            value: status_label(current.status()).to_owned(),
        },
    ];

    let canonical_arguments = serde_json::to_value(&host_ref)
        .map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title,
        fields,
        secret_fields: Vec::new(),
        security_sensitive,
        canonical_arguments,
    })
}

fn prepare_trust_host_key(
    hosts: &Hosts,
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    let input: TrustHostKey = serde_json::from_value(arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;

    if let Err(refusal) = input.validate() {
        let message = format!("invalid {}", refusal.field);
        return Err(ManagementError::invalid_field(refusal.field, message));
    }

    let current = hosts
        .by_id(&input.host_id)
        .ok_or_else(|| ManagementError::not_found("the SSH host is not registered"))?;

    let seen_matches = current
        .seen
        .as_ref()
        .is_some_and(|seen| seen.fingerprint == input.fingerprint);

    if !seen_matches {
        return Err(ManagementError::conflict(
            "the fingerprint does not match the last scanned host key; run scan_host_key first",
        ));
    }

    let algorithm = current
        .seen
        .as_ref()
        .map_or_else(|| "unknown".to_owned(), |seen| seen.algorithm.clone());

    let fields = vec![
        ConfigFieldPreview {
            label: "Name".to_owned(),
            value: current.name.clone(),
        },
        ConfigFieldPreview {
            label: "Endpoint".to_owned(),
            value: current.endpoint(),
        },
        ConfigFieldPreview {
            label: "Algorithm".to_owned(),
            value: algorithm,
        },
        ConfigFieldPreview {
            label: "Fingerprint".to_owned(),
            value: input.fingerprint.clone(),
        },
    ];

    let canonical_arguments =
        serde_json::to_value(&input).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title: format!("Trust SSH host key for {}", current.name),
        fields,
        secret_fields: Vec::new(),
        security_sensitive: true,
        canonical_arguments,
    })
}

async fn execute_add_host(
    hosts: &Hosts,
    mut request: ExecuteActionRequest,
) -> Result<ActionResult, ManagementError> {
    let args: AddHostArgs = serde_json::from_value(request.arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;

    let password = request.secrets.remove(SECRET_PASSWORD).ok_or_else(|| {
        ManagementError::invalid_field(SECRET_PASSWORD, "a password is required to add an SSH host")
    })?;

    if !is_password(password.expose()) {
        return Err(ManagementError::invalid_field(
            SECRET_PASSWORD,
            "password must be 1 to 1024 bytes and contain no NUL bytes",
        ));
    }

    let view = hosts
        .add(AddHost {
            name: args.name,
            address: args.address,
            port: args.port,
            username: args.username,
            password,
            note: args.note,
        })
        .await
        .map_err(map_ssh_error)?;

    let message = format!(
        "Added SSH host {} ({}). Scan and trust its host key before running commands.",
        view.name, view.endpoint
    );
    let data =
        serde_json::to_value(&view).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(ActionResult::new(message).with_data(data))
}

async fn execute_update_host(
    hosts: &Hosts,
    mut request: ExecuteActionRequest,
) -> Result<ActionResult, ManagementError> {
    let args: CanonicalUpdateHost = serde_json::from_value(request.arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;

    let password = request.secrets.remove(SECRET_PASSWORD);

    if let Some(secret) = &password
        && !is_password(secret.expose())
    {
        return Err(ManagementError::invalid_field(
            SECRET_PASSWORD,
            "password must be 1 to 1024 bytes and contain no NUL bytes",
        ));
    }

    let view = hosts
        .update(UpdateHost {
            host_id: args.host_id,
            name: args.name,
            address: args.address,
            port: args.port,
            username: args.username,
            password,
            note: args.note,
        })
        .await
        .map_err(map_ssh_error)?;

    let message = format!(
        "Updated SSH host {} ({}, status: {}).",
        view.name,
        view.endpoint,
        status_label(view.status)
    );
    let data =
        serde_json::to_value(&view).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(ActionResult::new(message).with_data(data))
}

async fn execute_host_verification_action(
    hosts: &Hosts,
    action: &str,
    arguments: serde_json::Value,
) -> Result<ActionResult, ManagementError> {
    match action {
        ACTION_REMOVE_HOST => {
            let host_ref = parse_host_ref(arguments)?;
            hosts
                .remove(&host_ref.host_id)
                .await
                .map_err(map_ssh_error)?;

            Ok(
                ActionResult::new("Removed SSH host and deleted its stored password.")
                    .with_data(json!({ "hostId": host_ref.host_id })),
            )
        }
        ACTION_SCAN_HOST_KEY => {
            let host_ref = parse_host_ref(arguments)?;
            let scan = hosts.scan(&host_ref.host_id).await.map_err(map_ssh_error)?;

            let message = format!(
                "Scanned SSH host key: {} {} ({}).",
                scan.key.algorithm,
                scan.key.fingerprint,
                comparison_label(scan.comparison)
            );
            let data = serde_json::to_value(&scan)
                .map_err(|error| ManagementError::failed(error.to_string()))?;

            Ok(ActionResult::new(message).with_data(data))
        }
        ACTION_TRUST_HOST_KEY => {
            let input: TrustHostKey = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;

            let view = hosts
                .trust(&input.host_id, &input.fingerprint)
                .await
                .map_err(map_ssh_error)?;

            let message = format!(
                "Pinned SSH host key {} for {} (status: {}).",
                input.fingerprint,
                view.name,
                status_label(view.status)
            );
            let data = serde_json::to_value(&view)
                .map_err(|error| ManagementError::failed(error.to_string()))?;

            Ok(ActionResult::new(message).with_data(data))
        }
        ACTION_TEST_HOST => {
            let host_ref = parse_host_ref(arguments)?;
            hosts.test(&host_ref.host_id).await.map_err(map_ssh_error)?;

            Ok(ActionResult::new("SSH login test succeeded.")
                .with_data(json!({ "hostId": host_ref.host_id, "ok": true })))
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

impl ManagementProvider for SshManagement {
    fn guide(&self) -> ManagementGuide {
        ManagementGuide {
            summary: "Manages remote SSH hosts that do not run a Mannie worker, pins each host's \
                SSH public key by SHA-256 fingerprint, runs foreground and background commands \
                under the master's policy and journal, and offers hosts to the Terminal plugin."
                .to_owned(),
            storage: "Stores host metadata and pinned/scanned public keys in the `ssh_hosts` \
                SQLite table. Stores each host's login password only in the master's OS \
                credential store under the keyed family `ssh/host/<host-id>`; passwords never \
                enter SQLite, logs, or inspect output."
                .to_owned(),
            troubleshooting: "New hosts start in status `key_not_checked`: call `scan_host_key`, \
                confirm the SHA-256 fingerprint with the owner, and call `trust_host_key` before \
                using `test_host` or running commands. If a host reports `key_changed`, no \
                password is sent until the owner verifies and trusts the new key. Updating a \
                host's address or port clears its pinned key. Only password and \
                keyboard-interactive authentication are supported."
                .to_owned(),
            actions: guide_actions(),
        }
    }

    fn inspect(
        &self,
        query: Option<String>,
    ) -> BoxFuture<'static, Result<ManagementSnapshot, ManagementError>> {
        let hosts = self.hosts.clone();

        Box::pin(async move {
            let trimmed = query.as_deref().map(str::trim).filter(|q| !q.is_empty());

            if let Some(selector) = trimmed {
                let Some(key) = selector.strip_prefix("host:") else {
                    return Err(ManagementError::invalid_field(
                        "query",
                        format!(
                            "unsupported ssh inspect query {selector:?}; omit query or use \"host:<id-or-name>\""
                        ),
                    ));
                };

                let record = hosts
                    .by_id(key)
                    .or_else(|| hosts.by_name(key))
                    .ok_or_else(|| {
                        ManagementError::not_found(format!("no SSH host matching {key:?}"))
                    })?;

                let view = record.view();
                let summary = format!(
                    "SSH host {} ({}, status: {}).",
                    view.name,
                    view.endpoint,
                    status_label(view.status)
                );
                let data = serde_json::to_value(&view)
                    .map_err(|error| ManagementError::failed(error.to_string()))?;

                return Ok(ManagementSnapshot::new(summary, data));
            }

            let views: Vec<_> = hosts
                .records()
                .iter()
                .map(crate::store::HostRecord::view)
                .collect();
            let ready = views
                .iter()
                .filter(|view| view.status == HostStatus::Ready)
                .count();
            let summary = format!("{} registered SSH host(s) ({ready} ready).", views.len());
            let data = json!({
                "hosts": views,
                "count": views.len(),
                "readyCount": ready,
            });

            Ok(ManagementSnapshot::new(summary, data))
        })
    }

    fn prepare_action(
        &self,
        request: PrepareActionRequest,
    ) -> BoxFuture<'static, Result<PreparedAction, ManagementError>> {
        let hosts = self.hosts.clone();

        Box::pin(async move {
            match request.action.as_str() {
                ACTION_ADD_HOST => prepare_add_host(&hosts, request),
                ACTION_UPDATE_HOST => prepare_update_host(&hosts, request),
                ACTION_REMOVE_HOST | ACTION_SCAN_HOST_KEY | ACTION_TEST_HOST => {
                    prepare_host_action(&hosts, &request.action, request.arguments)
                }
                ACTION_TRUST_HOST_KEY => prepare_trust_host_key(&hosts, request.arguments),
                other => Err(ManagementError::unknown_action(other)),
            }
        })
    }

    fn execute_action(
        &self,
        request: ExecuteActionRequest,
    ) -> BoxFuture<'static, Result<ActionResult, ManagementError>> {
        let hosts = self.hosts.clone();

        Box::pin(async move {
            match request.action.as_str() {
                ACTION_ADD_HOST => execute_add_host(&hosts, request).await,
                ACTION_UPDATE_HOST => execute_update_host(&hosts, request).await,
                ACTION_REMOVE_HOST
                | ACTION_SCAN_HOST_KEY
                | ACTION_TRUST_HOST_KEY
                | ACTION_TEST_HOST => {
                    execute_host_verification_action(&hosts, &request.action, request.arguments)
                        .await
                }
                other => Err(ManagementError::unknown_action(other)),
            }
        })
    }
}
