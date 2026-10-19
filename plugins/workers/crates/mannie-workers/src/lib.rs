//! The master's side of the Workers plugin (ADR-0031; plugins/workers/docs/
//! README.md): the paired workers and the plugin's tables, pairing, the
//! hub that keeps a connection to each worker over the link, and the
//! executor that gives tasks to workers through the runtime's task
//! executors.
//!
//! The core never depends on this crate (ADR-0030, decision 10). The crate
//! is the plugin's entry crate: it exports the plugin with its manifest,
//! `plugins/workers/plugin.toml`, for the list of plugins (ADR-0034,
//! decision 4), and the plugin host starts and stops it and dispatches its
//! commands and channel, which reach it through its inlined Tauri plugin
//! (decision 8).

pub mod commands;
pub mod delegation;
pub mod executor;
mod follow;
pub mod hub;
pub mod keys;
pub mod plugin;
pub mod store;
pub mod texts;
pub mod view;

pub use executor::{DELEGATE_TASK, WORKER_AGENT, WORKERS_KIND, WorkersExecutor};
pub use plugin::WorkersPlugin;

mannie_plugin_sdk::export_plugin!(WorkersPlugin::new);
