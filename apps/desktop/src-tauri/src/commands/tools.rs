//! Commands of the system tools feature (docs/features/system-tools/
//! operations-and-approvals.md, contracts). The runtime checks every request;
//! these commands find the runtime and log refusals by their code. Paths and
//! commands appear in logs only at debug level.

use mannie_runtime::Runtime;
use mannie_types::chat::ConversationRef;
use mannie_types::ipc::CommandError;
use mannie_types::tools::{
    CommandRule, ConversationGrant, DecideApproval, GrantRef, ReadToolOutput, RuleRef, SharedGrant,
    SharedGrantRef, ToolOutputPage, ToolsError, ToolsSettings, UpdateToolsSettings,
};
use tauri::State;

use crate::state::Core;

fn runtime(core: &Core) -> Result<&Runtime, ToolsError> {
    core.runtime().ok_or(ToolsError::StoreUnavailable)
}

/// Logs a refused or failed command by its code, and wraps the error for the
/// UI.
fn refused(command: &'static str) -> impl FnOnce(ToolsError) -> CommandError<ToolsError> {
    move |error| {
        tracing::warn!(
            command,
            error.code = error.code(),
            "a tools command was refused"
        );
        CommandError::new(error)
    }
}

/// The owner's answer to a tool call that waits for approval.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn decide_approval(
    core: State<'_, Core>,
    input: DecideApproval,
) -> Result<(), CommandError<ToolsError>> {
    runtime(&core)
        .and_then(|runtime| runtime.decide_approval(input))
        .map_err(refused("decide_approval"))
}

/// The tools settings.
#[tauri::command]
#[specta::specta]
pub(crate) async fn get_tools_settings(
    core: State<'_, Core>,
) -> Result<ToolsSettings, CommandError<ToolsError>> {
    runtime(&core)
        .map_err(refused("get_tools_settings"))?
        .tools_settings()
        .await
        .map_err(refused("get_tools_settings"))
}

/// Saves the tools settings, which apply from the next tool call.
#[tauri::command]
#[specta::specta]
pub(crate) async fn update_tools_settings(
    core: State<'_, Core>,
    input: UpdateToolsSettings,
) -> Result<ToolsSettings, CommandError<ToolsError>> {
    runtime(&core)
        .map_err(refused("update_tools_settings"))?
        .update_tools_settings(input)
        .await
        .map_err(refused("update_tools_settings"))
}

/// The command rules.
#[tauri::command]
#[specta::specta]
pub(crate) async fn list_command_rules(
    core: State<'_, Core>,
) -> Result<Vec<CommandRule>, CommandError<ToolsError>> {
    runtime(&core)
        .map_err(refused("list_command_rules"))?
        .command_rules()
        .await
        .map_err(refused("list_command_rules"))
}

/// Deletes a command rule.
#[tauri::command]
#[specta::specta]
pub(crate) async fn delete_command_rule(
    core: State<'_, Core>,
    input: RuleRef,
) -> Result<(), CommandError<ToolsError>> {
    runtime(&core)
        .map_err(refused("delete_command_rule"))?
        .delete_command_rule(input)
        .await
        .map_err(refused("delete_command_rule"))
}

/// The grants of a conversation.
#[tauri::command]
#[specta::specta]
pub(crate) async fn list_conversation_grants(
    core: State<'_, Core>,
    input: ConversationRef,
) -> Result<Vec<ConversationGrant>, CommandError<ToolsError>> {
    runtime(&core)
        .map_err(refused("list_conversation_grants"))?
        .conversation_grants(&input.conversation_id)
        .await
        .map_err(refused("list_conversation_grants"))
}

/// Revokes a grant of a conversation.
#[tauri::command]
#[specta::specta]
pub(crate) async fn revoke_conversation_grant(
    core: State<'_, Core>,
    input: GrantRef,
) -> Result<(), CommandError<ToolsError>> {
    runtime(&core)
        .map_err(refused("revoke_conversation_grant"))?
        .revoke_conversation_grant(input)
        .await
        .map_err(refused("revoke_conversation_grant"))
}

/// The grants for a project or for every conversation.
#[tauri::command]
#[specta::specta]
pub(crate) async fn list_shared_grants(
    core: State<'_, Core>,
) -> Result<Vec<SharedGrant>, CommandError<ToolsError>> {
    runtime(&core)
        .map_err(refused("list_shared_grants"))?
        .shared_grants()
        .await
        .map_err(refused("list_shared_grants"))
}

/// Revokes a grant for a project or for every conversation.
#[tauri::command]
#[specta::specta]
pub(crate) async fn revoke_shared_grant(
    core: State<'_, Core>,
    input: SharedGrantRef,
) -> Result<(), CommandError<ToolsError>> {
    runtime(&core)
        .map_err(refused("revoke_shared_grant"))?
        .revoke_shared_grant(input)
        .await
        .map_err(refused("revoke_shared_grant"))
}

/// A page of an archived tool output.
#[tauri::command]
#[specta::specta]
pub(crate) async fn read_tool_output(
    core: State<'_, Core>,
    input: ReadToolOutput,
) -> Result<ToolOutputPage, CommandError<ToolsError>> {
    runtime(&core)
        .map_err(refused("read_tool_output"))?
        .tool_output(input)
        .await
        .map_err(refused("read_tool_output"))
}
