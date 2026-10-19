//! The SSH plugin (ADR-0036; plugins/ssh/docs/README.md): computers without
//! Mannie that the owner registers by hand, with an address, a port, a user
//! name and a password, and the tool `run_ssh_command`, which runs commands
//! on them under the master's policy and journal.
//!
//! The plugin speaks SSH through russh on ring, pins each host's key once
//! the owner checked its fingerprint, and sends a password only to a host
//! that shows the pinned key (plugins/ssh/docs/adr/0001-speak-ssh-with-
//! russh-and-pinned-host-keys.md). The passwords live in the credential
//! store, one entry for each host.

mod client;
pub mod commands;
mod hosts;
mod mentions;
mod output;
pub mod plugin;
pub mod rules;
mod schema;
mod section;
mod sessions;
mod store;
mod texts;
mod tool;

pub use plugin::{SSH, SshPlugin};
pub use store::MODULE;
pub use tool::RUN_SSH_COMMAND;

mannie_plugin_sdk::export_plugin!(SshPlugin::new);
