//! Static self-knowledge guides and action schemas for Mannie's 15 built-in
//! core configuration domains (ADR-0081).

use mannie_plugin_sdk::points::management::{
    ManagementActionSpec, ManagementGuide, SecretFieldSpec,
};
use serde_json::{Value, json};

/// Every built-in core configuration domain name.
pub(crate) const CORE_DOMAINS: [&str; 15] = [
    "plugins",
    "projects",
    "providers",
    "search_profiles",
    "chatgpt",
    "agents",
    "skills",
    "chat",
    "tools",
    "policy",
    "child_agents",
    "interface",
    "window",
    "notifications",
    "updates",
];

/// Returns static architecture prose for top-level `mannie_inspect` topics.
pub(crate) fn topic_guide(topic: &str) -> &'static str {
    match topic {
        "overview" => {
            "Mannie is a single-owner desktop AI agent harness (Tauri 2 + Rust core + SvelteKit UI). \
             The webview is untrusted; all files, processes, network calls, database access, and \
             credentials stay in Rust. Optional features live in plugins that the owner turns on \
             and off. Use mannie_inspect to read configuration and plugin guides, mannie_read_logs \
             to diagnose errors from Mannie's redacted JSON logs, and mannie_configure to make \
             changes under explicit one-time owner approval. Never edit Mannie's SQLite database or \
             files in its data/log directories directly."
        }
        "storage" => {
            "Mannie stores all structured state in one SQLite database in WAL mode (mannie.sqlite \
             in release, or a schema-scoped dev database under data_dir/dev/ in development). Before \
             any schema migration, Mannie copies the database to backups_dir with VACUUM INTO and \
             keeps the 3 newest backups. Logs are daily rotating redacted JSON files in log_dir \
             (kept for 14 days, 32 MiB cap per day) plus crash logs on panic. Secrets and API keys \
             are stored only in the operating system credential store (Windows Credential Manager, \
             macOS Keychain, or Linux Secret Service), never in SQLite, logs, or transcripts. User \
             skills live in ~/.agents/skills and project skills live in <project>/.agents/skills."
        }
        "policy" => {
            "Every side effect in Mannie is a journaled, authorized operation (ADR-0013). Content \
             from files, commands, web pages, or external tools never grants permission. Reads \
             inside the active project workspace run without asking; writes/commands require \
             workspace settings, grants, rules, or explicit approval. Mannie's own data and log \
             folders are always protected. Every mannie_configure change produces Access::AppConfig, \
             which always asks the owner for explicit Allow once or Deny approval, never runs under \
             Conversation Full Access, never creates reusable grants or saved rules, and is refused \
             on workers."
        }
        "plugins" => {
            "Plugins are optional capabilities in plugins/<id>/. Each plugin has an enabled switch \
             persisted in the plugins settings section, and some plugins (such as workers) also have \
             optional features (such as workers.databases) with their own switches. When a plugin is \
             off, its tools and management provider are not running. If you enable a plugin with \
             mannie_configure(target=\"plugins\", action=\"set_enabled\"), its management provider \
             becomes immediately inspectable and configurable on the very next step of the same \
             reply, while its operational chat tools appear starting on the next user turn."
        }
        "troubleshooting" => {
            "To diagnose issues: (1) call mannie_inspect(topic=\"plugins\") to check for failed or \
             disabled plugins and their failure codes; (2) call mannie_inspect(topic=\"plugin\", \
             target=\"<id>\") for plugin-specific status (or query=\"log:<id>\" / \"tools:<id>\" on \
             mcp); (3) call mannie_read_logs with level=\"warn\" or \"error\" and optional \
             target_prefix (such as \"mannie_ssh\", \"mannie_mcp\", \"mannie_proxmox\", \
             \"mannie_runtime\") to inspect recent redacted errors; (4) check storage/module \
             versions via mannie_inspect(topic=\"storage\")."
        }
        _ => "",
    }
}

/// Returns the static guide and action schemas for one core configuration
/// domain.
pub(crate) fn domain_guide(domain: &str) -> Option<ManagementGuide> {
    let guide = match domain {
        "plugins" => plugins_guide(),
        "projects" => projects_guide(),
        "providers" => providers_guide(),
        "search_profiles" => search_profiles_guide(),
        "chatgpt" => chatgpt_guide(),
        "agents" => agents_guide(),
        "skills" => skills_guide(),
        "chat" => chat_guide(),
        "tools" => tools_guide(),
        "policy" => policy_guide(),
        "child_agents" => child_agents_guide(),
        "interface" => interface_guide(),
        "window" => window_guide(),
        "notifications" => notifications_guide(),
        "updates" => updates_guide(),
        _ => return None,
    };
    Some(guide)
}

fn action(
    name: &str,
    description: &str,
    security_sensitive: bool,
    parameters: Value,
    secret_fields: Vec<SecretFieldSpec>,
) -> ManagementActionSpec {
    ManagementActionSpec {
        name: name.to_owned(),
        description: description.to_owned(),
        security_sensitive,
        parameters,
        secret_fields,
    }
}

fn plugins_guide() -> ManagementGuide {
    ManagementGuide {
        summary: "Controls which installed plugins and optional plugin features are enabled."
            .to_owned(),
        storage: "Stored in SQLite table settings_sections under key 'plugins'.".to_owned(),
        troubleshooting: "If a plugin state is 'failed', check its failure code in \
                          mannie_inspect(topic=\"plugins\") and read recent errors with \
                          mannie_read_logs. Turning a plugin on starts its management provider \
                          immediately for the next step."
            .to_owned(),
        actions: vec![
            action(
                "set_enabled",
                "Turn an installed plugin on or off.",
                true,
                json!({
                    "type": "object",
                    "properties": {
                        "pluginId": { "type": "string", "description": "Plugin ID, for example 'ssh' or 'mcp'." },
                        "enabled": { "type": "boolean", "description": "True to turn the plugin on, false to turn it off." }
                    },
                    "required": ["pluginId", "enabled"],
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
            action(
                "set_feature_enabled",
                "Turn an optional feature of a plugin on or off (for example pluginId 'workers', featureId 'databases').",
                true,
                json!({
                    "type": "object",
                    "properties": {
                        "pluginId": { "type": "string", "description": "Parent plugin ID." },
                        "featureId": { "type": "string", "description": "Optional feature ID within the plugin." },
                        "enabled": { "type": "boolean", "description": "True to turn the feature on, false to turn it off." }
                    },
                    "required": ["pluginId", "featureId", "enabled"],
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
        ],
    }
}

fn projects_guide() -> ManagementGuide {
    ManagementGuide {
        summary: "Manages registered project folders shown in the sidebar and used as conversation workspaces."
            .to_owned(),
        storage: "Stored in SQLite table runtime_projects.".to_owned(),
        troubleshooting: "Project folders must already exist on disk and resolve without forbidden paths. \
                          Removing a project also deletes its conversations."
            .to_owned(),
        actions: vec![
            action(
                "add",
                "Register an existing local folder as a project.",
                false,
                json!({
                    "type": "object",
                    "properties": {
                        "folder": { "type": "string", "description": "Absolute path to an existing directory." }
                    },
                    "required": ["folder"],
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
            action(
                "rename",
                "Rename a registered project.",
                false,
                json!({
                    "type": "object",
                    "properties": {
                        "projectId": { "type": "string", "description": "Project UUID." },
                        "name": { "type": "string", "description": "New project display name." }
                    },
                    "required": ["projectId", "name"],
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
            action(
                "remove",
                "Remove a registered project and all of its conversations from Mannie (leaves files on disk untouched).",
                true,
                json!({
                    "type": "object",
                    "properties": {
                        "projectId": { "type": "string", "description": "Project UUID to remove." }
                    },
                    "required": ["projectId"],
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
        ],
    }
}

fn provider_id_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string", "description": "Profile UUID." }
        },
        "required": ["id"],
        "additionalProperties": false
    })
}

fn providers_guide() -> ManagementGuide {
    ManagementGuide {
        summary: "Manages AI model provider profiles (OpenAI Responses, Chat Completions, local servers, and ChatGPT plan profiles)."
            .to_owned(),
        storage: "Profile metadata is stored in SQLite table provider_profiles. API keys are stored only in the OS credential store under provider/<profile-id>."
            .to_owned(),
        troubleshooting: "Credentials require HTTPS unless targeting localhost or a private LAN address. \
                          Use action 'test' to verify credential reading, endpoint reachability, and tool support."
            .to_owned(),
        actions: vec![
            action(
                "create",
                "Create a new provider profile. Omit the API key so the owner enters it on the approval card (or pass a throwaway key in top-level secrets.credential).",
                true,
                json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Display name for the profile." },
                        "access": { "type": "string", "enum": ["endpoint", "chat_gpt_plan"], "description": "Defaults to 'endpoint'." },
                        "interface": { "type": "string", "enum": ["responses", "chat_completions"], "description": "API protocol." },
                        "baseUrl": { "type": "string", "description": "Server base URL, for example https://api.openai.com/v1 or http://127.0.0.1:11434/v1." },
                        "model": { "type": "string", "description": "Model identifier." },
                        "contextWindow": { "type": "integer", "description": "Optional context window in tokens (defaults from known models or 128000)." },
                        "outputLimit": { "type": "integer", "description": "Optional max output tokens (defaults from known models or 16384)." },
                        "reasoningEffort": { "type": "string", "enum": ["default", "none", "minimal", "low", "medium", "high", "xhigh", "max"] },
                        "useTools": { "type": "boolean", "description": "Whether tool calls are enabled (defaults to true)." }
                    },
                    "required": ["name", "interface", "baseUrl", "model"],
                    "additionalProperties": false
                }),
                vec![SecretFieldSpec {
                    key: "credential".to_owned(),
                    label: "API key".to_owned(),
                    description: "Optional API key for endpoint profiles (omit for local servers or ChatGPT plan).".to_owned(),
                    required: false,
                }],
            ),
            action(
                "update",
                "Update fields of an existing provider profile. Omitted fields keep their current values.",
                true,
                json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "Profile UUID." },
                        "name": { "type": "string" },
                        "access": { "type": "string", "enum": ["endpoint", "chat_gpt_plan"] },
                        "interface": { "type": "string", "enum": ["responses", "chat_completions"] },
                        "baseUrl": { "type": "string" },
                        "model": { "type": "string" },
                        "contextWindow": { "type": "integer" },
                        "outputLimit": { "type": "integer" },
                        "reasoningEffort": { "type": "string", "enum": ["default", "none", "minimal", "low", "medium", "high", "xhigh", "max"] },
                        "useTools": { "type": "boolean" },
                        "credentialAction": { "type": "string", "enum": ["keep", "replace", "remove"], "description": "Defaults to 'keep'." }
                    },
                    "required": ["id"],
                    "additionalProperties": false
                }),
                vec![SecretFieldSpec {
                    key: "credential".to_owned(),
                    label: "New API key".to_owned(),
                    description: "Required when credentialAction is 'replace'.".to_owned(),
                    required: false,
                }],
            ),
            action(
                "delete",
                "Delete a provider profile and remove its stored credential.",
                true,
                provider_id_schema(),
                Vec::new(),
            ),
            action(
                "set_default",
                "Make an existing provider profile the default for new conversations.",
                false,
                provider_id_schema(),
                Vec::new(),
            ),
            action(
                "test",
                "Run Mannie's provider connection test against an existing provider profile.",
                false,
                provider_id_schema(),
                Vec::new(),
            ),
        ],
    }
}

fn search_profiles_guide() -> ManagementGuide {
    ManagementGuide {
        summary: "Manages embedding and rerank model profiles used by search features such as Notebooks."
            .to_owned(),
        storage: "Stored in SQLite table provider_search_profiles; optional API keys in the OS credential store."
            .to_owned(),
        troubleshooting: "Only 'embeddings' profiles accept queryPrefix and documentPrefix; 'rerank' profiles must leave both empty."
            .to_owned(),
        actions: vec![
            action(
                "create",
                "Create a new embedding or rerank search profile.",
                true,
                json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "kind": { "type": "string", "enum": ["embeddings", "rerank"] },
                        "baseUrl": { "type": "string" },
                        "model": { "type": "string" },
                        "queryPrefix": { "type": "string" },
                        "documentPrefix": { "type": "string" }
                    },
                    "required": ["name", "kind", "baseUrl", "model"],
                    "additionalProperties": false
                }),
                vec![SecretFieldSpec {
                    key: "credential".to_owned(),
                    label: "API key".to_owned(),
                    description: "Optional API key for the search server.".to_owned(),
                    required: false,
                }],
            ),
            action(
                "update",
                "Update an existing search profile. Omitted fields keep their current values.",
                true,
                json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "kind": { "type": "string", "enum": ["embeddings", "rerank"] },
                        "baseUrl": { "type": "string" },
                        "model": { "type": "string" },
                        "queryPrefix": { "type": "string" },
                        "documentPrefix": { "type": "string" },
                        "credentialAction": { "type": "string", "enum": ["keep", "replace", "remove"] }
                    },
                    "required": ["id"],
                    "additionalProperties": false
                }),
                vec![SecretFieldSpec {
                    key: "credential".to_owned(),
                    label: "New API key".to_owned(),
                    description: "Required when credentialAction is 'replace'.".to_owned(),
                    required: false,
                }],
            ),
            action(
                "delete",
                "Delete a search profile and its stored credential.",
                true,
                json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" }
                    },
                    "required": ["id"],
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
            action(
                "test",
                "Test an existing search profile against its server.",
                false,
                json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" }
                    },
                    "required": ["id"],
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
        ],
    }
}

fn chatgpt_guide() -> ManagementGuide {
    ManagementGuide {
        summary: "Inspects the connected ChatGPT plan account and refreshes its model catalog or signs out."
            .to_owned(),
        storage: "Account metadata is stored in SQLite table provider_chatgpt_account; OAuth tokens stay only in the master OS credential store and never reach workers."
            .to_owned(),
        troubleshooting: "Interactive sign-in opens the browser from Settings > Providers. Refresh the catalog if new models are missing, or sign out to reset an expired session."
            .to_owned(),
        actions: vec![
            action(
                "refresh_catalog",
                "Refresh the list of models available on the signed-in ChatGPT account.",
                false,
                json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
            action(
                "sign_out",
                "Sign out of the connected ChatGPT account and remove its OAuth tokens from the credential store.",
                true,
                json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
        ],
    }
}

fn agents_guide() -> ManagementGuide {
    ManagementGuide {
        summary: "Manages the owner's custom conversation and child agents (instructions, tool subsets, plugins, child rosters, and workflow steps)."
            .to_owned(),
        storage: "Stored in SQLite table runtime_agents.".to_owned(),
        troubleshooting: "Built-in agents (conversation, explore, general) cannot be edited or deleted; only custom.* agents can be created, updated, or deleted."
            .to_owned(),
        actions: vec![
            action(
                "create",
                "Create a new custom conversation or child agent.",
                true,
                json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "description": { "type": "string", "description": "Required for child agents." },
                        "kind": { "type": "string", "enum": ["conversation", "child"] },
                        "readOnly": { "type": "boolean" },
                        "profileId": { "type": ["string", "null"] },
                        "instructions": { "type": "string" },
                        "steps": { "type": "array", "items": { "type": "object" } },
                        "tools": { "type": "array", "items": { "type": "string" } },
                        "plugins": { "type": ["array", "null"], "items": { "type": "string" } },
                        "children": { "type": ["array", "null"], "items": { "type": "string" } },
                        "maxTasks": { "type": "integer" }
                    },
                    "required": ["name", "kind"],
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
            action(
                "update",
                "Update an existing custom agent by its key (for example 'custom.reviewer'). Omitted fields keep their current values.",
                true,
                json!({
                    "type": "object",
                    "properties": {
                        "key": { "type": "string" },
                        "name": { "type": "string" },
                        "description": { "type": "string" },
                        "readOnly": { "type": "boolean" },
                        "profileId": { "type": ["string", "null"] },
                        "instructions": { "type": "string" },
                        "steps": { "type": "array", "items": { "type": "object" } },
                        "tools": { "type": "array", "items": { "type": "string" } },
                        "plugins": { "type": ["array", "null"], "items": { "type": "string" } },
                        "children": { "type": ["array", "null"], "items": { "type": "string" } },
                        "maxTasks": { "type": "integer" }
                    },
                    "required": ["key"],
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
            action(
                "delete",
                "Delete a custom agent by its key.",
                true,
                json!({
                    "type": "object",
                    "properties": {
                        "key": { "type": "string" }
                    },
                    "required": ["key"],
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
        ],
    }
}

fn skills_guide() -> ManagementGuide {
    ManagementGuide {
        summary: "Inspects installed Agent Skills in ~/.agents/skills and project .agents/skills folders and turns individual skills on or off."
            .to_owned(),
        storage: "Skill files live on disk in ~/.agents/skills and <project>/.agents/skills; per-skill switches and project skill digests live in SQLite table runtime_skill_switches."
            .to_owned(),
        troubleshooting: "Project skills start off and turn off automatically if their SKILL.md metadata changes until the owner turns them on again. Inspect the 'problems' list for malformed SKILL.md files."
            .to_owned(),
        actions: vec![action(
            "set_on",
            "Turn an installed computer or project skill on or off.",
            true,
            json!({
                "type": "object",
                "properties": {
                    "place": { "type": "string", "enum": ["computer", "project"] },
                    "projectId": { "type": "string", "description": "Required when place is 'project'." },
                    "name": { "type": "string", "description": "Skill name." },
                    "on": { "type": "boolean" }
                },
                "required": ["place", "name", "on"],
                "additionalProperties": false
            }),
            Vec::new(),
        )],
    }
}

fn chat_guide() -> ManagementGuide {
    ManagementGuide {
        summary: "Manages global owner instructions applied to every conversation."
            .to_owned(),
        storage: "Stored in SQLite table settings_sections under key 'chat'.".to_owned(),
        troubleshooting: "Global instructions are limited to 32,768 characters and take effect on the next model call."
            .to_owned(),
        actions: vec![action(
            "update_settings",
            "Replace the owner's global instructions for all conversations.",
            true,
            json!({
                "type": "object",
                "properties": {
                    "instructions": { "type": "string", "description": "Complete new global instructions text (empty string clears them)." }
                },
                "required": ["instructions"],
                "additionalProperties": false
            }),
            Vec::new(),
        )],
    }
}

fn tools_guide() -> ManagementGuide {
    ManagementGuide {
        summary: "Manages system tool authorization defaults, extra command environment variable names, default full access for new conversations, and background command wake-ups."
            .to_owned(),
        storage: "Stored in SQLite table settings_sections under key 'tools'.".to_owned(),
        troubleshooting: "Environment variable names that look like secrets (KEY, TOKEN, SECRET, PASSWORD, etc.) are always rejected."
            .to_owned(),
        actions: vec![action(
            "update_settings",
            "Update system tools settings. Omitted fields keep their current values.",
            true,
            json!({
                "type": "object",
                "properties": {
                    "allowChangesInWorkspace": { "type": "boolean" },
                    "commandEnvironment": { "type": "array", "items": { "type": "string" } },
                    "fullAccessForNewConversations": { "type": "boolean" },
                    "wakeOnCommandEnd": { "type": "boolean" }
                },
                "additionalProperties": false
            }),
            Vec::new(),
        )],
    }
}

fn policy_guide() -> ManagementGuide {
    ManagementGuide {
        summary: "Inspects and revokes saved command rules, remote command rules, program rules, terminal rules, shared grants, and conversation grants."
            .to_owned(),
        storage: "Stored in SQLite tables tools_rules, tools_remote_rules, tools_program_rules, tools_terminal_rules, tools_shared_grants, and tools_grants."
            .to_owned(),
        troubleshooting: "Only deletion/revocation is supported here; new rules and grants are created only when the owner approves a tool call."
            .to_owned(),
        actions: vec![
            action(
                "delete_command_rule",
                "Delete a saved local shell command rule.",
                false,
                json!({
                    "type": "object",
                    "properties": { "ruleId": { "type": "string" } },
                    "required": ["ruleId"],
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
            action(
                "delete_remote_command_rule",
                "Delete a saved remote-host command rule.",
                false,
                json!({
                    "type": "object",
                    "properties": { "ruleId": { "type": "string" } },
                    "required": ["ruleId"],
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
            action(
                "delete_program_rule",
                "Delete a saved installed-program rule (for example git).",
                false,
                json!({
                    "type": "object",
                    "properties": { "ruleId": { "type": "string" } },
                    "required": ["ruleId"],
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
            action(
                "delete_terminal_command_rule",
                "Delete a saved terminal command rule.",
                false,
                json!({
                    "type": "object",
                    "properties": { "ruleId": { "type": "string" } },
                    "required": ["ruleId"],
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
            action(
                "revoke_shared_grant",
                "Revoke a project-wide or global approval grant.",
                false,
                json!({
                    "type": "object",
                    "properties": { "grantId": { "type": "string" } },
                    "required": ["grantId"],
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
            action(
                "revoke_conversation_grant",
                "Revoke an approval grant on a specific conversation.",
                false,
                json!({
                    "type": "object",
                    "properties": {
                        "conversationId": { "type": "string" },
                        "grantId": { "type": "string" }
                    },
                    "required": ["conversationId", "grantId"],
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
        ],
    }
}

fn child_agents_guide() -> ManagementGuide {
    ManagementGuide {
        summary: "Controls how many child agents may run concurrently on this machine.".to_owned(),
        storage: "Stored in SQLite table settings_sections under key 'child_agents'.".to_owned(),
        troubleshooting: "parallelChildren accepts values from 1 to 16 (default 4).".to_owned(),
        actions: vec![action(
            "update_settings",
            "Set the maximum number of concurrent child agents (1..=16).",
            false,
            json!({
                "type": "object",
                "properties": {
                    "parallelChildren": { "type": "integer", "description": "Integer from 1 to 16." }
                },
                "required": ["parallelChildren"],
                "additionalProperties": false
            }),
            Vec::new(),
        )],
    }
}

fn interface_guide() -> ManagementGuide {
    ManagementGuide {
        summary: "Controls desktop UI locale override."
            .to_owned(),
        storage: "Stored in SQLite table settings_sections under key 'interface'.".to_owned(),
        troubleshooting: "Pass a valid BCP 47 tag (for example 'en' or 'pt-BR') or null to follow the operating system locale."
            .to_owned(),
        actions: vec![action(
            "update_settings",
            "Set or clear the desktop interface locale.",
            false,
            json!({
                "type": "object",
                "properties": {
                    "locale": { "type": ["string", "null"], "description": "BCP 47 locale tag, or null to follow OS locale." }
                },
                "required": ["locale"],
                "additionalProperties": false
            }),
            Vec::new(),
        )],
    }
}

fn window_guide() -> ManagementGuide {
    ManagementGuide {
        summary: "Controls what closing the desktop window does (ask, minimize to tray, or quit)."
            .to_owned(),
        storage: "Stored in SQLite table settings_sections under key 'window'.".to_owned(),
        troubleshooting: "Valid closeAction values are 'ask', 'tray', and 'quit'.".to_owned(),
        actions: vec![action(
            "update_settings",
            "Update what happens when the main desktop window is closed.",
            false,
            json!({
                "type": "object",
                "properties": {
                    "closeAction": { "type": "string", "enum": ["ask", "tray", "quit"] }
                },
                "required": ["closeAction"],
                "additionalProperties": false
            }),
            Vec::new(),
        )],
    }
}

fn notifications_guide() -> ManagementGuide {
    ManagementGuide {
        summary: "Controls desktop notification triggers and taskbar/tray attention badges."
            .to_owned(),
        storage: "Stored in SQLite table settings_sections under key 'notifications'.".to_owned(),
        troubleshooting: "If notifications do not appear, check both 'enabled' and the specific event toggle (onQuestion, onApproval, onCompletion, onFailure)."
            .to_owned(),
        actions: vec![action(
            "update_settings",
            "Update desktop notification and badge settings. Omitted fields keep their current values.",
            false,
            json!({
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean" },
                    "onQuestion": { "type": "boolean" },
                    "onApproval": { "type": "boolean" },
                    "onCompletion": { "type": "boolean" },
                    "onFailure": { "type": "boolean" },
                    "taskbarBadge": { "type": "boolean" },
                    "trayBadge": { "type": "boolean" }
                },
                "additionalProperties": false
            }),
            Vec::new(),
        )],
    }
}

fn updates_guide() -> ManagementGuide {
    ManagementGuide {
        summary: "Inspects desktop update status, toggles automatic release checks, and triggers a check for newer signed releases."
            .to_owned(),
        storage: "Settings stored in SQLite table settings_sections under key 'updates'; install journal in updates_operations."
            .to_owned(),
        troubleshooting: "Development and unpackaged builds report update state 'off'. Installing a downloaded update requires the owner to click Restart and update."
            .to_owned(),
        actions: vec![
            action(
                "update_settings",
                "Turn automatic background update checks on or off.",
                false,
                json!({
                    "type": "object",
                    "properties": {
                        "checkAutomatically": { "type": "boolean" }
                    },
                    "required": ["checkAutomatically"],
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
            action(
                "check_for_updates",
                "Start an immediate check for a newer signed desktop release.",
                false,
                json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
                Vec::new(),
            ),
        ],
    }
}
