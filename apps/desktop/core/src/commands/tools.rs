//! Commands of the system tools feature (docs/features/system-tools/
//! operations-and-approvals.md, contracts). The runtime checks every request;
//! these commands find the runtime and log refusals by their code. Paths and
//! commands appear in logs only at debug level.

use mannie_runtime::Runtime;
use mannie_types::chat::ConversationRef;
use mannie_types::images::ImageData;
use mannie_types::ipc::{ChannelBatch, CommandError};
use mannie_types::tools::{
    BackgroundCommandView, CommandOutputUpdate, CommandRule, ConversationGrant, DecideApproval,
    GrantRef, ProgramRule, ProgramRuleRef, ReadToolImage, ReadToolOutput, RemoteCommandRule,
    RemoteRuleRef, RuleRef, SetFullAccess, SharedGrant, SharedGrantRef, StopBackgroundCommand,
    SubscribeCommandOutput, ToolOutputPage, ToolsError, ToolsSettings, UpdateToolsSettings,
};
use mannie_types::transcript::SubscriptionId;
use tauri::State;
use tauri::ipc::Channel;

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

/// The owner's answer to a tool call that waits for approval. The answer to
/// a call of a worker's task goes to the worker through the link.
#[tauri::command]
#[specta::specta]
pub(crate) async fn decide_approval(
    core: State<'_, Core>,
    input: DecideApproval,
) -> Result<(), CommandError<ToolsError>> {
    runtime(&core)
        .map_err(refused("decide_approval"))?
        .decide_approval(input)
        .await
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

/// Switches a conversation's full access, which applies from the next tool
/// call (ADR-0029).
#[tauri::command]
#[specta::specta]
pub(crate) async fn set_full_access(
    core: State<'_, Core>,
    input: SetFullAccess,
) -> Result<(), CommandError<ToolsError>> {
    runtime(&core)
        .map_err(refused("set_full_access"))?
        .set_full_access(input)
        .await
        .map_err(refused("set_full_access"))
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

/// The rules for commands on remote hosts, each limited to its host
/// (ADR-0036).
#[tauri::command]
#[specta::specta]
pub(crate) async fn list_remote_command_rules(
    core: State<'_, Core>,
) -> Result<Vec<RemoteCommandRule>, CommandError<ToolsError>> {
    runtime(&core)
        .map_err(refused("list_remote_command_rules"))?
        .remote_command_rules()
        .await
        .map_err(refused("list_remote_command_rules"))
}

/// Deletes a rule for commands on a remote host.
#[tauri::command]
#[specta::specta]
pub(crate) async fn delete_remote_command_rule(
    core: State<'_, Core>,
    input: RemoteRuleRef,
) -> Result<(), CommandError<ToolsError>> {
    runtime(&core)
        .map_err(refused("delete_remote_command_rule"))?
        .delete_remote_command_rule(input)
        .await
        .map_err(refused("delete_remote_command_rule"))
}

/// The rules for runs of installed programs, each limited to its plugin's
/// program and to the effects of the run that it was saved from (ADR-0038).
#[tauri::command]
#[specta::specta]
pub(crate) async fn list_program_rules(
    core: State<'_, Core>,
) -> Result<Vec<ProgramRule>, CommandError<ToolsError>> {
    runtime(&core)
        .map_err(refused("list_program_rules"))?
        .program_rules()
        .await
        .map_err(refused("list_program_rules"))
}

/// Deletes a rule for runs of an installed program.
#[tauri::command]
#[specta::specta]
pub(crate) async fn delete_program_rule(
    core: State<'_, Core>,
    input: ProgramRuleRef,
) -> Result<(), CommandError<ToolsError>> {
    runtime(&core)
        .map_err(refused("delete_program_rule"))?
        .delete_program_rule(input)
        .await
        .map_err(refused("delete_program_rule"))
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

/// An image of a tool call's result, as the model saw it (ADR-0044,
/// decision 7).
#[tauri::command]
#[specta::specta]
pub(crate) async fn read_tool_image(
    core: State<'_, Core>,
    input: ReadToolImage,
) -> Result<ImageData, CommandError<ToolsError>> {
    runtime(&core)
        .map_err(refused("read_tool_image"))?
        .read_tool_image(input)
        .await
        .map_err(refused("read_tool_image"))
}

/// Stops a background command at the owner's request, also one of a child's
/// task conversation, and returns how it ended (ADR-0041).
#[tauri::command]
#[specta::specta]
pub(crate) async fn stop_background_command(
    core: State<'_, Core>,
    input: StopBackgroundCommand,
) -> Result<BackgroundCommandView, CommandError<ToolsError>> {
    runtime(&core)
        .map_err(refused("stop_background_command"))?
        .stop_background_command(input)
        .await
        .map_err(refused("stop_background_command"))
}

/// Follows the output of a background command of this machine: the lines
/// after the line that the view holds, or its last lines, then new lines as
/// they arrive, until the command ended (ADR-0041).
#[tauri::command]
#[specta::specta]
pub(crate) async fn subscribe_command_output(
    core: State<'_, Core>,
    input: SubscribeCommandOutput,
    updates: Channel<ChannelBatch<CommandOutputUpdate>>,
) -> Result<SubscriptionId, CommandError<ToolsError>> {
    let page = core.subscriptions().page();
    let mut output = runtime(&core)
        .map_err(refused("subscribe_command_output"))?
        .command_output(input)
        .await
        .map_err(refused("subscribe_command_output"))?;
    Ok(core
        .subscriptions()
        .start(page, updates, |sender| async move {
            while let Some(update) = output.next().await {
                let ended = matches!(update, CommandOutputUpdate::Ended { .. });
                if sender.send(update).await.is_err() || ended {
                    break;
                }
            }
        }))
}
