//! Built-in core configuration domains (ADR-0081): inspection, preparation,
//! and authorized execution for all 15 core domains.

mod guides;
mod profiles_and_agents;

use std::collections::BTreeMap;

use mannie_plugin_sdk::points::management::{
    ActionResult, ManagementError, ManagementGuide, ManagementSnapshot, PrepareActionRequest,
    PreparedAction,
};
use mannie_types::chat::{Instructions, UpdateChatSettings};
use mannie_types::ids::{ConversationId, GrantId, ProjectId, RuleId};
use mannie_types::interface::UpdateInterfaceSettings;
use mannie_types::notifications::UpdateNotificationSettings;
use mannie_types::projects::{RemoveProject, RenameProject, check_project_name};
use mannie_types::secret::Secret;
use mannie_types::skills::{SetSkillOn, SkillPlace, is_skill_name};
use mannie_types::tasks::{ParallelChildren, UpdateChildAgentsSettings};
use mannie_types::tools::{
    ConfigFieldPreview, GrantRef, ProgramRuleRef, RemoteRuleRef, RuleRef, SharedGrantRef,
    TerminalRuleRef, UpdateToolsSettings,
};
use mannie_types::updates::UpdateUpdatesSettings;
use mannie_types::window::{CloseAction, UpdateWindowSettings};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Runtime;
use crate::self_management::AppHost;

pub(crate) use self::guides::{CORE_DOMAINS, domain_guide, topic_guide};

/// Whether `name` is one of Mannie's 15 built-in core domains.
pub(crate) fn is_core_domain(name: &str) -> bool {
    CORE_DOMAINS.contains(&name)
}

/// Returns guides for all core domains, for `mannie_inspect(topic="actions")`
/// and `mannie_inspect(topic="core")`.
pub(crate) fn all_domain_guides() -> Vec<(&'static str, ManagementGuide)> {
    CORE_DOMAINS
        .iter()
        .filter_map(|&domain| domain_guide(domain).map(|guide| (domain, guide)))
        .collect()
}

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

fn format_work_warning(items: &[mannie_types::plugins::WorkItem]) -> Option<String> {
    let active: Vec<String> = items
        .iter()
        .filter(|item| item.count > 0)
        .map(|item| format!("{}: {}", item.code, item.count))
        .collect();
    if active.is_empty() {
        None
    } else {
        Some(active.join(", "))
    }
}

/// Inspects one of the 15 built-in core domains without side effects.
pub(crate) async fn inspect_core_domain(
    runtime: &Runtime,
    host: &dyn AppHost,
    domain: &str,
    conversation_id: &ConversationId,
    query: Option<&str>,
) -> Result<ManagementSnapshot, ManagementError> {
    match domain {
        "plugins" | "projects" | "providers" | "search_profiles" | "chatgpt" | "agents"
        | "skills" => inspect_runtime_domain(runtime, host, domain).await,
        "chat" | "tools" | "policy" | "child_agents" => {
            inspect_conversation_domain(runtime, domain, conversation_id, query).await
        }
        "interface" | "window" | "notifications" | "updates" => {
            inspect_host_domain(host, domain).await
        }
        other => Err(ManagementError::not_found(format!(
            "unknown core domain {other:?}"
        ))),
    }
}

async fn inspect_runtime_domain(
    runtime: &Runtime,
    host: &dyn AppHost,
    domain: &str,
) -> Result<ManagementSnapshot, ManagementError> {
    match domain {
        "plugins" => {
            let plugins = host.list_plugins();
            let live_managers: Vec<String> = runtime
                .management_providers()
                .into_iter()
                .map(|(id, _)| id)
                .collect();
            let count = plugins.len();
            Ok(ManagementSnapshot::new(
                format!("{count} plugin(s) installed."),
                json!({
                    "plugins": plugins,
                    "liveManagementProviders": live_managers,
                }),
            ))
        }
        "projects" => {
            let projects = runtime
                .inner
                .store
                .projects()
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            let count = projects.len();
            Ok(ManagementSnapshot::new(
                format!("{count} project(s) registered."),
                json!({ "projects": projects }),
            ))
        }
        "providers" => profiles_and_agents::inspect_providers(runtime).await,
        "search_profiles" => profiles_and_agents::inspect_search_profiles(runtime).await,
        "chatgpt" => {
            let account = runtime.chatgpt_account_view();
            let models = runtime.chatgpt_models().await.unwrap_or_default();
            Ok(ManagementSnapshot::new(
                format!("ChatGPT account state: {:?}.", account.state),
                json!({
                    "account": account,
                    "models": models,
                }),
            ))
        }
        "agents" => Ok(profiles_and_agents::inspect_agents(runtime)),
        "skills" => {
            let overview = runtime
                .list_skills()
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ManagementSnapshot::new(
                "Installed skills overview.".to_owned(),
                serde_json::to_value(&overview)
                    .map_err(|error| ManagementError::failed(error.to_string()))?,
            ))
        }
        other => Err(ManagementError::not_found(format!(
            "unknown core domain {other:?}"
        ))),
    }
}

async fn inspect_conversation_domain(
    runtime: &Runtime,
    domain: &str,
    conversation_id: &ConversationId,
    query: Option<&str>,
) -> Result<ManagementSnapshot, ManagementError> {
    match domain {
        "chat" => {
            let settings = runtime
                .chat_settings()
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            let chars = settings.instructions.as_str().chars().count();
            Ok(ManagementSnapshot::new(
                format!("Global instructions ({chars} characters)."),
                serde_json::to_value(&settings)
                    .map_err(|error| ManagementError::failed(error.to_string()))?,
            ))
        }
        "tools" => {
            let settings = runtime
                .tools_settings()
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ManagementSnapshot::new(
                format!(
                    "Workspace changes without asking: {}; new chats full access: {}; wake on command end: {}.",
                    settings.allow_changes_in_workspace,
                    settings.full_access_for_new_conversations,
                    settings.wake_on_command_end
                ),
                serde_json::to_value(&settings)
                    .map_err(|error| ManagementError::failed(error.to_string()))?,
            ))
        }
        "policy" => inspect_policy(runtime, conversation_id, query).await,
        "child_agents" => {
            let settings = runtime
                .child_agents_settings()
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ManagementSnapshot::new(
                format!(
                    "Maximum concurrent child agents: {}.",
                    settings.parallel_children.get()
                ),
                serde_json::to_value(&settings)
                    .map_err(|error| ManagementError::failed(error.to_string()))?,
            ))
        }
        other => Err(ManagementError::not_found(format!(
            "unknown core domain {other:?}"
        ))),
    }
}

async fn inspect_host_domain(
    host: &dyn AppHost,
    domain: &str,
) -> Result<ManagementSnapshot, ManagementError> {
    match domain {
        "interface" => {
            let settings = host
                .interface_settings()
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ManagementSnapshot::new(
                format!(
                    "Interface locale: {}.",
                    settings
                        .locale
                        .as_ref()
                        .map_or("system default", |tag| tag.as_str())
                ),
                serde_json::to_value(&settings)
                    .map_err(|error| ManagementError::failed(error.to_string()))?,
            ))
        }
        "window" => {
            let settings = host.window_settings();
            Ok(ManagementSnapshot::new(
                format!("Window close action: {:?}.", settings.close_action),
                serde_json::to_value(&settings)
                    .map_err(|error| ManagementError::failed(error.to_string()))?,
            ))
        }
        "notifications" => {
            let settings = host.notification_settings();
            Ok(ManagementSnapshot::new(
                format!(
                    "Desktop notifications enabled: {} (taskbar badge: {}, tray badge: {}).",
                    settings.enabled, settings.taskbar_badge, settings.tray_badge
                ),
                serde_json::to_value(settings)
                    .map_err(|error| ManagementError::failed(error.to_string()))?,
            ))
        }
        "updates" => {
            let status = host.update_status();
            let settings = host
                .updates_settings()
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ManagementSnapshot::new(
                format!(
                    "Version {}, automatic checks: {}, state: {:?}.",
                    status.current_version, settings.check_automatically, status.state
                ),
                json!({
                    "settings": settings,
                    "status": status,
                }),
            ))
        }
        other => Err(ManagementError::not_found(format!(
            "unknown core domain {other:?}"
        ))),
    }
}

async fn inspect_policy(
    runtime: &Runtime,
    conversation_id: &ConversationId,
    query: Option<&str>,
) -> Result<ManagementSnapshot, ManagementError> {
    let target_conversation = match query.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => ConversationId::parse(text)
            .map_err(|error| ManagementError::invalid_field("query", error.to_string()))?,
        None => conversation_id.clone(),
    };
    let command_rules = runtime
        .command_rules()
        .await
        .map_err(|error| ManagementError::failed(error.to_string()))?;
    let remote_rules = runtime
        .remote_command_rules()
        .await
        .map_err(|error| ManagementError::failed(error.to_string()))?;
    let program_rules = runtime
        .program_rules()
        .await
        .map_err(|error| ManagementError::failed(error.to_string()))?;
    let terminal_rules = runtime
        .terminal_command_rules()
        .await
        .map_err(|error| ManagementError::failed(error.to_string()))?;
    let shared_grants = runtime
        .shared_grants()
        .await
        .map_err(|error| ManagementError::failed(error.to_string()))?;
    let conversation_grants = runtime
        .conversation_grants(&target_conversation)
        .await
        .unwrap_or_default();

    Ok(ManagementSnapshot::new(
        format!(
            "{} command rule(s), {} remote rule(s), {} program rule(s), {} terminal rule(s), {} shared grant(s), {} conversation grant(s).",
            command_rules.len(),
            remote_rules.len(),
            program_rules.len(),
            terminal_rules.len(),
            shared_grants.len(),
            conversation_grants.len(),
        ),
        json!({
            "conversationId": target_conversation.as_str(),
            "commandRules": command_rules,
            "remoteCommandRules": remote_rules,
            "programRules": program_rules,
            "terminalCommandRules": terminal_rules,
            "sharedGrants": shared_grants,
            "conversationGrants": conversation_grants,
        }),
    ))
}

/// Validates and prepares one core configuration action for `Access::AppConfig`.
pub(crate) async fn prepare_core_action(
    runtime: &Runtime,
    host: &dyn AppHost,
    domain: &str,
    request: PrepareActionRequest,
) -> Result<PreparedAction, ManagementError> {
    match domain {
        "plugins" => prepare_plugins(host, request).await,
        "projects" => prepare_projects(runtime, request).await,
        "providers" => profiles_and_agents::prepare_providers(runtime, request).await,
        "search_profiles" => profiles_and_agents::prepare_search_profiles(runtime, request).await,
        "chatgpt" => prepare_chatgpt(runtime, &request),
        "agents" => profiles_and_agents::prepare_agents(runtime, request),
        "skills" => prepare_skills(request),
        "chat" => prepare_chat(request),
        "tools" => prepare_tools(runtime, request).await,
        "policy" => prepare_policy(request),
        "child_agents" => prepare_child_agents(request),
        "interface" => prepare_interface(request),
        "window" => prepare_window(request),
        "notifications" => prepare_notifications(host, request),
        "updates" => prepare_updates(request),
        other => Err(ManagementError::not_found(format!(
            "unknown core domain {other:?}"
        ))),
    }
}

/// Executes an owner-approved core configuration action.
pub(crate) async fn execute_core_action(
    runtime: &Runtime,
    host: &dyn AppHost,
    domain: &str,
    action: &str,
    arguments: Value,
    secrets: BTreeMap<String, Secret>,
) -> Result<ActionResult, ManagementError> {
    match domain {
        "plugins" => execute_plugins(host, action, arguments).await,
        "projects" => execute_projects(runtime, host, action, arguments).await,
        "providers" => {
            profiles_and_agents::execute_providers(runtime, action, arguments, secrets).await
        }
        "search_profiles" => {
            profiles_and_agents::execute_search_profiles(runtime, action, arguments, secrets).await
        }
        "chatgpt" => execute_chatgpt(runtime, action).await,
        "agents" => profiles_and_agents::execute_agents(runtime, action, arguments).await,
        "skills" => execute_skills(runtime, action, arguments).await,
        "chat" => execute_chat(runtime, action, arguments).await,
        "tools" => execute_tools(runtime, action, arguments).await,
        "policy" => execute_policy(runtime, action, arguments).await,
        "child_agents" => execute_child_agents(runtime, action, arguments).await,
        "interface" => execute_interface(host, action, arguments).await,
        "window" => execute_window(host, action, arguments).await,
        "notifications" => execute_notifications(host, action, arguments).await,
        "updates" => execute_updates(host, action, arguments).await,
        other => Err(ManagementError::not_found(format!(
            "unknown core domain {other:?}"
        ))),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetPluginEnabledArgs {
    plugin_id: String,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetFeatureEnabledArgs {
    plugin_id: String,
    feature_id: String,
    enabled: bool,
}

async fn prepare_plugins(
    host: &dyn AppHost,
    request: PrepareActionRequest,
) -> Result<PreparedAction, ManagementError> {
    let plugins = host.list_plugins();
    match request.action.as_str() {
        "set_enabled" => {
            let args: SetPluginEnabledArgs = parse_args(request.arguments)?;
            let plugin = plugins
                .iter()
                .find(|plugin| plugin.id == args.plugin_id)
                .ok_or_else(|| {
                    ManagementError::not_found(format!("unknown plugin {:?}", args.plugin_id))
                })?;
            let verb = if args.enabled { "Enable" } else { "Disable" };
            let mut fields = vec![
                field("Plugin", &plugin.id),
                field("Current state", plugin.state.as_str()),
                field("Target enabled", args.enabled.to_string()),
            ];
            let mut has_active_work = false;
            if !args.enabled
                && let Ok(work) = host.plugin_work(plugin.id.clone()).await
                && let Some(warning) = format_work_warning(&work.items)
            {
                has_active_work = true;
                fields.push(field("Active work interrupted", warning));
            }
            Ok(PreparedAction {
                title: format!("{verb} plugin {}", plugin.id),
                fields,
                secret_fields: Vec::new(),
                security_sensitive: args.enabled || has_active_work,
                canonical_arguments: json!({
                    "pluginId": plugin.id,
                    "enabled": args.enabled,
                }),
            })
        }
        "set_feature_enabled" => {
            let args: SetFeatureEnabledArgs = parse_args(request.arguments)?;
            let plugin = plugins
                .iter()
                .find(|plugin| plugin.id == args.plugin_id)
                .ok_or_else(|| {
                    ManagementError::not_found(format!("unknown plugin {:?}", args.plugin_id))
                })?;
            let feature = plugin
                .features
                .iter()
                .find(|feature| feature.id == args.feature_id)
                .ok_or_else(|| {
                    ManagementError::not_found(format!(
                        "plugin {:?} has no optional feature {:?}",
                        args.plugin_id, args.feature_id
                    ))
                })?;
            let verb = if args.enabled { "Enable" } else { "Disable" };
            let mut fields = vec![
                field("Plugin", &plugin.id),
                field("Feature", &feature.id),
                field("Current state", feature.state.as_str()),
                field("Target enabled", args.enabled.to_string()),
            ];
            let mut has_active_work = false;
            if !args.enabled
                && let Ok(work) = host
                    .plugin_feature_work(plugin.id.clone(), feature.id.clone())
                    .await
                && let Some(warning) = format_work_warning(&work.items)
            {
                has_active_work = true;
                fields.push(field("Active work interrupted", warning));
            }
            Ok(PreparedAction {
                title: format!("{verb} feature {}.{}", plugin.id, feature.id),
                fields,
                secret_fields: Vec::new(),
                security_sensitive: args.enabled || has_active_work,
                canonical_arguments: json!({
                    "pluginId": plugin.id,
                    "featureId": feature.id,
                    "enabled": args.enabled,
                }),
            })
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

async fn execute_plugins(
    host: &dyn AppHost,
    action: &str,
    arguments: Value,
) -> Result<ActionResult, ManagementError> {
    match action {
        "set_enabled" => {
            let args: SetPluginEnabledArgs = parse_args(arguments)?;
            let view = host
                .set_plugin_enabled(args.plugin_id.clone(), args.enabled)
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Plugin {} is now {} (enabled: {}).",
                view.id,
                view.state.as_str(),
                view.enabled
            ))
            .with_data(json!({ "plugin": view })))
        }
        "set_feature_enabled" => {
            let args: SetFeatureEnabledArgs = parse_args(arguments)?;
            let view = host
                .set_plugin_feature_enabled(
                    args.plugin_id.clone(),
                    args.feature_id.clone(),
                    args.enabled,
                )
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Updated optional feature {}.{} (enabled: {}).",
                args.plugin_id, args.feature_id, args.enabled
            ))
            .with_data(json!({ "plugin": view })))
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddProjectArgs {
    folder: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RenameProjectArgs {
    project_id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoveProjectArgs {
    project_id: String,
    #[serde(default)]
    conversations: Option<u32>,
}

async fn prepare_projects(
    runtime: &Runtime,
    request: PrepareActionRequest,
) -> Result<PreparedAction, ManagementError> {
    match request.action.as_str() {
        "add" => {
            let args: AddProjectArgs = parse_args(request.arguments)?;
            let folder = args.folder.trim();
            if folder.is_empty() {
                return Err(ManagementError::invalid_field(
                    "folder",
                    "folder cannot be empty",
                ));
            }
            Ok(PreparedAction {
                title: format!("Add project at {folder}"),
                fields: vec![field("Folder", folder)],
                secret_fields: Vec::new(),
                security_sensitive: false,
                canonical_arguments: json!({ "folder": folder }),
            })
        }
        "rename" => {
            let args: RenameProjectArgs = parse_args(request.arguments)?;
            let project_id = ProjectId::parse(&args.project_id)
                .map_err(|error| ManagementError::invalid_field("projectId", error.to_string()))?;
            let cleaned = check_project_name(&args.name)
                .map_err(|error| ManagementError::invalid_field("name", error.to_string()))?;
            let existing = runtime
                .inner
                .store
                .project(&project_id)
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?
                .ok_or_else(|| ManagementError::not_found("project not found"))?;
            Ok(PreparedAction {
                title: format!("Rename project {} to {cleaned}", existing.name),
                fields: vec![
                    field("Project ID", project_id.as_str()),
                    field("Current name", &existing.name),
                    field("New name", &cleaned),
                ],
                secret_fields: Vec::new(),
                security_sensitive: false,
                canonical_arguments: json!({
                    "projectId": project_id.as_str(),
                    "name": cleaned,
                }),
            })
        }
        "remove" => {
            let args: RemoveProjectArgs = parse_args(request.arguments)?;
            let project_id = ProjectId::parse(&args.project_id)
                .map_err(|error| ManagementError::invalid_field("projectId", error.to_string()))?;
            let existing = runtime
                .inner
                .store
                .project(&project_id)
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?
                .ok_or_else(|| ManagementError::not_found("project not found"))?;
            let conversations = runtime
                .inner
                .store
                .project_conversations(&project_id)
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            let draft = runtime
                .inner
                .store
                .draft_conversation(Some(&project_id), None)
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            let listed = conversations
                .iter()
                .filter(|id| Some(*id) != draft.as_ref())
                .count();
            let count = u32::try_from(listed).unwrap_or(u32::MAX);
            Ok(PreparedAction {
                title: format!("Remove project {}", existing.name),
                fields: vec![
                    field("Project", &existing.name),
                    field("Folder", &existing.folder),
                    field("Conversations deleted", count.to_string()),
                ],
                secret_fields: Vec::new(),
                security_sensitive: true,
                canonical_arguments: json!({
                    "projectId": project_id.as_str(),
                    "conversations": count,
                }),
            })
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

async fn execute_projects(
    runtime: &Runtime,
    host: &dyn AppHost,
    action: &str,
    arguments: Value,
) -> Result<ActionResult, ManagementError> {
    match action {
        "add" => {
            let args: AddProjectArgs = parse_args(arguments)?;
            let project = runtime
                .add_project(args.folder)
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Added project {} ({}) at {}.",
                project.name, project.id, project.folder
            ))
            .with_data(json!({ "project": project })))
        }
        "rename" => {
            let args: RenameProjectArgs = parse_args(arguments)?;
            let project_id = ProjectId::parse(&args.project_id)
                .map_err(|error| ManagementError::invalid_field("projectId", error.to_string()))?;
            let project = runtime
                .rename_project(RenameProject {
                    project_id,
                    name: args.name,
                })
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Renamed project {} to {}.",
                project.id, project.name
            ))
            .with_data(json!({ "project": project })))
        }
        "remove" => {
            let args: RemoveProjectArgs = parse_args(arguments)?;
            let project_id = ProjectId::parse(&args.project_id)
                .map_err(|error| ManagementError::invalid_field("projectId", error.to_string()))?;
            let deleted_ids = runtime
                .remove_project_ids(RemoveProject {
                    project_id: project_id.clone(),
                    conversations: args.conversations.unwrap_or(0),
                })
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            if !deleted_ids.is_empty() {
                host.conversations_deleted(deleted_ids).await;
            }
            Ok(ActionResult::new(format!(
                "Removed project {}.",
                project_id.as_str()
            )))
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

fn prepare_chatgpt(
    runtime: &Runtime,
    request: &PrepareActionRequest,
) -> Result<PreparedAction, ManagementError> {
    let view = runtime.chatgpt_account_view();
    match request.action.as_str() {
        "refresh_catalog" => Ok(PreparedAction {
            title: "Refresh ChatGPT model catalog".to_owned(),
            fields: vec![field("Account state", format!("{:?}", view.state))],
            secret_fields: Vec::new(),
            security_sensitive: false,
            canonical_arguments: json!({}),
        }),
        "sign_out" => Ok(PreparedAction {
            title: "Sign out of ChatGPT account".to_owned(),
            fields: vec![field("Account state", format!("{:?}", view.state))],
            secret_fields: Vec::new(),
            security_sensitive: true,
            canonical_arguments: json!({}),
        }),
        other => Err(ManagementError::unknown_action(other)),
    }
}

async fn execute_chatgpt(runtime: &Runtime, action: &str) -> Result<ActionResult, ManagementError> {
    match action {
        "refresh_catalog" => {
            runtime
                .refresh_chatgpt_catalog()
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            let models = runtime.chatgpt_models().await.unwrap_or_default();
            Ok(ActionResult::new(format!(
                "Refreshed ChatGPT catalog ({} model(s)).",
                models.len()
            ))
            .with_data(json!({ "models": models })))
        }
        "sign_out" => {
            let report = runtime
                .sign_out_chatgpt()
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new("Signed out of ChatGPT account.")
                .with_data(json!({ "report": report })))
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetSkillOnArgs {
    place: String,
    project_id: Option<String>,
    name: String,
    on: bool,
}

fn parse_skill_place(args: &SetSkillOnArgs) -> Result<SkillPlace, ManagementError> {
    match args.place.as_str() {
        "computer" => Ok(SkillPlace::Computer),
        "project" => {
            let id = args.project_id.as_deref().ok_or_else(|| {
                ManagementError::invalid_field(
                    "projectId",
                    "projectId is required for project skills",
                )
            })?;
            let project_id = ProjectId::parse(id)
                .map_err(|error| ManagementError::invalid_field("projectId", error.to_string()))?;
            Ok(SkillPlace::Project { project_id })
        }
        other => Err(ManagementError::invalid_field(
            "place",
            format!("invalid skill place {other:?}; expected 'computer' or 'project'"),
        )),
    }
}

fn prepare_skills(request: PrepareActionRequest) -> Result<PreparedAction, ManagementError> {
    if request.action != "set_on" {
        return Err(ManagementError::unknown_action(&request.action));
    }
    let args: SetSkillOnArgs = parse_args(request.arguments)?;
    if !is_skill_name(&args.name) {
        return Err(ManagementError::invalid_field("name", "invalid skill name"));
    }
    let place = parse_skill_place(&args)?;
    let place_label = match &place {
        SkillPlace::Computer => "computer (~/.agents/skills)".to_owned(),
        SkillPlace::Project { project_id } => format!("project {}", project_id.as_str()),
    };
    let verb = if args.on { "Enable" } else { "Disable" };
    Ok(PreparedAction {
        title: format!("{verb} skill {}", args.name),
        fields: vec![
            field("Skill", &args.name),
            field("Place", place_label),
            field("Enabled", args.on.to_string()),
        ],
        secret_fields: Vec::new(),
        security_sensitive: args.on,
        canonical_arguments: json!({
            "place": args.place,
            "projectId": args.project_id,
            "name": args.name,
            "on": args.on,
        }),
    })
}

async fn execute_skills(
    runtime: &Runtime,
    action: &str,
    arguments: Value,
) -> Result<ActionResult, ManagementError> {
    if action != "set_on" {
        return Err(ManagementError::unknown_action(action));
    }
    let args: SetSkillOnArgs = parse_args(arguments)?;
    let place = parse_skill_place(&args)?;
    let overview = runtime
        .set_skill_on(SetSkillOn {
            place,
            name: args.name.clone(),
            on: args.on,
        })
        .await
        .map_err(|error| ManagementError::failed(error.to_string()))?;
    Ok(ActionResult::new(format!(
        "Skill {} is now {}.",
        args.name,
        if args.on { "on" } else { "off" }
    ))
    .with_data(json!({ "overview": overview })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateChatArgs {
    instructions: String,
}

fn prepare_chat(request: PrepareActionRequest) -> Result<PreparedAction, ManagementError> {
    if request.action != "update_settings" {
        return Err(ManagementError::unknown_action(&request.action));
    }
    let args: UpdateChatArgs = parse_args(request.arguments)?;
    let checked = Instructions::parse(args.instructions)
        .map_err(|error| ManagementError::invalid_field("instructions", error.to_string()))?;
    let chars = checked.as_str().chars().count();
    Ok(PreparedAction {
        title: "Update global owner instructions".to_owned(),
        fields: vec![field("Length", format!("{chars} characters"))],
        secret_fields: Vec::new(),
        security_sensitive: true,
        canonical_arguments: json!({ "instructions": checked.as_str() }),
    })
}

async fn execute_chat(
    runtime: &Runtime,
    action: &str,
    arguments: Value,
) -> Result<ActionResult, ManagementError> {
    if action != "update_settings" {
        return Err(ManagementError::unknown_action(action));
    }
    let args: UpdateChatArgs = parse_args(arguments)?;
    let saved = runtime
        .update_chat_settings(UpdateChatSettings {
            instructions: args.instructions,
        })
        .await
        .map_err(|error| ManagementError::failed(error.to_string()))?;
    Ok(ActionResult::new("Updated global chat instructions.")
        .with_data(json!({ "settings": saved })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateToolsArgs {
    allow_changes_in_workspace: Option<bool>,
    command_environment: Option<Vec<String>>,
    full_access_for_new_conversations: Option<bool>,
    wake_on_command_end: Option<bool>,
}

async fn prepare_tools(
    runtime: &Runtime,
    request: PrepareActionRequest,
) -> Result<PreparedAction, ManagementError> {
    if request.action != "update_settings" {
        return Err(ManagementError::unknown_action(&request.action));
    }
    let args: UpdateToolsArgs = parse_args(request.arguments)?;
    let current = runtime
        .tools_settings()
        .await
        .map_err(|error| ManagementError::failed(error.to_string()))?;
    let allow_changes_in_workspace = args
        .allow_changes_in_workspace
        .unwrap_or(current.allow_changes_in_workspace);
    let command_environment = args.command_environment.unwrap_or_else(|| {
        current
            .command_environment
            .iter()
            .map(|name| name.as_str().to_owned())
            .collect()
    });
    let full_access_for_new_conversations = args
        .full_access_for_new_conversations
        .unwrap_or(current.full_access_for_new_conversations);
    let wake_on_command_end = args
        .wake_on_command_end
        .unwrap_or(current.wake_on_command_end);
    let checked = UpdateToolsSettings {
        allow_changes_in_workspace,
        command_environment,
        full_access_for_new_conversations,
        wake_on_command_end,
    }
    .into_settings()
    .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
    let env_names: Vec<String> = checked
        .command_environment
        .iter()
        .map(|name| name.as_str().to_owned())
        .collect();
    Ok(PreparedAction {
        title: "Update system tools settings".to_owned(),
        fields: vec![
            field(
                "Allow workspace changes without asking",
                checked.allow_changes_in_workspace.to_string(),
            ),
            field(
                "Full access for new conversations",
                checked.full_access_for_new_conversations.to_string(),
            ),
            field(
                "Wake on command end",
                checked.wake_on_command_end.to_string(),
            ),
            field(
                "Extra command env vars",
                if env_names.is_empty() {
                    "(none)".to_owned()
                } else {
                    env_names.join(", ")
                },
            ),
        ],
        secret_fields: Vec::new(),
        security_sensitive: true,
        canonical_arguments: json!({
            "allowChangesInWorkspace": checked.allow_changes_in_workspace,
            "commandEnvironment": env_names,
            "fullAccessForNewConversations": checked.full_access_for_new_conversations,
            "wakeOnCommandEnd": checked.wake_on_command_end,
        }),
    })
}

async fn execute_tools(
    runtime: &Runtime,
    action: &str,
    arguments: Value,
) -> Result<ActionResult, ManagementError> {
    if action != "update_settings" {
        return Err(ManagementError::unknown_action(action));
    }
    let update: UpdateToolsSettings = parse_args(arguments)?;
    let saved = runtime
        .update_tools_settings(update)
        .await
        .map_err(|error| ManagementError::failed(error.to_string()))?;
    Ok(ActionResult::new("Updated system tools settings.").with_data(json!({ "settings": saved })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuleIdArg {
    rule_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GrantIdArg {
    grant_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConversationGrantArg {
    conversation_id: String,
    grant_id: String,
}

fn prepare_policy(request: PrepareActionRequest) -> Result<PreparedAction, ManagementError> {
    match request.action.as_str() {
        "delete_command_rule"
        | "delete_remote_command_rule"
        | "delete_program_rule"
        | "delete_terminal_command_rule" => {
            let args: RuleIdArg = parse_args(request.arguments)?;
            let id = RuleId::parse(&args.rule_id)
                .map_err(|error| ManagementError::invalid_field("ruleId", error.to_string()))?;
            Ok(PreparedAction {
                title: format!("Delete policy rule {}", id.as_str()),
                fields: vec![
                    field("Rule kind", &request.action),
                    field("Rule ID", id.as_str()),
                ],
                secret_fields: Vec::new(),
                security_sensitive: false,
                canonical_arguments: json!({ "ruleId": id.as_str() }),
            })
        }
        "revoke_shared_grant" => {
            let args: GrantIdArg = parse_args(request.arguments)?;
            let id = GrantId::parse(&args.grant_id)
                .map_err(|error| ManagementError::invalid_field("grantId", error.to_string()))?;
            Ok(PreparedAction {
                title: format!("Revoke shared grant {}", id.as_str()),
                fields: vec![field("Grant ID", id.as_str())],
                secret_fields: Vec::new(),
                security_sensitive: false,
                canonical_arguments: json!({ "grantId": id.as_str() }),
            })
        }
        "revoke_conversation_grant" => {
            let args: ConversationGrantArg = parse_args(request.arguments)?;
            let conversation_id =
                ConversationId::parse(&args.conversation_id).map_err(|error| {
                    ManagementError::invalid_field("conversationId", error.to_string())
                })?;
            let grant_id = GrantId::parse(&args.grant_id)
                .map_err(|error| ManagementError::invalid_field("grantId", error.to_string()))?;
            Ok(PreparedAction {
                title: format!("Revoke conversation grant {}", grant_id.as_str()),
                fields: vec![
                    field("Conversation ID", conversation_id.as_str()),
                    field("Grant ID", grant_id.as_str()),
                ],
                secret_fields: Vec::new(),
                security_sensitive: false,
                canonical_arguments: json!({
                    "conversationId": conversation_id.as_str(),
                    "grantId": grant_id.as_str(),
                }),
            })
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

async fn execute_policy(
    runtime: &Runtime,
    action: &str,
    arguments: Value,
) -> Result<ActionResult, ManagementError> {
    match action {
        "delete_command_rule" => {
            let args: RuleIdArg = parse_args(arguments)?;
            let rule_id = RuleId::parse(&args.rule_id)
                .map_err(|error| ManagementError::invalid_field("ruleId", error.to_string()))?;
            runtime
                .delete_command_rule(RuleRef {
                    rule_id: rule_id.clone(),
                })
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Deleted command rule {}.",
                rule_id.as_str()
            )))
        }
        "delete_remote_command_rule" => {
            let args: RuleIdArg = parse_args(arguments)?;
            let rule_id = RuleId::parse(&args.rule_id)
                .map_err(|error| ManagementError::invalid_field("ruleId", error.to_string()))?;
            runtime
                .delete_remote_command_rule(RemoteRuleRef {
                    rule_id: rule_id.clone(),
                })
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Deleted remote command rule {}.",
                rule_id.as_str()
            )))
        }
        "delete_program_rule" => {
            let args: RuleIdArg = parse_args(arguments)?;
            let rule_id = RuleId::parse(&args.rule_id)
                .map_err(|error| ManagementError::invalid_field("ruleId", error.to_string()))?;
            runtime
                .delete_program_rule(ProgramRuleRef {
                    rule_id: rule_id.clone(),
                })
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Deleted program rule {}.",
                rule_id.as_str()
            )))
        }
        "delete_terminal_command_rule" => {
            let args: RuleIdArg = parse_args(arguments)?;
            let rule_id = RuleId::parse(&args.rule_id)
                .map_err(|error| ManagementError::invalid_field("ruleId", error.to_string()))?;
            runtime
                .delete_terminal_command_rule(TerminalRuleRef {
                    rule_id: rule_id.clone(),
                })
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Deleted terminal command rule {}.",
                rule_id.as_str()
            )))
        }
        "revoke_shared_grant" => {
            let args: GrantIdArg = parse_args(arguments)?;
            let grant_id = GrantId::parse(&args.grant_id)
                .map_err(|error| ManagementError::invalid_field("grantId", error.to_string()))?;
            runtime
                .revoke_shared_grant(SharedGrantRef {
                    grant_id: grant_id.clone(),
                })
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Revoked shared grant {}.",
                grant_id.as_str()
            )))
        }
        "revoke_conversation_grant" => {
            let args: ConversationGrantArg = parse_args(arguments)?;
            let conversation_id =
                ConversationId::parse(&args.conversation_id).map_err(|error| {
                    ManagementError::invalid_field("conversationId", error.to_string())
                })?;
            let grant_id = GrantId::parse(&args.grant_id)
                .map_err(|error| ManagementError::invalid_field("grantId", error.to_string()))?;
            runtime
                .revoke_conversation_grant(GrantRef {
                    conversation_id,
                    grant_id: grant_id.clone(),
                })
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Revoked conversation grant {}.",
                grant_id.as_str()
            )))
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateChildAgentsArgs {
    parallel_children: u32,
}

fn prepare_child_agents(request: PrepareActionRequest) -> Result<PreparedAction, ManagementError> {
    if request.action != "update_settings" {
        return Err(ManagementError::unknown_action(&request.action));
    }
    let args: UpdateChildAgentsArgs = parse_args(request.arguments)?;
    let checked = ParallelChildren::new(args.parallel_children)
        .map_err(|error| ManagementError::invalid_field("parallelChildren", error.to_string()))?;
    Ok(PreparedAction {
        title: format!("Set concurrent child agent limit to {}", checked.get()),
        fields: vec![field("Parallel children", checked.get().to_string())],
        secret_fields: Vec::new(),
        security_sensitive: false,
        canonical_arguments: json!({ "parallelChildren": checked.get() }),
    })
}

async fn execute_child_agents(
    runtime: &Runtime,
    action: &str,
    arguments: Value,
) -> Result<ActionResult, ManagementError> {
    if action != "update_settings" {
        return Err(ManagementError::unknown_action(action));
    }
    let args: UpdateChildAgentsArgs = parse_args(arguments)?;
    let saved = runtime
        .update_child_agents_settings(UpdateChildAgentsSettings {
            parallel_children: args.parallel_children,
        })
        .await
        .map_err(|error| ManagementError::failed(error.to_string()))?;
    Ok(ActionResult::new(format!(
        "Updated parallel child agent limit to {}.",
        saved.parallel_children.get()
    ))
    .with_data(json!({ "settings": saved })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateInterfaceArgs {
    locale: Option<String>,
}

fn prepare_interface(request: PrepareActionRequest) -> Result<PreparedAction, ManagementError> {
    if request.action != "update_settings" {
        return Err(ManagementError::unknown_action(&request.action));
    }
    let args: UpdateInterfaceArgs = parse_args(request.arguments)?;
    let checked = UpdateInterfaceSettings {
        locale: args.locale,
    }
    .into_settings()
    .map_err(|error| ManagementError::invalid_field("locale", error.to_string()))?;
    let locale_text = checked.locale.as_ref().map(|tag| tag.as_str().to_owned());
    Ok(PreparedAction {
        title: "Update interface locale".to_owned(),
        fields: vec![field(
            "Locale",
            locale_text.as_deref().unwrap_or("system default"),
        )],
        secret_fields: Vec::new(),
        security_sensitive: false,
        canonical_arguments: json!({ "locale": locale_text }),
    })
}

async fn execute_interface(
    host: &dyn AppHost,
    action: &str,
    arguments: Value,
) -> Result<ActionResult, ManagementError> {
    if action != "update_settings" {
        return Err(ManagementError::unknown_action(action));
    }
    let args: UpdateInterfaceArgs = parse_args(arguments)?;
    let saved = host
        .update_interface_settings(UpdateInterfaceSettings {
            locale: args.locale,
        })
        .await
        .map_err(|error| ManagementError::failed(error.to_string()))?;
    Ok(ActionResult::new("Updated interface settings.").with_data(json!({ "settings": saved })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateWindowArgs {
    close_action: CloseAction,
}

fn prepare_window(request: PrepareActionRequest) -> Result<PreparedAction, ManagementError> {
    if request.action != "update_settings" {
        return Err(ManagementError::unknown_action(&request.action));
    }
    let args: UpdateWindowArgs = parse_args(request.arguments)?;
    Ok(PreparedAction {
        title: "Update window close behavior".to_owned(),
        fields: vec![field("Close action", format!("{:?}", args.close_action))],
        secret_fields: Vec::new(),
        security_sensitive: false,
        canonical_arguments: json!({ "closeAction": args.close_action }),
    })
}

async fn execute_window(
    host: &dyn AppHost,
    action: &str,
    arguments: Value,
) -> Result<ActionResult, ManagementError> {
    if action != "update_settings" {
        return Err(ManagementError::unknown_action(action));
    }
    let args: UpdateWindowArgs = parse_args(arguments)?;
    let saved = host
        .update_window_settings(UpdateWindowSettings {
            close_action: args.close_action,
        })
        .await
        .map_err(|error| ManagementError::failed(error.to_string()))?;
    Ok(ActionResult::new(format!(
        "Updated window close action to {:?}.",
        saved.close_action
    ))
    .with_data(json!({ "settings": saved })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateNotificationArgs {
    enabled: Option<bool>,
    on_question: Option<bool>,
    on_approval: Option<bool>,
    on_completion: Option<bool>,
    on_failure: Option<bool>,
    taskbar_badge: Option<bool>,
    tray_badge: Option<bool>,
}

fn prepare_notifications(
    host: &dyn AppHost,
    request: PrepareActionRequest,
) -> Result<PreparedAction, ManagementError> {
    if request.action != "update_settings" {
        return Err(ManagementError::unknown_action(&request.action));
    }
    let args: UpdateNotificationArgs = parse_args(request.arguments)?;
    let current = host.notification_settings();
    let enabled = args.enabled.unwrap_or(current.enabled);
    let on_question = args.on_question.unwrap_or(current.on_question);
    let on_approval = args.on_approval.unwrap_or(current.on_approval);
    let on_completion = args.on_completion.unwrap_or(current.on_completion);
    let on_failure = args.on_failure.unwrap_or(current.on_failure);
    let taskbar_badge = args.taskbar_badge.unwrap_or(current.taskbar_badge);
    let tray_badge = args.tray_badge.unwrap_or(current.tray_badge);
    Ok(PreparedAction {
        title: "Update notification settings".to_owned(),
        fields: vec![
            field("Notifications enabled", enabled.to_string()),
            field(
                "On question / approval",
                format!("{on_question} / {on_approval}"),
            ),
            field(
                "On completion / failure",
                format!("{on_completion} / {on_failure}"),
            ),
            field(
                "Taskbar / tray badge",
                format!("{taskbar_badge} / {tray_badge}"),
            ),
        ],
        secret_fields: Vec::new(),
        security_sensitive: false,
        canonical_arguments: json!({
            "enabled": enabled,
            "onQuestion": on_question,
            "onApproval": on_approval,
            "onCompletion": on_completion,
            "onFailure": on_failure,
            "taskbarBadge": taskbar_badge,
            "trayBadge": tray_badge,
        }),
    })
}

async fn execute_notifications(
    host: &dyn AppHost,
    action: &str,
    arguments: Value,
) -> Result<ActionResult, ManagementError> {
    if action != "update_settings" {
        return Err(ManagementError::unknown_action(action));
    }
    let update: UpdateNotificationSettings = parse_args(arguments)?;
    let saved = host
        .update_notification_settings(update)
        .await
        .map_err(|error| ManagementError::failed(error.to_string()))?;
    Ok(ActionResult::new("Updated notification settings.").with_data(json!({ "settings": saved })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateUpdatesArgs {
    check_automatically: bool,
}

fn prepare_updates(request: PrepareActionRequest) -> Result<PreparedAction, ManagementError> {
    match request.action.as_str() {
        "update_settings" => {
            let args: UpdateUpdatesArgs = parse_args(request.arguments)?;
            Ok(PreparedAction {
                title: format!(
                    "{} automatic update checks",
                    if args.check_automatically {
                        "Enable"
                    } else {
                        "Disable"
                    }
                ),
                fields: vec![field(
                    "Check automatically",
                    args.check_automatically.to_string(),
                )],
                secret_fields: Vec::new(),
                security_sensitive: false,
                canonical_arguments: json!({ "checkAutomatically": args.check_automatically }),
            })
        }
        "check_for_updates" => Ok(PreparedAction {
            title: "Check for Mannie updates".to_owned(),
            fields: vec![field(
                "Action",
                "Check GitHub releases for newer signed installer",
            )],
            secret_fields: Vec::new(),
            security_sensitive: false,
            canonical_arguments: json!({}),
        }),
        other => Err(ManagementError::unknown_action(other)),
    }
}

async fn execute_updates(
    host: &dyn AppHost,
    action: &str,
    arguments: Value,
) -> Result<ActionResult, ManagementError> {
    match action {
        "update_settings" => {
            let args: UpdateUpdatesArgs = parse_args(arguments)?;
            let saved = host
                .update_updates_settings(UpdateUpdatesSettings {
                    check_automatically: args.check_automatically,
                })
                .await
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new(format!(
                "Automatic update checks are now {}.",
                if saved.check_automatically {
                    "on"
                } else {
                    "off"
                }
            ))
            .with_data(json!({ "settings": saved })))
        }
        "check_for_updates" => {
            host.check_for_updates()
                .map_err(|error| ManagementError::failed(error.to_string()))?;
            Ok(ActionResult::new("Started checking for updates."))
        }
        other => Err(ManagementError::unknown_action(other)),
    }
}
