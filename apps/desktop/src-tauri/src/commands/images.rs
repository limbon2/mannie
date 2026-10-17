//! Commands of the Images plugin (docs/features/images.md, contracts):
//! choosing image files in the system's dialog, preparing a chosen, pasted or
//! dropped image, and dropping a waiting one. They are thin calls into the
//! plugin's crate (ADR-0030, decision 10): the plugin checks every file, and
//! while Images is off every command refuses with `plugin_off`. Paths stay
//! in the core: the dialog's files go from here to the plugin (ADR-0004).

use mannie_images::ImagesPlugin;
use mannie_images::commands::{
    EXTENSIONS, ImagesError, PickImages, PickedImage, PickedImageRef, PrepareImage, PreparedImage,
    PreparedImageRef,
};
use mannie_types::ipc::CommandError;
use tauri::{AppHandle, State};

use crate::file_dialog;
use crate::state::Core;

fn plugin(core: &Core) -> Result<&ImagesPlugin, ImagesError> {
    core.images().ok_or(ImagesError::StoreUnavailable)
}

/// Logs a refused or failed command by its code, and wraps the error for the
/// UI.
fn refused(command: &'static str) -> impl FnOnce(ImagesError) -> CommandError<ImagesError> {
    move |error| {
        tracing::warn!(
            command,
            error.code = error.code(),
            "an images command was refused"
        );
        CommandError::new(error)
    }
}

/// Shows the system's file dialog for images and keeps the files that the
/// owner chose, which `prepare_picked_image` then prepares one by one. None
/// when the owner closed the dialog.
#[tauri::command]
#[specta::specta]
pub(crate) async fn pick_images(
    app: AppHandle,
    core: State<'_, Core>,
    input: PickImages,
) -> Result<Vec<PickedImage>, CommandError<ImagesError>> {
    let filter_name = input.check().map_err(refused("pick_images"))?;
    let images = plugin(&core).map_err(refused("pick_images"))?;
    images.check_on().map_err(refused("pick_images"))?;
    let paths = file_dialog::choose_files(&app, filter_name, EXTENSIONS)
        .await
        .map_err(|_failed| ImagesError::DialogFailed)
        .map_err(refused("pick_images"))?;
    images.pick(paths).map_err(refused("pick_images"))
}

/// Reads and prepares an image file that the owner chose.
#[tauri::command]
#[specta::specta]
pub(crate) async fn prepare_picked_image(
    core: State<'_, Core>,
    input: PickedImageRef,
) -> Result<PreparedImage, CommandError<ImagesError>> {
    plugin(&core)
        .map_err(refused("prepare_picked_image"))?
        .prepare_picked(input)
        .await
        .map_err(refused("prepare_picked_image"))
}

/// Prepares an image that the owner pasted or dropped on the message box.
#[tauri::command]
#[specta::specta]
pub(crate) async fn prepare_image(
    core: State<'_, Core>,
    input: PrepareImage,
) -> Result<PreparedImage, CommandError<ImagesError>> {
    plugin(&core)
        .map_err(refused("prepare_image"))?
        .prepare(input)
        .await
        .map_err(refused("prepare_image"))
}

/// Drops a waiting image that the owner removed from the message box.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn discard_prepared_image(
    core: State<'_, Core>,
    input: PreparedImageRef,
) -> Result<(), CommandError<ImagesError>> {
    plugin(&core)
        .map_err(refused("discard_prepared_image"))?
        .discard(&input)
        .map_err(refused("discard_prepared_image"))
}
