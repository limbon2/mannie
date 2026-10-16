//! What a worker reports about its host (docs/features/workers/
//! host-and-ceiling.md, host facts): the operating system and the shell that
//! runs commands, the host's name, its other shells, and which programs of a
//! fixed list are on `PATH`.

use mannie_link::host::HostFacts;
use mannie_tools::SystemFacts;

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
    shells.extend(mannie_tools::programs_on_path(&OTHER_SHELLS));
    HostFacts {
        system: system.system.clone(),
        hostname: mannie_tools::hostname(),
        shells,
        tools: mannie_tools::programs_on_path(&REPORTED_TOOLS),
    }
}

#[cfg(test)]
mod tests {
    use mannie_tools::SystemFacts;

    use super::{REPORTED_TOOLS, host_facts};

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
