//! The Notebooks plugin (plugins/notebooks/docs/README.md): notebooks of the
//! owner's documents, each with chats of its own, which the conversations
//! that attach them search with the plugin's tools and whose passages reach
//! the messages that name them (ADR-0046, ADR-0049 to ADR-0052), found with
//! the owner's embedding and rerank models (ADR-0047).

pub mod commands;
mod indexer;
mod library;
mod mentions;
pub mod plugin;
mod provider;
mod search;
pub mod store;
mod tools;
mod uploads;

pub use plugin::{NOTEBOOKS, NotebooksPlugin};

mannie_plugin_sdk::export_plugin!(NotebooksPlugin::new);
