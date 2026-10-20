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
    /// What the plugin's manifest lets it use.
    pub uses: PluginUses,
}

/// What a plugin's manifest lets it use, as the Plugins page lists it
/// (ADR-0034, decision 13). The names are the manifest's and the core's, so
/// the page shows them as code beside its own words.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginUses {
    /// The core's services that the manifest declares, by name.
    pub services: Vec<ServiceUse>,
    /// The plugin's entries in the credential store, as the store names
    /// them, such as `link/master-key`, and its keyed families with
    /// `<key>` in place of each entry's key, such as `ssh/host/<key>`
    /// (ADR-0036, decision 8).
    pub secrets: Vec<String>,
    /// The commands that the plugin's UI may call, by name, as its
    /// interface declares them (ADR-0034, decision 8).
    pub commands: Vec<String>,
    /// The channels that the plugin's UI may follow, by name.
    pub channels: Vec<String>,
    /// The programs that ship beside Mannie for the plugin, by their
    /// packages, such as `mannie-worker`.
    pub programs: Vec<String>,
    /// The installed programs that the plugin runs through the service
    /// `processes`, by the names that the core finds them by, such as `git`
    /// (ADR-0038, decision 7).
    pub installed_programs: Vec<String>,
    /// The third-party crates that only the plugin may use, such as
    /// `tonic`.
    pub crates: Vec<String>,
    /// The kinds of direct native calls that the manifest grants the
    /// plugin's library crates past the SDK's services, as `[native]` names
    /// them, such as `sockets` (ADR-0035, decision 6).
    pub native: Vec<String>,
}

/// A service of the core that a plugin's manifest declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ServiceUse {
    /// The service's name, such as `executor-host`.
    pub name: String,
    /// The major version of the service that the plugin builds on.
    pub major: u16,
    /// Whether the service reaches sensitive data, such as provider
    /// credentials.
    pub sensitive: bool,
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

/// One kind of work that a plugin has in progress, which turning the plugin
/// off lets go as its feature document says (ADR-0034, decision 4). The
/// plugin's UI words it by its code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
// Plugins build it with `new`, so a new field does not change every plugin
// (ADR-0034, decision 1). A plain comment keeps this out of the TypeScript
// bindings.
#[non_exhaustive]
pub struct WorkItem {
    /// The plugin's code for the work, such as `active_tasks`.
    pub code: String,
    /// How much of the work is in progress, at least 1.
    pub count: u32,
}

impl WorkItem {
    /// A kind of work in progress, by the plugin's code and how much of it
    /// there is.
    pub fn new(code: impl Into<String>, count: u32) -> Self {
        Self {
            code: code.into(),
            count,
        }
    }
}

/// The work that a plugin has in progress, which the switch asks about
/// before it turns the plugin off. It is a list of items (ADR-0034, decision
/// 15), so each plugin reports its own kinds of work.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginWork {
    /// Each kind of work in progress, in the plugin's order. Empty when
    /// nothing is in progress.
    pub items: Vec<WorkItem>,
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
    use super::{
        PluginState, PluginUses, PluginView, PluginWork, PluginsError, PluginsSettings, ServiceUse,
        WorkItem,
    };

    #[test]
    fn views_and_errors_serialize_for_the_ui() {
        let view = PluginView {
            id: "workers".to_owned(),
            state: PluginState::Failed,
            enabled: true,
            failure: Some("storage_failed".to_owned()),
            uses: PluginUses {
                services: vec![ServiceUse {
                    name: "credentials".to_owned(),
                    major: 1,
                    sensitive: true,
                }],
                secrets: vec!["link/master-key".to_owned()],
                commands: vec!["pair_worker".to_owned()],
                channels: vec!["subscribe_workers".to_owned()],
                programs: vec!["mannie-worker".to_owned()],
                installed_programs: vec!["git".to_owned()],
                crates: vec!["tonic".to_owned()],
                native: vec!["sockets".to_owned()],
            },
        };
        assert_eq!(
            serde_json::to_value(&view).unwrap(),
            serde_json::json!({
                "id": "workers",
                "state": "failed",
                "enabled": true,
                "failure": "storage_failed",
                "uses": {
                    "services": [{ "name": "credentials", "major": 1, "sensitive": true }],
                    "secrets": ["link/master-key"],
                    "commands": ["pair_worker"],
                    "channels": ["subscribe_workers"],
                    "programs": ["mannie-worker"],
                    "installedPrograms": ["git"],
                    "crates": ["tonic"],
                    "native": ["sockets"],
                },
            })
        );
        assert_eq!(
            serde_json::to_value(PluginsError::UnknownPlugin).unwrap(),
            serde_json::json!({ "code": "unknown_plugin" })
        );
    }

    #[test]
    fn work_serializes_as_items_with_a_code_and_a_count() {
        let busy = PluginWork {
            items: vec![WorkItem::new("active_tasks", 2)],
        };

        assert_eq!(
            serde_json::to_value(&busy).unwrap(),
            serde_json::json!({ "items": [{ "code": "active_tasks", "count": 2 }] })
        );
        assert_eq!(
            serde_json::to_value(PluginWork::default()).unwrap(),
            serde_json::json!({ "items": [] })
        );
        assert_eq!(
            serde_json::from_value::<PluginWork>(
                serde_json::json!({ "items": [{ "code": "active_tasks", "count": 2 }] })
            )
            .unwrap(),
            busy
        );
    }

    #[test]
    fn work_without_its_items_is_refused() {
        let error = serde_json::from_value::<PluginWork>(serde_json::json!({ "activeTasks": 2 }))
            .unwrap_err();

        assert!(
            error.to_string().contains("missing field `items`"),
            "{error}"
        );
    }

    #[test]
    fn settings_without_choices_read_as_empty() {
        let settings: PluginsSettings = serde_json::from_str("{}").unwrap();
        assert!(settings.enabled.is_empty());
    }
}
