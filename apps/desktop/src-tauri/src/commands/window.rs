//! Commands of the window and tray feature (docs/features/window-and-tray.md):
//! the questions the core asks the window, the answers, the settings section,
//! and the tray's labels.

use mannie_logging::ErrorChain;
use mannie_store::SettingsStoreError;
use mannie_types::ipc::{ChannelBatch, CommandError};
use mannie_types::window::{
    AnswerCloseQuestion, CloseChoice, TrayLabels, UpdateWindowSettings, WindowError, WindowRequest,
    WindowRequestRef, WindowRequestUpdate, WindowSettings,
};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::ipc::channel;
use crate::state::Core;
use crate::window::{close, tray};

fn refused(command: &'static str) -> impl FnOnce(WindowError) -> CommandError<WindowError> {
    move |error| {
        tracing::warn!(
            command,
            error.code = error.code(),
            "a window command was refused"
        );
        CommandError::new(error)
    }
}

fn storage_error(error: &SettingsStoreError) -> WindowError {
    tracing::error!(error = %ErrorChain(error), error.code = error.code(), "the window settings failed");
    match error {
        SettingsStoreError::NewerVersion { .. } => WindowError::WrittenByNewerVersion,
        _ => WindowError::StorageFailed,
    }
}

/// Saves the window settings durably, then keeps them in memory.
async fn save_settings(
    core: &Core,
    settings: WindowSettings,
) -> Result<WindowSettings, WindowError> {
    let store = core.store().ok_or(WindowError::StoreUnavailable)?;
    store
        .save_settings(&settings)
        .await
        .map_err(|error| storage_error(&error))?;
    core.set_window_settings(settings.clone());
    tracing::info!(close_action = ?settings.close_action, "changed the window settings");
    Ok(settings)
}

/// Opens the channel of the core's questions to the window. A question that
/// waits is sent again at once.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn subscribe_window_requests(
    core: State<'_, Core>,
    updates: Channel<ChannelBatch<WindowRequestUpdate>>,
) {
    let (sender, _sending) = channel::batched(updates, channel::FRAME_INTERVAL);
    core.window_requests().subscribe(sender);
}

/// Answers the question of what closing does: saves the choice, then hides
/// the window or quits. The question already told the owner about active
/// replies.
#[tauri::command]
#[specta::specta]
pub(crate) async fn answer_close_question(
    app: AppHandle,
    core: State<'_, Core>,
    input: AnswerCloseQuestion,
) -> Result<(), CommandError<WindowError>> {
    let request = core
        .window_requests()
        .pending(input.request_id)
        .map_err(refused("answer_close_question"))?;
    if !matches!(request, WindowRequest::AskCloseAction { .. }) {
        return Err(refused("answer_close_question")(
            WindowError::RequestNotPending,
        ));
    }
    let settings = WindowSettings {
        close_action: input.choice.into(),
    };
    // The question stays until the choice is saved, so the window can show a
    // failure and the owner can try again.
    save_settings(&core, settings)
        .await
        .map_err(refused("answer_close_question"))?;
    core.window_requests()
        .take(input.request_id)
        .map_err(refused("answer_close_question"))?;
    match input.choice {
        CloseChoice::Tray => close::hide_main(&app),
        CloseChoice::Quit => close::quit(&app),
    }
    Ok(())
}

/// Quits after the owner confirmed that active replies are interrupted.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn confirm_quit(
    app: AppHandle,
    core: State<'_, Core>,
    input: WindowRequestRef,
) -> Result<(), CommandError<WindowError>> {
    let request = core
        .window_requests()
        .take(input.request_id)
        .map_err(refused("confirm_quit"))?;
    if !matches!(request, WindowRequest::ConfirmQuit { .. }) {
        return Err(refused("confirm_quit")(WindowError::RequestNotPending));
    }
    close::quit(&app);
    Ok(())
}

/// Dismisses a question: the window stays open and nothing changes.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn dismiss_window_request(
    core: State<'_, Core>,
    input: WindowRequestRef,
) -> Result<(), CommandError<WindowError>> {
    core.window_requests()
        .take(input.request_id)
        .map(|_request| ())
        .map_err(refused("dismiss_window_request"))
}

/// The window settings.
#[tauri::command]
#[specta::specta]
pub(crate) async fn get_window_settings(
    core: State<'_, Core>,
) -> Result<WindowSettings, CommandError<WindowError>> {
    if core.store().is_none() {
        return Err(refused("get_window_settings")(
            WindowError::StoreUnavailable,
        ));
    }
    Ok(core.window_settings())
}

/// Replaces the window settings and returns what was stored.
#[tauri::command]
#[specta::specta]
pub(crate) async fn update_window_settings(
    core: State<'_, Core>,
    update: UpdateWindowSettings,
) -> Result<WindowSettings, CommandError<WindowError>> {
    let settings = WindowSettings {
        close_action: update.close_action,
    };
    save_settings(&core, settings)
        .await
        .map_err(refused("update_window_settings"))
}

/// Creates the tray with the window's labels, or gives it new ones. A tray
/// that cannot be created is logged; closing to the tray then only hides the
/// window, and starting Mannie again shows it.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn set_tray_labels(
    app: AppHandle,
    labels: TrayLabels,
) -> Result<(), CommandError<WindowError>> {
    labels.check().map_err(refused("set_tray_labels"))?;
    if let Err(error) = tray::apply_labels(&app, &labels) {
        tracing::warn!(error = %ErrorChain(&error), "the tray could not be set up; Mannie runs without it");
    }
    Ok(())
}
