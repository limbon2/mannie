//! The system's file dialog, which the core opens when the owner attaches
//! images to a message (docs/features/images.md, adding images). The webview
//! never sees a path: the core shows the dialog over the main window and hands
//! the chosen files to the Images plugin (ADR-0004, ADR-0033). On Linux the
//! dialog comes from the XDG desktop portal.

use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use crate::folder_dialog::DialogFailed;
use crate::window::MAIN_WINDOW;

/// Shows the file dialog with one filter, `filter_name` for `extensions`, and
/// returns the files that the owner chose: none when the owner closed it.
pub(crate) async fn choose_files(
    app: &AppHandle,
    filter_name: &str,
    extensions: &[&str],
) -> Result<Vec<PathBuf>, DialogFailed> {
    // The end-to-end tests cannot click in a dialog of the system:
    // validation builds answer with the files that a test set.
    #[cfg(feature = "validation")]
    if let Some(files) = app.state::<crate::state::Core>().take_file_choice() {
        return Ok(files.into_iter().map(PathBuf::from).collect());
    }
    let mut dialog = rfd::AsyncFileDialog::new().add_filter(filter_name, extensions);
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        dialog = dialog.set_parent(&window);
    }
    let chosen = dialog.pick_files().await.unwrap_or_default();
    Ok(chosen
        .into_iter()
        .map(|file| file.path().to_path_buf())
        .collect())
}
