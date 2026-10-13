//! Commands for the interface settings section: the locale of the UI
//! (ADR-0019).

use mannie_logging::ErrorChain;
use mannie_store::SettingsStoreError;
use mannie_types::interface::{
    InterfaceSettings, InterfaceSettingsError, LocaleTag, UpdateInterfaceSettings,
};
use mannie_types::ipc::CommandError;
use tauri::State;

use crate::state::Core;

/// Reads the interface settings.
#[tauri::command]
#[specta::specta]
pub(crate) async fn get_interface_settings(
    core: State<'_, Core>,
) -> Result<InterfaceSettings, CommandError<InterfaceSettingsError>> {
    let store = core
        .store()
        .ok_or(InterfaceSettingsError::StoreUnavailable)?;
    store
        .load_settings::<InterfaceSettings>()
        .await
        .map_err(|error| storage_error(&error))
}

/// Replaces the interface settings and returns what was stored. The UI reloads
/// itself afterwards so that it shows the new locale.
#[tauri::command]
#[specta::specta]
pub(crate) async fn update_interface_settings(
    core: State<'_, Core>,
    update: UpdateInterfaceSettings,
) -> Result<InterfaceSettings, CommandError<InterfaceSettingsError>> {
    // The webview is untrusted: the core checks the request (ADR-0004).
    let settings = update.into_settings().inspect_err(|error| {
        tracing::warn!(error.code = error.code(), %error, "refused an interface settings update");
    })?;
    let store = core
        .store()
        .ok_or(InterfaceSettingsError::StoreUnavailable)?;
    store
        .save_settings(&settings)
        .await
        .map_err(|error| storage_error(&error))?;
    tracing::info!(
        locale = settings.locale.as_ref().map(LocaleTag::as_str),
        "changed the interface settings"
    );
    Ok(settings)
}

/// Logs a store failure with its cause and turns it into the error the UI
/// shows.
fn storage_error(error: &SettingsStoreError) -> CommandError<InterfaceSettingsError> {
    tracing::error!(error = %ErrorChain(error), error.code = error.code(), "the interface settings failed");
    let shown = match error {
        SettingsStoreError::NewerVersion { .. } => InterfaceSettingsError::WrittenByNewerVersion,
        _ => InterfaceSettingsError::StorageFailed,
    };
    CommandError::new(shown)
}
