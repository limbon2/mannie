//! Commands of the child agents feature (docs/features/child-agents/
//! child-agent-activity.md, settings): how many child agents may run at once.
//! The runtime checks every request; these commands find the runtime and log
//! refusals by their code.

use mannie_runtime::Runtime;
use mannie_types::ipc::CommandError;
use mannie_types::tasks::{ChildAgentsError, ChildAgentsSettings, UpdateChildAgentsSettings};
use tauri::State;

use crate::state::Core;

fn runtime(core: &Core) -> Result<&Runtime, ChildAgentsError> {
    core.runtime().ok_or(ChildAgentsError::StoreUnavailable)
}

/// Logs a refused or failed command by its code, and wraps the error for the
/// UI.
fn refused(
    command: &'static str,
) -> impl FnOnce(ChildAgentsError) -> CommandError<ChildAgentsError> {
    move |error| {
        tracing::warn!(
            command,
            error.code = error.code(),
            "a child agents command was refused"
        );
        CommandError::new(error)
    }
}

/// The child agents settings.
#[tauri::command]
#[specta::specta]
pub(crate) async fn get_child_agents_settings(
    core: State<'_, Core>,
) -> Result<ChildAgentsSettings, CommandError<ChildAgentsError>> {
    runtime(&core)
        .map_err(refused("get_child_agents_settings"))?
        .child_agents_settings()
        .await
        .map_err(refused("get_child_agents_settings"))
}

/// Saves the child agents settings. A larger limit starts waiting children at
/// once; a smaller one takes effect as running children end.
#[tauri::command]
#[specta::specta]
pub(crate) async fn update_child_agents_settings(
    core: State<'_, Core>,
    input: UpdateChildAgentsSettings,
) -> Result<ChildAgentsSettings, CommandError<ChildAgentsError>> {
    runtime(&core)
        .map_err(refused("update_child_agents_settings"))?
        .update_child_agents_settings(input)
        .await
        .map_err(refused("update_child_agents_settings"))
}
