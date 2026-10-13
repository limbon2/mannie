//! The worker as a service of the operating system (architecture overview,
//! deployment): a Windows service, a systemd unit or an rc.d service.
//! `install` registers the worker to start with the system and starts it;
//! `uninstall` stops and removes it and keeps the data folder.
//!
//! The service manager runs the worker under the chosen account. On Windows
//! that is the virtual account of the service, which needs no password; on
//! Linux and FreeBSD it is the account given to `install`, and the service
//! manager creates the data folder for it, readable only by it.

#[cfg(target_os = "freebsd")]
mod freebsd;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

use anyhow::Context;

use crate::cli::InstallArguments;

/// The service's name in the service manager. FreeBSD's rc.d needs a shell
/// variable name and uses `mannie_worker`.
const SERVICE_NAME: &str = "mannie-worker";

/// Installs the worker as a service and starts it.
pub(crate) fn install(arguments: &InstallArguments) -> anyhow::Result<()> {
    let executable = std::env::current_exe().context("could not find the worker's executable")?;
    let data_directory = std::path::absolute(&arguments.data_dir).with_context(|| {
        format!(
            "the data folder {} is not a usable path",
            arguments.data_dir.display()
        )
    })?;
    #[cfg(target_os = "linux")]
    let installed = linux::install(&executable, &data_directory, &arguments.user);
    #[cfg(target_os = "freebsd")]
    let installed = freebsd::install(&executable, &data_directory, &arguments.user);
    #[cfg(windows)]
    let installed = windows::install(&executable, &data_directory);
    installed
}

/// Stops the service and removes it. The data folder stays.
pub(crate) fn uninstall() -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    let removed = linux::uninstall();
    #[cfg(target_os = "freebsd")]
    let removed = freebsd::uninstall();
    #[cfg(windows)]
    let removed = windows::uninstall();
    removed
}

/// Runs the worker under the Windows service control manager.
#[cfg(windows)]
pub(crate) fn run_as_windows_service(data_directory: &std::path::Path) -> anyhow::Result<()> {
    windows::run(data_directory)
}

/// Prints what an install or uninstall step did.
#[expect(
    clippy::print_stdout,
    reason = "the service commands report each step on the console"
)]
fn report(step: &str) {
    println!("{step}");
}
