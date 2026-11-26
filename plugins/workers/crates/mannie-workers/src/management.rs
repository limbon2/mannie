//! Self-management provider for the Workers plugin (ADR-0081).
//!
//! Exposes paired remote workers, connection statuses, host capabilities, and
//! typed actions for pairing, renaming, and removing workers under the master's
//! always-ask `Access::AppConfig` policy.

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{ConfigFieldPreview, ConfigSecretField, WorkerId};
use mannie_plugin_sdk::ipc::PluginError as _;
use mannie_plugin_sdk::points::management::{
    ActionResult, ExecuteActionRequest, ManagementActionSpec, ManagementError, ManagementGuide,
    ManagementProvider, ManagementSnapshot, PrepareActionRequest, PreparedAction, SecretFieldSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::commands::{
    MAX_WORKER_NAME_CHARACTERS, PairWorker, RenameWorker, WorkerRef, WorkersError,
    check_worker_name,
};
use crate::hub::{DEFAULT_PORT, Hub, normalize_address};
use crate::view::WorkerStatus;

/// Plugin ID for the Workers management provider.
pub(crate) const WORKERS: &str = "workers";

const ACTION_PAIR_WORKER: &str = "pair_worker";
const ACTION_RENAME_WORKER: &str = "rename_worker";
const ACTION_REMOVE_WORKER: &str = "remove_worker";
const SECRET_CODE: &str = "code";

/// Management provider for the Workers plugin.
#[derive(Debug, Clone)]
pub(crate) struct WorkersManagement {
    hub: Hub,
}

impl WorkersManagement {
    /// Creates a management provider backed by the running workers hub.
    pub(crate) fn new(hub: Hub) -> Self {
        Self { hub }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PairWorkerArgs {
    address: String,
    #[serde(default)]
    name: Option<String>,
}

fn map_workers_error(error: WorkersError) -> ManagementError {
    match error {
        WorkersError::WorkerNotFound => ManagementError::not_found(error.to_string()),
        WorkersError::AlreadyPaired | WorkersError::AlreadyAdded | WorkersError::NameTaken => {
            ManagementError::conflict(error.to_string())
        }
        WorkersError::AddressInvalid | WorkersError::NameInvalid { .. } => {
            ManagementError::invalid_arguments(error.to_string())
        }
        other => ManagementError::new(other.code(), other.to_string()),
    }
}

fn guide_actions() -> Vec<ManagementActionSpec> {
    vec![
        ManagementActionSpec {
            name: ACTION_PAIR_WORKER.to_owned(),
            description: "Pair Mannie with a remote `mannie-worker` service using its address \
                and the one-time pairing code printed by `mannie-worker pair`."
                .to_owned(),
            security_sensitive: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "address": {
                        "type": "string",
                        "description": format!(
                            "Hostname or IP address with optional :port (defaults to {DEFAULT_PORT})."
                        )
                    },
                    "name": {
                        "type": ["string", "null"],
                        "maxLength": MAX_WORKER_NAME_CHARACTERS,
                        "description": "Optional display name in Mannie (defaults to the worker's configured name)."
                    }
                },
                "required": ["address"],
                "additionalProperties": false
            }),
            secret_fields: vec![SecretFieldSpec {
                key: SECRET_CODE.to_owned(),
                label: "Pairing code".to_owned(),
                description: "One-time pairing code printed by `mannie-worker pair` on the host."
                    .to_owned(),
                required: true,
            }],
        },
        ManagementActionSpec {
            name: ACTION_RENAME_WORKER.to_owned(),
            description: "Rename a paired worker in Mannie, or pass an empty name to revert to \
                the worker's configured name."
                .to_owned(),
            security_sensitive: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "workerId": {
                        "type": "string",
                        "description": "ID of the paired worker."
                    },
                    "name": {
                        "type": "string",
                        "maxLength": MAX_WORKER_NAME_CHARACTERS,
                        "description": "New display name, or empty string to use the configured name."
                    }
                },
                "required": ["workerId", "name"],
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_REMOVE_WORKER.to_owned(),
            description: "Unpair (when reachable) and remove a paired worker from Mannie."
                .to_owned(),
            security_sensitive: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "workerId": {
                        "type": "string",
                        "description": "ID of the paired worker to remove."
                    }
                },
                "required": ["workerId"],
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
    ]
}

fn find_worker(
    views: &[crate::view::WorkerView],
    id_or_name: &str,
) -> Option<crate::view::WorkerView> {
    views
        .iter()
        .find(|worker| {
            worker.id.as_str() == id_or_name || worker.name.eq_ignore_ascii_case(id_or_name)
        })
        .cloned()
}

fn find_worker_by_id(
    views: &[crate::view::WorkerView],
    worker_id: &WorkerId,
) -> Option<crate::view::WorkerView> {
    views.iter().find(|worker| worker.id == *worker_id).cloned()
}

fn prepare_pair_worker(
    views: &[crate::view::WorkerView],
    request: PrepareActionRequest,
) -> Result<PreparedAction, ManagementError> {
    let provided_in_chat = request.has_provided_secret(SECRET_CODE);
    let args: PairWorkerArgs = serde_json::from_value(request.arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;

    let normalized = normalize_address(&args.address).map_err(|_err| {
        ManagementError::invalid_field(
            "address",
            "address must be a valid hostname or IP address with an optional :port",
        )
    })?;

    let cleaned_name = match args.name.as_deref() {
        Some(raw) => check_worker_name(raw).map_err(map_workers_error)?,
        None => None,
    };

    if let Some(name) = &cleaned_name
        && views.iter().any(|w| w.name.eq_ignore_ascii_case(name))
    {
        return Err(ManagementError::conflict(format!(
            "another worker is already named {name:?}"
        )));
    }

    let canonical = PairWorkerArgs {
        address: normalized.clone(),
        name: cleaned_name.clone(),
    };

    let mut fields = vec![ConfigFieldPreview {
        label: "Address".to_owned(),
        value: normalized.clone(),
    }];
    if let Some(name) = cleaned_name {
        fields.push(ConfigFieldPreview {
            label: "Display name".to_owned(),
            value: name,
        });
    }

    let canonical_arguments = serde_json::to_value(&canonical)
        .map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title: format!("Pair worker at {normalized}"),
        fields,
        secret_fields: vec![ConfigSecretField {
            key: SECRET_CODE.to_owned(),
            label: "Pairing code".to_owned(),
            required: true,
            provided_in_chat,
        }],
        security_sensitive: true,
        canonical_arguments,
    })
}

fn prepare_rename_worker(
    views: &[crate::view::WorkerView],
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    let input: RenameWorker = serde_json::from_value(arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    let worker = find_worker_by_id(views, &input.worker_id)
        .ok_or_else(|| ManagementError::not_found("the worker is not paired"))?;
    let checked = check_worker_name(&input.name).map_err(map_workers_error)?;
    let display_next =
        checked.unwrap_or_else(|| format!("{} (configured default)", worker.configured_name));

    let canonical_arguments =
        serde_json::to_value(&input).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title: format!("Rename worker {}", worker.name),
        fields: vec![
            ConfigFieldPreview {
                label: "Worker".to_owned(),
                value: worker.name,
            },
            ConfigFieldPreview {
                label: "New name".to_owned(),
                value: display_next,
            },
        ],
        secret_fields: Vec::new(),
        security_sensitive: false,
        canonical_arguments,
    })
}

fn prepare_remove_worker(
    views: &[crate::view::WorkerView],
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    let input: WorkerRef = serde_json::from_value(arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    let worker = find_worker_by_id(views, &input.worker_id)
        .ok_or_else(|| ManagementError::not_found("the worker is not paired"))?;

    let canonical_arguments =
        serde_json::to_value(&input).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title: format!("Remove worker {}", worker.name),
        fields: vec![
            ConfigFieldPreview {
                label: "Worker".to_owned(),
                value: worker.name,
            },
            ConfigFieldPreview {
                label: "Address".to_owned(),
                value: worker.address,
            },
            ConfigFieldPreview {
                label: "Status".to_owned(),
                value: worker.status.as_str().to_owned(),
            },
        ],
        secret_fields: Vec::new(),
        security_sensitive: true,
        canonical_arguments,
    })
}

impl ManagementProvider for WorkersManagement {
    fn guide(&self) -> ManagementGuide {
        ManagementGuide {
            summary: "Connects Mannie to headless `mannie-worker` services on Windows, Linux, and \
                FreeBSD over mutually authenticated TLS (mTLS), allowing agents to delegate tasks \
                that run under each worker host's policy ceiling."
                .to_owned(),
            storage: "Stores paired worker records and pinned worker public keys in SQLite under \
                `workers_*` tables. Stores the master's mTLS private key only in the master's OS \
                credential store under `workers/link/master-key`. Provider credentials never \
                leave the master."
                .to_owned(),
            troubleshooting: "Workers own their hosts and enforce their own policy ceiling. If a \
                worker shows `offline`, verify the `mannie-worker` service is running and \
                reachable on its port (default 7445). If status is `key_changed` or `unpaired`, \
                remove the worker and pair it again with a fresh code from `mannie-worker pair`. \
                If `incompatible`, update `mannie-worker` to match the desktop app version."
                .to_owned(),
            actions: guide_actions(),
        }
    }

    fn inspect(
        &self,
        query: Option<String>,
    ) -> BoxFuture<'static, Result<ManagementSnapshot, ManagementError>> {
        let hub = self.hub.clone();

        Box::pin(async move {
            let views = hub.worker_views().borrow().clone();
            let trimmed = query.as_deref().map(str::trim).filter(|q| !q.is_empty());

            if let Some(selector) = trimmed {
                let Some(key) = selector.strip_prefix("worker:") else {
                    return Err(ManagementError::invalid_field(
                        "query",
                        format!(
                            "unsupported workers inspect query {selector:?}; omit query or use \"worker:<id-or-name>\""
                        ),
                    ));
                };

                let worker = find_worker(&views, key).ok_or_else(|| {
                    ManagementError::not_found(format!("no paired worker matching {key:?}"))
                })?;
                let summary = format!(
                    "Worker {} ({}, status: {}).",
                    worker.name,
                    worker.address,
                    worker.status.as_str()
                );
                let data = serde_json::to_value(&worker)
                    .map_err(|error| ManagementError::failed(error.to_string()))?;

                return Ok(ManagementSnapshot::new(summary, data));
            }

            let connected = views
                .iter()
                .filter(|view| view.status == WorkerStatus::Connected)
                .count();
            let summary = format!("{} paired worker(s) ({connected} connected).", views.len());
            let data = json!({
                "workers": views,
                "count": views.len(),
                "connectedCount": connected,
            });

            Ok(ManagementSnapshot::new(summary, data))
        })
    }

    fn prepare_action(
        &self,
        request: PrepareActionRequest,
    ) -> BoxFuture<'static, Result<PreparedAction, ManagementError>> {
        let hub = self.hub.clone();

        Box::pin(async move {
            let views = hub.worker_views().borrow().clone();

            match request.action.as_str() {
                ACTION_PAIR_WORKER => prepare_pair_worker(&views, request),
                ACTION_RENAME_WORKER => prepare_rename_worker(&views, request.arguments),
                ACTION_REMOVE_WORKER => prepare_remove_worker(&views, request.arguments),
                other => Err(ManagementError::unknown_action(other)),
            }
        })
    }

    fn execute_action(
        &self,
        mut request: ExecuteActionRequest,
    ) -> BoxFuture<'static, Result<ActionResult, ManagementError>> {
        let hub = self.hub.clone();

        Box::pin(async move {
            match request.action.as_str() {
                ACTION_PAIR_WORKER => {
                    let args: PairWorkerArgs = serde_json::from_value(request.arguments)
                        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
                    let code = request.secrets.remove(SECRET_CODE).ok_or_else(|| {
                        ManagementError::invalid_field(
                            SECRET_CODE,
                            "a pairing code is required to pair a worker",
                        )
                    })?;
                    let code_text = code.expose().trim().to_owned();
                    if code_text.is_empty() {
                        return Err(ManagementError::invalid_field(
                            SECRET_CODE,
                            "pairing code cannot be empty",
                        ));
                    }

                    let view = hub
                        .pair(PairWorker {
                            address: args.address,
                            code: code_text,
                            name: args.name,
                        })
                        .await
                        .map_err(map_workers_error)?;
                    let message = format!("Paired worker {} ({}).", view.name, view.address);
                    let data = serde_json::to_value(&view)
                        .map_err(|error| ManagementError::failed(error.to_string()))?;

                    Ok(ActionResult::new(message).with_data(data))
                }
                ACTION_RENAME_WORKER => {
                    let input: RenameWorker = serde_json::from_value(request.arguments)
                        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
                    let view = hub
                        .rename(&input.worker_id, &input.name)
                        .await
                        .map_err(map_workers_error)?;
                    let data = serde_json::to_value(&view)
                        .map_err(|error| ManagementError::failed(error.to_string()))?;

                    Ok(
                        ActionResult::new(format!("Renamed worker to {}.", view.name))
                            .with_data(data),
                    )
                }
                ACTION_REMOVE_WORKER => {
                    let input: WorkerRef = serde_json::from_value(request.arguments)
                        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
                    hub.remove(&input.worker_id)
                        .await
                        .map_err(map_workers_error)?;

                    Ok(ActionResult::new("Removed paired worker.")
                        .with_data(json!({ "workerId": input.worker_id })))
                }
                other => Err(ManagementError::unknown_action(other)),
            }
        })
    }
}
