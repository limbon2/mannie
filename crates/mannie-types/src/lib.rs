//! Types that Mannie's crates share: IPC contracts, the domain's identifiers,
//! the transcript and its events, settings sections, the injected clock and the
//! redacting secret type.
//!
//! This crate performs no I/O, so the worker, the desktop app and the tests can
//! all depend on it (docs/engineering/repository.md). Types that cross IPC
//! derive `specta::Type`, and the desktop app generates the TypeScript bindings
//! from them (ADR-0007).

pub mod ceiling;
pub mod chat;
pub mod clock;
pub mod ids;
pub mod interface;
pub mod ipc;
pub mod links;
pub mod mentions;
pub mod plugins;
pub mod projects;
pub mod provider;
pub mod run;
pub mod secret;
pub mod settings;
pub mod startup;
pub mod tasks;
pub mod tools;
pub mod transcript;
#[cfg(feature = "validation")]
pub mod validation;
pub mod window;
