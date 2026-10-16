//! Plugins (ADR-0030; docs/features/plugins.md): the owner's choices, the
//! plugins as the Plugins page shows them, and the errors of their commands.
//! A plugin's name and description are messages of the UI under its key
//! prefix, so the core sends only IDs, states and codes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::settings::SettingsSection;
use crate::transcript::SubscriptionEnd;

/// The plugins settings section: the owner's choice for each plugin.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginsSettings {
    /// Whether each plugin is on, by its ID. A plugin without an entry follows
    /// its default once, and the outcome is kept here.
    #[serde(default)]
    pub enabled: BTreeMap<String, bool>,
}

impl SettingsSection for PluginsSettings {
    const KEY: &'static str = "plugins";
    const VERSION: u32 = 1;
}

/// Where a plugin is in its life (ADR-0030, decision 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PluginState {
    /// The plugin does not run, and none of its parts show.
    Off,
    /// The plugin is starting.
    Starting,
    /// The plugin runs, and its parts show.
    On,
    /// The plugin is stopping.
    Stopping,
    /// The plugin's start failed. The rest of Mannie keeps working.
    Failed,
}

impl PluginState {
    /// The state's name, as the logs write it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Starting => "starting",
            Self::On => "on",
            Self::Stopping => "stopping",
            Self::Failed => "failed",
        }
    }
}

/// A plugin as the Plugins page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginView {
    /// The plugin's ID, such as `workers`, which prefixes its messages.
    pub id: String,
    /// Where the plugin is in its life.
    pub state: PluginState,
    /// Whether the owner turned the plugin on.
    pub enabled: bool,
    /// Why the plugin's last start failed, as the plugin's stable code, while
    /// its state is `failed`.
    pub failure: Option<String>,
}

/// A message of the plugins channel: the plugins, first as they are and then
/// after every change. The list stays short, so each change sends all of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum PluginListUpdate {
    /// The plugins, in the order that Mannie lists them.
    Plugins {
        /// The plugins.
        plugins: Vec<PluginView>,
    },
    /// The channel ended.
    Ended {
        /// Why.
        reason: SubscriptionEnd,
    },
}

/// The argument of `set_plugin_enabled`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SetPluginEnabled {
    /// The plugin.
    pub plugin_id: String,
    /// Whether the owner turns it on.
    pub enabled: bool,
}

/// The argument of `plugin_work`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginRef {
    /// The plugin.
    pub plugin_id: String,
}

/// The work that a plugin has in progress, which turning it off leaves as
/// its feature document says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginWork {
    /// The tasks of the plugin's executor that have not ended.
    pub active_tasks: u32,
}

/// Why a plugins command failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum PluginsError {
    /// No plugin of this version of Mannie has this ID.
    #[error("no plugin has this ID")]
    UnknownPlugin,
    /// The owner's choice could not be saved. The log has the cause.
    #[error("the choice could not be saved")]
    StorageFailed,
    /// The core started without its database.
    #[error("the database is not open")]
    StoreUnavailable,
    /// Mannie is quitting.
    #[error("Mannie is quitting")]
    ShuttingDown,
}

impl PluginsError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownPlugin => "plugins.unknown_plugin",
            Self::StorageFailed => "plugins.storage_failed",
            Self::StoreUnavailable => "plugins.store_unavailable",
            Self::ShuttingDown => "plugins.shutting_down",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PluginState, PluginView, PluginsError, PluginsSettings};

    #[test]
    fn views_and_errors_serialize_for_the_ui() {
        let view = PluginView {
            id: "workers".to_owned(),
            state: PluginState::Failed,
            enabled: true,
            failure: Some("storage_failed".to_owned()),
        };
        assert_eq!(
            serde_json::to_value(&view).unwrap(),
            serde_json::json!({
                "id": "workers",
                "state": "failed",
                "enabled": true,
                "failure": "storage_failed",
            })
        );
        assert_eq!(
            serde_json::to_value(PluginsError::UnknownPlugin).unwrap(),
            serde_json::json!({ "code": "unknown_plugin" })
        );
    }

    #[test]
    fn settings_without_choices_read_as_empty() {
        let settings: PluginsSettings = serde_json::from_str("{}").unwrap();
        assert!(settings.enabled.is_empty());
    }
}
