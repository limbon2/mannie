//! The owner's native export action. The command takes no destination path
//! or content from the untrusted webview (ADR-0040).

use mannie_types::chat::ConversationRef;
use mannie_types::export::{ExportError, ExportOutcome};
use mannie_types::ipc::CommandError;
use tauri::{AppHandle, State};

use crate::export::Exports;
use crate::state::Core;

#[tauri::command]
#[specta::specta]
pub(crate) async fn export_conversation(
    app: AppHandle,
    core: State<'_, Core>,
    exports: State<'_, Exports>,
    input: ConversationRef,
) -> Result<ExportOutcome, CommandError<ExportError>> {
    let runtime = core.runtime().ok_or(ExportError::StoreUnavailable)?;
    let store = core.store().ok_or(ExportError::StoreUnavailable)?;
    exports
        .run(&app, runtime, store, &input.conversation_id)
        .await
        .map_err(|error| {
            tracing::warn!(error.code = error.code(), "a conversation export failed");
            CommandError::new(error)
        })
}
