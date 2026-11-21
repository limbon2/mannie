//! Commands of the notifications and tray badge feature
//! (docs/features/notifications.md; ADR-0074): settings, localized labels from
//! the window's message catalog, viewed conversation tracking and the
//! notifications channel.

use mannie_logging::ErrorChain;
use mannie_store::SettingsStoreError;
use mannie_types::ipc::{ChannelBatch, CommandError};
use mannie_types::notifications::{
    NotificationError, NotificationLabels, NotificationSettings, NotificationUpdate,
    SetViewedConversation, UpdateNotificationSettings,
};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::ipc::channel;
use crate::state::Core;

fn refused(
    command: &'static str,
) -> impl FnOnce(NotificationError) -> CommandError<NotificationError> {
    move |error| {
        tracing::warn!(
            command,
            error.code = error.code(),
            "a notifications command was refused"
        );
        CommandError::new(error)
    }
}

fn storage_error(error: &SettingsStoreError) -> NotificationError {
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

/// Returns the notification settings.
#[tauri::command]
#[specta::specta]
pub(crate) async fn get_notification_settings(
    core: State<'_, Core>,
) -> Result<NotificationSettings, CommandError<NotificationError>> {
    if core.store().is_none() {
        return Err(refused("get_notification_settings")(
            NotificationError::StoreUnavailable,
        ));
    }

    Ok(core.notifications().settings())
}

/// Persists updated notification settings and applies them in memory.
#[tauri::command]
#[specta::specta]
pub(crate) async fn update_notification_settings(
    app: AppHandle,
    core: State<'_, Core>,
    update: UpdateNotificationSettings,
) -> Result<NotificationSettings, CommandError<NotificationError>> {
    let store = core.store().ok_or_else(|| {
        refused("update_notification_settings")(NotificationError::StoreUnavailable)
    })?;

    let settings = NotificationSettings::from(update);
    store
        .save_settings(&settings)
        .await
        .map_err(|error| refused("update_notification_settings")(storage_error(&error)))?;
    core.notifications().set_settings(&app, settings);
    tracing::info!(
        enabled = settings.enabled,
        taskbar_badge = settings.taskbar_badge,
        tray_badge = settings.tray_badge,
        "changed the notification settings"
    );
    Ok(settings)
}

/// Gives the core the localized titles for desktop notifications and the
/// attention tray tooltip from the window's message catalog (ADR-0019).
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn set_notification_labels(
    app: AppHandle,
    core: State<'_, Core>,
    labels: NotificationLabels,
) -> Result<(), CommandError<NotificationError>> {
    labels.check().map_err(refused("set_notification_labels"))?;
    core.notifications().set_labels(&app, labels);
    Ok(())
}

/// Reports which conversation the main window currently displays, or `None`
/// when another view is open. Opening an unread conversation while the window
/// is focused clears its unread mark and updates the tray badge.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn set_viewed_conversation(
    app: AppHandle,
    core: State<'_, Core>,
    input: SetViewedConversation,
) {
    core.notifications()
        .set_viewed_conversation(&app, core.store(), input.conversation_id);
}

/// Opens the notifications channel from the core to the window: current unread
/// finished conversations and click-to-open navigation directives.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn subscribe_notifications(
    app: AppHandle,
    core: State<'_, Core>,
    updates: Channel<ChannelBatch<NotificationUpdate>>,
) {
    let (sender, _sending) = channel::batched(updates, channel::FRAME_INTERVAL);
    core.notifications().subscribe(sender);
    core.notifications().refresh_taskbar(&app);
}
