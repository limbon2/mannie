//! Commands of the owner's custom agents (docs/features/custom-agents.md,
//! contracts). The runtime checks every draft against what its kind may name,
//! because the webview is untrusted (ADR-0004).

use mannie_runtime::Runtime;
use mannie_types::agents::{
    AgentDraft, AgentError, AgentOptions, AgentRef, AgentView, UpdateAgent,
};
use mannie_types::ipc::CommandError;
use tauri::State;

use crate::state::Core;

fn runtime(core: &Core) -> Result<&Runtime, AgentError> {
    core.runtime().ok_or(AgentError::StoreUnavailable)
}

/// Logs a refused or failed command by its code, and wraps the error for the
/// UI.
fn refused(command: &'static str) -> impl FnOnce(AgentError) -> CommandError<AgentError> {
    move |error| {
        tracing::warn!(
            command,
            error.code = error.code(),
            "an agents command was refused"
        );
        CommandError::new(error)
    }
}

/// Every agent: the built-in ones, those of the plugins that are on, then the
/// owner's, the oldest first.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn list_agents(
    core: State<'_, Core>,
) -> Result<Vec<AgentView>, CommandError<AgentError>> {
    Ok(runtime(&core)
        .map_err(refused("list_agents"))?
        .list_agents())
}

/// What the editor offers: the groups of tools, the plugins with tools and the
/// templates.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn agent_options(
    core: State<'_, Core>,
) -> Result<AgentOptions, CommandError<AgentError>> {
    Ok(runtime(&core)
        .map_err(refused("agent_options"))?
        .agent_options())
}

/// Adds an agent of the owner's, and returns it with its new key.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_agent(
    core: State<'_, Core>,
    input: AgentDraft,
) -> Result<AgentView, CommandError<AgentError>> {
    runtime(&core)
        .map_err(refused("create_agent"))?
        .create_agent(&input)
        .await
        .map_err(refused("create_agent"))
}

/// Changes an agent of the owner's, from its next reply or task.
#[tauri::command]
#[specta::specta]
pub(crate) async fn update_agent(
    core: State<'_, Core>,
    input: UpdateAgent,
) -> Result<AgentView, CommandError<AgentError>> {
    runtime(&core)
        .map_err(refused("update_agent"))?
        .update_agent(&input.key, &input.draft)
        .await
        .map_err(refused("update_agent"))
}

/// Deletes an agent of the owner's, and sets the conversations that use it back
/// to the built-in agent.
#[tauri::command]
#[specta::specta]
pub(crate) async fn delete_agent(
    core: State<'_, Core>,
    input: AgentRef,
) -> Result<(), CommandError<AgentError>> {
    runtime(&core)
        .map_err(refused("delete_agent"))?
        .delete_agent(&input.key)
        .await
        .map_err(refused("delete_agent"))
}
