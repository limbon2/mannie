//! Desktop composition-root implementation of [`mannie_runtime::AppHost`]
//! (ADR-0081).
//!
//! Bridges the runtime's self-management tools to the desktop host's plugins,
//! window settings, notification settings, interface settings, desktop updates
//! and canonical storage paths without creating a reference cycle back to
//! [`Runtime`](mannie_runtime::Runtime).

use std::sync::{Arc, PoisonError, RwLock};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_logging::ErrorChain;
use mannie_plugins::PluginHost;
use mannie_runtime::{AppHost, AppRuntimeInfo};
use mannie_store::{SettingsStoreError, Store};
use mannie_types::ids::ConversationId;
use mannie_types::interface::{
    InterfaceSettings, InterfaceSettingsError, LocaleTag, UpdateInterfaceSettings,
};
use mannie_types::notifications::{
    NotificationError, NotificationSettings, UpdateNotificationSettings,
};
use mannie_types::plugins::{PluginView, PluginWork, PluginsError};
use mannie_types::updates::{UpdateStatus, UpdateUpdatesSettings, UpdatesError, UpdatesSettings};
use mannie_types::window::{UpdateWindowSettings, WindowError, WindowSettings};
use tauri::AppHandle;

use crate::browser::BrowserProvider;
use crate::notifications::Notifications;
use crate::updates::Updates;

/// Desktop host port attached once to the master runtime after startup.
#[derive(Debug)]
pub(crate) struct DesktopAppHost {
    app: AppHandle,
    store: Store,
    plugins: PluginHost,
    browser: Arc<BrowserProvider>,
    updates: Updates,
    notifications: Notifications,
    window_settings: Arc<RwLock<WindowSettings>>,
    info: AppRuntimeInfo,
}

impl DesktopAppHost {
    /// Builds the desktop [`AppHost`] from non-cyclic handles to the master
    /// services.
    #[expect(
        clippy::too_many_arguments,
        reason = "composition root wires each independent desktop subsystem explicitly"
    )]
    pub(crate) fn new(
        app: AppHandle,
        store: Store,
        plugins: PluginHost,
        browser: Arc<BrowserProvider>,
        updates: Updates,
        notifications: Notifications,
        window_settings: Arc<RwLock<WindowSettings>>,
        info: AppRuntimeInfo,
    ) -> Self {
        Self {
            app,
            store,
            plugins,
            browser,
            updates,
            notifications,
            window_settings,
            info,
        }
    }
}

fn interface_storage_error(error: &SettingsStoreError) -> InterfaceSettingsError {
    tracing::error!(
        error = %ErrorChain(error),
        error.code = error.code(),
        "the interface settings failed"
    );
    match error {
        SettingsStoreError::NewerVersion { .. } => InterfaceSettingsError::WrittenByNewerVersion,
        _ => InterfaceSettingsError::StorageFailed,
    }
}

fn window_storage_error(error: &SettingsStoreError) -> WindowError {
    tracing::error!(
        error = %ErrorChain(error),
        error.code = error.code(),
        "the window settings failed"
    );
    match error {
        SettingsStoreError::NewerVersion { .. } => WindowError::WrittenByNewerVersion,
        _ => WindowError::StorageFailed,
    }
}

fn notification_storage_error(error: &SettingsStoreError) -> NotificationError {
    tracing::error!(
        error = %ErrorChain(error),
        error.code = error.code(),
        "the notification settings failed"
    );
    match error {
        SettingsStoreError::NewerVersion { .. } => NotificationError::WrittenByNewerVersion,
        _ => NotificationError::StorageFailed,
    }
}

fn updates_storage_error(error: &SettingsStoreError) -> UpdatesError {
    tracing::error!(
        error = %ErrorChain(error),
        error.code = error.code(),
        "the updates settings failed"
    );
    match error {
        SettingsStoreError::NewerVersion { .. } => UpdatesError::WrittenByNewerVersion,
        _ => UpdatesError::StorageFailed,
    }
}

/// Human-readable description of the active OS credential store backend.
pub(crate) fn credential_store_description() -> String {
    if cfg!(target_os = "windows") {
        "Windows Credential Manager".to_owned()
    } else if cfg!(target_os = "macos") {
        "macOS Keychain".to_owned()
    } else {
        "OS keyring (Secret Service / keyutils)".to_owned()
    }
}

impl AppHost for DesktopAppHost {
    fn app_info(&self) -> AppRuntimeInfo {
        self.info.clone()
    }

    fn list_plugins(&self) -> Vec<PluginView> {
        self.plugins.views()
    }

    fn plugin_work(
        &self,
        plugin_id: String,
    ) -> BoxFuture<'static, Result<PluginWork, PluginsError>> {
        let plugins = self.plugins.clone();
        async move { plugins.work(&plugin_id).await }.boxed()
    }

    fn plugin_feature_work(
        &self,
        plugin_id: String,
        feature_id: String,
    ) -> BoxFuture<'static, Result<PluginWork, PluginsError>> {
        let plugins = self.plugins.clone();
        async move { plugins.feature_work(&plugin_id, &feature_id).await }.boxed()
    }

    fn set_plugin_enabled(
        &self,
        plugin_id: String,
        enabled: bool,
    ) -> BoxFuture<'static, Result<PluginView, PluginsError>> {
        let plugins = self.plugins.clone();
        async move { plugins.set_enabled(&plugin_id, enabled).await }.boxed()
    }

    fn set_plugin_feature_enabled(
        &self,
        plugin_id: String,
        feature_id: String,
        enabled: bool,
    ) -> BoxFuture<'static, Result<PluginView, PluginsError>> {
        let plugins = self.plugins.clone();
        async move {
            plugins
                .set_feature_enabled(&plugin_id, &feature_id, enabled)
                .await
        }
        .boxed()
    }

    fn window_settings(&self) -> WindowSettings {
        self.window_settings
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn update_window_settings(
        &self,
        update: UpdateWindowSettings,
    ) -> BoxFuture<'static, Result<WindowSettings, WindowError>> {
        let store = self.store.clone();
        let cache = Arc::clone(&self.window_settings);
        let settings = WindowSettings {
            close_action: update.close_action,
        };
        async move {
            store
                .save_settings(&settings)
                .await
                .map_err(|error| window_storage_error(&error))?;
            *cache.write().unwrap_or_else(PoisonError::into_inner) = settings.clone();
            tracing::info!(
                close_action = ?settings.close_action,
                "changed the window settings"
            );
            Ok(settings)
        }
        .boxed()
    }

    fn notification_settings(&self) -> NotificationSettings {
        self.notifications.settings()
    }

    fn update_notification_settings(
        &self,
        update: UpdateNotificationSettings,
    ) -> BoxFuture<'static, Result<NotificationSettings, NotificationError>> {
        let store = self.store.clone();
        let notifications = self.notifications.clone();
        let app = self.app.clone();
        let settings = NotificationSettings::from(update);
        async move {
            store
                .save_settings(&settings)
                .await
                .map_err(|error| notification_storage_error(&error))?;
            notifications.set_settings(&app, settings);
            tracing::info!(
                enabled = settings.enabled,
                taskbar_badge = settings.taskbar_badge,
                tray_badge = settings.tray_badge,
                "changed the notification settings"
            );
            Ok(settings)
        }
        .boxed()
    }

    fn update_status(&self) -> UpdateStatus {
        self.updates.status()
    }

    fn updates_settings(&self) -> BoxFuture<'static, Result<UpdatesSettings, UpdatesError>> {
        let store = self.store.clone();
        async move {
            store
                .load_settings::<UpdatesSettings>()
                .await
                .map_err(|error| updates_storage_error(&error))
        }
        .boxed()
    }

    fn update_updates_settings(
        &self,
        update: UpdateUpdatesSettings,
    ) -> BoxFuture<'static, Result<UpdatesSettings, UpdatesError>> {
        let store = self.store.clone();
        let updates = self.updates.clone();
        let settings = UpdatesSettings {
            check_automatically: update.check_automatically,
        };
        async move {
            store
                .save_settings(&settings)
                .await
                .map_err(|error| updates_storage_error(&error))?;
            tracing::info!(
                check_automatically = settings.check_automatically,
                "changed the updates settings"
            );
            updates.set_automatic(settings.check_automatically);
            Ok(settings)
        }
        .boxed()
    }

    fn check_for_updates(&self) -> Result<(), UpdatesError> {
        self.updates.check()
    }

    fn interface_settings(
        &self,
    ) -> BoxFuture<'static, Result<InterfaceSettings, InterfaceSettingsError>> {
        let store = self.store.clone();
        async move {
            store
                .load_settings::<InterfaceSettings>()
                .await
                .map_err(|error| interface_storage_error(&error))
        }
        .boxed()
    }

    fn update_interface_settings(
        &self,
        update: UpdateInterfaceSettings,
    ) -> BoxFuture<'static, Result<InterfaceSettings, InterfaceSettingsError>> {
        let store = self.store.clone();
        async move {
            let settings = update.into_settings().inspect_err(|error| {
                tracing::warn!(
                    error.code = error.code(),
                    %error,
                    "refused an interface settings update"
                );
            })?;
            store
                .save_settings(&settings)
                .await
                .map_err(|error| interface_storage_error(&error))?;
            tracing::info!(
                locale = settings.locale.as_ref().map(LocaleTag::as_str),
                "changed the interface settings"
            );
            Ok(settings)
        }
        .boxed()
    }

    fn conversations_deleted(
        &self,
        conversation_ids: Vec<ConversationId>,
    ) -> BoxFuture<'static, ()> {
        let browser = Arc::clone(&self.browser);
        let plugins = self.plugins.clone();
        async move {
            if conversation_ids.is_empty() {
                return;
            }
            for conversation in &conversation_ids {
                browser.deleted(conversation).await;
            }
            plugins.conversations_deleted(&conversation_ids).await;
        }
        .boxed()
    }
}
