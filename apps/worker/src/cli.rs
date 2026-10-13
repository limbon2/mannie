//! The command line. Its output stays in English ([ADR-0019]).
//!
//! [ADR-0019]: ../../../docs/adr/0019-keep-visible-text-in-message-catalogs.md

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// The Mannie worker runs Mannie's agent runtime on this host for a paired
/// Mannie desktop app.
#[derive(Debug, Parser)]
#[command(name = "mannie-worker", version, about)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Run the worker in this terminal until Ctrl+C or a termination signal.
    Run(RunArguments),
    /// Install or remove the worker as a service of the operating system.
    #[command(subcommand)]
    Service(ServiceCommand),
}

#[derive(Debug, Args)]
pub(crate) struct RunArguments {
    /// The folder for the worker's database, its backups and its logs. It is
    /// created when missing.
    #[arg(long, value_name = "FOLDER")]
    pub(crate) data_dir: PathBuf,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ServiceCommand {
    /// Install the worker as a service that starts with the system, and start
    /// it. Needs administrator (Windows) or root (Linux, FreeBSD) rights.
    Install(InstallArguments),
    /// Stop and remove the service. The data folder is kept.
    Uninstall,
    /// Run as the Windows service. The Windows service control manager starts
    /// the worker this way.
    #[cfg(windows)]
    #[command(hide = true)]
    Run(RunArguments),
}

#[derive(Debug, Args)]
pub(crate) struct InstallArguments {
    /// The folder for the worker's database, its backups and its logs. Only
    /// the service's account (and, on Windows, SYSTEM and the administrators)
    /// can open it.
    #[arg(long, value_name = "FOLDER")]
    pub(crate) data_dir: PathBuf,
    /// The account that runs the service. The worker's tools act on this host
    /// with this account's rights.
    #[cfg(unix)]
    #[arg(long, value_name = "ACCOUNT")]
    pub(crate) user: String,
}

#[cfg(test)]
mod tests;
