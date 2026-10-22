//! Questions from the core to the window (docs/features/window-and-tray.md,
//! closing and quitting): what closing should do, and whether to quit while
//! replies run. The window receives them over a channel that it opens at
//! startup; a question that waits when the window subscribes is sent again.

use std::sync::{Mutex, MutexGuard, PoisonError};

use mannie_types::window::{WindowError, WindowRequest, WindowRequestUpdate};

use crate::ipc::channel::BatchSender;

#[derive(Debug, Default)]
struct State {
    next_id: u32,
    pending: Option<(u32, WindowRequest)>,
    subscriber: Option<BatchSender<WindowRequestUpdate>>,
}

/// The question that waits for an answer, and the window that receives it.
#[derive(Debug, Default)]
pub(crate) struct WindowRequests {
    state: Mutex<State>,
}

impl State {
    fn send(&self, update: WindowRequestUpdate) {
        if let Some(subscriber) = &self.subscriber
            && subscriber.try_send(update).is_err()
        {
            tracing::warn!(
                "the window did not receive a question; it gets it again when it subscribes"
            );
        }
    }
}

impl WindowRequests {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Every critical section replaces whole fields, so the state stays
        // consistent even if a holder panicked.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Asks the window, replacing a question that still waits. Returns the
    /// question's ID.
    pub(crate) fn ask(&self, request: WindowRequest) -> u32 {
        let mut state = self.lock();
        if let Some((replaced, _request)) = state.pending.take() {
            state.send(WindowRequestUpdate::Cleared {
                request_id: replaced,
            });
        }
        let request_id = state.next_id;
        state.next_id = state.next_id.wrapping_add(1);
        state.pending = Some((request_id, request));
        state.send(WindowRequestUpdate::Pending {
            request_id,
            request,
        });
        request_id
    }

    /// Makes `subscriber` the window that receives questions, and sends it the
    /// question that waits, if any.
    pub(crate) fn subscribe(&self, subscriber: BatchSender<WindowRequestUpdate>) {
        let mut state = self.lock();
        state.subscriber = Some(subscriber);
        if let Some((request_id, request)) = state.pending {
            state.send(WindowRequestUpdate::Pending {
                request_id,
                request,
            });
        }
    }

    /// Forgets the window, because its page loads again.
    pub(crate) fn unsubscribe(&self) {
        self.lock().subscriber = None;
    }

    /// Whether a window receives questions.
    pub(crate) fn has_subscriber(&self) -> bool {
        self.lock()
            .subscriber
            .as_ref()
            .is_some_and(|subscriber| !subscriber.is_closed())
    }

    /// Whether the question `request_id` still waits.
    pub(crate) fn is_pending(&self, request_id: u32) -> bool {
        self.lock()
            .pending
            .is_some_and(|(pending, _request)| pending == request_id)
    }

    /// The question `request_id`, if it still waits.
    pub(crate) fn pending(&self, request_id: u32) -> Result<WindowRequest, WindowError> {
        let pending = self.lock().pending;
        match pending {
            Some((pending, request)) if pending == request_id => Ok(request),
            _ => Err(WindowError::RequestNotPending),
        }
    }

    /// Takes the question `request_id` for its answer.
    pub(crate) fn take(&self, request_id: u32) -> Result<WindowRequest, WindowError> {
        let mut state = self.lock();
        match state.pending {
            Some((pending, request)) if pending == request_id => {
                state.pending = None;
                state.send(WindowRequestUpdate::Cleared { request_id });
                Ok(request)
            }
            _ => Err(WindowError::RequestNotPending),
        }
    }
}

#[cfg(test)]
mod tests {
    use mannie_types::window::{QuitSource, WindowError, WindowRequest};

    use super::WindowRequests;

    #[test]
    fn a_question_is_answered_once_and_a_new_one_replaces_it() {
        let requests = WindowRequests::default();

        let first = requests.ask(WindowRequest::AskCloseAction {
            active_runs: 0,
            background_commands: 0,
        });
        let second = requests.ask(WindowRequest::ConfirmQuit {
            active_runs: 2,
            background_commands: 1,
            source: QuitSource::Tray,
        });

        assert!(!requests.is_pending(first));
        assert_eq!(requests.take(first), Err(WindowError::RequestNotPending));
        assert_eq!(
            requests.take(second),
            Ok(WindowRequest::ConfirmQuit {
                active_runs: 2,
                background_commands: 1,
                source: QuitSource::Tray
            })
        );
        assert_eq!(requests.take(second), Err(WindowError::RequestNotPending));
        assert!(!requests.has_subscriber());
    }
}
