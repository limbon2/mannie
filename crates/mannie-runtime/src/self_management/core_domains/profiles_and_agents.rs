//! Core management handlers for `providers`, `search_profiles`, and `agents`.

use std::collections::BTreeMap;

use mannie_plugin_sdk::points::management::{
    ActionResult, ManagementError, ManagementSnapshot, PrepareActionRequest, PreparedAction,
};
use mannie_provider::{check_base_url, known_model};
use mannie_types::agents::{AgentDraft, AgentKind, AgentStep, CUSTOM_AGENT_PREFIX};
use mannie_types::ids::ProfileId;
use mannie_types::provider::{
    ApiInterface, CreateProviderProfile, CreateSearchProfile, CredentialChange, ProfileAccess,
    ProfileDraft, ReasoningEffort, SearchKind, SearchProfileDraft,
};
use mannie_types::secret::Secret;
use mannie_types::tools::{ConfigFieldPreview, ConfigSecretField};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Runtime;

const DEFAULT_CONTEXT_WINDOW: u32 = 128_000;
const DEFAULT_OUTPUT_LIMIT: u32 = 16_384;

fn field(label: &str, value: impl Into<String>) -> ConfigFieldPreview {
    ConfigFieldPreview {
        label: label.to_owned(),
        value: value.into(),
    }
}

fn parse_args<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, ManagementError> {
    serde_json::from_value(value)
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))
}

fn parse_profile_id(text: &str) -> Result<ProfileId, ManagementError> {
    ProfileId::parse(text).map_err(|error| ManagementError::invalid_field("id", error.to_string()))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CredentialAction {
    #[default]
    Keep,
    Replace,
    Remove,
}

impl CredentialAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Replace => "replace",
            Self::Remove => "remove",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateProviderArgs {
    name: String,
    #[serde(default)]
    access: ProfileAccess,
    interface: ApiInterface,
    base_url: String,
    model: String,
    context_window: Option<u32>,
    output_limit: Option<u32>,
    #[serde(default)]
    reasoning_effort: ReasoningEffort,
    use_tools: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateProviderArgs {
    id: String,
    name: Option<String>,
    access: Option<ProfileAccess>,
    interface: Option<ApiInterface>,
    base_url: Option<String>,
    model: Option<String>,
    context_window: Option<u32>,
    output_limit: Option<u32>,
    reasoning_effort: Option<ReasoningEffort>,
    use_tools: Option<bool>,
    #[serde(default)]
    credential_action: CredentialAction,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IdArg {
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateSearchArgs {
    name: String,
    kind: SearchKind,
    base_url: String,
    model: String,
    #[serde(default)]
    query_prefix: String,
    #[serde(default)]
    document_prefix: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateSearchArgs {
    id: String,
    name: Option<String>,
    kind: Option<SearchKind>,
    base_url: Option<String>,
    model: Option<String>,
    query_prefix: Option<String>,
    document_prefix: Option<String>,
    #[serde(default)]
    credential_action: CredentialAction,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateAgentArgs {
    name: String,
    #[serde(default)]
    description: String,
    kind: AgentKind,
    #[serde(default)]
    read_only: bool,
    #[serde(default)]
    profile_id: Option<String>,
    #[serde(default)]
    instructions: String,
    #[serde(default)]
    steps: Vec<AgentStep>,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    plugins: Option<Vec<String>>,
    #[serde(default)]
    children: Option<Vec<String>>,
    #[serde(default)]
    max_tasks: Option<u32>,
}

#[derive(Debug, Default)]
enum PatchField<T> {
    #[default]
    Unchanged,
    Clear,
    Set(T),
}

fn deserialize_patch_field<'de, D, T>(deserializer: D) -> Result<PatchField<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(match Option::<T>::deserialize(deserializer)? {
        Some(value) => PatchField::Set(value),
        None => PatchField::Clear,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateAgentArgs {
    key: String,
    name: Option<String>,
    description: Option<String>,
    read_only: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_patch_field")]
    profile_id: PatchField<String>,
    instructions: Option<String>,
    steps: Option<Vec<AgentStep>>,
    tools: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_patch_field")]
    plugins: PatchField<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_patch_field")]
    children: PatchField<Vec<String>>,
    max_tasks: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeleteAgentArgs {
    key: String,
}

pub(super) async fn inspect_providers(
    runtime: &Runtime,
) -> Result<ManagementSnapshot, ManagementError> {
    let profiles = runtime
        .list_profiles()
        .await
        .map_err(|error| ManagementError::failed(error.to_string()))?;
    let count = profiles.len();
    Ok(ManagementSnapshot::new(
        format!("{count} provider profile(s) configured."),
        json!({ "profiles": profiles }),
    ))
}

fn prepare_create_provider(
    arguments: Value,
    provided_in_chat: bool,
) -> Result<PreparedAction, ManagementError> {
    let args: CreateProviderArgs = parse_args(arguments)?;
    let defaults = known_model(&args.model);
    let context_window = args
        .context_window
        .unwrap_or_else(|| defaults.map_or(DEFAULT_CONTEXT_WINDOW, |m| m.limits.context_window));
    let output_limit = args
        .output_limit
        .unwrap_or_else(|| defaults.map_or(DEFAULT_OUTPUT_LIMIT, |m| m.limits.output_limit));
    let use_tools = args.use_tools.unwrap_or(true);
    let draft = ProfileDraft {
        name: args.name.clone(),
        access: args.access,
        interface: args.interface,
        base_url: args.base_url.clone(),
        model: args.model.clone(),
        context_window,
        output_limit,
        reasoning_effort: args.reasoning_effort,
        use_tools,
    };
    let (name, model) = draft
        .check_fields()
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    if args.access == ProfileAccess::Endpoint {
        check_base_url(&args.base_url)
            .map_err(|error| ManagementError::invalid_field("baseUrl", error.to_string()))?;
    }
    let secret_fields = if args.access == ProfileAccess::Endpoint {
        vec![ConfigSecretField {
            key: "credential".to_owned(),
            label: "API key".to_owned(),
            required: false,
            provided_in_chat,
        }]
    } else {
        Vec::new()
    };
    Ok(PreparedAction {
        title: format!("Create provider profile {name}"),
        fields: vec![
            field("Name", &name),
            field("Access", format!("{:?}", args.access)),
            field("Interface", args.interface.as_str()),
            field("Base URL", &args.base_url),
            field("Model", model.as_str()),
            field(
                "Limits",
                format!("{context_window} context / {output_limit} output"),
            ),
            field("Tools enabled", use_tools.to_string()),
        ],
        secret_fields,
        security_sensitive: true,
        canonical_arguments: json!({
            "name": name,
            "access": args.access,
            "interface": args.interface,
            "baseUrl": args.base_url,
            "model": model.as_str(),
            "contextWindow": context_window,
            "outputLimit": output_limit,
            "reasoningEffort": args.reasoning_effort,
            "useTools": use_tools,
        }),
    })
}

async fn prepare_update_provider(
    runtime: &Runtime,
    arguments: Value,
    provided_in_chat: bool,
) -> Result<PreparedAction, ManagementError> {
    let args: UpdateProviderArgs = parse_args(arguments)?;
    let id = parse_profile_id(&args.id)?;
    let profiles = runtime
        .list_profiles()
        .await
        .map_err(|error| ManagementError::failed(error.to_string()))?;
    let existing = profiles
        .into_iter()
        .find(|profile| profile.id == id && profile.catalog.is_none())
        .ok_or_else(|| ManagementError::not_found("provider profile not found"))?;
    let access = args.access.unwrap_or(existing.access);
    let interface = args.interface.unwrap_or(existing.interface);
    let base_url = args.base_url.unwrap_or(existing.base_url);
    let model_text = args.model.unwrap_or(existing.model);
    let name_text = args.name.unwrap_or(existing.name);
    let context_window = args.context_window.unwrap_or(existing.context_window);
    let output_limit = args.output_limit.unwrap_or(existing.output_limit);
    let reasoning_effort = args.reasoning_effort.unwrap_or(existing.reasoning_effort);
    let use_tools = args.use_tools.unwrap_or(existing.use_tools);
    let draft = ProfileDraft {
        name: name_text,
        access,
        interface,
        base_url: base_url.clone(),
        model: model_text,
        context_window,
        output_limit,
        reasoning_effort,
        use_tools,
    };
    let (name, model) = draft
        .check_fields()
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    if access == ProfileAccess::Endpoint {
        check_base_url(&base_url)
            .map_err(|error| ManagementError::invalid_field("baseUrl", error.to_string()))?;
    }
    let secret_fields = if access == ProfileAccess::Endpoint
        && args.credential_action != CredentialAction::Remove
    {
        vec![ConfigSecretField {
            key: "credential".to_owned(),
            label: "API key".to_owned(),
            required: args.credential_action == CredentialAction::Replace,
            provided_in_chat,
        }]
    } else {
        Vec::new()
    };
    Ok(PreparedAction {
        title: format!("Update provider profile {name}"),
        fields: vec![
            field("Profile ID", id.as_str()),
            field("Name", &name),
            field("Interface", interface.as_str()),
            field("Base URL", &base_url),
            field("Model", model.as_str()),
            field("Credential", args.credential_action.as_str()),
        ],
        secret_fields,
        security_sensitive: true,
        canonical_arguments: json!({
            "id": id.as_str(),
            "name": name,
            "access": access,
            "interface": interface,
            "baseUrl": base_url,
            "model": model.as_str(),
            "contextWindow": context_window,
            "outputLimit": output_limit,
            "reasoningEffort": reasoning_effort,
            "useTools": use_tools,
            "credentialAction": args.credential_action.as_str(),
        }),
    })
}

async fn prepare_provider_id_action(
    runtime: &Runtime,
    action: &str,
    arguments: Value,
) -> Result<PreparedAction, ManagementError> {
    let args: IdArg = parse_args(arguments)?;
    let id = parse_profile_id(&args.id)?;
    let profiles = runtime
        .list_profiles()
        .await
        .map_err(|error| ManagementError::failed(error.to_string()))?;
    let existing = profiles
        .into_iter()
        .find(|profile| profile.id == id)
        .ok_or_else(|| ManagementError::not_found("provider profile not found"))?;
    let (title, sensitive) = match action {
        "delete" => (format!("Delete provider profile {}", existing.name), true),
        "set_default" => (
            format!("Set default provider profile to {}", existing.name),
            false,
        ),
        _ => (format!("Test provider profile {}", existing.name), false),
    };
    Ok(PreparedAction {
        title,
        fields: vec![
            field("Profile ID", id.as_str()),
            field("Name", &existing.name),
            field("Model", &existing.model),
            field("Base URL", &existing.base_url),
        ],
        secret_fields: Vec::new(),
        security_sensitive: sensitive,
        canonical_arguments: json!({ "id": id.as_str() }),
    })
}

pub(super) async fn prepare_providers(
    runtime: &Runtime,
    request: PrepareActionRequest,
) -> Result<PreparedAction, ManagementError> {
    let provided_in_chat = request.has_provided_secret("credential");
    match request.action.as_str() {
        "create" => prepare_create_provider(request.arguments, provided_in_chat),
        "update" => prepare_update_provider(runtime, request.arguments, provided_in_chat).await,
        "delete" | "set_default" | "test" => {
            prepare_provider_id_action(runtime, &request.action, request.arguments).await
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

async fn execute_update_provider(
    runtime: &Runtime,
    arguments: Value,
    secrets: &mut BTreeMap<String, Secret>,
) -> Result<ActionResult, ManagementError> {
    let args: UpdateProviderArgs = parse_args(arguments)?;
    let id = parse_profile_id(&args.id)?;
    let draft = ProfileDraft {
        name: args.name.unwrap_or_default(),
        access: args.access.unwrap_or_default(),
        interface: args.interface.unwrap_or(ApiInterface::Responses),
        base_url: args.base_url.unwrap_or_default(),
        model: args.model.unwrap_or_default(),
        context_window: args.context_window.unwrap_or(DEFAULT_CONTEXT_WINDOW),
        output_limit: args.output_limit.unwrap_or(DEFAULT_OUTPUT_LIMIT),
        reasoning_effort: args.reasoning_effort.unwrap_or_default(),
        use_tools: args.use_tools.unwrap_or(true),
    };
    let entered = secrets
        .remove("credential")
        .filter(|secret| !secret.is_empty());
    let change = match (args.credential_action, entered) {
        (CredentialAction::Remove, _) => CredentialChange::Remove,
        (_, Some(credential)) => CredentialChange::Replace { credential },
        (CredentialAction::Replace, None) => {
            return Err(ManagementError::invalid_field(
                "credential",
                "a new API key is required when credentialAction is 'replace'",
            ));
        }
        (CredentialAction::Keep, None) => CredentialChange::Keep,
    };
    let updated = runtime
        .update_profile(&id, draft, change)
        .await
        .map_err(|error| ManagementError::failed(error.to_string()))?;
    Ok(ActionResult::new(format!(
        "Updated provider profile {} ({}).",
        updated.name, updated.id
    ))
    .with_data(json!({ "profile": updated })))
}

pub(super) async fn execute_providers(
    runtime: &Runtime,
    action: &str,
    arguments: Value,
    mut secrets: BTreeMap<String, Secret>,
) -> Result<ActionResult, ManagementError> {
    match action {
        "create" => {
            let args: CreateProviderArgs = parse_args(arguments)?;
            let draft = ProfileDraft {
                name: args.name,
                access: args.access,
                interface: args.interface,
                base_url: args.base_url,
                model: args.model,
                context_window: args.context_window.unwrap_or(DEFAULT_CONTEXT_WINDOW),
                output_limit: args.output_limit.unwrap_or(DEFAULT_OUTPUT_LIMIT),
                reasoning_effort: args.reasoning_effort,
                use_tools: args.use_tools.unwrap_or(true),
            };
            let credential = secrets
                .remove("credential")
                .filter(|secret| !secret.is_empty());
            let created = runtime
                .create_profile(CreateProviderProfile { draft, credential })
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Created provider profile {} ({}).",
                created.name, created.id
            ))
            .with_data(json!({ "profile": created })))
        }
        "update" => execute_update_provider(runtime, arguments, &mut secrets).await,
        "delete" => {
            let args: IdArg = parse_args(arguments)?;
            let id = parse_profile_id(&args.id)?;
            runtime
                .delete_profile(&id)
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Deleted provider profile {}.",
                id.as_str()
            )))
        }
        "set_default" => {
            let args: IdArg = parse_args(arguments)?;
            let id = parse_profile_id(&args.id)?;
            runtime
                .set_default_profile(&id)
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Set default provider profile to {}.",
                id.as_str()
            )))
        }
        "test" => {
            let args: IdArg = parse_args(arguments)?;
            let id = parse_profile_id(&args.id)?;
            let report = runtime
                .test_profile(&id)
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Completed test for provider profile {}.",
                id.as_str()
            ))
            .with_data(json!({ "report": report })))
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

pub(super) async fn inspect_search_profiles(
    runtime: &Runtime,
) -> Result<ManagementSnapshot, ManagementError> {
    let profiles = runtime
        .list_search_profiles()
        .await
        .map_err(|error| ManagementError::failed(error.to_string()))?;
    let count = profiles.len();
    Ok(ManagementSnapshot::new(
        format!("{count} search profile(s) configured."),
        json!({ "profiles": profiles }),
    ))
}

fn prepare_create_search_profile(
    arguments: Value,
    provided_in_chat: bool,
) -> Result<PreparedAction, ManagementError> {
    let args: CreateSearchArgs = parse_args(arguments)?;
    let draft = SearchProfileDraft {
        name: args.name.clone(),
        kind: args.kind,
        base_url: args.base_url.clone(),
        model: args.model.clone(),
        query_prefix: args.query_prefix.clone(),
        document_prefix: args.document_prefix.clone(),
    };
    let checked = draft
        .check_fields()
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    check_base_url(&args.base_url)
        .map_err(|error| ManagementError::invalid_field("baseUrl", error.to_string()))?;
    Ok(PreparedAction {
        title: format!("Create search profile {}", checked.name),
        fields: vec![
            field("Name", &checked.name),
            field("Kind", args.kind.as_str()),
            field("Base URL", &args.base_url),
            field("Model", checked.model.as_str()),
        ],
        secret_fields: vec![ConfigSecretField {
            key: "credential".to_owned(),
            label: "API key".to_owned(),
            required: false,
            provided_in_chat,
        }],
        security_sensitive: true,
        canonical_arguments: json!({
            "name": checked.name,
            "kind": args.kind,
            "baseUrl": args.base_url,
            "model": checked.model.as_str(),
            "queryPrefix": checked.query_prefix,
            "documentPrefix": checked.document_prefix,
        }),
    })
}

async fn prepare_update_search_profile(
    runtime: &Runtime,
    arguments: Value,
    provided_in_chat: bool,
) -> Result<PreparedAction, ManagementError> {
    let args: UpdateSearchArgs = parse_args(arguments)?;
    let id = parse_profile_id(&args.id)?;
    let profiles = runtime
        .list_search_profiles()
        .await
        .map_err(|error| ManagementError::failed(error.to_string()))?;
    let existing = profiles
        .into_iter()
        .find(|profile| profile.id == id)
        .ok_or_else(|| ManagementError::not_found("search profile not found"))?;
    let kind = args.kind.unwrap_or(existing.kind);
    let base_url = args.base_url.unwrap_or(existing.base_url);
    let draft = SearchProfileDraft {
        name: args.name.unwrap_or(existing.name),
        kind,
        base_url: base_url.clone(),
        model: args.model.unwrap_or(existing.model),
        query_prefix: args.query_prefix.unwrap_or(existing.query_prefix),
        document_prefix: args.document_prefix.unwrap_or(existing.document_prefix),
    };
    let checked = draft
        .check_fields()
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    check_base_url(&base_url)
        .map_err(|error| ManagementError::invalid_field("baseUrl", error.to_string()))?;
    let secret_fields = if args.credential_action == CredentialAction::Remove {
        Vec::new()
    } else {
        vec![ConfigSecretField {
            key: "credential".to_owned(),
            label: "API key".to_owned(),
            required: args.credential_action == CredentialAction::Replace,
            provided_in_chat,
        }]
    };
    Ok(PreparedAction {
        title: format!("Update search profile {}", checked.name),
        fields: vec![
            field("Profile ID", id.as_str()),
            field("Name", &checked.name),
            field("Kind", kind.as_str()),
            field("Base URL", &base_url),
            field("Model", checked.model.as_str()),
            field("Credential", args.credential_action.as_str()),
        ],
        secret_fields,
        security_sensitive: true,
        canonical_arguments: json!({
            "id": id.as_str(),
            "name": checked.name,
            "kind": kind,
            "baseUrl": base_url,
            "model": checked.model.as_str(),
            "queryPrefix": checked.query_prefix,
            "documentPrefix": checked.document_prefix,
            "credentialAction": args.credential_action.as_str(),
        }),
    })
}

pub(super) async fn prepare_search_profiles(
    runtime: &Runtime,
    request: PrepareActionRequest,
) -> Result<PreparedAction, ManagementError> {
    let provided_in_chat = request.has_provided_secret("credential");
    match request.action.as_str() {
        "create" => prepare_create_search_profile(request.arguments, provided_in_chat),
        "update" => {
            prepare_update_search_profile(runtime, request.arguments, provided_in_chat).await
        }
        "delete" | "test" => {
            let args: IdArg = parse_args(request.arguments)?;
            let id = parse_profile_id(&args.id)?;
            let profiles = runtime
                .list_search_profiles()
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            let existing = profiles
                .into_iter()
                .find(|profile| profile.id == id)
                .ok_or_else(|| ManagementError::not_found("search profile not found"))?;
            let (title, sensitive) = if request.action == "delete" {
                (format!("Delete search profile {}", existing.name), true)
            } else {
                (format!("Test search profile {}", existing.name), false)
            };
            Ok(PreparedAction {
                title,
                fields: vec![
                    field("Profile ID", id.as_str()),
                    field("Name", &existing.name),
                    field("Kind", existing.kind.as_str()),
                    field("Model", &existing.model),
                ],
                secret_fields: Vec::new(),
                security_sensitive: sensitive,
                canonical_arguments: json!({ "id": id.as_str() }),
            })
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

pub(super) async fn execute_search_profiles(
    runtime: &Runtime,
    action: &str,
    arguments: Value,
    mut secrets: BTreeMap<String, Secret>,
) -> Result<ActionResult, ManagementError> {
    match action {
        "create" => {
            let args: CreateSearchArgs = parse_args(arguments)?;
            let draft = SearchProfileDraft {
                name: args.name,
                kind: args.kind,
                base_url: args.base_url,
                model: args.model,
                query_prefix: args.query_prefix,
                document_prefix: args.document_prefix,
            };
            let credential = secrets
                .remove("credential")
                .filter(|secret| !secret.is_empty());
            let created = runtime
                .create_search_profile(CreateSearchProfile { draft, credential })
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Created search profile {} ({}).",
                created.name, created.id
            ))
            .with_data(json!({ "profile": created })))
        }
        "update" => {
            let args: UpdateSearchArgs = parse_args(arguments)?;
            let id = parse_profile_id(&args.id)?;
            let draft = SearchProfileDraft {
                name: args.name.unwrap_or_default(),
                kind: args.kind.unwrap_or(SearchKind::Embeddings),
                base_url: args.base_url.unwrap_or_default(),
                model: args.model.unwrap_or_default(),
                query_prefix: args.query_prefix.unwrap_or_default(),
                document_prefix: args.document_prefix.unwrap_or_default(),
            };
            let entered = secrets
                .remove("credential")
                .filter(|secret| !secret.is_empty());
            let change = match (args.credential_action, entered) {
                (CredentialAction::Remove, _) => CredentialChange::Remove,
                (_, Some(credential)) => CredentialChange::Replace { credential },
                (CredentialAction::Replace, None) => {
                    return Err(ManagementError::invalid_field(
                        "credential",
                        "a new API key is required when credentialAction is 'replace'",
                    ));
                }
                (CredentialAction::Keep, None) => CredentialChange::Keep,
            };
            let updated = runtime
                .update_search_profile(&id, draft, change)
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Updated search profile {} ({}).",
                updated.name, updated.id
            ))
            .with_data(json!({ "profile": updated })))
        }
        "delete" => {
            let args: IdArg = parse_args(arguments)?;
            let id = parse_profile_id(&args.id)?;
            runtime
                .delete_search_profile(&id)
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Deleted search profile {}.",
                id.as_str()
            )))
        }
        "test" => {
            let args: IdArg = parse_args(arguments)?;
            let id = parse_profile_id(&args.id)?;
            let report = runtime
                .test_search_profile(&id)
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Completed test for search profile {}.",
                id.as_str()
            ))
            .with_data(json!({ "report": report })))
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

pub(super) fn inspect_agents(runtime: &Runtime) -> ManagementSnapshot {
    let agents = runtime.list_agents();
    let options = runtime.agent_options();
    let count = agents.len();
    ManagementSnapshot::new(
        format!("{count} agent(s) registered (built-in and custom)."),
        json!({ "agents": agents, "options": options }),
    )
}

fn prepare_create_agent(arguments: Value) -> Result<PreparedAction, ManagementError> {
    let args: CreateAgentArgs = parse_args(arguments)?;
    let profile_id = args
        .profile_id
        .as_deref()
        .map(parse_profile_id)
        .transpose()?;
    let draft = AgentDraft {
        name: args.name,
        description: args.description,
        kind: args.kind,
        read_only: args.read_only,
        profile_id,
        instructions: args.instructions,
        steps: args.steps,
        tools: args.tools,
        plugins: args.plugins,
        children: args.children,
        max_tasks: args.max_tasks.unwrap_or(20),
        max_tool_rounds: 0,
        max_tool_calls: 0,
    };
    let checked = draft
        .check_fields()
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    Ok(PreparedAction {
        title: format!("Create custom agent {}", checked.name),
        fields: vec![
            field("Name", &checked.name),
            field("Kind", format!("{:?}", checked.kind)),
            field("Read only", checked.read_only.to_string()),
            field("Tools", checked.tools.join(", ")),
        ],
        secret_fields: Vec::new(),
        security_sensitive: true,
        canonical_arguments: serde_json::to_value(&checked)
            .map_err(|error| ManagementError::failed(error.to_string()))?,
    })
}

fn prepare_update_agent(
    runtime: &Runtime,
    arguments: Value,
) -> Result<PreparedAction, ManagementError> {
    let args: UpdateAgentArgs = parse_args(arguments)?;
    if !args.key.starts_with(CUSTOM_AGENT_PREFIX) {
        return Err(ManagementError::invalid_field(
            "key",
            "only custom.* agents can be updated",
        ));
    }
    let existing = runtime
        .list_agents()
        .into_iter()
        .find(|agent| agent.key == args.key && !agent.builtin)
        .ok_or_else(|| ManagementError::not_found("custom agent not found"))?;
    let current = existing.draft;
    let profile_id = match args.profile_id {
        PatchField::Set(id) => Some(parse_profile_id(&id)?),
        PatchField::Clear => None,
        PatchField::Unchanged => current.profile_id,
    };
    let plugins = match args.plugins {
        PatchField::Set(list) => Some(list),
        PatchField::Clear => None,
        PatchField::Unchanged => current.plugins,
    };
    let children = match args.children {
        PatchField::Set(list) => Some(list),
        PatchField::Clear => None,
        PatchField::Unchanged => current.children,
    };
    let draft = AgentDraft {
        name: args.name.unwrap_or(current.name),
        description: args.description.unwrap_or(current.description),
        kind: current.kind,
        read_only: args.read_only.unwrap_or(current.read_only),
        profile_id,
        instructions: args.instructions.unwrap_or(current.instructions),
        steps: args.steps.unwrap_or(current.steps),
        tools: args.tools.unwrap_or(current.tools),
        plugins,
        children,
        max_tasks: args.max_tasks.unwrap_or(current.max_tasks),
        max_tool_rounds: 0,
        max_tool_calls: 0,
    };
    let checked = draft
        .check_fields()
        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    Ok(PreparedAction {
        title: format!("Update custom agent {}", checked.name),
        fields: vec![
            field("Key", &args.key),
            field("Name", &checked.name),
            field("Read only", checked.read_only.to_string()),
            field("Tools", checked.tools.join(", ")),
        ],
        secret_fields: Vec::new(),
        security_sensitive: true,
        canonical_arguments: json!({
            "key": args.key,
            "draft": checked,
        }),
    })
}

pub(super) fn prepare_agents(
    runtime: &Runtime,
    request: PrepareActionRequest,
) -> Result<PreparedAction, ManagementError> {
    match request.action.as_str() {
        "create" => prepare_create_agent(request.arguments),
        "update" => prepare_update_agent(runtime, request.arguments),
        "delete" => {
            let args: DeleteAgentArgs = parse_args(request.arguments)?;
            if !args.key.starts_with(CUSTOM_AGENT_PREFIX) {
                return Err(ManagementError::invalid_field(
                    "key",
                    "only custom.* agents can be deleted",
                ));
            }
            let existing = runtime
                .list_agents()
                .into_iter()
                .find(|agent| agent.key == args.key && !agent.builtin)
                .ok_or_else(|| ManagementError::not_found("custom agent not found"))?;
            Ok(PreparedAction {
                title: format!("Delete custom agent {}", existing.draft.name),
                fields: vec![
                    field("Key", &existing.key),
                    field("Name", &existing.draft.name),
                ],
                secret_fields: Vec::new(),
                security_sensitive: true,
                canonical_arguments: json!({ "key": existing.key }),
            })
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

pub(super) async fn execute_agents(
    runtime: &Runtime,
    action: &str,
    arguments: Value,
) -> Result<ActionResult, ManagementError> {
    match action {
        "create" => {
            let draft: AgentDraft = parse_args(arguments)?;
            let created = runtime
                .create_agent(&draft)
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Created custom agent {} ({}).",
                created.draft.name, created.key
            ))
            .with_data(json!({ "agent": created })))
        }
        "update" => {
            #[derive(Deserialize)]
            struct Payload {
                key: String,
                draft: AgentDraft,
            }
            let payload: Payload = parse_args(arguments)?;
            let updated = runtime
                .update_agent(&payload.key, &payload.draft)
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Updated custom agent {} ({}).",
                updated.draft.name, updated.key
            ))
            .with_data(json!({ "agent": updated })))
        }
        "delete" => {
            let args: DeleteAgentArgs = parse_args(arguments)?;
            runtime
                .delete_agent(&args.key)
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Deleted custom agent {}.",
                args.key
            )))
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}
