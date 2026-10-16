//! What the tools tell the model about the machine: the operating system and
//! the shell that runs commands. The host's name and the programs on `PATH`
//! are there for a composition root that describes its machine, such as the
//! Workers plugin's service (ADR-0031, decision 2).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

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

/// The machine's name, or an empty name where the system does not say it.
pub fn hostname() -> String {
    platform::hostname()
}

/// The names of `programs` that a folder of `PATH` holds as a program, in
/// the order of `programs`.
pub fn programs_on_path(programs: &[&str]) -> Vec<String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    programs_in(&path, programs)
}

/// The names of `programs` that a folder of `path` holds as a program, in the
/// order of `programs`. Relative folders are skipped: they would name
/// whatever folder the process happens to run in.
fn programs_in(path: &OsStr, programs: &[&str]) -> Vec<String> {
    let folders: Vec<PathBuf> = std::env::split_paths(path)
        .filter(|folder| folder.is_absolute())
        .collect();
    programs
        .iter()
        .filter(|program| {
            folders
                .iter()
                .any(|folder| platform::is_program(folder, program))
        })
        .map(|program| (*program).to_owned())
        .collect()
}

/// Whether `file` is a regular file, following links.
fn is_file(file: &Path) -> bool {
    file.metadata().is_ok_and(|metadata| metadata.is_file())
}

#[cfg(windows)]
mod platform {
    use std::path::Path;

    use windows_sys::Win32::System::SystemInformation::{
        ComputerNameDnsHostname, GetComputerNameExW,
    };
    use windows_version::OsVersion;

    /// The extensions that make a file a program when `PATHEXT` is not set.
    const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

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

    /// The host's DNS name, or an empty name when Windows does not say it.
    #[expect(
        unsafe_code,
        reason = "GetComputerNameExW is a Win32 function that windows-sys exposes as unsafe"
    )]
    pub(super) fn hostname() -> String {
        let mut buffer = vec![0_u16; 64];
        loop {
            let mut length = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
            // SAFETY: the pointer and the length describe `buffer`, which
            // lives until the call returns; the function writes at most
            // `length` UTF-16 units into it, and sets `length` to the units it
            // wrote or to the size it needs.
            let written = unsafe {
                GetComputerNameExW(
                    ComputerNameDnsHostname,
                    buffer.as_mut_ptr(),
                    &raw mut length,
                )
            };
            let length = usize::try_from(length).unwrap_or(usize::MAX);
            if written != 0 {
                buffer.truncate(length);
                return String::from_utf16_lossy(&buffer);
            }
            if length <= buffer.len() || length > 1024 {
                return String::new();
            }
            buffer.resize(length, 0);
        }
    }

    /// Whether `folder` holds `name` with one of the extensions of
    /// `PATHEXT`.
    pub(super) fn is_program(folder: &Path, name: &str) -> bool {
        let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| DEFAULT_PATHEXT.to_owned());
        extensions
            .split(';')
            .filter(|extension| !extension.is_empty())
            .any(|extension| super::is_file(&folder.join(format!("{name}{extension}"))))
    }
}

#[cfg(unix)]
mod platform {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

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

    /// The host's name, as the kernel knows it.
    pub(super) fn hostname() -> String {
        rustix::system::uname()
            .nodename()
            .to_string_lossy()
            .into_owned()
    }

    /// Whether `folder` holds `name` as a file that someone may execute.
    pub(super) fn is_program(folder: &Path, name: &str) -> bool {
        let file = folder.join(name);
        super::is_file(&file)
            && file
                .metadata()
                .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
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

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;

    use super::{hostname, programs_in};

    /// Places a program named `name` in `folder`, as the system would find
    /// it.
    fn place_program(folder: &Path, name: &str) {
        #[cfg(windows)]
        fs::write(folder.join(format!("{name}.exe")), b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let file = folder.join(name);
            fs::write(&file, b"#!/bin/sh\n").unwrap();
            fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn programs_are_found_in_absolute_folders_of_path_in_the_lists_order() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        place_program(first.path(), "make");
        place_program(second.path(), "git");
        // A file that is not a program on this system.
        fs::write(second.path().join("docker.txt"), b"").unwrap();
        let path = std::env::join_paths([
            first.path().as_os_str(),
            OsString::from("relative").as_os_str(),
            second.path().as_os_str(),
        ])
        .unwrap();

        let found = programs_in(&path, &["git", "docker", "make", "nginx"]);

        assert_eq!(found, ["git", "make"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_nobody_may_execute_is_not_a_program() {
        use std::os::unix::fs::PermissionsExt;
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("make");
        fs::write(&file, b"").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();

        let found = programs_in(folder.path().as_os_str(), &["make"]);

        assert_eq!(found, Vec::<String>::new());
    }

    #[test]
    fn the_machine_has_a_name() {
        assert_ne!(hostname(), "");
    }
}
