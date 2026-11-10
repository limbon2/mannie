//! Commands for updates of the app (docs/features/updates.md, ADR-0063). No
//! capability grants the updater plugin's own commands, so these are the
//! window's only way to an update (ADR-0063, decision 2).

use mannie_logging::ErrorChain;
use mannie_store::SettingsStoreError;
use mannie_types::ipc::{ChannelBatch, CommandError};
use mannie_types::transcript::{SubscriptionEnd, SubscriptionId};
use mannie_types::updates::{
    OpenReleasePage, RestartToUpdate, RestartToUpdateAnswer, UpdateStatusUpdate,
    UpdateUpdatesSettings, UpdatesError, UpdatesSettings,
};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::state::Core;
use crate::updates::RunningWork;

/// Logs a refused or failed command by its code, and wraps the error for the
/// UI.
fn refused(command: &'static str) -> impl FnOnce(UpdatesError) -> CommandError<UpdatesError> {
    move |error| {
        tracing::warn!(
            command,
            error.code = error.code(),
            "an updates command was refused"
        );
        CommandError::new(error)
    }
}

/// Logs a store failure with its cause and turns it into the error the UI
/// shows.
fn storage_error(error: &SettingsStoreError) -> CommandError<UpdatesError> {
    tracing::error!(error = %ErrorChain(error), error.code = error.code(), "the updates settings failed");
    let shown = match error {
        SettingsStoreError::NewerVersion { .. } => UpdatesError::WrittenByNewerVersion,
        _ => UpdatesError::StorageFailed,
    };
    CommandError::new(shown)
}

/// Reads the updates settings.
#[tauri::command]
#[specta::specta]
pub(crate) async fn get_updates_settings(
    core: State<'_, Core>,
) -> Result<UpdatesSettings, CommandError<UpdatesError>> {
    let store = core.store().ok_or(UpdatesError::StoreUnavailable)?;
    store
        .load_settings::<UpdatesSettings>()
        .await
        .map_err(|error| storage_error(&error))
}

/// Replaces the updates settings and returns what was stored. Turning the
/// automatic checks on starts a check.
#[tauri::command]
#[specta::specta]
pub(crate) async fn update_updates_settings(
    core: State<'_, Core>,
    update: UpdateUpdatesSettings,
) -> Result<UpdatesSettings, CommandError<UpdatesError>> {
    let settings = UpdatesSettings {
        check_automatically: update.check_automatically,
    };
    let store = core.store().ok_or(UpdatesError::StoreUnavailable)?;
    store
        .save_settings(&settings)
        .await
        .map_err(|error| storage_error(&error))?;
    tracing::info!(
        check_automatically = settings.check_automatically,
        "changed the updates settings"
    );
    // The schedule follows what was stored, never what was only asked for.
    core.updates().set_automatic(settings.check_automatically);
    Ok(settings)
}

/// Follows the status of the updates: as it is first, then after each change.
#[tauri::command]
#[specta::specta]
pub(crate) async fn subscribe_updates(
    core: State<'_, Core>,
    updates: Channel<ChannelBatch<UpdateStatusUpdate>>,
) -> Result<SubscriptionId, CommandError<UpdatesError>> {
    let page = core.subscriptions().page();
    let mut status = core.updates().subscribe();
    Ok(core
        .subscriptions()
        .start(page, updates, |sender| async move {
            loop {
                let current = status.borrow_and_update().clone();
                if sender
                    .send(UpdateStatusUpdate::Status { status: current })
                    .await
                    .is_err()
                {
                    break;
                }
                if status.changed().await.is_err() {
                    // The core is gone: Mannie quits.
                    let _sent = sender
                        .send(UpdateStatusUpdate::Ended {
                            reason: SubscriptionEnd::Shutdown,
                        })
                        .await;
                    break;
                }
            }
        }))
}

/// Starts a check for a newer release and answers at once. What the check
/// finds arrives through the status. Nothing starts while a check or a
/// download runs.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn check_for_updates(core: State<'_, Core>) -> Result<(), CommandError<UpdatesError>> {
    // An update installs only with its record in the journal (ADR-0063,
    // decision 9), so Mannie offers none while the database is not open.
    if core.store().is_none() {
        return Err(refused("check_for_updates")(UpdatesError::StoreUnavailable));
    }
    core.updates().check().map_err(refused("check_for_updates"))
}

/// "Restart and update": installs the version that is ready and starts
/// Mannie again. When the restart would end work that runs and the owner did
/// not confirm it, the answer names that work and nothing changes.
#[tauri::command]
#[specta::specta]
pub(crate) async fn restart_to_update(
    app: AppHandle,
    core: State<'_, Core>,
    input: RestartToUpdate,
) -> Result<RestartToUpdateAnswer, CommandError<UpdatesError>> {
    // The webview is untrusted (ADR-0004): the core installs only the version
    // that is ready, and only when it is the one that the owner saw (ADR-0063,
    // decision 8). The check comes first, so that the question about running
    // work is never asked for a version that would not install.
    core.updates()
        .require_ready(&input.version)
        .map_err(refused("restart_to_update"))?;
    let running = RunningWork::in_core(&core);
    if let Some(answer) = crate::updates::confirmation(running, input.confirmed) {
        return Ok(answer);
    }
    crate::updates::restart(&app, &core, &input.version)
        .await
        .map_err(refused("restart_to_update"))?;
    Ok(RestartToUpdateAnswer::Restarting)
}

/// Drops the result of the last install from the status, when the owner
/// closes its notice.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn dismiss_update_outcome(core: State<'_, Core>) {
    core.updates().dismiss_outcome();
}

/// Opens a page of Mannie's releases in the system browser, only on the
/// owner's click. The window names the page, and the core builds its address
/// from the versions that it holds.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn open_release_page(
    core: State<'_, Core>,
    input: OpenReleasePage,
) -> Result<(), CommandError<UpdatesError>> {
    crate::updates::open_release_page(&core, input.page).map_err(refused("open_release_page"))
}
