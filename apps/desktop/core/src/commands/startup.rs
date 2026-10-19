//! The startup status: whether the core started, and if not, why. The shell
//! asks for it before it renders anything.

use mannie_types::startup::StartupStatus;
use tauri::State;

use crate::state::Core;

/// Reports whether the core started, and if not, why.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn get_startup_status(core: State<'_, Core>) -> StartupStatus {
    StartupStatus {
        problem: core.problem().cloned(),
    }
}
