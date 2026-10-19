//! The worker as a service of the operating system (plugins/workers/docs/
//! service.md): a Windows service, a systemd unit or an rc.d service.
//! `install` copies the program into its folder, writes a configuration and
//! host instructions when there are none, registers the worker to start
//! with the system and starts it; `uninstall` stops and removes the service
//! and keeps the program, the configuration and the data.
//!
//! The service manager runs the worker under the chosen account. On Windows
//! that is the virtual account of the service, which needs no password; on
//! Linux and FreeBSD it is the account given to `install`, and the service
//! manager creates the data folder for it, readable only by it. The
//! configuration belongs to root or the administrators, and the service's
//! account can only read it.

#[cfg(target_os = "freebsd")]
mod freebsd;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::Context;
use mannie_host_kit::types::SystemClock;
use mannie_plugin_sdk::contracts::Clock;

use crate::cli::InstallArguments;
use crate::config;
use crate::paths::{Folders, HOST_INSTRUCTIONS_FILE, PREVIOUS_PROGRAM_FILE, PROGRAM_FILE};

/// The service's name in the service manager. FreeBSD's rc.d needs a shell
/// variable name and uses `mannie_worker`.
const SERVICE_NAME: &str = "mannie-worker";

/// What `pair --reset` and `update` do with the installed service.
pub(crate) trait ServiceControl {
    /// Whether the service is installed.
    fn is_installed(&self) -> anyhow::Result<bool>;
    /// Whether it runs.
    fn is_running(&self) -> anyhow::Result<bool>;
    /// Stops it, and returns once it stopped.
    fn stop(&self) -> anyhow::Result<()>;
    /// Starts it.
    fn start(&self) -> anyhow::Result<()>;
}

/// The service manager of this system.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SystemService;

impl ServiceControl for SystemService {
    fn is_installed(&self) -> anyhow::Result<bool> {
        #[cfg(target_os = "linux")]
        let installed = linux::is_installed();
        #[cfg(target_os = "freebsd")]
        let installed = freebsd::is_installed();
        #[cfg(windows)]
        let installed = windows::is_installed();
        installed
    }

    fn is_running(&self) -> anyhow::Result<bool> {
        #[cfg(target_os = "linux")]
        let running = linux::is_running();
        #[cfg(target_os = "freebsd")]
        let running = freebsd::is_running();
        #[cfg(windows)]
        let running = windows::is_running();
        running
    }

    fn stop(&self) -> anyhow::Result<()> {
        #[cfg(target_os = "linux")]
        let stopped = linux::stop();
        #[cfg(target_os = "freebsd")]
        let stopped = freebsd::stop();
        #[cfg(windows)]
        let stopped = windows::stop();
        stopped
    }

    fn start(&self) -> anyhow::Result<()> {
        #[cfg(target_os = "linux")]
        let started = linux::start();
        #[cfg(target_os = "freebsd")]
        let started = freebsd::start();
        #[cfg(windows)]
        let started = windows::start();
        started
    }
}

/// Copies the program into its folder, prepares the configuration, installs
/// the worker as a service and starts it.
pub(crate) fn install(arguments: &InstallArguments) -> anyhow::Result<()> {
    if SystemService.is_installed()? {
        anyhow::bail!(
            "the service {SERVICE_NAME} is already installed. Run `mannie-worker service uninstall` first"
        );
    }
    let folders = Folders::of_this_system()?;
    let config_file = arguments
        .config
        .config
        .clone()
        .unwrap_or_else(|| folders.config_file());
    let config_file = std::path::absolute(&config_file).with_context(|| {
        format!(
            "the configuration {} is not a usable path",
            config_file.display()
        )
    })?;
    let program = install_program(&folders)?;
    prepare_configuration(&config_file, &folders)?;
    let config = config::load(&config_file, &folders, SystemClock.now())?;
    #[cfg(target_os = "linux")]
    let installed = linux::install(&program, &config_file, &config, &arguments.user);
    #[cfg(target_os = "freebsd")]
    let installed = freebsd::install(&program, &config_file, &config, &arguments.user);
    #[cfg(windows)]
    let installed = windows::install(&program, &config_file, &config);
    installed
}

/// Stops the service and removes it. The program, the configuration and the
/// data stay.
pub(crate) fn uninstall() -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    let removed = linux::uninstall();
    #[cfg(target_os = "freebsd")]
    let removed = freebsd::uninstall();
    #[cfg(windows)]
    let removed = windows::uninstall();
    removed?;
    let folders = Folders::of_this_system()?;
    report(&format!(
        "The program in {}, the configuration in {} and the data were kept; delete their folders to remove them.",
        folders.program.display(),
        folders.config.display()
    ));
    Ok(())
}

/// Runs the worker under the Windows service control manager.
#[cfg(windows)]
pub(crate) fn run_as_windows_service(arguments: &crate::cli::RunArguments) -> anyhow::Result<()> {
    windows::run(arguments)
}

/// Copies this program into the program folder and returns its path there.
/// A program that is there already becomes the previous one. The copy goes
/// to a temporary name first, so the folder never holds half a program.
fn install_program(folders: &Folders) -> anyhow::Result<PathBuf> {
    let source = std::env::current_exe().context("could not find this program")?;
    let target = folders.program.join(PROGRAM_FILE);
    create_system_folder(&folders.program)?;
    let same = fs::canonicalize(&source).ok() == fs::canonicalize(&target).ok();
    if !same {
        let staged = folders.program.join(format!("{PROGRAM_FILE}.new"));
        fs::copy(&source, &staged).with_context(|| {
            format!(
                "could not copy the program to {}. Run the command as root or administrator",
                staged.display()
            )
        })?;
        if target.exists() {
            let previous = folders.program.join(PREVIOUS_PROGRAM_FILE);
            fs::rename(&target, &previous).with_context(|| {
                format!(
                    "could not keep the previous program as {}",
                    previous.display()
                )
            })?;
        }
        fs::rename(&staged, &target)
            .with_context(|| format!("could not put the program at {}", target.display()))?;
        report(&format!("Copied the program to {}.", target.display()));
    }
    #[cfg(unix)]
    unix::link_program(&target)?;
    Ok(target)
}

/// Creates a folder that root or the administrators own, readable by every
/// account.
fn create_system_folder(folder: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(folder).with_context(|| {
        format!(
            "could not create {}. Run the command as root or administrator",
            folder.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(folder, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("could not set the permissions of {}", folder.display()))?;
    }
    Ok(())
}

/// Creates the configuration's folder, and writes the configuration and an
/// empty host instructions file when they are missing. Both belong to root
/// or the administrators, and every account can read them.
fn prepare_configuration(config_file: &Path, folders: &Folders) -> anyhow::Result<()> {
    let folder = config_file.parent().unwrap_or(&folders.config);
    create_system_folder(folder)?;
    let name = crate::host::hostname();
    if write_new(config_file, &config::initial_file(&name, folders))? {
        report(&format!(
            "Wrote the configuration {}. Edit it as root or administrator, then restart the service.",
            config_file.display()
        ));
    }
    let instructions = folder.join(HOST_INSTRUCTIONS_FILE);
    if write_new(&instructions, "")? {
        report(&format!(
            "Wrote the empty host instructions {}.",
            instructions.display()
        ));
    }
    Ok(())
}

/// Writes a new file that every account can read, and returns whether it
/// wrote one: a file that exists is kept as it is.
fn write_new(path: &Path, content: &str) -> anyhow::Result<bool> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o644);
    }
    match options.open(path) {
        Ok(mut file) => {
            io::Write::write_all(&mut file, content.as_bytes())
                .and_then(|()| file.sync_all())
                .with_context(|| format!("could not write {}", path.display()))?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error).with_context(|| format!("could not write {}", path.display())),
    }
}

/// Prints what an install or uninstall step did.
#[expect(
    clippy::print_stdout,
    reason = "the service commands report each step on the console"
)]
fn report(step: &str) {
    println!("{step}");
}
