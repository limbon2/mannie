//! The worker's folders on each system (plugins/workers/docs/service.md,
//! folders): the program, the configuration with the host instructions, and
//! the data.

use std::path::PathBuf;

#[cfg(windows)]
use anyhow::Context;

/// The configuration's file name in its folder.
pub(crate) const CONFIG_FILE: &str = "worker.toml";

/// The host instructions' file name beside the configuration.
pub(crate) const HOST_INSTRUCTIONS_FILE: &str = "host-instructions.md";

/// The program's file name.
pub(crate) const PROGRAM_FILE: &str = if cfg!(windows) {
    "mannie-worker.exe"
} else {
    "mannie-worker"
};

/// The previous program, which `service install` and `update` keep beside
/// the program.
pub(crate) const PREVIOUS_PROGRAM_FILE: &str = if cfg!(windows) {
    "mannie-worker.previous.exe"
} else {
    "mannie-worker.previous"
};

/// The link to the program on the search path, on Linux and FreeBSD.
#[cfg(unix)]
pub(crate) const PROGRAM_LINK: &str = "/usr/local/bin/mannie-worker";

/// The system's folders of the worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Folders {
    /// The folder of the configuration and the host instructions.
    pub(crate) config: PathBuf,
    /// The data folder, unless the configuration names another.
    pub(crate) data: PathBuf,
    /// The program's folder.
    pub(crate) program: PathBuf,
}

impl Folders {
    /// The folders of this system.
    pub(crate) fn of_this_system() -> anyhow::Result<Self> {
        system_folders()
    }

    /// The configuration file in its folder.
    pub(crate) fn config_file(&self) -> PathBuf {
        self.config.join(CONFIG_FILE)
    }
}

#[cfg(target_os = "linux")]
#[expect(
    clippy::unnecessary_wraps,
    reason = "Windows reads its folders from the environment, which can fail"
)]
fn system_folders() -> anyhow::Result<Folders> {
    Ok(Folders {
        config: PathBuf::from("/etc/mannie-worker"),
        data: PathBuf::from("/var/lib/mannie-worker"),
        program: PathBuf::from("/usr/local/libexec/mannie-worker"),
    })
}

#[cfg(target_os = "freebsd")]
#[expect(
    clippy::unnecessary_wraps,
    reason = "Windows reads its folders from the environment, which can fail"
)]
fn system_folders() -> anyhow::Result<Folders> {
    Ok(Folders {
        config: PathBuf::from("/usr/local/etc/mannie-worker"),
        data: PathBuf::from("/var/db/mannie-worker"),
        program: PathBuf::from("/usr/local/libexec/mannie-worker"),
    })
}

#[cfg(windows)]
fn system_folders() -> anyhow::Result<Folders> {
    let program_data = std::env::var_os("ProgramData")
        .context("the ProgramData environment variable is missing")?;
    let program_files = std::env::var_os("ProgramFiles")
        .context("the ProgramFiles environment variable is missing")?;
    let config = PathBuf::from(program_data).join("Mannie").join("worker");
    Ok(Folders {
        data: config.join("data"),
        config,
        program: PathBuf::from(program_files).join("Mannie").join("Worker"),
    })
}
