//! Commands of plans (docs/features/plans.md, contracts). The runtime checks
//! every title and text, because the webview is untrusted (ADR-0004).

use mannie_runtime::Runtime;
use mannie_types::ipc::CommandError;
use mannie_types::plans::{PlanError, PlanRef, PlanSummary, PlanView, UpdatePlan};
use tauri::State;

use crate::state::Core;

fn runtime(core: &Core) -> Result<&Runtime, PlanError> {
    core.runtime().ok_or(PlanError::StoreUnavailable)
}

/// Logs a refused or failed command by its code, and wraps the error for the
/// UI.
fn refused(command: &'static str) -> impl FnOnce(PlanError) -> CommandError<PlanError> {
    move |error| {
        tracing::warn!(
            command,
            error.code = error.code(),
            "a plans command was refused"
        );
        CommandError::new(error)
    }
}

/// The plans, the one that changed last first, without their texts.
#[tauri::command]
#[specta::specta]
pub(crate) async fn list_plans(
    core: State<'_, Core>,
) -> Result<Vec<PlanSummary>, CommandError<PlanError>> {
    runtime(&core)
        .map_err(refused("list_plans"))?
        .list_plans()
        .await
        .map_err(refused("list_plans"))
}

/// One plan, with its text.
#[tauri::command]
#[specta::specta]
pub(crate) async fn get_plan(
    core: State<'_, Core>,
    input: PlanRef,
) -> Result<PlanView, CommandError<PlanError>> {
    runtime(&core)
        .map_err(refused("get_plan"))?
        .get_plan(input.number)
        .await
        .map_err(refused("get_plan"))
}

/// The owner's edit of a plan.
#[tauri::command]
#[specta::specta]
pub(crate) async fn update_plan(
    core: State<'_, Core>,
    input: UpdatePlan,
) -> Result<PlanView, CommandError<PlanError>> {
    runtime(&core)
        .map_err(refused("update_plan"))?
        .update_plan(input)
        .await
        .map_err(refused("update_plan"))
}

/// Deletes a plan.
#[tauri::command]
#[specta::specta]
pub(crate) async fn delete_plan(
    core: State<'_, Core>,
    input: PlanRef,
) -> Result<(), CommandError<PlanError>> {
    runtime(&core)
        .map_err(refused("delete_plan"))?
        .delete_plan(input.number)
        .await
        .map_err(refused("delete_plan"))
}
