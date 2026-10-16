//! The master's side of the Workers plugin (ADR-0031; docs/features/workers/
//! README.md): the paired workers and the plugin's tables, pairing, the
//! hub that keeps a connection to each worker over the link, and the
//! executor that gives tasks to workers through the runtime's task
//! executors.
//!
//! The core never depends on this crate (ADR-0030, decision 10). The
//! desktop app's composition root starts and stops the plugin, and its IPC
//! commands call into it.

pub mod commands;
pub mod delegation;
pub mod executor;
mod follow;
pub mod hub;
pub mod keys;
pub mod plugin;
pub mod store;
#[cfg(feature = "testing")]
pub mod testing;
pub mod texts;
pub mod view;

pub use executor::{DELEGATE_TASK, WORKER_AGENT, WORKERS_KIND, WorkersExecutor};
pub use plugin::WorkersPlugin;
