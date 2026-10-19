//! What the tools tell the model about the machine: the operating system and
//! the shell that runs commands. A program that describes its host further,
//! such as the Workers plugin's worker, finds the rest itself (ADR-0034,
//! decision 12).

use std::path::PathBuf;

/// The facts that `system_info` reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemFacts {
    /// The operating system with its version and architecture, such as
    /// `Windows 11 (10.0.26100), x86_64`.
    pub system: String,
    /// The shell that runs commands, or `None` where `run_command` is not
    /// available.
    pub shell: Option<ShellFacts>,
}

/// The shell that runs commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellFacts {
    /// The shell's name and version, such as `Windows PowerShell 5.1`.
    pub name: String,
    /// The shell's path.
    pub path: PathBuf,
}

impl SystemFacts {
    /// The facts of this machine.
    pub fn detect() -> Self {
        Self {
            system: platform::system(),
            shell: crate::supervisor::shell(),
        }
    }
}

#[cfg(windows)]
mod platform {
    use windows_version::OsVersion;

    /// The edition and version of Windows. Windows 11 still reports version
    /// 10, so its build number tells it apart.
    pub(super) fn system() -> String {
        let version = OsVersion::current();
        let name = if windows_version::is_server() {
            "Windows Server".to_owned()
        } else if version.major == 10 && version.build >= 22_000 {
            "Windows 11".to_owned()
        } else {
            format!("Windows {}", version.major)
        };
        format!(
            "{name} ({}.{}.{}), {}",
            version.major,
            version.minor,
            version.build,
            std::env::consts::ARCH
        )
    }
}

#[cfg(unix)]
mod platform {
    use std::fs;

    /// The distribution from `os-release`, the kernel and its release.
    pub(super) fn system() -> String {
        let name = rustix::system::uname();
        let kernel = format!(
            "{} {}",
            name.sysname().to_string_lossy(),
            name.release().to_string_lossy()
        );
        let architecture = std::env::consts::ARCH;
        match pretty_name() {
            Some(distribution) => format!("{distribution} ({kernel}), {architecture}"),
            None => format!("{kernel}, {architecture}"),
        }
    }

    /// `PRETTY_NAME` from `/etc/os-release`, such as `Ubuntu 24.04.1 LTS`.
    fn pretty_name() -> Option<String> {
        let release = fs::read_to_string("/etc/os-release").ok()?;
        release.lines().find_map(|line| {
            let value = line.strip_prefix("PRETTY_NAME=")?;
            Some(value.trim_matches('"').to_owned())
        })
    }
}
