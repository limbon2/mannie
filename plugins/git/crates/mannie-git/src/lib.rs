//! The git plugin (ADR-0038; plugins/git/docs/README.md): tools that read
//! and change the owner's repositories through the installed git, under the
//! core's policy, so agents work with git without a shell.
//!
//! git runs through the service `processes`, which finds it on Mannie's
//! `PATH` and starts it without a shell, with the manifest's fixed arguments
//! first (plugins/git/docs/adr/0001-run-the-owners-installed-git.md). The
//! tools pass arguments exactly, read git's machine formats, and never
//! start a program that the repository's own configuration names on a read
//! (T-44).

mod arguments;
mod changes;
mod commands;
mod config;
mod describe;
mod effects;
mod failure;
mod git;
mod layout;
mod mentions;
mod operation;
mod outcome;
mod output;
mod panel;
mod parse;
pub mod plugin;
mod redact;
mod schema;
mod section;
mod texts;
mod tools;
mod version;

pub use plugin::{GIT, GitPlugin};

mannie_plugin_sdk::export_plugin!(GitPlugin::new);
