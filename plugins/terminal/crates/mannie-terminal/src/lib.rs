//! The terminal plugin (ADR-0053; plugins/terminal/docs/README.md):
//! terminals beside a conversation, on this computer and on hosts that
//! plugins offer, such as the SSH plugin's. The owner types in them as in any
//! terminal, the tool `run_in_terminal` lets the conversation's agent run
//! commands in them under the master's policy and journal, and a reply cites
//! a command's output by its link.
//!
//! The core starts the shells and contains them (ADR-0053, decisions 1 and
//! 4). The plugin keeps a headless emulator of each terminal, which numbers
//! its rows and reads the setup's markers
//! (plugins/terminal/docs/adr/0002-keep-a-headless-emulator-of-each-terminal-in-rust.md),
//! and the views in the webview get only what they can show.

mod blocks;
mod cite;
pub mod commands;
mod emulator;
mod marks;
mod mentions;
pub mod plugin;
mod registry;
mod schema;
mod section;
mod session;
mod setup;
mod sgr;
mod store;
mod texts;
mod tools;
#[cfg(feature = "validation")]
pub mod validation;
mod views;

pub use blocks::Author;
pub use plugin::{TERMINAL, TerminalPlugin};
pub use store::MODULE;
pub use tools::{LIST_TERMINALS, RUN_IN_TERMINAL};

mannie_plugin_sdk::export_plugin!(TerminalPlugin::new);
