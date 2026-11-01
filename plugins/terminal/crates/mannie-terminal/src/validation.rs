//! The headless emulator as the benchmark of `task bench` drives it
//! (plugins/terminal/docs/adr/0002-keep-a-headless-emulator-of-each-terminal-in-rust.md):
//! the emulator of a terminal with its capture of rows and markers, fed as a
//! terminal's pump feeds it, and the snapshots that its views get. Only the
//! feature `validation` builds it.

use crate::emulator::Emulator;

/// A terminal's emulator.
#[derive(Debug)]
pub struct BenchEmulator {
    emulator: Emulator,
}

impl BenchEmulator {
    /// An emulator of `columns` by `rows` whose markers carry `nonce`, and
    /// whose text log keeps at most `log_cap` bytes.
    pub fn new(columns: u16, rows: u16, nonce: &str, log_cap: usize) -> Self {
        Self {
            emulator: Emulator::new(columns, rows, nonce, log_cap),
        }
    }

    /// Feeds output, and returns how many of this terminal's markers it held.
    pub fn feed(&mut self, bytes: &[u8]) -> usize {
        self.emulator.process(bytes).len()
    }

    /// The bytes that the text log holds.
    pub fn log_bytes(&self) -> usize {
        self.emulator.log_bytes()
    }

    /// The size in bytes of the full snapshot with at most `lines` lines
    /// above the screen, as a view that opens gets it.
    pub fn full_snapshot_bytes(&self, lines: usize) -> usize {
        self.emulator.full_snapshot(lines).0.len()
    }

    /// The size in bytes of the snapshot of the screen, as a view that fell
    /// behind gets it.
    pub fn screen_snapshot_bytes(&self) -> usize {
        self.emulator.screen_snapshot().len()
    }
}
