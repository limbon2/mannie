//! The fixture plugin (ADR-0035, decision 9): notes that the owner keeps,
//! with one command that adds a note, one channel that follows the notes,
//! one instruction section and one table. Validation builds bundle it, and
//! the core's tests use it in place of the plugins that ship, so they test
//! the host, the dispatch, the grants and the store through a plugin that
//! uses the plugin SDK and nothing else.

pub mod commands;
pub mod plugin;

pub use plugin::{EXAMPLE, ExamplePlugin};

mannie_plugin_sdk::export_plugin!(ExamplePlugin::new);
