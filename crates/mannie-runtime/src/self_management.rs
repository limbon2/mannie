//! Self-awareness, read-only diagnostics and chat-driven self-configuration
//! (ADR-0081; docs/features/self-management.md).
//!
//! When the desktop composition root binds an [`AppHost`] via
//! [`Runtime::attach_app_host`], conversation agents answering to the owner
//! gain up to three runtime-owned tools:
//!
//! - [`MANNIE_INSPECT`] (`ReadOnly`): returns Mannie's paths, storage layout,
//!   database module versions, plugin states (including off and failed
//!   plugins), secret-free core settings snapshots, and live plugin
//!   [`ManagementProvider`](mannie_plugin_sdk::points::management::ManagementProvider)
//!   guides, snapshots and action schemas.
//! - [`MANNIE_READ_LOGS`] (`ReadOnly`): reads bounded, filtered lines strictly
//!   from Mannie's canonical log directory, where lines already passed through
//!   `mannie-logging`'s redactor.
//! - [`MANNIE_CONFIGURE`] (`SideEffects`, non-read-only conversation agents
//!   only): prepares and executes typed core and plugin configuration actions
//!   under [`Access::AppConfig`](mannie_types::tools::Access::AppConfig), which
//!   always asks the owner for explicit one-time approval, never runs under
//!   full access, and accepts secrets through masked inputs on the approval
//!   card (with optional scrubbed throwaway values passed in top-level
//!   `secrets`).
//!
//! Workers never attach an [`AppHost`], so workers never expose these tools or
//! prompt sections.

mod core_domains;
mod logs;
mod scrub;
mod tools;

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use mannie_plugin_sdk::contributions::{Contributions, ExtensionPoint};
use mannie_plugin_sdk::host::{ContributionRefusal, ID_TAKEN};
use mannie_plugin_sdk::points::management::{
    Management, ManagementProvider, check_management_guide,
};
use mannie_tools::Tool;
use mannie_types::ids::{ConversationId, PartId};
use mannie_types::interface::{InterfaceSettings, InterfaceSettingsError, UpdateInterfaceSettings};
use mannie_types::notifications::{
    NotificationError, NotificationSettings, UpdateNotificationSettings,
};
use mannie_types::plugins::{PluginState, PluginView, PluginWork, PluginsError};
use mannie_types::secret::Secret;
use mannie_types::updates::{UpdateStatus, UpdateUpdatesSettings, UpdatesError, UpdatesSettings};
use mannie_types::window::{UpdateWindowSettings, WindowError, WindowSettings};
use serde::Serialize;

use crate::plugins::Snapshot;
use crate::{Runtime, lock};

pub(crate) use self::scrub::{has_misplaced_secret_marker, scrub_configure_arguments};

/// Tool that inspects Mannie's paths, storage layout, plugins, core settings
/// and live plugin management guides/snapshots without side effects.
pub const MANNIE_INSPECT: &str = "mannie_inspect";

/// Tool that reads bounded, filtered entries from Mannie's redacted log folder.
pub const MANNIE_READ_LOGS: &str = "mannie_read_logs";

/// Tool that prepares and executes authorized configuration actions on Mannie's
/// core or enabled plugins.
pub const MANNIE_CONFIGURE: &str = "mannie_configure";

/// Every runtime-owned self-management tool name (reserved against plugins).
pub const SELF_MANAGEMENT_TOOLS: [&str; 3] = [MANNIE_INSPECT, MANNIE_READ_LOGS, MANNIE_CONFIGURE];

/// Maximum size of one secret entered on an `AppConfig` approval card (8 KiB).
pub(crate) const MAX_CONFIG_SECRET_BYTES: usize = 8 * 1024;

/// Static and canonical runtime path facts about the desktop application
/// reported by the composition root (ADR-0081).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRuntimeInfo {
    /// Mannie application version string.
    pub version: String,
    /// Build profile (`development`, `validation` or `release`).
    pub build_profile: String,
    /// Desktop bundle identifier (for example `dev.mannie.desktop`).
    pub bundle_identifier: String,
    /// Operating system name.
    pub os: String,
    /// CPU architecture.
    pub arch: String,
    /// Canonical data directory holding the SQLite database and backups.
    pub data_dir: PathBuf,
    /// Canonical path to the open SQLite database file.
    pub database_path: PathBuf,
    /// Canonical directory where pre-migration database backups are stored.
    pub backups_dir: PathBuf,
    /// Canonical directory holding rotating daily JSON logs and crash logs.
    pub log_dir: PathBuf,
    /// Canonical user-level skills folder (`~/.agents/skills`), if available.
    pub skills_dir: Option<PathBuf>,
    /// Canonical browser runtime data directory, if configured.
    pub browser_data_dir: Option<PathBuf>,
    /// Description of the active OS credential store backend.
    pub credential_store: String,
}

/// Host port implemented by the desktop composition root so the runtime can
/// inspect and configure host-owned domains (plugins, window, notifications,
/// updates, interface) without depending on Tauri or `mannie-plugins`
/// (ADR-0081).
pub trait AppHost: Send + Sync + fmt::Debug + 'static {
    /// Returns Mannie's version, build profile and canonical local storage paths.
    fn app_info(&self) -> AppRuntimeInfo;

    /// Returns the live state of every installed plugin and optional feature.
    fn list_plugins(&self) -> Vec<PluginView>;

    /// Reports active work in progress for a plugin before turning it off.
    fn plugin_work(
        &self,
        plugin_id: String,
    ) -> BoxFuture<'static, Result<PluginWork, PluginsError>>;

    /// Reports active work in progress for an optional feature before turning
    /// it off.
    fn plugin_feature_work(
        &self,
        plugin_id: String,
        feature_id: String,
    ) -> BoxFuture<'static, Result<PluginWork, PluginsError>>;

    /// Persists and applies a plugin's enabled switch.
    fn set_plugin_enabled(
        &self,
        plugin_id: String,
        enabled: bool,
    ) -> BoxFuture<'static, Result<PluginView, PluginsError>>;

    /// Persists and applies an optional feature's enabled switch.
    fn set_plugin_feature_enabled(
        &self,
        plugin_id: String,
        feature_id: String,
        enabled: bool,
    ) -> BoxFuture<'static, Result<PluginView, PluginsError>>;

    /// Returns the current window settings.
    fn window_settings(&self) -> WindowSettings;

    /// Persists and applies updated window settings.
    fn update_window_settings(
        &self,
        update: UpdateWindowSettings,
    ) -> BoxFuture<'static, Result<WindowSettings, WindowError>>;

    /// Returns the current notification settings.
    fn notification_settings(&self) -> NotificationSettings;

    /// Persists and applies updated notification settings.
    fn update_notification_settings(
        &self,
        update: UpdateNotificationSettings,
    ) -> BoxFuture<'static, Result<NotificationSettings, NotificationError>>;

    /// Returns the live desktop update status.
    fn update_status(&self) -> UpdateStatus;

    /// Reads the persisted updates settings.
    fn updates_settings(&self) -> BoxFuture<'static, Result<UpdatesSettings, UpdatesError>>;

    /// Persists and applies updated desktop update settings.
    fn update_updates_settings(
        &self,
        update: UpdateUpdatesSettings,
    ) -> BoxFuture<'static, Result<UpdatesSettings, UpdatesError>>;

    /// Starts an asynchronous check for a newer desktop release.
    fn check_for_updates(&self) -> Result<(), UpdatesError>;

    /// Reads the persisted interface settings.
    fn interface_settings(
        &self,
    ) -> BoxFuture<'static, Result<InterfaceSettings, InterfaceSettingsError>>;

    /// Persists updated interface settings.
    fn update_interface_settings(
        &self,
        update: UpdateInterfaceSettings,
    ) -> BoxFuture<'static, Result<InterfaceSettings, InterfaceSettingsError>>;

    /// Notifies host services and running plugins when conversations are
    /// removed (for example when removing a project).
    fn conversations_deleted(
        &self,
        conversation_ids: Vec<ConversationId>,
    ) -> BoxFuture<'static, ()>;
}

/// Ephemeral in-memory secrets held only for the lifetime of one active
/// `mannie_configure` tool call in a conversation (ADR-0081).
#[derive(Debug, Default)]
pub(crate) struct ConfigSecrets {
    active: Mutex<HashMap<ConversationId, PendingSecretEntry>>,
}

#[derive(Debug)]
struct PendingSecretEntry {
    part_id: PartId,
    /// Throwaway secrets extracted by `scrub_configure_arguments` before
    /// persisting `Change::Delta`.
    chat_secrets: BTreeMap<String, Secret>,
    /// Masked secrets entered directly by the owner on the approval card.
    /// Always take precedence over `chat_secrets` on execution.
    owner_secrets: BTreeMap<String, Secret>,
}

impl ConfigSecrets {
    /// Registers the scrubbed chat secrets for an active `mannie_configure`
    /// call and returns an RAII guard that unconditionally clears and zeroizes
    /// them when the call exits for any reason.
    pub(crate) fn begin(
        runtime: Runtime,
        conversation: ConversationId,
        part_id: PartId,
        chat_secrets: BTreeMap<String, Secret>,
    ) -> SecretGuard {
        lock(&runtime.inner.config_secrets.active).insert(
            conversation.clone(),
            PendingSecretEntry {
                part_id: part_id.clone(),
                chat_secrets,
                owner_secrets: BTreeMap::new(),
            },
        );
        SecretGuard {
            runtime,
            conversation,
            part_id,
        }
    }

    /// Returns the sorted keys (never values) of throwaway secrets supplied in
    /// chat for the conversation's active `mannie_configure` call.
    pub(crate) fn chat_secret_keys(&self, conversation: &ConversationId) -> Vec<String> {
        lock(&self.active)
            .get(conversation)
            .map(|entry| entry.chat_secrets.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Stores secrets entered by the owner on the approval card for the active
    /// call. Does not modify `chat_secrets`, so re-preparing before execution
    /// produces the exact approval digest the owner saw (T-2).
    pub(crate) fn set_owner_secrets(
        &self,
        conversation: &ConversationId,
        part_id: &PartId,
        secrets: BTreeMap<String, Secret>,
    ) {
        if let Some(entry) = lock(&self.active)
            .get_mut(conversation)
            .filter(|entry| entry.part_id == *part_id)
        {
            entry.owner_secrets = secrets;
        }
    }

    /// Takes the merged secret map for execution, with owner card values
    /// overriding chat values, and clears both maps immediately.
    pub(crate) fn take_merged(
        &self,
        conversation: &ConversationId,
        part_id: Option<&PartId>,
    ) -> BTreeMap<String, Secret> {
        let mut active = lock(&self.active);
        let matches = active
            .get(conversation)
            .is_some_and(|entry| part_id.is_none_or(|id| entry.part_id == *id));

        if !matches {
            return BTreeMap::new();
        }

        let Some(entry) = active.remove(conversation) else {
            return BTreeMap::new();
        };

        let mut merged = entry.chat_secrets;
        for (key, value) in entry.owner_secrets {
            merged.insert(key, value);
        }
        merged
    }
}

/// RAII guard that removes and zeroizes pending configuration secrets when a
/// `mannie_configure` call ends, fails, is denied, stopped, or panicked.
#[derive(Debug)]
pub(crate) struct SecretGuard {
    runtime: Runtime,
    conversation: ConversationId,
    part_id: PartId,
}

impl Drop for SecretGuard {
    fn drop(&mut self) {
        let mut active = lock(&self.runtime.inner.config_secrets.active);
        let matches = active
            .get(&self.conversation)
            .is_some_and(|entry| entry.part_id == self.part_id);
        if matches {
            active.remove(&self.conversation);
        }
    }
}

/// Validates a plugin's `Management` contributions before registration.
pub(crate) fn check_management(
    snapshot: &Snapshot,
    contributions: &Contributions,
) -> Result<(), ContributionRefusal> {
    for entry in contributions.entries::<Management>() {
        if snapshot.find::<Management>(&entry.id).is_some() {
            return Err(ContributionRefusal::new(
                Management::NAME,
                ID_TAKEN,
                format!(
                    "a management provider with the ID {} is registered already",
                    entry.id
                ),
            ));
        }

        let guide = entry.provider.guide();
        if let Err(reason) = check_management_guide(&guide) {
            return Err(ContributionRefusal::new(
                Management::NAME,
                "invalid_guide",
                format!("management provider {}: {reason}", entry.id),
            ));
        }
    }

    Ok(())
}

impl Runtime {
    /// Attaches the desktop [`AppHost`] port once after plugin host startup so
    /// conversation agents can inspect and configure Mannie (ADR-0081).
    ///
    /// Workers never call this method. Calling it more than once ignores later
    /// hosts.
    pub fn attach_app_host(&self, host: Arc<dyn AppHost>) {
        if self.inner.app_host.set(host).is_err() {
            tracing::warn!("attach_app_host was called more than once; keeping the first host");
        }
    }

    /// Returns the attached desktop host port, or `None` on workers and tests
    /// without one.
    pub(crate) fn app_host(&self) -> Option<&Arc<dyn AppHost>> {
        self.inner.app_host.get()
    }

    /// Returns the runtime-owned self-management tools for a conversation
    /// agent answering to the owner when an [`AppHost`] is attached. Read-only
    /// agents (such as Plan) receive only the read-only inspection and log
    /// tools, never `mannie_configure`.
    pub(crate) fn self_management_tools(&self, read_only: bool) -> Vec<Arc<dyn Tool>> {
        let Some(host) = self.app_host().cloned() else {
            return Vec::new();
        };
        let mut offered: Vec<Arc<dyn Tool>> = vec![
            Arc::new(tools::MannieInspectTool::new(
                self.clone(),
                Arc::clone(&host),
            )),
            Arc::new(tools::MannieReadLogsTool::new(Arc::clone(&host))),
        ];
        if !read_only {
            offered.push(Arc::new(tools::MannieConfigureTool::new(
                self.clone(),
                host,
            )));
        }
        offered
    }

    /// Returns a compact live summary of Mannie's version and installed plugin
    /// states for the dynamic `mannie-state` prompt section (ADR-0081).
    pub(crate) fn mannie_state_section(&self) -> Option<String> {
        let host = self.app_host()?;
        let info = host.app_info();
        let plugins = host.list_plugins();
        let summary = if plugins.is_empty() {
            "none".to_owned()
        } else {
            plugins
                .iter()
                .map(|plugin| {
                    let state = match plugin.state {
                        PluginState::On => "on",
                        PluginState::Off => "off",
                        PluginState::Starting => "starting",
                        PluginState::Stopping => "stopping",
                        PluginState::Failed => "failed",
                    };
                    format!("{} ({state})", plugin.id)
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        Some(format!(
            "Mannie {} ({}) on {} {}. Installed plugins: {summary}.",
            info.version, info.build_profile, info.os, info.arch
        ))
    }

    /// Returns every live plugin [`ManagementProvider`] currently registered,
    /// reading live contributions rather than any turn's frozen snapshot so a
    /// plugin enabled mid-reply becomes inspectable and configurable on the
    /// next step (ADR-0081).
    pub fn management_providers(&self) -> Vec<(String, Arc<dyn ManagementProvider>)> {
        self.contributions()
            .items::<Management>()
            .iter()
            .map(|registered| {
                (
                    registered.entry.id.clone(),
                    Arc::clone(&registered.entry.provider),
                )
            })
            .collect()
    }

    /// Looks up one live plugin [`ManagementProvider`] by its owned ID (such as
    /// `"ssh"` or `"workers.databases"`).
    pub fn management_provider(&self, id: &str) -> Option<Arc<dyn ManagementProvider>> {
        self.contributions()
            .find::<Management>(id)
            .map(|registered| Arc::clone(&registered.entry.provider))
    }

    /// Returns the in-memory ephemeral config secrets table.
    pub(crate) fn config_secrets(&self) -> &ConfigSecrets {
        &self.inner.config_secrets
    }
}
