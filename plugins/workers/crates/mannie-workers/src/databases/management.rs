//! Self-management provider for the optional `workers.databases` feature
//! (ADR-0055, ADR-0056, ADR-0057, ADR-0081).
//!
//! Exposes registered SQLite databases on this computer and on paired workers,
//! their read-only flags and file states, and typed actions for registering
//! worker databases, renaming, marking read-only, accepting replaced files,
//! removing registrations, and refreshing state under the master's always-ask
//! `Access::AppConfig` policy.

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::ConfigFieldPreview;
use mannie_plugin_sdk::ipc::{PluginError as _, Validate as _};
use mannie_plugin_sdk::points::management::{
    ActionResult, ExecuteActionRequest, ManagementActionSpec, ManagementError, ManagementGuide,
    ManagementProvider, ManagementSnapshot, PrepareActionRequest, PreparedAction,
};
use mannie_plugin_sdk::services::sqlite_files::{MAX_NAME_CHARS, SqliteFilesError};
use serde_json::json;

use super::commands::{
    AddWorkerDatabase, DatabaseMachine, DatabaseRef, DatabasesError, MAX_PATH_CHARS, MarkDatabase,
    RenameDatabase,
};
use super::registry::Databases;

/// Management provider ID for the optional Databases feature of Workers.
pub(crate) const WORKERS_DATABASES: &str = "workers.databases";

const ACTION_ADD_WORKER_DATABASE: &str = "add_worker_database";
const ACTION_RENAME_DATABASE: &str = "rename_database";
const ACTION_MARK_DATABASE: &str = "mark_database";
const ACTION_ACCEPT_DATABASE: &str = "accept_database";
const ACTION_REMOVE_DATABASE: &str = "remove_database";
const ACTION_REFRESH_DATABASES: &str = "refresh_databases";

/// Management provider for the `workers.databases` feature.
#[derive(Debug, Clone)]
pub(crate) struct DatabasesManagement {
    databases: Databases,
}

impl DatabasesManagement {
    /// Creates a management provider backed by the running databases registry.
    pub(crate) fn new(databases: Databases) -> Self {
        Self { databases }
    }
}

fn map_databases_error(error: &DatabasesError) -> ManagementError {
    match error {
        DatabasesError::NameTaken
        | DatabasesError::Files {
            error: SqliteFilesError::AlreadyRegistered { .. },
        } => ManagementError::conflict(error.to_string()),
        DatabasesError::Files {
            error: SqliteFilesError::NotRegistered,
        } => ManagementError::not_found(error.to_string()),
        DatabasesError::Files {
            error: SqliteFilesError::InvalidName,
        } => ManagementError::invalid_field("name", error.to_string()),
        _ => ManagementError::new(error.code(), error.to_string()),
    }
}

fn machine_label(machine: &DatabaseMachine) -> String {
    match machine {
        DatabaseMachine::ThisComputer => "This computer".to_owned(),
        DatabaseMachine::Worker { name, .. } => format!("Worker {name}"),
    }
}

fn database_key_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "key": {
                "type": "string",
                "description": "Database key from inspect."
            }
        },
        "required": ["key"],
        "additionalProperties": false
    })
}

fn add_worker_database_spec() -> ManagementActionSpec {
    ManagementActionSpec {
        name: ACTION_ADD_WORKER_DATABASE.to_owned(),
        description: "Register an existing SQLite file at an absolute path on a paired \
            worker's host (subject to the worker's host ceiling). Local files on this \
            computer must be chosen through the OS file dialog in the Databases view."
            .to_owned(),
        security_sensitive: true,
        parameters: json!({
            "type": "object",
            "properties": {
                "worker": {
                    "type": "string",
                    "description": "Worker ID of the paired worker."
                },
                "path": {
                    "type": "string",
                    "maxLength": MAX_PATH_CHARS,
                    "description": "Absolute path to the SQLite file on the worker's host."
                },
                "name": {
                    "type": ["string", "null"],
                    "maxLength": MAX_NAME_CHARS,
                    "description": "Optional unique display name (defaults to file name)."
                },
                "readOnly": {
                    "type": "boolean",
                    "description": "Whether to mark the database read-only."
                }
            },
            "required": ["worker", "path", "readOnly"],
            "additionalProperties": false
        }),
        secret_fields: Vec::new(),
    }
}

fn guide_actions() -> Vec<ManagementActionSpec> {
    vec![
        add_worker_database_spec(),
        ManagementActionSpec {
            name: ACTION_RENAME_DATABASE.to_owned(),
            description: "Rename a registered SQLite database.".to_owned(),
            security_sensitive: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string" },
                    "name": {
                        "type": "string",
                        "maxLength": MAX_NAME_CHARS
                    }
                },
                "required": ["key", "name"],
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_MARK_DATABASE.to_owned(),
            description: "Mark a registered database read-only or allow writes (subject to \
                approvals and host ceilings)."
                .to_owned(),
            security_sensitive: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string" },
                    "readOnly": { "type": "boolean" }
                },
                "required": ["key", "readOnly"],
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_ACCEPT_DATABASE.to_owned(),
            description: "Accept the current file at a registered database's path after it was \
                replaced on disk (resolves `changed` state)."
                .to_owned(),
            security_sensitive: true,
            parameters: database_key_schema(),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_REMOVE_DATABASE.to_owned(),
            description: "Unregister a SQLite database from Mannie (leaves the file on disk \
                untouched)."
                .to_owned(),
            security_sensitive: true,
            parameters: database_key_schema(),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_REFRESH_DATABASES.to_owned(),
            description: "Re-scan registered SQLite files on this computer and reachable workers."
                .to_owned(),
            security_sensitive: false,
            parameters: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
    ]
}

fn prepare_add_worker_database(
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    let input: AddWorkerDatabase = serde_json::from_value(arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    if let Err(refusal) = input.validate() {
        let message = format!("invalid value for {}", refusal.field);
        return Err(ManagementError::invalid_field(refusal.field, message));
    }

    let mut fields = vec![
        ConfigFieldPreview {
            label: "Worker".to_owned(),
            value: input.worker.as_str().to_owned(),
        },
        ConfigFieldPreview {
            label: "Path".to_owned(),
            value: input.path.clone(),
        },
        ConfigFieldPreview {
            label: "Read-only".to_owned(),
            value: if input.read_only { "Yes" } else { "No" }.to_owned(),
        },
    ];
    if let Some(name) = &input.name {
        fields.push(ConfigFieldPreview {
            label: "Name".to_owned(),
            value: name.clone(),
        });
    }

    let canonical_arguments =
        serde_json::to_value(&input).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title: "Register SQLite database on worker".to_owned(),
        fields,
        secret_fields: Vec::new(),
        security_sensitive: true,
        canonical_arguments,
    })
}

fn prepare_rename_database(
    databases: &Databases,
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    let input: RenameDatabase = serde_json::from_value(arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    let existing = databases
        .by_key(&input.key)
        .ok_or_else(|| ManagementError::not_found("the database is not registered"))?;
    let trimmed = input.name.trim();
    let name_ok = !trimmed.is_empty() && trimmed.chars().count() <= MAX_NAME_CHARS;
    if !name_ok {
        return Err(ManagementError::invalid_field(
            "name",
            format!("name must be 1 to {MAX_NAME_CHARS} characters"),
        ));
    }

    let canonical_arguments =
        serde_json::to_value(&input).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title: format!("Rename database {}", existing.name),
        fields: vec![
            ConfigFieldPreview {
                label: "Current name".to_owned(),
                value: existing.name,
            },
            ConfigFieldPreview {
                label: "New name".to_owned(),
                value: trimmed.to_owned(),
            },
            ConfigFieldPreview {
                label: "Machine".to_owned(),
                value: machine_label(&existing.machine),
            },
        ],
        secret_fields: Vec::new(),
        security_sensitive: false,
        canonical_arguments,
    })
}

fn prepare_mark_database(
    databases: &Databases,
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    let input: MarkDatabase = serde_json::from_value(arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    let existing = databases
        .by_key(&input.key)
        .ok_or_else(|| ManagementError::not_found("the database is not registered"))?;
    let mode = if input.read_only {
        "Read-only"
    } else {
        "Read and write"
    };

    let canonical_arguments =
        serde_json::to_value(&input).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title: format!("Mark database {} as {mode}", existing.name),
        fields: vec![
            ConfigFieldPreview {
                label: "Database".to_owned(),
                value: existing.name,
            },
            ConfigFieldPreview {
                label: "Machine".to_owned(),
                value: machine_label(&existing.machine),
            },
            ConfigFieldPreview {
                label: "Access".to_owned(),
                value: mode.to_owned(),
            },
        ],
        secret_fields: Vec::new(),
        security_sensitive: true,
        canonical_arguments,
    })
}

fn prepare_database_ref_action(
    databases: &Databases,
    action: &str,
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    let input: DatabaseRef = serde_json::from_value(arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    let existing = databases
        .by_key(&input.key)
        .ok_or_else(|| ManagementError::not_found("the database is not registered"))?;
    let title = if action == ACTION_ACCEPT_DATABASE {
        format!("Accept replaced file for database {}", existing.name)
    } else {
        format!("Remove database registration {}", existing.name)
    };

    let canonical_arguments =
        serde_json::to_value(&input).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title,
        fields: vec![
            ConfigFieldPreview {
                label: "Database".to_owned(),
                value: existing.name,
            },
            ConfigFieldPreview {
                label: "File".to_owned(),
                value: existing.file_name,
            },
            ConfigFieldPreview {
                label: "Machine".to_owned(),
                value: machine_label(&existing.machine),
            },
        ],
        secret_fields: Vec::new(),
        security_sensitive: true,
        canonical_arguments,
    })
}

fn prepare_refresh_databases(
    arguments: &serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    let is_empty_obj = arguments.as_object().is_some_and(serde_json::Map::is_empty);
    if !is_empty_obj {
        return Err(ManagementError::invalid_arguments(
            "refresh_databases takes no arguments",
        ));
    }

    Ok(PreparedAction {
        title: "Refresh registered SQLite databases".to_owned(),
        fields: Vec::new(),
        secret_fields: Vec::new(),
        security_sensitive: false,
        canonical_arguments: json!({}),
    })
}

impl ManagementProvider for DatabasesManagement {
    fn guide(&self) -> ManagementGuide {
        ManagementGuide {
            summary: "Optional feature of Workers that registers SQLite files on this computer \
                and on paired workers, presents an Adminer-style database view, and offers \
                `describe_database`, `query_database`, and `change_database` under policy and \
                host ceilings."
                .to_owned(),
            storage: "Local SQLite file registrations are kept by the core's `sqlite-files` \
                service; cached lists of paired workers' databases are kept in Workers' SQLite \
                tables so offline workers still display their last known databases. No \
                credentials are stored by this feature."
                .to_owned(),
            troubleshooting: "If a database reports `changed`, the underlying file was replaced \
                on disk; review and call `accept_database` before querying it again. If a worker \
                database shows `not_allowed` or `worker_needs_update`, check the worker host's \
                configuration and version. Local databases on this computer are registered only \
                through the OS file picker in the Databases view so the core never accepts \
                arbitrary local paths from chat."
                .to_owned(),
            actions: guide_actions(),
        }
    }

    fn inspect(
        &self,
        query: Option<String>,
    ) -> BoxFuture<'static, Result<ManagementSnapshot, ManagementError>> {
        let databases = self.databases.clone();

        Box::pin(async move {
            let trimmed = query.as_deref().map(str::trim).filter(|q| !q.is_empty());

            if let Some(selector) = trimmed {
                let Some(name_or_key) = selector.strip_prefix("database:") else {
                    return Err(ManagementError::invalid_field(
                        "query",
                        format!(
                            "unsupported workers.databases inspect query {selector:?}; omit query or use \"database:<name>\""
                        ),
                    ));
                };

                let found = databases.by_name(name_or_key).or_else(|| {
                    databases
                        .views()
                        .into_iter()
                        .find(|db| db.key.as_str() == name_or_key)
                });
                let view = found.ok_or_else(|| {
                    ManagementError::not_found(format!(
                        "no registered database matching {name_or_key:?}"
                    ))
                })?;

                let summary = format!(
                    "Database {} ({}, read-only: {}).",
                    view.name,
                    machine_label(&view.machine),
                    view.read_only
                );
                let data = serde_json::to_value(&view)
                    .map_err(|error| ManagementError::failed(error.to_string()))?;

                return Ok(ManagementSnapshot::new(summary, data));
            }

            let views = databases.views();
            let summary = format!("{} registered SQLite database(s).", views.len());
            let data = json!({
                "databases": views,
                "count": views.len(),
            });

            Ok(ManagementSnapshot::new(summary, data))
        })
    }

    fn prepare_action(
        &self,
        request: PrepareActionRequest,
    ) -> BoxFuture<'static, Result<PreparedAction, ManagementError>> {
        let databases = self.databases.clone();

        Box::pin(async move {
            match request.action.as_str() {
                ACTION_ADD_WORKER_DATABASE => prepare_add_worker_database(request.arguments),
                ACTION_RENAME_DATABASE => prepare_rename_database(&databases, request.arguments),
                ACTION_MARK_DATABASE => prepare_mark_database(&databases, request.arguments),
                ACTION_ACCEPT_DATABASE | ACTION_REMOVE_DATABASE => {
                    prepare_database_ref_action(&databases, &request.action, request.arguments)
                }
                ACTION_REFRESH_DATABASES => prepare_refresh_databases(&request.arguments),
                other => Err(ManagementError::unknown_action(other)),
            }
        })
    }

    fn execute_action(
        &self,
        request: ExecuteActionRequest,
    ) -> BoxFuture<'static, Result<ActionResult, ManagementError>> {
        let databases = self.databases.clone();

        Box::pin(async move {
            match request.action.as_str() {
                ACTION_ADD_WORKER_DATABASE => {
                    let input: AddWorkerDatabase = serde_json::from_value(request.arguments)
                        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
                    let view = databases
                        .add_on_worker(input)
                        .await
                        .map_err(|error| map_databases_error(&error))?;
                    let message = format!(
                        "Registered database {} ({}).",
                        view.name,
                        machine_label(&view.machine)
                    );
                    let data = serde_json::to_value(&view)
                        .map_err(|error| ManagementError::failed(error.to_string()))?;

                    Ok(ActionResult::new(message).with_data(data))
                }
                ACTION_RENAME_DATABASE => {
                    let input: RenameDatabase = serde_json::from_value(request.arguments)
                        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
                    databases
                        .rename(input.key.clone(), &input.name)
                        .await
                        .map_err(|error| map_databases_error(&error))?;

                    Ok(
                        ActionResult::new(format!("Renamed database to {}.", input.name))
                            .with_data(json!({ "key": input.key, "name": input.name })),
                    )
                }
                ACTION_MARK_DATABASE => {
                    let input: MarkDatabase = serde_json::from_value(request.arguments)
                        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
                    databases
                        .mark(input.key.clone(), input.read_only)
                        .await
                        .map_err(|error| map_databases_error(&error))?;
                    let mode = if input.read_only {
                        "read-only"
                    } else {
                        "read-write"
                    };

                    Ok(ActionResult::new(format!("Marked database as {mode}."))
                        .with_data(json!({ "key": input.key, "readOnly": input.read_only })))
                }
                ACTION_ACCEPT_DATABASE => {
                    let input: DatabaseRef = serde_json::from_value(request.arguments)
                        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
                    databases
                        .accept(input.key.clone())
                        .await
                        .map_err(|error| map_databases_error(&error))?;

                    Ok(ActionResult::new("Accepted replaced database file.")
                        .with_data(json!({ "key": input.key })))
                }
                ACTION_REMOVE_DATABASE => {
                    let input: DatabaseRef = serde_json::from_value(request.arguments)
                        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
                    databases
                        .remove(input.key.clone())
                        .await
                        .map_err(|error| map_databases_error(&error))?;

                    Ok(ActionResult::new("Removed database registration.")
                        .with_data(json!({ "key": input.key })))
                }
                ACTION_REFRESH_DATABASES => {
                    databases
                        .refresh()
                        .await
                        .map_err(|error| map_databases_error(&error))?;
                    let count = databases.views().len();

                    Ok(ActionResult::new(format!(
                        "Refreshed {count} registered SQLite database(s)."
                    ))
                    .with_data(json!({ "count": count })))
                }
                other => Err(ManagementError::unknown_action(other)),
            }
        })
    }
}
