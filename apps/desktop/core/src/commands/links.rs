//! Commands for links in model output (docs/features/chat/conversation-view.md,
//! Markdown rendering): web links require confirmation. Local paths open only
//! through owner-initiated navigation under ADR-0069.

use mannie_types::file_links::{
    FileLinkError, FileLinkRef, FileLinkTarget, OpenFileLink, ResolvedFileLink,
};
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

/// Resolves a local reference without opening anything.
#[tauri::command]
#[specta::specta]
pub(crate) async fn resolve_file_link(
    core: State<'_, Core>,
    input: FileLinkRef,
) -> Result<ResolvedFileLink, CommandError<FileLinkError>> {
    crate::file_links::resolve(core.store(), input)
        .await
        .map_err(file_link_error)
}

/// Opens only the file manager, rechecking the original reference.
#[tauri::command]
#[specta::specta]
pub(crate) async fn open_file_link(
    core: State<'_, Core>,
    input: OpenFileLink,
) -> Result<(), CommandError<FileLinkError>> {
    match input.target {
        FileLinkTarget::FileManager => crate::file_links::open(&core, input.link)
            .await
            .map_err(file_link_error),
        FileLinkTarget::Unsupported => Err(file_link_error(FileLinkError::UnsupportedTarget)),
    }
}

fn file_link_error(error: FileLinkError) -> CommandError<FileLinkError> {
    tracing::warn!(error.code = error.code(), "local file navigation failed");
    CommandError::new(error)
}
