//! The views of a terminal
//! (plugins/terminal/docs/adr/0002-keep-a-headless-emulator-of-each-terminal-in-rust.md,
//! views and floods).
//!
//! A view exists while its channel is open: the UI opens it while the tab
//! shows, its conversation is open and the window is visible, and closes it
//! otherwise, so a hidden terminal costs the webview nothing. A view starts
//! from a full snapshot. Then it gets the output as it comes, until more than
//! [`HIGH_WATER`] bytes wait for its acknowledgment: it then gets a snapshot
//! of the screen at most every [`SCREEN_INTERVAL`] instead, so a flood
//! becomes a jump to the current state and never slows the program. Once it
//! caught up and the output is quiet, it gets a full snapshot and the output
//! again. A view whose channel filled up all the same starts over from a
//! full snapshot once it has room.

use std::cell::Cell;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::commands::{Count, TerminalUpdate};
use crate::emulator::Emulator;

/// The most bytes that wait for a view's acknowledgment before it falls
/// behind.
pub(crate) const HIGH_WATER: u64 = 512 * 1024;

/// The most bytes that may wait when a view that fell behind catches up.
const LOW_WATER: u64 = 64 * 1024;

/// How often a view that fell behind gets the screen.
pub(crate) const SCREEN_INTERVAL: Duration = Duration::from_millis(100);

/// How long the output must pause before a view that caught up streams
/// again.
const QUIET: Duration = Duration::from_millis(100);

/// The most updates that wait in a view's channel.
pub(crate) const VIEW_QUEUE: usize = 1024;

/// The room that a view's channel needs before the view starts over: a
/// snapshot, the commands, the state and an end.
const RESYNC_ROOM: usize = 4;

/// A view of a terminal.
#[derive(Debug)]
pub(crate) struct View {
    /// The ID that the view gave, which its acknowledgments name.
    pub(crate) id: String,
    sender: mpsc::Sender<TerminalUpdate>,
    /// The lines of scrollback that its snapshots fill.
    scrollback: usize,
    /// The bytes of data sent.
    sent: u64,
    /// The bytes of data that the view wrote.
    acknowledged: u64,
    /// Whether it gets snapshots of the screen in place of output.
    behind: bool,
    /// Whether the screen changed since its last snapshot of the screen.
    dirty: bool,
    /// When it got its last snapshot of the screen.
    last_screen: Option<Instant>,
    /// Whether an update did not fit its channel, so the view starts over.
    lost: Cell<bool>,
}

impl View {
    /// A view that sends its updates to `sender`.
    pub(crate) fn new(id: String, sender: mpsc::Sender<TerminalUpdate>, scrollback: usize) -> Self {
        Self {
            id,
            sender,
            scrollback,
            sent: 0,
            acknowledged: 0,
            behind: false,
            dirty: false,
            last_screen: None,
            lost: Cell::new(false),
        }
    }

    /// Whether the view's channel closed.
    pub(crate) fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }

    /// Sends an update. One that does not fit makes the view start over; a
    /// closed channel ends the view on the next pass.
    pub(crate) fn send(&self, update: TerminalUpdate) {
        if self.lost.get() {
            return;
        }
        if let Err(mpsc::error::TrySendError::Full(_update)) = self.sender.try_send(update) {
            self.lost.set(true);
        }
    }

    /// Whether the view must start over, and its channel has room for it.
    pub(crate) fn must_start_over(&self) -> bool {
        self.lost.get() && self.sender.capacity() >= RESYNC_ROOM
    }

    /// Lets the view send again, before it starts over.
    pub(crate) fn start_over(&mut self) {
        self.lost.set(false);
        self.acknowledged = self.sent;
    }

    /// Rebuilds the view from a full snapshot and streams the output again.
    pub(crate) fn send_snapshot(&mut self, emulator: &Emulator) {
        let (data, first_row) = emulator.full_snapshot(self.scrollback);
        self.sent += data.len() as u64;
        self.behind = false;
        self.dirty = false;
        self.send(TerminalUpdate::Snapshot {
            seq: Count(self.sent),
            data,
            first_row: Count(first_row),
            top_row: Count(emulator.top_row()),
        });
    }

    /// Sends output, or falls behind when too much waits.
    pub(crate) fn output(&mut self, text: &str, top_row: u64) {
        if self.behind {
            self.dirty = true;
            return;
        }
        let waiting = self.sent - self.acknowledged.min(self.sent);
        if waiting + text.len() as u64 > HIGH_WATER {
            self.behind = true;
            self.dirty = true;
            return;
        }
        self.sent += text.len() as u64;
        self.send(TerminalUpdate::Output {
            seq: Count(self.sent),
            data: text.to_owned(),
            top_row: Count(top_row),
        });
    }

    /// Records what the view wrote.
    pub(crate) fn acknowledge(&mut self, seq: u64) {
        self.acknowledged = self.acknowledged.max(seq.min(self.sent));
    }

    /// Catches a view that fell behind up: the screen when it changed and
    /// its interval passed, a full snapshot once it caught up and the output
    /// is quiet. Returns when it next needs a pass.
    pub(crate) fn tick(
        &mut self,
        emulator: &Emulator,
        now: Instant,
        last_output: Option<Instant>,
    ) -> Option<Instant> {
        if !self.behind {
            return None;
        }
        let waiting = self.sent - self.acknowledged.min(self.sent);
        let quiet_at = last_output.map(|at| at + QUIET);
        let is_quiet = quiet_at.is_none_or(|at| at <= now);
        if waiting <= LOW_WATER && is_quiet {
            self.send_snapshot(emulator);
            return None;
        }
        let screen_due = self.last_screen.map_or(now, |last| last + SCREEN_INTERVAL);
        if self.dirty && screen_due <= now && waiting <= HIGH_WATER {
            let data = emulator.screen_snapshot();
            self.sent += data.len() as u64;
            self.dirty = false;
            self.last_screen = Some(now);
            self.send(TerminalUpdate::Screen {
                seq: Count(self.sent),
                data,
                top_row: Count(emulator.top_row()),
            });
        }
        let next_screen = self.last_screen.map_or(now, |last| last + SCREEN_INTERVAL);
        let next = match quiet_at {
            Some(quiet) if quiet > now => quiet.min(next_screen.max(now)),
            _ => next_screen.max(now + Duration::from_millis(10)),
        };
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use tokio::sync::mpsc;

    use super::{HIGH_WATER, View};
    use crate::commands::TerminalUpdate;
    use crate::emulator::Emulator;

    const NONCE: &str = "0123456789abcdef0123456789abcdef";

    fn kinds(receiver: &mut mpsc::Receiver<TerminalUpdate>) -> Vec<&'static str> {
        let mut kinds = Vec::new();
        while let Ok(update) = receiver.try_recv() {
            kinds.push(match update {
                TerminalUpdate::Snapshot { .. } => "snapshot",
                TerminalUpdate::Screen { .. } => "screen",
                TerminalUpdate::Output { .. } => "output",
                _ => "other",
            });
        }
        kinds
    }

    #[test]
    fn a_view_that_keeps_up_gets_the_output() {
        let emulator = Emulator::new(80, 24, NONCE, 1 << 20);
        let (sender, mut receiver) = mpsc::channel(64);
        let mut view = View::new("v".to_owned(), sender, 2000);

        view.send_snapshot(&emulator);
        view.output("hello", 0);
        view.acknowledge(1 << 40);
        view.output("again", 0);

        assert_eq!(kinds(&mut receiver), ["snapshot", "output", "output"]);
    }

    #[test]
    fn a_flood_turns_into_screens_and_then_a_snapshot() {
        let emulator = Emulator::new(80, 24, NONCE, 1 << 20);
        let (sender, mut receiver) = mpsc::channel(64);
        let mut view = View::new("v".to_owned(), sender, 2000);
        view.send_snapshot(&emulator);
        let chunk = "x".repeat(64 * 1024);
        let start = Instant::now();

        // Nothing is acknowledged, so the view falls behind at the mark.
        let mut chunks = 0;
        while !view.behind {
            view.output(&chunk, 0);
            chunks += 1;
        }
        // Seven chunks fit beside the snapshot; the eighth would pass the mark.
        assert_eq!(chunks, HIGH_WATER / (64 * 1024));
        view.output(&chunk, 0);
        // While the flood goes on, the view gets the screen at its interval.
        let last_output = Some(start);
        assert!(view.tick(&emulator, start, last_output).is_some());
        view.acknowledge(view.sent);
        let later = start + Duration::from_millis(50);
        view.tick(&emulator, later, Some(later));
        // Once it caught up and the output paused, a full snapshot.
        let quiet = later + Duration::from_millis(150);
        assert_eq!(view.tick(&emulator, quiet, Some(later)), None);

        let sent = kinds(&mut receiver);
        assert_eq!(sent.first(), Some(&"snapshot"));
        assert_eq!(sent.iter().filter(|kind| **kind == "screen").count(), 1);
        assert_eq!(sent.last(), Some(&"snapshot"));
        assert!(!view.behind);
    }
}
