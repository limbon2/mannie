//! Closing the window and quitting (docs/features/window-and-tray.md, closing
//! and quitting). The core decides; the window only asks the owner when the
//! decision says so.

use std::time::Duration;

use mannie_types::window::{CloseAction, QuitSource, WindowRequest};
use tauri::{AppHandle, Manager};

use super::{MAIN_WINDOW, show_main};
use crate::state::Core;

/// How long a question waits for a window to show it. Without a window that
/// can answer, for example because the page failed to load, Mannie quits;
/// active replies become interrupted and can be retried.
const UNANSWERED_QUESTION_TIMEOUT: Duration = Duration::from_secs(5);

/// What a close request or the tray's "Quit" does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseDecision {
    /// Hide the window; Mannie keeps running in the tray.
    Hide,
    /// Quit now.
    Quit,
    /// Ask the owner what closing should do.
    Ask {
        /// The replies that quitting would interrupt.
        active_runs: u32,
    },
    /// Ask the owner to confirm quitting, because replies run.
    ConfirmQuit {
        /// The replies that quitting would interrupt.
        active_runs: u32,
    },
}

/// What closing the window does. A core that did not start has nothing to
/// keep running and no setting to save, so closing quits.
pub(crate) fn close_decision(
    core_started: bool,
    action: CloseAction,
    active_runs: u32,
) -> CloseDecision {
    if !core_started {
        return CloseDecision::Quit;
    }
    match action {
        CloseAction::Ask => CloseDecision::Ask { active_runs },
        CloseAction::Tray => CloseDecision::Hide,
        CloseAction::Quit => quit_decision(core_started, active_runs),
    }
}

/// What quitting does: at once without active replies, after a confirmation
/// with them.
pub(crate) fn quit_decision(core_started: bool, active_runs: u32) -> CloseDecision {
    if core_started && active_runs > 0 {
        CloseDecision::ConfirmQuit { active_runs }
    } else {
        CloseDecision::Quit
    }
}

/// Handles the window's close button and every other close request.
pub(crate) fn on_close_requested(app: &AppHandle) {
    let core = app.state::<Core>();
    let decision = close_decision(
        core.is_started(),
        core.window_settings().close_action,
        core.active_run_count(),
    );
    act(app, &core, decision, QuitSource::Window);
}

/// Handles "Quit" in the tray's menu.
pub(crate) fn quit_from_tray(app: &AppHandle) {
    let core = app.state::<Core>();
    let decision = quit_decision(core.is_started(), core.active_run_count());
    act(app, &core, decision, QuitSource::Tray);
}

fn act(app: &AppHandle, core: &Core, decision: CloseDecision, source: QuitSource) {
    tracing::info!(
        ?decision,
        ?source,
        "the owner closed the window or chose Quit"
    );
    match decision {
        CloseDecision::Hide => hide_main(app),
        CloseDecision::Quit => quit(app),
        CloseDecision::Ask { active_runs } => {
            ask(app, core, WindowRequest::AskCloseAction { active_runs });
        }
        CloseDecision::ConfirmQuit { active_runs } => {
            ask(
                app,
                core,
                WindowRequest::ConfirmQuit {
                    active_runs,
                    source,
                },
            );
        }
    }
}

/// Shows the window and asks it. Without a window that answers in time,
/// Mannie quits.
fn ask(app: &AppHandle, core: &Core, request: WindowRequest) {
    show_main(app);
    let requests = core.window_requests();
    let request_id = requests.ask(request);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(UNANSWERED_QUESTION_TIMEOUT).await;
        let core = app.state::<Core>();
        let requests = core.window_requests();
        if requests.is_pending(request_id) && !requests.has_subscriber() {
            tracing::warn!("no window can answer the question about closing; Mannie quits");
            quit(&app);
        }
    });
}

/// Hides the main window.
pub(crate) fn hide_main(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    if let Err(error) = window.hide() {
        tracing::warn!(%error, "could not hide the main window");
    }
}

/// Ends the event loop. `run` then shuts the core down: the runtime marks
/// active replies as interrupted, and the store and the logs close.
pub(crate) fn quit(app: &AppHandle) {
    tracing::info!("Mannie is quitting");
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use mannie_types::window::CloseAction;

    use super::{CloseDecision, close_decision, quit_decision};

    #[test]
    fn closing_follows_the_owners_choice_and_the_active_runs() {
        let cases = [
            (CloseAction::Ask, 0, CloseDecision::Ask { active_runs: 0 }),
            (CloseAction::Ask, 3, CloseDecision::Ask { active_runs: 3 }),
            (CloseAction::Tray, 0, CloseDecision::Hide),
            (CloseAction::Tray, 2, CloseDecision::Hide),
            (CloseAction::Quit, 0, CloseDecision::Quit),
            (
                CloseAction::Quit,
                1,
                CloseDecision::ConfirmQuit { active_runs: 1 },
            ),
        ];

        for (action, active_runs, expected) in cases {
            assert_eq!(
                close_decision(true, action, active_runs),
                expected,
                "{action:?} with {active_runs}"
            );
        }
    }

    #[test]
    fn a_core_that_did_not_start_quits_on_close() {
        for action in [CloseAction::Ask, CloseAction::Tray, CloseAction::Quit] {
            assert_eq!(close_decision(false, action, 0), CloseDecision::Quit);
        }
        assert_eq!(quit_decision(false, 0), CloseDecision::Quit);
    }

    #[test]
    fn quitting_from_the_tray_confirms_only_with_active_runs() {
        assert_eq!(quit_decision(true, 0), CloseDecision::Quit);
        assert_eq!(
            quit_decision(true, 4),
            CloseDecision::ConfirmQuit { active_runs: 4 }
        );
    }
}
