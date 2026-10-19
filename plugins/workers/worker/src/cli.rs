//! The command line (plugins/workers/docs/service.md, the command line). Its
//! output stays in English ([ADR-0019]).
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
    /// Create a one-time code that pairs this worker with a Mannie desktop
    /// app, or forget the paired one with --reset. Needs root or
    /// administrator rights.
    Pair(PairArguments),
    /// Say whether the configuration is valid, whether the service runs,
    /// whether the worker is paired, and where it listens.
    Status(ConfigArgument),
    /// Print the facts about this host that the worker reports to Mannie.
    Facts,
    /// Print this program's version, system and release key, for `update`.
    SelfCheck(SelfCheckArguments),
    /// Install a signed release of the worker, and put the previous program
    /// back if the new one does not start. Needs root or administrator
    /// rights.
    Update(UpdateArguments),
    /// Install or remove the worker as a service of the operating system.
    #[command(subcommand)]
    Service(ServiceCommand),
}

/// The configuration file to use.
#[derive(Debug, Clone, Args)]
pub(crate) struct ConfigArgument {
    /// The configuration file; `worker.toml` in the system's configuration
    /// folder when left out.
    #[arg(long, value_name = "FILE")]
    pub(crate) config: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub(crate) struct RunArguments {
    #[command(flatten)]
    pub(crate) config: ConfigArgument,
    /// The data folder, in place of the configuration's. Without a
    /// configuration file, the worker runs with the default configuration
    /// and this folder.
    #[arg(long, value_name = "FOLDER")]
    pub(crate) data_dir: Option<PathBuf>,
    /// Run although this account can change the configuration, which lets
    /// the worker widen its own ceiling. Only for a worker in a terminal or
    /// a test; the services never pass it.
    #[arg(long)]
    pub(crate) allow_writable_config: bool,
}

#[derive(Debug, Args)]
pub(crate) struct PairArguments {
    #[command(flatten)]
    pub(crate) config: ConfigArgument,
    /// Forget the paired Mannie and restart the service, which drops every
    /// lease and interrupts the tasks, so that another Mannie can pair.
    #[arg(long)]
    pub(crate) reset: bool,
}

#[derive(Debug, Args)]
pub(crate) struct SelfCheckArguments {
    /// Print the check as JSON, the only form it has.
    #[arg(long, required = true)]
    pub(crate) json: bool,
}

#[derive(Debug, Args)]
pub(crate) struct UpdateArguments {
    /// The release's archive for this system. Its signature is the same
    /// file with `.sig` added.
    #[arg(value_name = "ARCHIVE")]
    pub(crate) archive: PathBuf,
    /// Only check the archive's signature.
    #[arg(long)]
    pub(crate) check: bool,
    #[command(flatten)]
    pub(crate) config: ConfigArgument,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ServiceCommand {
    /// Copy this program into its folder, write a configuration and host
    /// instructions when there are none, install the worker as a service
    /// that starts with the system, and start it. Needs administrator
    /// (Windows) or root (Linux, FreeBSD) rights.
    Install(InstallArguments),
    /// Stop and remove the service. The program, the configuration and the
    /// data stay.
    Uninstall,
    /// Run as the Windows service. The Windows service control manager starts
    /// the worker this way.
    #[cfg(windows)]
    #[command(hide = true)]
    Run(RunArguments),
}

#[derive(Debug, Args)]
pub(crate) struct InstallArguments {
    #[command(flatten)]
    pub(crate) config: ConfigArgument,
    /// The account that runs the service. The worker's tools act on this host
    /// with this account's rights.
    #[cfg(unix)]
    #[arg(long, value_name = "ACCOUNT")]
    pub(crate) user: String,
}

#[cfg(test)]
mod tests;
