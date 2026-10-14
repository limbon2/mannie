//! Commands for links in model output (docs/features/chat/conversation-view.md,
//! Markdown rendering): the webview asks the core to check a link before the
//! owner confirms it, and to open it after.

use mannie_types::ipc::CommandError;
use mannie_types::links::{CheckedLink, LinkError, LinkRef};
use tauri::State;

use crate::links;
use crate::state::Core;

/// Checks a link and returns it as the owner confirms it.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn check_link(input: LinkRef) -> Result<CheckedLink, CommandError<LinkError>> {
    links::check(&input.url).map_err(|error| {
        tracing::info!(
            error.code = error.code(),
            "refused a link from model output"
        );
        CommandError::new(error)
    })
}

/// Checks a link again and opens it in the system browser.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn open_link(
    core: State<'_, Core>,
    input: LinkRef,
) -> Result<(), CommandError<LinkError>> {
    links::open(&core, &input.url).map_err(|error| {
        tracing::warn!(error.code = error.code(), "did not open a link");
        CommandError::new(error)
    })
}
