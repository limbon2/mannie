//! The three runtime-owned self-management tools: `mannie_inspect`,
//! `mannie_read_logs`, and `mannie_configure` (ADR-0081).

use std::collections::BTreeMap;
use std::sync::Arc;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_plugin_sdk::points::management::{
    ExecuteActionRequest, ManagementError, PrepareActionRequest, check_prepared_action,
};
use mannie_tools::{Done, Execution, PermissionClass, PrepareError, Prepared, Tool, ToolContext};
use mannie_types::plugins::PluginState;
use mannie_types::tools::{Access, ToolOutcome};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::Runtime;
use crate::self_management::{
    AppHost, MANNIE_CONFIGURE, MANNIE_INSPECT, MANNIE_READ_LOGS, core_domains,
    has_misplaced_secret_marker, logs,
};

/// Truncates JSON text cleanly to `limit` bytes so tool output never overflows
/// the run's result limit.
fn bounded_json(value: &Value, limit: usize) -> String {
    let rendered = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_owned());
    if rendered.len() <= limit {
        return rendered;
    }
    let compact = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_owned());
    if compact.len() <= limit {
        return compact;
    }
    let suffix = "\n...[output truncated to result limit]";
    let keep = limit.saturating_sub(suffix.len());
    let mut end = keep.min(compact.len());
    while end > 0 && !compact.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{suffix}", &compact[..end])
}

/// Canonicalizes a JSON value by sorting object keys recursively so the
/// approval digest is deterministic across prepares.
fn canonical_json_string(value: &Value) -> String {
    serde_json::to_string(&sort_value(value)).unwrap_or_else(|_| "{}".to_owned())
}

fn sort_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<&String, Value> = map
                .iter()
                .map(|(key, entry)| (key, sort_value(entry)))
                .collect();
            let mut out = Map::with_capacity(sorted.len());
            for (key, entry) in sorted {
                out.insert(key.clone(), entry);
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(sort_value).collect()),
        other => other.clone(),
    }
}

fn plugin_unavailable_hint(host: &dyn AppHost, target: &str) -> String {
    let plugins = host.list_plugins();
    if let Some((plugin_id, feature_id)) = target.split_once('.')
        && let Some(plugin) = plugins.iter().find(|plugin| plugin.id == plugin_id)
        && let Some(feature) = plugin
            .features
            .iter()
            .find(|feature| feature.id == feature_id)
    {
        if plugin.state != PluginState::On {
            return format!(
                "Plugin {plugin_id:?} is currently {}. Enable it with mannie_configure(target=\"plugins\", action=\"set_enabled\", arguments={{\"pluginId\":\"{plugin_id}\",\"enabled\":true}}) first, then enable feature {feature_id:?}.",
                plugin.state.as_str()
            );
        }
        if feature.state != PluginState::On {
            return format!(
                "Optional feature {target:?} is currently {}. Enable it with mannie_configure(target=\"plugins\", action=\"set_feature_enabled\", arguments={{\"pluginId\":\"{plugin_id}\",\"featureId\":\"{feature_id}\",\"enabled\":true}}) first; once enabled it is immediately available on the next step.",
                feature.state.as_str()
            );
        }
    }
    if let Some(plugin) = plugins.iter().find(|plugin| plugin.id == target) {
        return match plugin.state {
            PluginState::Off => format!(
                "Plugin {target:?} is currently off. Enable it with mannie_configure(target=\"plugins\", action=\"set_enabled\", arguments={{\"pluginId\":\"{target}\",\"enabled\":true}}) first; once enabled, its management provider becomes available on the very next step."
            ),
            PluginState::Failed => format!(
                "Plugin {target:?} failed to start (failure: {:?}). Inspect logs with mannie_read_logs to diagnose why it failed.",
                plugin.failure
            ),
            other => format!(
                "Plugin {target:?} is {} and has no live management provider registered.",
                other.as_str()
            ),
        };
    }
    let installed: Vec<&str> = plugins.iter().map(|plugin| plugin.id.as_str()).collect();
    format!(
        "No core domain or plugin named {target:?} was found. Core domains: {}. Installed plugins: {}.",
        core_domains::CORE_DOMAINS.join(", "),
        installed.join(", ")
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InspectArgs {
    topic: String,
    target: Option<String>,
    query: Option<String>,
}

/// Read-only tool `mannie_inspect`.
#[derive(Debug)]
pub(crate) struct MannieInspectTool {
    runtime: Runtime,
    host: Arc<dyn AppHost>,
    schema: Value,
}

impl MannieInspectTool {
    pub(crate) fn new(runtime: Runtime, host: Arc<dyn AppHost>) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "topic": {
                    "type": "string",
                    "enum": ["overview", "storage", "policy", "plugins", "troubleshooting", "core", "plugin", "actions"],
                    "description": "What to inspect: 'overview' for Mannie's architecture, version and paths; 'storage' for exact paths and SQLite module versions; 'policy' for rules and grants; 'plugins' for all installed plugins and states; 'troubleshooting' for diagnostics and log file list; 'core' for a core domain snapshot (pass target); 'plugin' for a live plugin guide and snapshot (pass target); 'actions' for action JSON schemas (optional target)."
                },
                "target": {
                    "type": ["string", "null"],
                    "description": "Core domain name (such as 'providers', 'plugins', 'projects') or plugin/feature ID (such as 'browser', 'ssh', 'mcp', 'workers.databases'). Pass null when inspecting top-level topics."
                },
                "query": {
                    "type": ["string", "null"],
                    "description": "Optional selector for plugin or policy inspection (for example 'tools:<server-id>', 'log:<server-id>', 'locate:<program>', or a conversation ID). Pass null when unused."
                }
            },
            "required": ["topic", "target", "query"],
            "additionalProperties": false
        });
        Self {
            runtime,
            host,
            schema,
        }
    }
}

impl Tool for MannieInspectTool {
    fn name(&self) -> &str {
        MANNIE_INSPECT
    }

    fn description(&self) -> &'static str {
        "Inspect Mannie's own paths, storage layout, SQLite module versions, plugins (including off or failed ones), secret-free core settings, live plugin guides/snapshots, and action schemas. Read-only and never prompts."
    }

    fn parameters(&self) -> &Value {
        &self.schema
    }

    fn class(&self) -> PermissionClass {
        PermissionClass::ReadOnly
    }

    fn prepare(
        &self,
        _context: ToolContext,
        arguments: String,
    ) -> BoxFuture<'static, Result<Prepared, PrepareError>> {
        async move {
            let args: InspectArgs = serde_json::from_str(&arguments).map_err(|error| {
                PrepareError::InvalidArguments {
                    detail: error.to_string(),
                }
            })?;
            let valid_topics = [
                "overview",
                "storage",
                "policy",
                "plugins",
                "troubleshooting",
                "core",
                "plugin",
                "actions",
            ];
            if !valid_topics.contains(&args.topic.as_str()) {
                return Err(PrepareError::InvalidArguments {
                    detail: format!("unknown topic {:?}", args.topic),
                });
            }
            if args.topic == "plugin"
                && args
                    .target
                    .as_deref()
                    .map(str::trim)
                    .is_none_or(str::is_empty)
            {
                return Err(PrepareError::InvalidArguments {
                    detail: "topic 'plugin' requires a non-empty target plugin or feature ID"
                        .to_owned(),
                });
            }
            Ok(Prepared {
                accesses: Vec::new(),
            })
        }
        .boxed()
    }

    fn execute(&self, execution: Execution) -> BoxFuture<'static, Done> {
        let runtime = self.runtime.clone();
        let host = Arc::clone(&self.host);
        async move {
            let args: InspectArgs = match serde_json::from_str(&execution.arguments) {
                Ok(args) => args,
                Err(error) => {
                    return Done {
                        outcome: ToolOutcome::MannieInspect {
                            topic: "invalid".to_owned(),
                            target: None,
                        },
                        output: format!("Invalid mannie_inspect arguments: {error}"),
                        archives: Vec::new(),
                        images: Vec::new(),
                        cited: Vec::new(),
                    };
                }
            };
            let topic = args.topic.clone();
            let target = args
                .target
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned);
            let query = args
                .query
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned);

            let value = inspect_payload(
                &runtime,
                host.as_ref(),
                &execution.context.conversation_id,
                &topic,
                target.as_deref(),
                query.as_deref(),
            )
            .await;

            Done {
                outcome: ToolOutcome::MannieInspect { topic, target },
                output: bounded_json(&value, execution.context.result_limit),
                archives: Vec::new(),
                images: Vec::new(),
                cited: Vec::new(),
            }
        }
        .boxed()
    }
}

async fn inspect_overview_or_storage(runtime: &Runtime, host: &dyn AppHost, topic: &str) -> Value {
    let info = host.app_info();
    if topic == "overview" {
        let plugins = host.list_plugins();
        let diagnostics = runtime.inner.store.diagnostics().await.ok();
        let live_managers: Vec<String> = runtime
            .management_providers()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        return json!({
            "guide": core_domains::topic_guide("overview"),
            "app": info,
            "database": diagnostics.map(|diag| json!({
                "path": diag.database_path,
                "journalMode": diag.journal_mode,
                "pageSize": diag.page_size,
                "pageCount": diag.page_count,
                "moduleCount": diag.module_versions.len(),
            })),
            "coreDomains": core_domains::CORE_DOMAINS,
            "plugins": plugins,
            "liveManagementProviders": live_managers,
        });
    }

    let diagnostics = runtime.inner.store.diagnostics().await;
    let log_files = logs::list_log_files(&info.log_dir).unwrap_or_default();
    json!({
        "guide": core_domains::topic_guide("storage"),
        "paths": {
            "dataDir": info.data_dir,
            "databasePath": info.database_path,
            "backupsDir": info.backups_dir,
            "logDir": info.log_dir,
            "skillsDir": info.skills_dir,
            "browserDataDir": info.browser_data_dir,
            "credentialStore": info.credential_store,
        },
        "database": match diagnostics {
            Ok(diag) => json!({
                "journalMode": diag.journal_mode,
                "pageSize": diag.page_size,
                "pageCount": diag.page_count,
                "approxBytes": diag.page_size.saturating_mul(diag.page_count),
                "modules": diag.module_versions.into_iter().map(|(name, version)| json!({ "module": name, "version": version })).collect::<Vec<_>>(),
            }),
            Err(error) => json!({ "error": error.to_string() }),
        },
        "logFiles": log_files,
    })
}

async fn inspect_troubleshooting(runtime: &Runtime, host: &dyn AppHost) -> Value {
    let info = host.app_info();
    let plugins = host.list_plugins();
    let failed_plugins: Vec<_> = plugins
        .iter()
        .filter(|plugin| plugin.state == PluginState::Failed)
        .collect();
    let log_files = logs::list_log_files(&info.log_dir).unwrap_or_default();
    let update_status = host.update_status();
    let diagnostics = runtime.inner.store.diagnostics().await.ok();
    json!({
        "guide": core_domains::topic_guide("troubleshooting"),
        "failedPlugins": failed_plugins,
        "updateStatus": update_status,
        "logDir": info.log_dir,
        "logFiles": log_files,
        "database": diagnostics.map(|diag| json!({
            "path": diag.database_path,
            "journalMode": diag.journal_mode,
            "moduleCount": diag.module_versions.len(),
        })),
    })
}

async fn inspect_core_topic(
    runtime: &Runtime,
    host: &dyn AppHost,
    conversation_id: &mannie_types::ids::ConversationId,
    target: Option<&str>,
    query: Option<&str>,
) -> Value {
    if let Some(domain) = target {
        let Some(guide) = core_domains::domain_guide(domain) else {
            return json!({
                "error": format!("unknown core domain {domain:?}"),
                "coreDomains": core_domains::CORE_DOMAINS,
            });
        };
        return match core_domains::inspect_core_domain(
            runtime,
            host,
            domain,
            conversation_id,
            query,
        )
        .await
        {
            Ok(snapshot) => json!({
                "domain": domain,
                "guide": guide,
                "snapshot": snapshot,
            }),
            Err(error) => json!({
                "domain": domain,
                "guide": guide,
                "error": error,
            }),
        };
    }

    let domains: Vec<Value> = core_domains::all_domain_guides()
        .into_iter()
        .map(|(name, guide)| {
            let actions: Vec<String> = guide
                .actions
                .into_iter()
                .map(|action| action.name)
                .collect();
            json!({
                "domain": name,
                "summary": guide.summary,
                "actions": actions,
            })
        })
        .collect();
    json!({ "coreDomains": domains })
}

fn inspect_actions_topic(runtime: &Runtime, host: &dyn AppHost, target: Option<&str>) -> Value {
    match target {
        Some(name) if core_domains::is_core_domain(name) => json!({
            "owner": "core",
            "target": name,
            "guide": core_domains::domain_guide(name),
        }),
        Some(name) => match runtime.management_provider(name) {
            Some(provider) => json!({
                "owner": name,
                "target": name,
                "guide": provider.guide(),
            }),
            None => json!({
                "error": plugin_unavailable_hint(host, name),
            }),
        },
        None => {
            let core: Vec<Value> = core_domains::all_domain_guides()
                .into_iter()
                .map(|(name, guide)| {
                    json!({
                        "target": name,
                        "owner": "core",
                        "actions": guide.actions.into_iter().map(|action| action.name).collect::<Vec<_>>(),
                    })
                })
                .collect();
            let plugins: Vec<Value> = runtime
                .management_providers()
                .into_iter()
                .map(|(id, provider)| {
                    let guide = provider.guide();
                    json!({
                        "target": id,
                        "owner": id,
                        "actions": guide.actions.into_iter().map(|action| action.name).collect::<Vec<_>>(),
                    })
                })
                .collect();
            json!({
                "core": core,
                "livePlugins": plugins,
            })
        }
    }
}

async fn inspect_payload(
    runtime: &Runtime,
    host: &dyn AppHost,
    conversation_id: &mannie_types::ids::ConversationId,
    topic: &str,
    target: Option<&str>,
    query: Option<&str>,
) -> Value {
    match topic {
        "overview" | "storage" => inspect_overview_or_storage(runtime, host, topic).await,
        "policy" => {
            let snapshot =
                core_domains::inspect_core_domain(runtime, host, "policy", conversation_id, query)
                    .await;
            let tools_settings = runtime.tools_settings().await.ok();
            json!({
                "guide": core_domains::topic_guide("policy"),
                "toolsSettings": tools_settings,
                "policy": match snapshot {
                    Ok(snapshot) => json!({ "summary": snapshot.summary, "data": snapshot.data }),
                    Err(error) => json!({ "error": error }),
                },
            })
        }
        "plugins" => {
            let plugins = host.list_plugins();
            let live_managers: Vec<String> = runtime
                .management_providers()
                .into_iter()
                .map(|(id, _)| id)
                .collect();
            json!({
                "guide": core_domains::topic_guide("plugins"),
                "plugins": plugins,
                "liveManagementProviders": live_managers,
            })
        }
        "troubleshooting" => inspect_troubleshooting(runtime, host).await,
        "core" => inspect_core_topic(runtime, host, conversation_id, target, query).await,
        "plugin" => {
            let Some(id) = target else {
                return json!({ "error": "target plugin ID is required" });
            };
            let Some(provider) = runtime.management_provider(id) else {
                return json!({
                    "error": plugin_unavailable_hint(host, id),
                });
            };
            let guide = provider.guide();
            match provider.inspect(query.map(str::to_owned)).await {
                Ok(snapshot) => json!({
                    "target": id,
                    "guide": guide,
                    "snapshot": snapshot,
                }),
                Err(error) => json!({
                    "target": id,
                    "guide": guide,
                    "error": error,
                }),
            }
        }
        "actions" => inspect_actions_topic(runtime, host, target),
        other => json!({ "error": format!("unknown topic {other:?}") }),
    }
}

/// Read-only bounded log reader tool `mannie_read_logs`.
#[derive(Debug)]
pub(crate) struct MannieReadLogsTool {
    host: Arc<dyn AppHost>,
    schema: Value,
}

impl MannieReadLogsTool {
    pub(crate) fn new(host: Arc<dyn AppHost>) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "file": {
                    "type": ["string", "null"],
                    "description": "Optional Mannie log file name inside Mannie's log directory (pass null to read the newest daily log)."
                },
                "level": {
                    "type": ["string", "null"],
                    "enum": ["trace", "debug", "info", "warn", "error", null],
                    "description": "Optional minimum severity filter. Pass null for all levels."
                },
                "targetPrefix": {
                    "type": ["string", "null"],
                    "description": "Optional tracing target prefix (for example 'mannie_ssh' or 'mannie_mcp'). Pass null for all targets."
                },
                "contains": {
                    "type": ["string", "null"],
                    "description": "Optional case-insensitive substring filter. Pass null for none."
                },
                "tailLines": {
                    "type": ["integer", "null"],
                    "description": "Maximum matching lines to return from the end of the file (1 to 500, default 100). Pass null for default."
                }
            },
            "required": ["file", "level", "targetPrefix", "contains", "tailLines"],
            "additionalProperties": false
        });
        Self { host, schema }
    }
}

impl Tool for MannieReadLogsTool {
    fn name(&self) -> &str {
        MANNIE_READ_LOGS
    }

    fn description(&self) -> &'static str {
        "Read bounded, filtered entries from Mannie's own redacted daily JSON logs and crash logs for troubleshooting. Confined strictly to Mannie's log directory. Read-only and never prompts."
    }

    fn parameters(&self) -> &Value {
        &self.schema
    }

    fn class(&self) -> PermissionClass {
        PermissionClass::ReadOnly
    }

    fn prepare(
        &self,
        _context: ToolContext,
        arguments: String,
    ) -> BoxFuture<'static, Result<Prepared, PrepareError>> {
        async move {
            let _request: logs::LogReadRequest =
                serde_json::from_str(&arguments).map_err(|error| {
                    PrepareError::InvalidArguments {
                        detail: error.to_string(),
                    }
                })?;
            Ok(Prepared {
                accesses: Vec::new(),
            })
        }
        .boxed()
    }

    fn execute(&self, execution: Execution) -> BoxFuture<'static, Done> {
        let log_dir = self.host.app_info().log_dir;
        async move {
            let request: logs::LogReadRequest = match serde_json::from_str(&execution.arguments) {
                Ok(request) => request,
                Err(error) => {
                    return Done {
                        outcome: ToolOutcome::MannieLogs {
                            entries: 0,
                            truncated: false,
                        },
                        output: format!("Invalid mannie_read_logs arguments: {error}"),
                        archives: Vec::new(),
                        images: Vec::new(),
                        cited: Vec::new(),
                    };
                }
            };
            let limit = execution.context.result_limit;
            let outcome = tokio::task::spawn_blocking(move || {
                logs::read_mannie_logs(&log_dir, &request, limit)
            })
            .await;
            match outcome {
                Ok(Ok(result)) => {
                    let entries = u32::try_from(result.entries.len()).unwrap_or(u32::MAX);
                    let truncated = result.truncated;
                    let value = serde_json::to_value(&result).unwrap_or_else(|_| json!({}));
                    Done {
                        outcome: ToolOutcome::MannieLogs { entries, truncated },
                        output: bounded_json(&value, limit),
                        archives: Vec::new(),
                        images: Vec::new(),
                        cited: Vec::new(),
                    }
                }
                Ok(Err(error)) => Done {
                    outcome: ToolOutcome::MannieLogs {
                        entries: 0,
                        truncated: false,
                    },
                    output: format!("Could not read Mannie logs: {error}"),
                    archives: Vec::new(),
                    images: Vec::new(),
                    cited: Vec::new(),
                },
                Err(join_error) => Done {
                    outcome: ToolOutcome::MannieLogs {
                        entries: 0,
                        truncated: false,
                    },
                    output: format!("Could not read Mannie logs: {join_error}"),
                    archives: Vec::new(),
                    images: Vec::new(),
                    cited: Vec::new(),
                },
            }
        }
        .boxed()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConfigureArgs {
    target: String,
    action: String,
    #[serde(default)]
    arguments: Value,
    #[serde(default)]
    #[expect(
        dead_code,
        reason = "values were already scrubbed to [redacted] before persistence; raw secrets travel via ConfigSecrets"
    )]
    secrets: Option<Value>,
}

/// Side-effecting self-configuration tool `mannie_configure`.
#[derive(Debug)]
pub(crate) struct MannieConfigureTool {
    runtime: Runtime,
    host: Arc<dyn AppHost>,
    schema: Value,
}

impl MannieConfigureTool {
    pub(crate) fn new(runtime: Runtime, host: Arc<dyn AppHost>) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "Core domain name (such as 'plugins', 'providers', 'projects', 'tools', 'chat') or enabled plugin/feature ID (such as 'ssh', 'mcp', 'proxmox', 'workers.databases')."
                },
                "action": {
                    "type": "string",
                    "description": "Typed action name returned by mannie_inspect(topic='actions', target=...)."
                },
                "arguments": {
                    "type": "object",
                    "description": "Non-secret JSON arguments matching the action schema. Never place passwords, API keys, or tokens inside arguments."
                },
                "secrets": {
                    "type": "object",
                    "description": "Optional map of secret keys to throwaway string values ONLY when the owner intentionally typed a throwaway credential in chat. Normally omit this field so the owner enters secrets via masked password inputs on the approval card."
                }
            },
            "required": ["target", "action", "arguments"],
            "additionalProperties": false
        });
        Self {
            runtime,
            host,
            schema,
        }
    }
}

impl Tool for MannieConfigureTool {
    fn name(&self) -> &str {
        MANNIE_CONFIGURE
    }

    fn description(&self) -> &'static str {
        "Apply a typed configuration change to Mannie's core or an enabled plugin. Always asks the owner for explicit one-time approval (even in Full Access). Call mannie_inspect(topic='actions', target=...) first to see valid action names and schemas. Omit real secrets so the owner enters them securely on the approval card; only pass throwaway credentials the owner explicitly typed in chat in top-level secrets."
    }

    fn parameters(&self) -> &Value {
        &self.schema
    }

    fn strict(&self) -> bool {
        // `arguments` and `secrets` hold dynamic action-specific keys.
        false
    }

    fn class(&self) -> PermissionClass {
        PermissionClass::SideEffects
    }

    fn prepare(
        &self,
        context: ToolContext,
        arguments: String,
    ) -> BoxFuture<'static, Result<Prepared, PrepareError>> {
        let runtime = self.runtime.clone();
        let host = Arc::clone(&self.host);
        async move {
            let parsed: ConfigureArgs =
                serde_json::from_str(&arguments).map_err(|error| PrepareError::InvalidArguments {
                    detail: error.to_string(),
                })?;
            let target = parsed.target.trim().to_owned();
            let action = parsed.action.trim().to_owned();
            if target.is_empty() || action.is_empty() {
                return Err(PrepareError::InvalidArguments {
                    detail: "target and action must be non-empty".to_owned(),
                });
            }
            let action_args = if parsed.arguments.is_null() {
                json!({})
            } else if parsed.arguments.is_object() {
                parsed.arguments
            } else {
                return Err(PrepareError::InvalidArguments {
                    detail: "arguments must be a JSON object".to_owned(),
                });
            };
            if let Some(misplaced) = has_misplaced_secret_marker(&action_args) {
                return Err(PrepareError::InvalidArguments {
                    detail: format!(
                        "do not place secret values inside arguments.{misplaced}; omit secrets so the owner enters them on the approval card, or pass throwaway credentials only in the top-level secrets object"
                    ),
                });
            }

            let provided_secrets = runtime
                .config_secrets()
                .chat_secret_keys(&context.conversation_id);
            let request = PrepareActionRequest {
                action: action.clone(),
                arguments: action_args,
                provided_secrets,
            };

            let (owner, prepared) = if core_domains::is_core_domain(&target) {
                let prepared =
                    core_domains::prepare_core_action(&runtime, host.as_ref(), &target, request)
                        .await
                        .map_err(|error| PrepareError::InvalidArguments {
                            detail: error.to_string(),
                        })?;
                ("core".to_owned(), prepared)
            } else if let Some(provider) = runtime.management_provider(&target) {
                let prepared = provider.prepare_action(request).await.map_err(|error| {
                    PrepareError::InvalidArguments {
                        detail: error.to_string(),
                    }
                })?;
                (target.clone(), prepared)
            } else {
                return Err(PrepareError::InvalidArguments {
                    detail: plugin_unavailable_hint(host.as_ref(), &target),
                });
            };

            if let Err(reason) = check_prepared_action(&prepared) {
                return Err(PrepareError::InvalidArguments {
                    detail: format!("prepared action was rejected: {reason}"),
                });
            }

            let payload = canonical_json_string(&prepared.canonical_arguments);
            Ok(Prepared {
                accesses: vec![Access::AppConfig {
                    owner,
                    domain: target,
                    action,
                    title: prepared.title,
                    fields: prepared.fields,
                    secret_fields: prepared.secret_fields,
                    security_sensitive: prepared.security_sensitive,
                    payload,
                    payload_cut: false,
                }],
            })
        }
        .boxed()
    }

    fn execute(&self, execution: Execution) -> BoxFuture<'static, Done> {
        let runtime = self.runtime.clone();
        let host = Arc::clone(&self.host);
        async move {
            let Some(Access::AppConfig {
                owner,
                domain,
                action,
                payload,
                ..
            }) = execution
                .accesses
                .iter()
                .find(|access| matches!(access, Access::AppConfig { .. }))
                .cloned()
            else {
                return Done {
                    outcome: ToolOutcome::MannieConfigure {
                        owner: "core".to_owned(),
                        domain: "unknown".to_owned(),
                        action: "unknown".to_owned(),
                    },
                    output: "Configuration call had no approved AppConfig access.".to_owned(),
                    archives: Vec::new(),
                    images: Vec::new(),
                    cited: Vec::new(),
                };
            };

            let canonical_args: Value =
                serde_json::from_str(&payload).unwrap_or_else(|_| json!({}));
            let secrets = runtime.config_secrets().take_merged(
                &execution.context.conversation_id,
                execution.part_id.as_ref(),
            );

            let result: Result<
                mannie_plugin_sdk::points::management::ActionResult,
                ManagementError,
            > = if owner == "core" {
                core_domains::execute_core_action(
                    &runtime,
                    host.as_ref(),
                    &domain,
                    &action,
                    canonical_args,
                    secrets,
                )
                .await
            } else if let Some(provider) = runtime.management_provider(&domain) {
                provider
                    .execute_action(ExecuteActionRequest {
                        action: action.clone(),
                        arguments: canonical_args,
                        secrets,
                    })
                    .await
            } else {
                Err(ManagementError::failed(plugin_unavailable_hint(
                    host.as_ref(),
                    &domain,
                )))
            };

            let output = match result {
                Ok(action_result) => {
                    let body = json!({
                        "ok": true,
                        "message": action_result.message,
                        "data": action_result.data,
                    });
                    bounded_json(&body, execution.context.result_limit)
                }
                Err(error) => {
                    let body = json!({
                        "ok": false,
                        "error": error,
                    });
                    bounded_json(&body, execution.context.result_limit)
                }
            };

            Done {
                outcome: ToolOutcome::MannieConfigure {
                    owner,
                    domain,
                    action,
                },
                output,
                archives: Vec::new(),
                images: Vec::new(),
                cited: Vec::new(),
            }
        }
        .boxed()
    }
}
