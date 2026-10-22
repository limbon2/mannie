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

/// What quitting would end: the replies that it interrupts and the
/// background commands that it ends (ADR-0041).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Running {
    /// The active replies.
    pub(crate) runs: u32,
    /// The background commands that run.
    pub(crate) commands: u32,
}

impl Running {
    /// What runs in the core now.
    fn in_core(core: &Core) -> Self {
        Self {
            runs: core.active_run_count(),
            commands: core.background_command_count(),
        }
    }

    fn any(self) -> bool {
        self.runs > 0 || self.commands > 0
    }
}

/// What a close request or the tray's "Quit" does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseDecision {
    /// Hide the window; Mannie keeps running in the tray.
    Hide,
    /// Quit now.
    Quit,
    /// Ask the owner what closing should do.
    Ask {
        /// What quitting would end.
        running: Running,
    },
    /// Ask the owner to confirm quitting, because replies or background
    /// commands run.
    ConfirmQuit {
        /// What quitting would end.
        running: Running,
    },
}

/// What closing the window does. A core that did not start has nothing to
/// keep running and no setting to save, so closing quits.
pub(crate) fn close_decision(
    core_started: bool,
    action: CloseAction,
    running: Running,
) -> CloseDecision {
    if !core_started {
        return CloseDecision::Quit;
    }
    match action {
        CloseAction::Ask => CloseDecision::Ask { running },
        CloseAction::Tray => CloseDecision::Hide,
        CloseAction::Quit => quit_decision(core_started, running),
    }
}

/// What quitting does: at once when nothing runs, after a confirmation while
/// replies or background commands run.
pub(crate) fn quit_decision(core_started: bool, running: Running) -> CloseDecision {
    if core_started && running.any() {
        CloseDecision::ConfirmQuit { running }
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
        Running::in_core(&core),
    );
    act(app, &core, decision, QuitSource::Window);
}

/// Handles "Quit" in the tray's menu.
pub(crate) fn quit_from_tray(app: &AppHandle) {
    let core = app.state::<Core>();
    let decision = quit_decision(core.is_started(), Running::in_core(&core));
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
        CloseDecision::Ask { running } => {
            ask(
                app,
                core,
                WindowRequest::AskCloseAction {
                    active_runs: running.runs,
                    background_commands: running.commands,
                },
            );
        }
        CloseDecision::ConfirmQuit { running } => {
            ask(
                app,
                core,
                WindowRequest::ConfirmQuit {
                    active_runs: running.runs,
                    background_commands: running.commands,
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
/// active replies as interrupted, ends the background commands, and the
/// store and the logs close.
pub(crate) fn quit(app: &AppHandle) {
    tracing::info!("Mannie is quitting");
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use mannie_types::window::CloseAction;

    use super::{CloseDecision, Running, close_decision, quit_decision};

    fn running(runs: u32, commands: u32) -> Running {
        Running { runs, commands }
    }

    #[test]
    fn closing_follows_the_owners_choice_and_what_runs() {
        let cases = [
            (
                CloseAction::Ask,
                running(0, 0),
                CloseDecision::Ask {
                    running: running(0, 0),
                },
            ),
            (
                CloseAction::Ask,
                running(3, 1),
                CloseDecision::Ask {
                    running: running(3, 1),
                },
            ),
            (CloseAction::Tray, running(0, 0), CloseDecision::Hide),
            (CloseAction::Tray, running(2, 2), CloseDecision::Hide),
            (CloseAction::Quit, running(0, 0), CloseDecision::Quit),
            (
                CloseAction::Quit,
                running(1, 0),
                CloseDecision::ConfirmQuit {
                    running: running(1, 0),
                },
            ),
            (
                CloseAction::Quit,
                running(0, 2),
                CloseDecision::ConfirmQuit {
                    running: running(0, 2),
                },
            ),
        ];

        for (action, what, expected) in cases {
            assert_eq!(
                close_decision(true, action, what),
                expected,
                "{action:?} with {what:?}"
            );
        }
    }

    #[test]
    fn a_core_that_did_not_start_quits_on_close() {
        for action in [CloseAction::Ask, CloseAction::Tray, CloseAction::Quit] {
            assert_eq!(
                close_decision(false, action, running(0, 0)),
                CloseDecision::Quit
            );
        }
        assert_eq!(quit_decision(false, running(0, 0)), CloseDecision::Quit);
    }

    #[test]
    fn quitting_from_the_tray_confirms_while_replies_or_background_commands_run() {
        assert_eq!(quit_decision(true, running(0, 0)), CloseDecision::Quit);
        assert_eq!(
            quit_decision(true, running(4, 0)),
            CloseDecision::ConfirmQuit {
                running: running(4, 0)
            }
        );
        assert_eq!(
            quit_decision(true, running(0, 1)),
            CloseDecision::ConfirmQuit {
                running: running(0, 1)
            }
        );
    }
}
