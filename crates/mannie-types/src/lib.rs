//! Types that Mannie's crates share: IPC contracts, settings sections, the
//! injected clock and the redacting secret type.
//!
//! This crate performs no I/O, so the worker, the desktop app and the tests can
//! all depend on it (docs/engineering/repository.md). Types that cross IPC
//! derive `specta::Type`, and the desktop app generates the TypeScript bindings
//! from them (ADR-0007).

pub mod clock;
pub mod interface;
pub mod ipc;
pub mod secret;
pub mod settings;
pub mod startup;
#[cfg(feature = "validation")]
pub mod validation;
