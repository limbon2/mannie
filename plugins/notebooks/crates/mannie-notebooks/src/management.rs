//! Self-management provider for the Notebooks plugin (ADR-0081).
//!
//! Exposes notebooks, document source statuses and indexing errors, search
//! profile preferences, and typed actions for creating/renaming/deleting
//! notebooks, adding text sources, managing sources, and saving search
//! preferences under the master's always-ask `Access::AppConfig` policy.

use std::sync::Arc;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::ConfigFieldPreview;
use mannie_plugin_sdk::ipc::PluginError as _;
use mannie_plugin_sdk::points::management::{
    ActionResult, ExecuteActionRequest, ManagementActionSpec, ManagementError, ManagementGuide,
    ManagementProvider, ManagementSnapshot, PrepareActionRequest, PreparedAction,
};
use serde::Deserialize;
use serde_json::json;

use crate::commands::{
    AddTextSource, CreateNotebook, DEFAULT_PASSAGES, MAX_NAME_CHARACTERS, MAX_TITLE_CHARACTERS,
    NotebookRef, NotebooksError, PASSAGE_RANGE, Preferences, RenameNotebook, SetSourceIncluded,
    SourceRef,
};
use crate::library::Library;

const ACTION_CREATE_NOTEBOOK: &str = "create_notebook";
const ACTION_RENAME_NOTEBOOK: &str = "rename_notebook";
const ACTION_DELETE_NOTEBOOK: &str = "delete_notebook";
const ACTION_ADD_TEXT_SOURCE: &str = "add_text_source";
const ACTION_REMOVE_SOURCE: &str = "remove_source";
const ACTION_RETRY_SOURCE: &str = "retry_source";
const ACTION_SET_SOURCE_INCLUDED: &str = "set_source_included";
const ACTION_SAVE_PREFERENCES: &str = "save_preferences";

/// Management provider for the Notebooks plugin.
#[derive(Debug, Clone)]
pub(crate) struct NotebooksManagement {
    library: Arc<Library>,
}

impl NotebooksManagement {
    /// Creates a management provider backed by the running notebooks library.
    pub(crate) fn new(library: Arc<Library>) -> Self {
        Self { library }
    }
}

#[derive(Debug, Default)]
enum OptionalId {
    #[default]
    Unchanged,
    Clear,
    Set(String),
}

impl OptionalId {
    fn is_changed(&self) -> bool {
        !matches!(self, Self::Unchanged)
    }

    fn apply_to(self, target: &mut Option<String>) {
        match self {
            Self::Unchanged => {}
            Self::Clear => *target = None,
            Self::Set(value) => *target = Some(value),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreferencesPatch {
    #[serde(default, deserialize_with = "deserialize_optional_id")]
    embedding_profile_id: OptionalId,
    #[serde(default, deserialize_with = "deserialize_optional_id")]
    rerank_profile_id: OptionalId,
    #[serde(default)]
    max_passages: Option<u32>,
}

fn deserialize_optional_id<'de, D>(deserializer: D) -> Result<OptionalId, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match Option::<String>::deserialize(deserializer)? {
        Some(value) => OptionalId::Set(value),
        None => OptionalId::Clear,
    })
}

fn map_notebooks_error(error: NotebooksError) -> ManagementError {
    match error {
        NotebooksError::NotebookNotFound
        | NotebooksError::SourceNotFound
        | NotebooksError::ProfileNotFound
        | NotebooksError::PassageNotFound
        | NotebooksError::UploadNotFound => ManagementError::not_found(error.to_string()),
        NotebooksError::NameTaken | NotebooksError::NothingToRetry => {
            ManagementError::conflict(error.to_string())
        }
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

fn notebook_actions() -> Vec<ManagementActionSpec> {
    vec![
        ManagementActionSpec {
            name: ACTION_CREATE_NOTEBOOK.to_owned(),
            description: "Create a new notebook.".to_owned(),
            security_sensitive: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "maxLength": MAX_NAME_CHARACTERS,
                        "description": "Unique notebook display name."
                    }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_RENAME_NOTEBOOK.to_owned(),
            description: "Rename an existing notebook.".to_owned(),
            security_sensitive: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "notebookId": { "type": "string" },
                    "name": { "type": "string", "maxLength": MAX_NAME_CHARACTERS }
                },
                "required": ["notebookId", "name"],
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_DELETE_NOTEBOOK.to_owned(),
            description: "Delete a notebook and all of its indexed sources.".to_owned(),
            security_sensitive: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "notebookId": { "type": "string" }
                },
                "required": ["notebookId"],
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
    ]
}

fn source_and_preferences_actions() -> Vec<ManagementActionSpec> {
    let (min_passages, max_passages) = PASSAGE_RANGE;

    vec![
        ManagementActionSpec {
            name: ACTION_ADD_TEXT_SOURCE.to_owned(),
            description: "Add a plain text source with a title to a notebook for indexing."
                .to_owned(),
            security_sensitive: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "notebookId": { "type": "string" },
                    "title": { "type": "string", "maxLength": MAX_TITLE_CHARACTERS },
                    "text": { "type": "string" }
                },
                "required": ["notebookId", "title", "text"],
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_REMOVE_SOURCE.to_owned(),
            description: "Remove a source and its passages from its notebook.".to_owned(),
            security_sensitive: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "sourceId": { "type": "string" }
                },
                "required": ["sourceId"],
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_RETRY_SOURCE.to_owned(),
            description: "Retry indexing a source whose state is `failed`.".to_owned(),
            security_sensitive: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "sourceId": { "type": "string" }
                },
                "required": ["sourceId"],
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_SET_SOURCE_INCLUDED.to_owned(),
            description: "Include or exclude a source from notebook retrieval.".to_owned(),
            security_sensitive: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "sourceId": { "type": "string" },
                    "included": { "type": "boolean" }
                },
                "required": ["sourceId", "included"],
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_SAVE_PREFERENCES.to_owned(),
            description: "Update notebook search preferences (embedding profile, rerank profile, \
                and max passages per message). Omitted fields keep their current values; pass \
                null to clear a profile."
                .to_owned(),
            security_sensitive: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "embeddingProfileId": {
                        "type": ["string", "null"],
                        "description": "Search profile ID for embeddings, or null."
                    },
                    "rerankProfileId": {
                        "type": ["string", "null"],
                        "description": "Optional search profile ID for reranking, or null."
                    },
                    "maxPassages": {
                        "type": "integer",
                        "minimum": min_passages,
                        "maximum": max_passages,
                        "default": DEFAULT_PASSAGES,
                        "description": "Maximum retrieved passages per message."
                    }
                },
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
    ]
}

fn guide_actions() -> Vec<ManagementActionSpec> {
    let mut actions = notebook_actions();
    actions.extend(source_and_preferences_actions());
    actions
}

fn find_notebook<'a>(
    list: &'a crate::commands::NotebookList,
    id_or_name: &str,
) -> Option<&'a crate::commands::NotebookView> {
    list.notebooks
        .iter()
        .find(|nb| nb.id == id_or_name || nb.name.eq_ignore_ascii_case(id_or_name))
}

fn find_source<'a>(
    list: &'a crate::commands::NotebookList,
    source_id: &str,
) -> Option<(
    &'a crate::commands::NotebookView,
    &'a crate::commands::SourceView,
)> {
    list.notebooks.iter().find_map(|nb| {
        nb.sources
            .iter()
            .find(|src| src.id == source_id)
            .map(|src| (nb, src))
    })
}

fn prepare_notebook_action(
    list: &crate::commands::NotebookList,
    action: &str,
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    match action {
        ACTION_CREATE_NOTEBOOK => {
            let input: CreateNotebook = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            validate_input(&input)?;
            if find_notebook(list, input.name.trim()).is_some() {
                return Err(ManagementError::conflict(
                    "another notebook already has that name",
                ));
            }
            let canonical = CreateNotebook {
                name: input.name.trim().to_owned(),
            };
            let canonical_arguments = serde_json::to_value(&canonical)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title: format!("Create notebook {}", canonical.name),
                fields: vec![ConfigFieldPreview {
                    label: "Name".to_owned(),
                    value: canonical.name,
                }],
                secret_fields: Vec::new(),
                security_sensitive: false,
                canonical_arguments,
            })
        }
        ACTION_RENAME_NOTEBOOK => {
            let input: RenameNotebook = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            validate_input(&input)?;
            let nb = list
                .notebooks
                .iter()
                .find(|nb| nb.id == input.notebook_id)
                .ok_or_else(|| ManagementError::not_found("the notebook does not exist"))?;
            let canonical = RenameNotebook {
                notebook_id: nb.id.clone(),
                name: input.name.trim().to_owned(),
            };
            let canonical_arguments = serde_json::to_value(&canonical)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title: format!("Rename notebook {} to {}", nb.name, canonical.name),
                fields: vec![
                    ConfigFieldPreview {
                        label: "Current name".to_owned(),
                        value: nb.name.clone(),
                    },
                    ConfigFieldPreview {
                        label: "New name".to_owned(),
                        value: canonical.name,
                    },
                ],
                secret_fields: Vec::new(),
                security_sensitive: false,
                canonical_arguments,
            })
        }
        ACTION_DELETE_NOTEBOOK => {
            let input: NotebookRef = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            let nb = list
                .notebooks
                .iter()
                .find(|nb| nb.id == input.notebook_id)
                .ok_or_else(|| ManagementError::not_found("the notebook does not exist"))?;
            let canonical_arguments = serde_json::to_value(&input)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title: format!("Delete notebook {}", nb.name),
                fields: vec![
                    ConfigFieldPreview {
                        label: "Notebook".to_owned(),
                        value: nb.name.clone(),
                    },
                    ConfigFieldPreview {
                        label: "Sources".to_owned(),
                        value: nb.sources.len().to_string(),
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

fn prepare_source_action(
    list: &crate::commands::NotebookList,
    action: &str,
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    match action {
        ACTION_ADD_TEXT_SOURCE => {
            let input: AddTextSource = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            validate_input(&input)?;
            let nb = list
                .notebooks
                .iter()
                .find(|nb| nb.id == input.notebook_id)
                .ok_or_else(|| ManagementError::not_found("the notebook does not exist"))?;
            let bytes = input.text.len();
            let title = input.title.trim().to_owned();
            let canonical = AddTextSource {
                notebook_id: nb.id.clone(),
                title: title.clone(),
                text: input.text,
            };
            let canonical_arguments = serde_json::to_value(&canonical)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title: format!("Add text source {title} to {}", nb.name),
                fields: vec![
                    ConfigFieldPreview {
                        label: "Notebook".to_owned(),
                        value: nb.name.clone(),
                    },
                    ConfigFieldPreview {
                        label: "Title".to_owned(),
                        value: title,
                    },
                    ConfigFieldPreview {
                        label: "Size".to_owned(),
                        value: format!("{bytes} bytes"),
                    },
                ],
                secret_fields: Vec::new(),
                security_sensitive: false,
                canonical_arguments,
            })
        }
        ACTION_REMOVE_SOURCE | ACTION_RETRY_SOURCE => {
            let input: SourceRef = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            let (nb, source) = find_source(list, &input.source_id)
                .ok_or_else(|| ManagementError::not_found("the source does not exist"))?;
            let verb = if action == ACTION_REMOVE_SOURCE {
                "Remove"
            } else {
                "Retry indexing"
            };
            let canonical_arguments = serde_json::to_value(&input)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title: format!("{verb} source {}", source.name),
                fields: vec![
                    ConfigFieldPreview {
                        label: "Notebook".to_owned(),
                        value: nb.name.clone(),
                    },
                    ConfigFieldPreview {
                        label: "Source".to_owned(),
                        value: source.name.clone(),
                    },
                ],
                secret_fields: Vec::new(),
                security_sensitive: false,
                canonical_arguments,
            })
        }
        ACTION_SET_SOURCE_INCLUDED => {
            let input: SetSourceIncluded = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            let (nb, source) = find_source(list, &input.source_id)
                .ok_or_else(|| ManagementError::not_found("the source does not exist"))?;
            let verb = if input.included { "Include" } else { "Exclude" };
            let canonical_arguments = serde_json::to_value(&input)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(PreparedAction {
                title: format!("{verb} notebook source {}", source.name),
                fields: vec![
                    ConfigFieldPreview {
                        label: "Notebook".to_owned(),
                        value: nb.name.clone(),
                    },
                    ConfigFieldPreview {
                        label: "Source".to_owned(),
                        value: source.name.clone(),
                    },
                    ConfigFieldPreview {
                        label: "Included in search".to_owned(),
                        value: if input.included { "Yes" } else { "No" }.to_owned(),
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

fn prepare_save_preferences(
    library: &Library,
    arguments: serde_json::Value,
) -> Result<PreparedAction, ManagementError> {
    let patch: PreferencesPatch = serde_json::from_value(arguments)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    let has_change = patch.embedding_profile_id.is_changed()
        || patch.rerank_profile_id.is_changed()
        || patch.max_passages.is_some();
    if !has_change {
        return Err(ManagementError::invalid_arguments(
            "specify at least one preference to update",
        ));
    }

    let mut next = library.preferences();
    patch
        .embedding_profile_id
        .apply_to(&mut next.embedding_profile_id);
    patch
        .rerank_profile_id
        .apply_to(&mut next.rerank_profile_id);
    if let Some(max_passages) = patch.max_passages {
        next.max_passages = max_passages;
    }
    validate_input(&next)?;

    let canonical_arguments =
        serde_json::to_value(&next).map_err(|error| ManagementError::failed(error.to_string()))?;

    Ok(PreparedAction {
        title: "Save Notebooks search preferences".to_owned(),
        fields: vec![
            ConfigFieldPreview {
                label: "Embedding profile".to_owned(),
                value: next
                    .embedding_profile_id
                    .unwrap_or_else(|| "None".to_owned()),
            },
            ConfigFieldPreview {
                label: "Rerank profile".to_owned(),
                value: next.rerank_profile_id.unwrap_or_else(|| "None".to_owned()),
            },
            ConfigFieldPreview {
                label: "Max passages".to_owned(),
                value: next.max_passages.to_string(),
            },
        ],
        secret_fields: Vec::new(),
        security_sensitive: false,
        canonical_arguments,
    })
}

async fn execute_notebook_action(
    library: &Library,
    action: &str,
    arguments: serde_json::Value,
) -> Result<ActionResult, ManagementError> {
    match action {
        ACTION_CREATE_NOTEBOOK => {
            let input: CreateNotebook = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            let nb = library
                .create_notebook(&input.name)
                .await
                .map_err(map_notebooks_error)?;
            let data = serde_json::to_value(&nb)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!("Created notebook {}.", nb.name)).with_data(data))
        }
        ACTION_RENAME_NOTEBOOK => {
            let input: RenameNotebook = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            let nb = library
                .rename_notebook(&input.notebook_id, &input.name)
                .await
                .map_err(map_notebooks_error)?;
            let data = serde_json::to_value(&nb)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!("Renamed notebook to {}.", nb.name)).with_data(data))
        }
        ACTION_DELETE_NOTEBOOK => {
            let input: NotebookRef = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            library
                .delete_notebook(&input.notebook_id)
                .await
                .map_err(map_notebooks_error)?;
            Ok(ActionResult::new("Deleted notebook.")
                .with_data(json!({ "notebookId": input.notebook_id })))
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

async fn execute_source_or_preferences_action(
    library: &Library,
    action: &str,
    arguments: serde_json::Value,
) -> Result<ActionResult, ManagementError> {
    match action {
        ACTION_ADD_TEXT_SOURCE => {
            let input: AddTextSource = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            let source = library
                .add_text(&input.notebook_id, &input.title, &input.text)
                .await
                .map_err(map_notebooks_error)?;
            let data = serde_json::to_value(&source)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!("Added text source {}.", source.name)).with_data(data))
        }
        ACTION_REMOVE_SOURCE => {
            let input: SourceRef = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            library
                .remove_source(&input.source_id)
                .await
                .map_err(map_notebooks_error)?;
            Ok(ActionResult::new("Removed notebook source.")
                .with_data(json!({ "sourceId": input.source_id })))
        }
        ACTION_RETRY_SOURCE => {
            let input: SourceRef = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            library
                .retry_source(&input.source_id)
                .await
                .map_err(map_notebooks_error)?;
            Ok(ActionResult::new("Queued notebook source for retry.")
                .with_data(json!({ "sourceId": input.source_id })))
        }
        ACTION_SET_SOURCE_INCLUDED => {
            let input: SetSourceIncluded = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            library
                .set_included(&input.source_id, input.included)
                .await
                .map_err(map_notebooks_error)?;
            let state = if input.included {
                "included"
            } else {
                "excluded"
            };
            Ok(
                ActionResult::new(format!("Notebook source is now {state}.")).with_data(json!({
                    "sourceId": input.source_id,
                    "included": input.included,
                })),
            )
        }
        ACTION_SAVE_PREFERENCES => {
            let input: Preferences = serde_json::from_value(arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
            let saved = library
                .save_preferences(input)
                .await
                .map_err(map_notebooks_error)?;
            let data = serde_json::to_value(&saved)
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new("Saved Notebooks search preferences.").with_data(data))
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

impl ManagementProvider for NotebooksManagement {
    fn guide(&self) -> ManagementGuide {
        ManagementGuide {
            summary: "Stores owner documents (PDF, Markdown, plain text) in notebooks, indexes \
                passages using the owner's embedding and optional rerank search profiles, and \
                exposes notebook context providers, `@` mentions, and retrieval tools."
                .to_owned(),
            storage: "Stores notebooks, sources, sections, passages, embeddings, and preferences \
                in SQLite under the `notebooks_*` tables. Stores no credentials of its own; \
                model credentials stay in the core's search profiles."
                .to_owned(),
            troubleshooting: "If sources stay in `waiting`, check `notebooks` preferences to make \
                sure an `embeddingProfileId` is configured (configure search profiles under core \
                target `search_profiles` first). Use query `notebook:<id-or-name>` to inspect \
                individual source indexing errors, and call `retry_source` after fixing the \
                search profile or server."
                .to_owned(),
            actions: guide_actions(),
        }
    }

    fn inspect(
        &self,
        query: Option<String>,
    ) -> BoxFuture<'static, Result<ManagementSnapshot, ManagementError>> {
        let library = Arc::clone(&self.library);

        Box::pin(async move {
            let list = library.list();
            let trimmed = query.as_deref().map(str::trim).filter(|q| !q.is_empty());

            if let Some(selector) = trimmed {
                let Some(key) = selector.strip_prefix("notebook:") else {
                    return Err(ManagementError::invalid_field(
                        "query",
                        format!(
                            "unsupported notebooks inspect query {selector:?}; omit query or use \"notebook:<id-or-name>\""
                        ),
                    ));
                };

                let nb = find_notebook(&list, key).ok_or_else(|| {
                    ManagementError::not_found(format!("no notebook matching {key:?}"))
                })?;
                let summary = format!("Notebook {} ({} source(s)).", nb.name, nb.sources.len());
                let data = serde_json::to_value(nb)
                    .map_err(|error| ManagementError::failed(error.to_string()))?;

                return Ok(ManagementSnapshot::new(summary, data));
            }

            let preferences = library.preferences();
            let indexing = library.indexing_count();
            let available_profiles = library.search_profiles().await.ok();
            let total_sources: usize = list.notebooks.iter().map(|nb| nb.sources.len()).sum();

            let notebooks_summary: Vec<serde_json::Value> = list
                .notebooks
                .iter()
                .map(|nb| {
                    let failed = nb
                        .sources
                        .iter()
                        .filter(|s| s.state == crate::commands::SourceState::Failed)
                        .count();
                    let ready = nb
                        .sources
                        .iter()
                        .filter(|s| s.state == crate::commands::SourceState::Ready)
                        .count();
                    json!({
                        "id": nb.id,
                        "name": nb.name,
                        "sourceCount": nb.sources.len(),
                        "readySources": ready,
                        "failedSources": failed,
                        "createdAt": nb.created_at,
                        "updatedAt": nb.updated_at,
                    })
                })
                .collect();

            let summary = format!(
                "{} notebook(s), {total_sources} source(s) ({indexing} indexing).",
                list.notebooks.len()
            );
            let data = json!({
                "notebooks": notebooks_summary,
                "preferences": preferences,
                "availableSearchProfiles": available_profiles,
                "indexingCount": indexing,
            });

            Ok(ManagementSnapshot::new(summary, data))
        })
    }

    fn prepare_action(
        &self,
        request: PrepareActionRequest,
    ) -> BoxFuture<'static, Result<PreparedAction, ManagementError>> {
        let library = Arc::clone(&self.library);

        Box::pin(async move {
            let list = library.list();

            match request.action.as_str() {
                ACTION_CREATE_NOTEBOOK | ACTION_RENAME_NOTEBOOK | ACTION_DELETE_NOTEBOOK => {
                    prepare_notebook_action(&list, &request.action, request.arguments)
                }
                ACTION_ADD_TEXT_SOURCE
                | ACTION_REMOVE_SOURCE
                | ACTION_RETRY_SOURCE
                | ACTION_SET_SOURCE_INCLUDED => {
                    prepare_source_action(&list, &request.action, request.arguments)
                }
                ACTION_SAVE_PREFERENCES => prepare_save_preferences(&library, request.arguments),
                other => Err(ManagementError::unknown_action(other)),
            }
        })
    }

    fn execute_action(
        &self,
        request: ExecuteActionRequest,
    ) -> BoxFuture<'static, Result<ActionResult, ManagementError>> {
        let library = Arc::clone(&self.library);

        Box::pin(async move {
            match request.action.as_str() {
                ACTION_CREATE_NOTEBOOK | ACTION_RENAME_NOTEBOOK | ACTION_DELETE_NOTEBOOK => {
                    execute_notebook_action(&library, &request.action, request.arguments).await
                }
                ACTION_ADD_TEXT_SOURCE
                | ACTION_REMOVE_SOURCE
                | ACTION_RETRY_SOURCE
                | ACTION_SET_SOURCE_INCLUDED
                | ACTION_SAVE_PREFERENCES => {
                    execute_source_or_preferences_action(
                        &library,
                        &request.action,
                        request.arguments,
                    )
                    .await
                }
                other => Err(ManagementError::unknown_action(other)),
            }
        })
    }
}
