//! The system's folder dialog, which the core opens when the owner adds a
//! project (docs/features/chat/projects.md, the sidebar). The webview never
//! touches the file system: it asks the core, which shows the dialog over the
//! main window and takes the folder that the owner chose (ADR-0004). On Linux
//! the dialog comes from the XDG desktop portal.

use tauri::{AppHandle, Manager};

use crate::window::MAIN_WINDOW;

/// The dialog could not give a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DialogFailed;

/// Shows the folder dialog and returns the folder the owner chose, or `None`
/// when the owner closed it.
pub(crate) async fn choose_folder(app: &AppHandle) -> Result<Option<String>, DialogFailed> {
    // The end-to-end tests cannot click in a dialog of the system: validation
    // builds answer with the folder that a test set.
    #[cfg(feature = "validation")]
    if let Some(choice) = app.state::<crate::state::Core>().take_folder_choice() {
        return Ok(match choice {
            crate::state::FolderChoice::Closed => None,
            crate::state::FolderChoice::Folder(folder) => Some(folder),
        });
    }
    let mut dialog = rfd::AsyncFileDialog::new();
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        dialog = dialog.set_parent(&window);
    }
    let Some(chosen) = dialog.pick_folder().await else {
        return Ok(None);
    };
    let Some(folder) = chosen.path().to_str() else {
        tracing::warn!("the chosen folder's path is not valid Unicode");
        return Err(DialogFailed);
    };
    Ok(Some(folder.to_owned()))
}
