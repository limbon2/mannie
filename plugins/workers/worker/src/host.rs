//! What a worker reports about its host (plugins/workers/docs/
//! host-and-ceiling.md, host facts): the operating system and the shell that
//! runs commands, the host's name, its other shells, and which programs of a
//! fixed list are on `PATH`. The worker finds the host's name and the
//! programs itself; the tools' facts give the system and the shell
//! (ADR-0034, decision 12).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use mannie_host_kit::tools::SystemFacts;
use mannie_link::host::HostFacts;

/// The shells that a worker reports besides the one that runs commands, when
/// they are on `PATH`.
const OTHER_SHELLS: [&str; 3] = ["pwsh", "bash", "zsh"];

/// The programs that a worker reports when they are on `PATH`. The list is
/// fixed, so the facts say what the host has and never what it holds.
const REPORTED_TOOLS: [&str; 24] = [
    "git",
    "python3",
    "python",
    "node",
    "npm",
    "cargo",
    "go",
    "java",
    "make",
    "cmake",
    "docker",
    "podman",
    "kubectl",
    "systemctl",
    "service",
    "apt",
    "dnf",
    "pkg",
    "zfs",
    "jexec",
    "winget",
    "choco",
    "psql",
    "nginx",
];

/// The host's facts: `system`'s, the host's name, its shells with the one
/// that runs commands first, and which programs of the fixed list are on
/// `PATH`.
pub(crate) fn host_facts(system: &SystemFacts) -> HostFacts {
    let mut shells: Vec<String> = system
        .shell
        .iter()
        .map(|shell| shell.name.clone())
        .collect();
    shells.extend(programs_on_path(&OTHER_SHELLS));
    HostFacts {
        system: system.system.clone(),
        hostname: hostname(),
        shells,
        tools: programs_on_path(&REPORTED_TOOLS),
    }
}

/// The host's name, or an empty name where the system does not say it.
pub(crate) fn hostname() -> String {
    platform::hostname()
}

/// The names of `programs` that a folder of `PATH` holds as a program, in
/// the order of `programs`.
fn programs_on_path(programs: &[&str]) -> Vec<String> {
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

    /// The extensions that make a file a program when `PATHEXT` is not set.
    const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

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
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

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
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;

    use mannie_host_kit::tools::SystemFacts;

    use super::{REPORTED_TOOLS, host_facts, hostname, programs_in};

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
    fn the_host_has_a_name() {
        assert_ne!(hostname(), "");
    }

    #[test]
    fn host_facts_name_the_host_and_the_shell_that_runs_commands_first() {
        let system = SystemFacts::detect();

        let facts = host_facts(&system);

        assert_eq!(facts.system, system.system);
        assert_ne!(facts.hostname, "");
        if let Some(shell) = &system.shell {
            assert_eq!(facts.shells.first(), Some(&shell.name));
        }
        assert!(
            facts
                .tools
                .iter()
                .all(|tool| REPORTED_TOOLS.contains(&tool.as_str()))
        );
    }
}
