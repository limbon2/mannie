//! Commands of provider profiles (docs/features/chat/providers.md, contracts).
//! The runtime checks every field and keeps credentials in the credential
//! store; the webview sends a credential once and never receives it back.

use mannie_provider::known_model;
use mannie_runtime::Runtime;
use mannie_types::ipc::CommandError;
use mannie_types::provider::{
    CreateProviderProfile, MAX_MODEL_CHARACTERS, ModelDefaults, ModelDefaultsRef, ProfileRef,
    ProviderProfile, ProviderProfileError, ProviderTestReport, UpdateProviderProfile,
};
use tauri::State;

use crate::state::Core;

fn runtime(core: &Core) -> Result<&Runtime, ProviderProfileError> {
    core.runtime().ok_or(ProviderProfileError::StoreUnavailable)
}

/// Logs a refused or failed command by its code, and wraps the error for the
/// UI.
fn refused(
    command: &'static str,
) -> impl FnOnce(ProviderProfileError) -> CommandError<ProviderProfileError> {
    move |error| {
        tracing::warn!(
            command,
            error.code = error.code(),
            "a provider command was refused"
        );
        CommandError::new(error)
    }
}

/// Every profile, the oldest first.
#[tauri::command]
#[specta::specta]
pub(crate) async fn list_provider_profiles(
    core: State<'_, Core>,
) -> Result<Vec<ProviderProfile>, CommandError<ProviderProfileError>> {
    runtime(&core)?
        .list_profiles()
        .await
        .map_err(refused("list_provider_profiles"))
}

/// Adds a profile. The first one becomes the default.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_provider_profile(
    core: State<'_, Core>,
    input: CreateProviderProfile,
) -> Result<ProviderProfile, CommandError<ProviderProfileError>> {
    runtime(&core)?
        .create_profile(input)
        .await
        .map_err(refused("create_provider_profile"))
}

/// Changes a profile, and keeps, replaces or removes its credential.
#[tauri::command]
#[specta::specta]
pub(crate) async fn update_provider_profile(
    core: State<'_, Core>,
    input: UpdateProviderProfile,
) -> Result<ProviderProfile, CommandError<ProviderProfileError>> {
    runtime(&core)?
        .update_profile(&input.id, input.draft, input.credential)
        .await
        .map_err(refused("update_provider_profile"))
}

/// Deletes a profile and its credential.
#[tauri::command]
#[specta::specta]
pub(crate) async fn delete_provider_profile(
    core: State<'_, Core>,
    input: ProfileRef,
) -> Result<(), CommandError<ProviderProfileError>> {
    runtime(&core)?
        .delete_profile(&input.id)
        .await
        .map_err(refused("delete_provider_profile"))
}

/// Makes a profile the default.
#[tauri::command]
#[specta::specta]
pub(crate) async fn set_default_provider_profile(
    core: State<'_, Core>,
    input: ProfileRef,
) -> Result<(), CommandError<ProviderProfileError>> {
    runtime(&core)?
        .set_default_profile(&input.id)
        .await
        .map_err(refused("set_default_provider_profile"))
}

/// Runs the provider test of a profile.
#[tauri::command]
#[specta::specta]
pub(crate) async fn test_provider_profile(
    core: State<'_, Core>,
    input: ProfileRef,
) -> Result<ProviderTestReport, CommandError<ProviderProfileError>> {
    runtime(&core)?
        .test_profile(&input.id)
        .await
        .map_err(refused("test_provider_profile"))
}

/// The known limits of a model, as defaults for a profile's fields. `None`
/// for a model that Mannie does not know.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn get_model_defaults(input: ModelDefaultsRef) -> Option<ModelDefaults> {
    if input.model.chars().count() > MAX_MODEL_CHARACTERS {
        return None;
    }
    known_model(input.model.trim()).map(|known| ModelDefaults {
        limits: known.limits,
        reasons: known.reasons,
    })
}
