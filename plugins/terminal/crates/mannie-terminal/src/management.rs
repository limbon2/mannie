//! Self-management provider for the Terminal plugin (ADR-0081).
//!
//! Exposes current terminal settings, discovered local shells, plugin-offered
//! remote terminal hosts, running command counts, and the `update_settings`
//! action so the owner can adjust terminal preferences from chat.

use std::path::PathBuf;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::ConfigFieldPreview;
use mannie_plugin_sdk::ipc::Validate as _;
use mannie_plugin_sdk::points::management::{
    ActionResult, ExecuteActionRequest, ManagementActionSpec, ManagementError, ManagementGuide,
    ManagementProvider, ManagementSnapshot, PrepareActionRequest, PreparedAction,
};
use mannie_plugin_sdk::services::terminals::TerminalHostState;
use serde::Deserialize;
use serde_json::json;

use crate::commands::{
    MAX_FONT_SIZE, MAX_SCROLLBACK, MIN_FONT_SIZE, MIN_SCROLLBACK, Renderer, TerminalSettings,
};
use crate::registry::Terminals;
use crate::store::SettingsStore;

/// Action name for updating terminal preferences.
const ACTION_UPDATE_SETTINGS: &str = "update_settings";

/// Management provider for the Terminal plugin.
#[derive(Debug, Clone)]
pub(crate) struct TerminalManagement {
    terminals: Terminals,
    settings: SettingsStore,
}

impl TerminalManagement {
    /// Creates a management provider backed by the running terminals registry
    /// and settings store.
    pub(crate) fn new(terminals: Terminals, settings: SettingsStore) -> Self {
        Self {
            terminals,
            settings,
        }
    }
}

/// Three-state patch field distinguishing omitted, explicit `null`, and a
/// string value.
#[derive(Debug, Default)]
enum OptionalShell {
    #[default]
    Unchanged,
    Clear,
    Set(String),
}

impl OptionalShell {
    fn is_changed(&self) -> bool {
        !matches!(self, Self::Unchanged)
    }
}

/// Partial update patch for `update_settings`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SettingsPatch {
    #[serde(default, deserialize_with = "deserialize_optional_shell")]
    default_shell: OptionalShell,
    #[serde(default)]
    scrollback: Option<u32>,
    #[serde(default)]
    font_size: Option<u32>,
    #[serde(default)]
    cursor_blink: Option<bool>,
    #[serde(default)]
    renderer: Option<Renderer>,
    #[serde(default)]
    screen_reader: Option<bool>,
}

/// Deserializes a present `defaultShell` JSON field as `Clear` (on `null`) or
/// `Set(value)`.
fn deserialize_optional_shell<'de, D>(deserializer: D) -> Result<OptionalShell, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match Option::<String>::deserialize(deserializer)? {
        Some(value) => OptionalShell::Set(value),
        None => OptionalShell::Clear,
    })
}

fn renderer_label(renderer: Renderer) -> &'static str {
    match renderer {
        Renderer::Automatic => "automatic",
        Renderer::Gpu => "gpu",
        Renderer::Compatible => "compatible",
    }
}

fn preview_fields(settings: &TerminalSettings) -> Vec<ConfigFieldPreview> {
    let default_shell = settings
        .default_shell
        .clone()
        .unwrap_or_else(|| "System default".to_owned());

    vec![
        ConfigFieldPreview {
            label: "Default shell".to_owned(),
            value: default_shell,
        },
        ConfigFieldPreview {
            label: "Scrollback".to_owned(),
            value: settings.scrollback.to_string(),
        },
        ConfigFieldPreview {
            label: "Font size".to_owned(),
            value: format!("{} px", settings.font_size),
        },
        ConfigFieldPreview {
            label: "Cursor blink".to_owned(),
            value: if settings.cursor_blink { "On" } else { "Off" }.to_owned(),
        },
        ConfigFieldPreview {
            label: "Renderer".to_owned(),
            value: renderer_label(settings.renderer).to_owned(),
        },
        ConfigFieldPreview {
            label: "Screen reader".to_owned(),
            value: if settings.screen_reader { "On" } else { "Off" }.to_owned(),
        },
    ]
}

impl ManagementProvider for TerminalManagement {
    fn guide(&self) -> ManagementGuide {
        ManagementGuide {
            summary: "Opens interactive local and remote terminals beside conversations, keeps a \
                headless terminal emulator in Rust, and provides `list_terminals` and \
                `run_in_terminal` under the master's policy and journal."
                .to_owned(),
            storage: "Stores preferences in the `terminal_settings` SQLite table (single row: \
                default shell, scrollback, font size, cursor blink, renderer, screen reader). \
                Terminal output and owner keystrokes stay in memory only, and no credentials are \
                stored by this plugin."
                .to_owned(),
            troubleshooting: "If a shell fails to open, inspect `terminal` to confirm the shell \
                program path appears in discovered local shells or clear `defaultShell` back to \
                null for the system default. Remote hosts come from other plugins (such as `ssh`); \
                if a remote host is unavailable, inspect that plugin."
                .to_owned(),
            actions: vec![ManagementActionSpec {
                name: ACTION_UPDATE_SETTINGS.to_owned(),
                description: "Update one or more terminal plugin settings. Omitted fields keep \
                    their current values; set defaultShell to null to use the system default."
                    .to_owned(),
                security_sensitive: false,
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "defaultShell": {
                            "type": ["string", "null"],
                            "description": "Program path of the default local shell, or null for the system default."
                        },
                        "scrollback": {
                            "type": "integer",
                            "minimum": MIN_SCROLLBACK,
                            "maximum": MAX_SCROLLBACK,
                            "description": "Lines of scrollback kept per terminal view."
                        },
                        "fontSize": {
                            "type": "integer",
                            "minimum": MIN_FONT_SIZE,
                            "maximum": MAX_FONT_SIZE,
                            "description": "Terminal font size in pixels."
                        },
                        "cursorBlink": {
                            "type": "boolean",
                            "description": "Whether the terminal cursor blinks."
                        },
                        "renderer": {
                            "type": "string",
                            "enum": ["automatic", "gpu", "compatible"],
                            "description": "Terminal rendering backend."
                        },
                        "screenReader": {
                            "type": "boolean",
                            "description": "Whether to enable screen-reader compatibility mode."
                        }
                    },
                    "additionalProperties": false
                }),
                secret_fields: Vec::new(),
            }],
        }
    }

    fn inspect(
        &self,
        query: Option<String>,
    ) -> BoxFuture<'static, Result<ManagementSnapshot, ManagementError>> {
        let terminals = self.terminals.clone();
        let settings = self.settings.clone();

        Box::pin(async move {
            if let Some(query) = query
                && !query.trim().is_empty()
            {
                return Err(ManagementError::invalid_field(
                    "query",
                    format!("terminal does not support inspect query {query:?}"),
                ));
            }

            let current = settings.read().await.map_err(|error| {
                ManagementError::failed(format!(
                    "could not read terminal settings ({})",
                    error.code()
                ))
            })?;
            let host_list = terminals.hosts();
            let running_commands = terminals.running_commands();

            let shells: Vec<serde_json::Value> = host_list
                .shells
                .iter()
                .map(|shell| {
                    json!({
                        "program": shell.program.to_string_lossy(),
                        "name": shell.name,
                        "default": shell.default,
                    })
                })
                .collect();

            let remote_hosts: Vec<serde_json::Value> = host_list
                .hosts
                .iter()
                .map(|host| {
                    let state = match &host.state {
                        TerminalHostState::Unavailable { code } => {
                            json!({ "kind": "unavailable", "code": code })
                        }
                        _ => json!({ "kind": "ready" }),
                    };

                    json!({
                        "plugin": host.plugin,
                        "key": host.key,
                        "label": host.label,
                        "endpoint": host.endpoint,
                        "state": state,
                    })
                })
                .collect();

            let summary = format!(
                "Terminal settings loaded ({} local shell(s), {} remote host(s), {running_commands} running command(s)).",
                shells.len(),
                remote_hosts.len(),
            );

            let data = json!({
                "settings": current,
                "localShells": shells,
                "remoteHosts": remote_hosts,
                "runningCommands": running_commands,
            });

            Ok(ManagementSnapshot::new(summary, data))
        })
    }

    fn prepare_action(
        &self,
        request: PrepareActionRequest,
    ) -> BoxFuture<'static, Result<PreparedAction, ManagementError>> {
        let settings = self.settings.clone();

        Box::pin(async move {
            if request.action != ACTION_UPDATE_SETTINGS {
                return Err(ManagementError::unknown_action(&request.action));
            }

            let patch: SettingsPatch = serde_json::from_value(request.arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;

            let has_change = patch.default_shell.is_changed()
                || patch.scrollback.is_some()
                || patch.font_size.is_some()
                || patch.cursor_blink.is_some()
                || patch.renderer.is_some()
                || patch.screen_reader.is_some();

            if !has_change {
                return Err(ManagementError::invalid_arguments(
                    "specify at least one setting to update",
                ));
            }

            let mut next = settings.read().await.map_err(|error| {
                ManagementError::failed(format!(
                    "could not read current terminal settings ({})",
                    error.code()
                ))
            })?;

            match patch.default_shell {
                OptionalShell::Unchanged => {}
                OptionalShell::Clear => next.default_shell = None,
                OptionalShell::Set(value) => next.default_shell = Some(value),
            }
            if let Some(scrollback) = patch.scrollback {
                next.scrollback = scrollback;
            }
            if let Some(font_size) = patch.font_size {
                next.font_size = font_size;
            }
            if let Some(cursor_blink) = patch.cursor_blink {
                next.cursor_blink = cursor_blink;
            }
            if let Some(renderer) = patch.renderer {
                next.renderer = renderer;
            }
            if let Some(screen_reader) = patch.screen_reader {
                next.screen_reader = screen_reader;
            }

            if let Err(refusal) = next.validate() {
                let message = format!("invalid value for {}", refusal.field);
                return Err(ManagementError::invalid_field(refusal.field, message));
            }

            let canonical_arguments = serde_json::to_value(&next)
                .map_err(|error| ManagementError::failed(error.to_string()))?;

            Ok(PreparedAction {
                title: "Update terminal settings".to_owned(),
                fields: preview_fields(&next),
                secret_fields: Vec::new(),
                security_sensitive: false,
                canonical_arguments,
            })
        })
    }

    fn execute_action(
        &self,
        request: ExecuteActionRequest,
    ) -> BoxFuture<'static, Result<ActionResult, ManagementError>> {
        let terminals = self.terminals.clone();
        let settings = self.settings.clone();

        Box::pin(async move {
            if request.action != ACTION_UPDATE_SETTINGS {
                return Err(ManagementError::unknown_action(&request.action));
            }

            let next: TerminalSettings = serde_json::from_value(request.arguments)
                .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;

            if let Err(refusal) = next.validate() {
                let message = format!("invalid value for {}", refusal.field);
                return Err(ManagementError::invalid_field(refusal.field, message));
            }

            settings.write(&next).await.map_err(|error| {
                ManagementError::failed(format!(
                    "could not store terminal settings ({})",
                    error.code()
                ))
            })?;
            terminals.set_default_shell(next.default_shell.clone().map(PathBuf::from));

            let data = serde_json::to_value(&next)
                .map_err(|error| ManagementError::failed(error.to_string()))?;

            Ok(ActionResult::new("Updated terminal settings.").with_data(data))
        })
    }
}
