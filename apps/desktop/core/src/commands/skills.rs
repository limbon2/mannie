//! Commands of skills (docs/features/skills.md, contracts). The runtime checks
//! every name and place against what the folders hold, because the webview is
//! untrusted (ADR-0004), and a switch only lists a skill's text, which grants
//! nothing (T-93).

use mannie_runtime::Runtime;
use mannie_types::ipc::CommandError;
use mannie_types::skills::{SetSkillOn, SkillError, SkillsOverview};
use tauri::State;

use crate::state::Core;

fn runtime(core: &Core) -> Result<&Runtime, SkillError> {
    core.runtime().ok_or(SkillError::StoreUnavailable)
}

/// Logs a refused or failed command by its code, and wraps the error for the
/// UI.
fn refused(command: &'static str) -> impl FnOnce(SkillError) -> CommandError<SkillError> {
    move |error| {
        tracing::warn!(
            command,
            error.code = error.code(),
            "a skills command was refused"
        );
        CommandError::new(error)
    }
}

/// The skills of this computer and of every project, read fresh.
#[tauri::command]
#[specta::specta]
pub(crate) async fn list_skills(
    core: State<'_, Core>,
) -> Result<SkillsOverview, CommandError<SkillError>> {
    runtime(&core)
        .map_err(refused("list_skills"))?
        .list_skills()
        .await
        .map_err(refused("list_skills"))
}

/// Turns a skill on or off in its place, and returns the skills as they are
/// then.
#[tauri::command]
#[specta::specta]
pub(crate) async fn set_skill_on(
    core: State<'_, Core>,
    input: SetSkillOn,
) -> Result<SkillsOverview, CommandError<SkillError>> {
    runtime(&core)
        .map_err(refused("set_skill_on"))?
        .set_skill_on(input)
        .await
        .map_err(refused("set_skill_on"))
}
