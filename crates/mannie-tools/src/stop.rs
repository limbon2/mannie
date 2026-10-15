//! The stop signal of a running tool call (docs/features/system-tools/
//! tool-calls.md, the loop): the owner stopped the reply, or Mannie quits. A
//! tool ends its work as soon as it can, and a command reports which of the
//! two ended it.

use std::sync::{Arc, OnceLock};

use tokio_util::sync::CancellationToken;

/// Why a running tool call was told to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The owner stopped the reply.
    Stopped,
    /// Mannie quits.
    Interrupted,
}

/// Tells a running tool to stop, and why. Clones share the signal.
#[derive(Debug, Clone, Default)]
pub struct StopSignal {
    token: CancellationToken,
    reason: Arc<OnceLock<StopReason>>,
}

impl StopSignal {
    /// A signal that has not stopped anything yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stops the tool. When the signal stops more than once, the first reason
    /// counts.
    pub fn stop(&self, reason: StopReason) {
        // The reason is set before the token wakes the tool, so the tool
        // always finds it.
        self.reason.get_or_init(|| reason);
        self.token.cancel();
    }

    /// Why the tool was stopped, once it was.
    pub fn reason(&self) -> Option<StopReason> {
        if self.token.is_cancelled() {
            self.reason.get().copied()
        } else {
            None
        }
    }

    /// Whether the tool was stopped.
    pub fn is_stopped(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Waits until the tool is stopped.
    pub async fn stopped(&self) {
        self.token.cancelled().await;
    }
}

#[cfg(test)]
mod tests {
    use super::{StopReason, StopSignal};

    #[test]
    fn the_first_reason_counts() {
        let signal = StopSignal::new();
        let clone = signal.clone();
        assert_eq!(signal.reason(), None);

        clone.stop(StopReason::Interrupted);
        signal.stop(StopReason::Stopped);

        assert!(signal.is_stopped());
        assert_eq!(signal.reason(), Some(StopReason::Interrupted));
    }
}
