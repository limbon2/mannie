//! Commands of the chat feature's projects (docs/features/chat/projects.md,
//! contracts). The runtime checks every request, and the core shows the
//! folder dialog, so the webview never touches the file system (ADR-0004).

use mannie_types::chat::ChatError;
use mannie_types::ipc::CommandError;
use mannie_types::projects::{AddProjectFromPath, ProjectView, RemoveProject, RenameProject};
use tauri::{AppHandle, State};

use super::chat::{refused, runtime};
use crate::folder_dialog;
use crate::state::Core;

/// Shows the system's folder dialog and adds the chosen folder as a project,
/// or returns the project that has it already. Nothing when the owner closed
/// the dialog.
#[tauri::command]
#[specta::specta]
pub(crate) async fn add_project(
    app: AppHandle,
    core: State<'_, Core>,
) -> Result<Option<ProjectView>, CommandError<ChatError>> {
    let runtime = runtime(&core).map_err(refused("add_project"))?;
    let chosen = folder_dialog::choose_folder(&app)
        .await
        .map_err(|_failed| ChatError::FolderDialogFailed)
        .map_err(refused("add_project"))?;
    let Some(folder) = chosen else {
        return Ok(None);
    };
    runtime
        .add_project(folder)
        .await
        .map(Some)
        .map_err(refused("add_project"))
}

/// Adds a folder that the owner typed as a project, for systems without a
/// folder dialog.
#[tauri::command]
#[specta::specta]
pub(crate) async fn add_project_from_path(
    core: State<'_, Core>,
    input: AddProjectFromPath,
) -> Result<ProjectView, CommandError<ChatError>> {
    runtime(&core)
        .map_err(refused("add_project_from_path"))?
        .add_project(input.folder)
        .await
        .map_err(refused("add_project_from_path"))
}

/// Renames a project.
#[tauri::command]
#[specta::specta]
pub(crate) async fn rename_project(
    core: State<'_, Core>,
    input: RenameProject,
) -> Result<ProjectView, CommandError<ChatError>> {
    runtime(&core)
        .map_err(refused("rename_project"))?
        .rename_project(input)
        .await
        .map_err(refused("rename_project"))
}

/// Removes a project and deletes its conversations, as many as the owner
/// confirmed.
#[tauri::command]
#[specta::specta]
pub(crate) async fn remove_project(
    core: State<'_, Core>,
    input: RemoveProject,
) -> Result<(), CommandError<ChatError>> {
    runtime(&core)
        .map_err(refused("remove_project"))?
        .remove_project(input)
        .await
        .map_err(refused("remove_project"))
}
