//! Desktop notifications, unread finished conversations and the system tray
//! badge (docs/features/notifications.md; ADR-0074).
//!
//! `tauri-plugin-notification` is registered without a webview capability so
//! the untrusted webview cannot send notifications directly (ADR-0004). Only
//! this module dispatches notifications, driven by the runtime's owner signal
//! stream, window focus, viewed conversation and persisted settings.

pub(crate) mod badge;
mod state;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mannie_logging::ErrorChain;
use mannie_runtime::{OwnerSignal, Runtime, SignalSubscription};
use mannie_store::Store;
use mannie_types::ids::ConversationId;
use mannie_types::notifications::{NotificationLabels, NotificationSettings, NotificationUpdate};
use mannie_types::window::TrayLabels;
use tauri::{App, AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;
use tokio_util::sync::CancellationToken;

use crate::ipc::channel::BatchSender;
use crate::window::{self, tray};

use self::state::{Effects, NotificationsModel, PendingToast};

#[cfg(feature = "validation")]
use mannie_types::validation::{RecordedNotification, ValidationNotificationsState};

#[derive(Debug, Default)]
struct Inner {
    model: Mutex<NotificationsModel>,
    tray_labels: Mutex<Option<TrayLabels>>,
    subscriber: Mutex<Option<BatchSender<NotificationUpdate>>>,
    pending_open: Mutex<Option<ConversationId>>,
    stop: Mutex<Option<CancellationToken>>,
    #[cfg(feature = "validation")]
    recorded: Mutex<Vec<RecordedNotification>>,
}

/// Shared desktop notifications and tray-badge coordinator.
#[derive(Debug, Clone, Default)]
pub(crate) struct Notifications {
    inner: Arc<Inner>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Registers the Rust click handler on `tauri-plugin-notification` so tapping
/// a toast brings Mannie to the front and opens the conversation (ADR-0074,
/// decision 6).
pub(crate) fn register_action_handler(app: &App) {
    let handle = app.handle().clone();

    if let Err(error) = app.notification().on_action(move |performed| {
        let notification_id = performed
            .notification()
            .map(tauri_plugin_notification::ActiveNotification::id);
        let app = handle.clone();
        tauri::async_runtime::spawn(async move {
            let core = app.state::<crate::state::Core>();

            core.notifications()
                .handle_click(&app, core.store(), notification_id)
                .await;
        });
    }) {
        tracing::warn!(%error, "could not register the desktop notification click handler");
    }
}

impl Notifications {
    /// Starts the signal observer with persisted settings and unread markers
    /// once the store and runtime are open.
    pub(crate) fn start(
        &self,
        app: AppHandle,
        store: Store,
        runtime: &Runtime,
        settings: NotificationSettings,
        unread: Vec<ConversationId>,
    ) {
        let mut effects = Effects::default();
        {
            let mut model = lock(&self.inner.model);
            let from_settings = model.set_settings(settings);
            let from_unread = model.load_unread(unread);
            effects.persist_clear = from_unread.persist_clear;
            effects.unread_changed = from_unread.unread_changed;
            effects.badge_changed = from_unread.badge_changed.or(from_settings.badge_changed);
            effects.taskbar_badge_changed = from_unread
                .taskbar_badge_changed
                .or(from_settings.taskbar_badge_changed);
        }
        self.apply_effects(&app, Some(&store), effects);

        let token = CancellationToken::new();
        let previous = lock(&self.inner.stop).replace(token.clone());

        if let Some(previous) = previous {
            previous.cancel();
        }

        let signals = runtime.subscribe_signals();
        let coordinator = self.clone();
        tauri::async_runtime::spawn(async move {
            coordinator.run_signals(app, store, signals, token).await;
        });
    }

    /// Stops the background signal observer when Mannie shuts down.
    pub(crate) fn stop(&self) {
        let token = lock(&self.inner.stop).take();

        if let Some(token) = token {
            token.cancel();
        }
    }

    /// The notification settings in memory.
    pub(crate) fn settings(&self) -> NotificationSettings {
        lock(&self.inner.model).settings()
    }

    /// Replaces the in-memory settings after they were saved to SQLite.
    pub(crate) fn set_settings(&self, app: &AppHandle, settings: NotificationSettings) {
        let effects = lock(&self.inner.model).set_settings(settings);
        self.apply_effects(app, None, effects);
    }

    /// Stores localized notification titles and tray attention tooltip, and
    /// refreshes the tray tooltip if the badge is currently active.
    pub(crate) fn set_labels(&self, app: &AppHandle, labels: NotificationLabels) {
        let badged = lock(&self.inner.model).set_labels(labels);

        if badged {
            self.refresh_tray(app);
        }
    }

    /// Stores the window's tray labels and refreshes the tray icon, preserving
    /// any active badge and attention tooltip.
    pub(crate) fn apply_tray_labels(
        &self,
        app: &AppHandle,
        labels: &TrayLabels,
    ) -> Result<(), tray::TrayError> {
        *lock(&self.inner.tray_labels) = Some(labels.clone());

        let (badged, attention) = self.tray_visual_state();
        tray::apply_labels(app, labels, badged, attention.as_deref())
    }

    /// Updates whether the main window is focused and clears the viewed
    /// conversation if it was unread.
    pub(crate) fn set_window_focused(&self, app: &AppHandle, store: Option<&Store>, focused: bool) {
        let effects = lock(&self.inner.model).set_window_focused(focused);
        self.apply_effects(app, store, effects);
    }

    /// Updates which conversation the main window currently shows.
    pub(crate) fn set_viewed_conversation(
        &self,
        app: &AppHandle,
        store: Option<&Store>,
        conversation_id: Option<ConversationId>,
    ) {
        let effects = lock(&self.inner.model).set_viewed_conversation(conversation_id);
        self.apply_effects(app, store, effects);
    }

    /// Connects the window's notifications channel, immediately sending the
    /// current unread set and any buffered click-to-open target.
    pub(crate) fn subscribe(&self, subscriber: BatchSender<NotificationUpdate>) {
        let unread_ids = lock(&self.inner.model).unread_ids();
        let pending_open = lock(&self.inner.pending_open).take();

        if subscriber
            .try_send(NotificationUpdate::UnreadChanged {
                conversation_ids: unread_ids,
            })
            .is_err()
        {
            tracing::warn!("could not send the initial unread set to the window");
        }
        if let Some(conversation_id) = pending_open
            && subscriber
                .try_send(NotificationUpdate::OpenConversation { conversation_id })
                .is_err()
        {
            tracing::warn!("could not deliver a pending notification click target");
        }

        *lock(&self.inner.subscriber) = Some(subscriber);
    }

    /// Forgets the window's channel and viewed conversation on page reload.
    pub(crate) fn page_loaded(&self) {
        *lock(&self.inner.subscriber) = None;
        lock(&self.inner.model).set_viewed_conversation(None);
    }

    /// Handles a click on notification `notification_id`: brings the window to
    /// the front and opens its conversation if it still exists.
    pub(crate) async fn handle_click(
        &self,
        app: &AppHandle,
        store: Option<&Store>,
        notification_id: Option<i32>,
    ) -> bool {
        window::show_main(app);

        let Some(id) = notification_id else {
            return false;
        };
        let Some(conversation_id) = lock(&self.inner.model).target_for_notification(id) else {
            return false;
        };

        if let Some(store) = store {
            match store.conversation_exists(&conversation_id).await {
                Ok(true) => {}
                Ok(false) => {
                    tracing::info!(
                        conversation = %conversation_id,
                        "clicked notification refers to a deleted conversation"
                    );
                    return false;
                }
                Err(error) => {
                    tracing::warn!(
                        error = %ErrorChain(&error),
                        "could not verify whether a clicked notification's conversation still exists"
                    );
                }
            }
        }

        self.send_open_conversation(conversation_id);
        true
    }

    /// Snapshot of notifications state for validation builds (V14).
    #[cfg(feature = "validation")]
    pub(crate) fn validation_state(&self) -> ValidationNotificationsState {
        let (window_focused, tray_badged, taskbar_badge_count, unread_conversation_ids) = {
            let model = lock(&self.inner.model);
            (
                model.window_focused(),
                model.badge_active(),
                model.taskbar_badge_count(),
                model.unread_ids(),
            )
        };
        let notifications = lock(&self.inner.recorded).clone();
        ValidationNotificationsState {
            window_focused,
            tray_badged,
            taskbar_badge_count,
            unread_conversation_ids,
            notifications,
        }
    }

    /// Applies the current taskbar or dock count badge to the main window.
    ///
    /// Called when the count changes and when the main window is created or
    /// shown again from the tray, since Windows clears `ITaskbarList3` overlay
    /// icons when a window is hidden.
    pub(crate) fn refresh_taskbar(&self, app: &AppHandle) {
        let count = lock(&self.inner.model).taskbar_badge_count();
        Self::apply_taskbar_badge(app, count);
    }

    async fn run_signals(
        self,
        app: AppHandle,
        store: Store,
        mut signals: SignalSubscription,
        token: CancellationToken,
    ) {
        loop {
            let next = tokio::select! {
                () = token.cancelled() => return,
                signal = signals.next() => signal,
            };
            let Some(signal) = next else {
                return;
            };

            if matches!(signal, OwnerSignal::Shutdown) {
                return;
            }

            let effects = lock(&self.inner.model).on_signal(signal);
            self.apply_effects(&app, Some(&store), effects);
        }
    }

    fn apply_effects(&self, app: &AppHandle, store: Option<&Store>, effects: Effects) {
        if let Some(conversation_id) = effects.persist_mark
            && let Some(store) = store.cloned()
        {
            tauri::async_runtime::spawn(async move {
                if let Err(error) = store.mark_conversation_unread(&conversation_id).await {
                    tracing::warn!(
                        error = %ErrorChain(&error),
                        conversation = %conversation_id,
                        "could not persist unread conversation mark"
                    );
                }
            });
        }

        if let Some(conversation_id) = effects.persist_clear
            && let Some(store) = store.cloned()
        {
            tauri::async_runtime::spawn(async move {
                if let Err(error) = store.clear_conversation_unread(&conversation_id).await {
                    tracing::warn!(
                        error = %ErrorChain(&error),
                        conversation = %conversation_id,
                        "could not clear persisted unread conversation mark"
                    );
                }
            });
        }

        if let Some(conversation_ids) = effects.unread_changed {
            self.send_update(NotificationUpdate::UnreadChanged { conversation_ids });
        }

        if effects.badge_changed.is_some() {
            self.refresh_tray(app);
        }

        if let Some(count) = effects.taskbar_badge_changed {
            Self::apply_taskbar_badge(app, count);
        }

        if let Some(toast) = effects.dispatch {
            self.dispatch_toast(app, toast);
        }
    }

    #[cfg(feature = "validation")]
    fn apply_taskbar_badge(_app: &AppHandle, _count: u32) {}

    #[cfg(not(feature = "validation"))]
    fn apply_taskbar_badge(app: &AppHandle, count: u32) {
        let Some(window) = app.get_webview_window(window::MAIN_WINDOW) else {
            return;
        };

        #[cfg(target_os = "windows")]
        {
            let icon = badge::taskbar_count_overlay(count);

            if let Err(error) = window.set_overlay_icon(icon) {
                tracing::warn!(%error, count, "could not update the taskbar badge");
            }
        }

        #[cfg(target_os = "linux")]
        {
            let badge_count = (count > 0).then(|| i64::from(count));

            if let Err(error) = window.set_badge_count(badge_count) {
                tracing::warn!(%error, count, "could not update the dock badge count");
            }
        }
    }

    fn tray_visual_state(&self) -> (bool, Option<String>) {
        let model = lock(&self.inner.model);
        (
            model.badge_active(),
            model.attention_tooltip().map(str::to_owned),
        )
    }

    fn refresh_tray(&self, app: &AppHandle) {
        let base_tooltip = lock(&self.inner.tray_labels)
            .as_ref()
            .map(|labels| labels.tooltip.clone());
        let (badged, attention) = self.tray_visual_state();
        let tooltip = if badged {
            attention.as_deref().or(base_tooltip.as_deref())
        } else {
            base_tooltip.as_deref()
        };

        if let Err(error) = tray::apply_badge(app, badged, tooltip) {
            tracing::warn!(error = %ErrorChain(&error), "could not update the tray badge");
        }
    }

    fn send_update(&self, update: NotificationUpdate) {
        if let Some(subscriber) = lock(&self.inner.subscriber).as_ref()
            && subscriber.try_send(update).is_err()
        {
            tracing::warn!(
                "the window missed a notification update; it receives it again on subscribe"
            );
        }
    }

    fn send_open_conversation(&self, conversation_id: ConversationId) {
        let sent = lock(&self.inner.subscriber)
            .as_ref()
            .is_some_and(|subscriber| {
                subscriber
                    .try_send(NotificationUpdate::OpenConversation {
                        conversation_id: conversation_id.clone(),
                    })
                    .is_ok()
            });

        if !sent {
            *lock(&self.inner.pending_open) = Some(conversation_id);
        }
    }

    #[cfg(feature = "validation")]
    fn dispatch_toast(&self, _app: &AppHandle, toast: PendingToast) {
        tracing::info!(
            id = toast.id,
            kind = toast.kind.as_str(),
            conversation = %toast.conversation_id,
            "recorded desktop notification in validation build"
        );
        lock(&self.inner.recorded).push(RecordedNotification {
            id: toast.id,
            kind: toast.kind,
            title: toast.title,
            body: toast.body,
            conversation_id: toast.conversation_id,
        });
    }

    #[cfg(not(feature = "validation"))]
    #[expect(
        clippy::unused_self,
        clippy::needless_pass_by_value,
        reason = "signature matches the validation build, which stores the owned toast on self"
    )]
    fn dispatch_toast(&self, app: &AppHandle, toast: PendingToast) {
        tracing::info!(
            id = toast.id,
            kind = toast.kind.as_str(),
            conversation = %toast.conversation_id,
            "sending desktop notification"
        );
        let mut builder = app
            .notification()
            .builder()
            .id(toast.id)
            .title(&toast.title);

        if !toast.body.is_empty() {
            builder = builder.body(&toast.body);
        }
        if let Err(error) = builder.show() {
            tracing::warn!(
                %error,
                kind = toast.kind.as_str(),
                "could not show a desktop notification"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    fn capability_files(directory: &Path, found: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();

            if path.is_dir() {
                capability_files(&path, found);
            } else if path.extension().is_some_and(|ext| ext == "json") {
                found.push(path);
            }
        }
    }

    /// `tauri-plugin-notification` is registered without a capability, so the
    /// untrusted webview cannot invoke any `notification:*` command directly
    /// (ADR-0004; ADR-0074, decision 2; T-111).
    #[test]
    fn no_capability_grants_a_permission_of_notifications() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src-tauri/capabilities");
        let mut files = Vec::new();
        capability_files(&folder, &mut files);
        assert!(
            !files.is_empty(),
            "{} holds no capability",
            folder.display()
        );

        for file in files {
            let capability: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
            let permissions = capability["permissions"].as_array().unwrap();
            assert!(!permissions.is_empty(), "{} grants nothing", file.display());

            for permission in permissions {
                let name = permission
                    .as_str()
                    .or_else(|| permission["identifier"].as_str())
                    .unwrap();
                assert!(
                    !name.starts_with("notification:"),
                    "{} grants {name}",
                    file.display()
                );
            }
        }
    }
}
