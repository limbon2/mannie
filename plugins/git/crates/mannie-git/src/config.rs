//! The repository's configuration, with the scope of each setting, as `git
//! config --list --show-scope -z` writes it (plugins/git/docs/approvals.md;
//! T-44). A test reads the real git's output too, since a reader that does
//! not match it would blank nothing. A repository's own configuration, its `local` and `worktree`
//! scopes, comes with the repository and may name programs: filters that
//! git starts on reads of the work tree, and programs that changes start. A
//! read blanks the filters that the repository defines for itself, so it
//! starts none of them; filters of the owner's global or system
//! configuration, such as Git LFS, still run.

use std::path::Path;

use mannie_plugin_sdk::services::processes::ReadPlace;

use crate::failure::GitFailure;
use crate::git::{Git, GitRun};

/// Where a setting comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    /// The system's configuration.
    System,
    /// The owner's own configuration.
    Global,
    /// The repository's `.git/config`, or a file that it includes.
    Local,
    /// The configuration of one of the repository's worktrees.
    Worktree,
    /// A setting of the command line, such as the manifest's fixed ones.
    Command,
    /// A scope that this version does not know.
    Other,
}

impl Scope {
    fn of(name: &str) -> Self {
        match name {
            "system" => Self::System,
            "global" => Self::Global,
            "local" => Self::Local,
            "worktree" => Self::Worktree,
            "command" => Self::Command,
            _other => Self::Other,
        }
    }

    /// Whether the setting came with the repository, which anyone who wrote
    /// to it may have written.
    pub(crate) fn is_repository(self) -> bool {
        matches!(self, Self::Local | Self::Worktree)
    }

    /// The scope's name, as git names it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Global => "global",
            Self::Local => "local",
            Self::Worktree => "worktree",
            Self::Command => "command",
            Self::Other => "other",
        }
    }
}

/// One setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Setting {
    pub(crate) scope: Scope,
    /// Its key as git prints it: the section and the name in lower case,
    /// and a subsection as written, such as `filter.LFS.clean`.
    pub(crate) key: String,
    /// Its value, or `None` for a key without one, which reads as true.
    pub(crate) value: Option<String>,
}

impl Setting {
    /// The subsection and the name of a key of `section`, such as `lfs` and
    /// `clean` for `filter.lfs.clean`. A subsection may hold dots.
    pub(crate) fn subsection_of(&self, section: &str) -> Option<(&str, &str)> {
        let rest = self.key.strip_prefix(section)?.strip_prefix('.')?;
        rest.rsplit_once('.')
    }
}

/// Reads the output of `git config --list --show-scope -z`: for each
/// setting, its scope ended by NUL, then `<key>\n<value>`, or `<key>` alone
/// for a key without a value, ended by NUL.
pub(crate) fn parse_settings(output: &[u8]) -> Vec<Setting> {
    let mut fields = output
        .split(|byte| *byte == 0)
        .map(|field| String::from_utf8_lossy(field).into_owned());
    let mut settings = Vec::new();
    while let (Some(scope), Some(entry)) = (fields.next(), fields.next()) {
        if entry.is_empty() {
            continue;
        }
        let (key, value) = match entry.split_once('\n') {
            Some((key, value)) => (key.to_owned(), Some(value.to_owned())),
            None => (entry, None),
        };
        settings.push(Setting {
            scope: Scope::of(&scope),
            key,
            value,
        });
    }
    settings
}

/// Reads every setting that applies in `folder`. `checks` checks the run's
/// reads, for a mention.
pub(crate) async fn read_settings(
    git: &Git,
    folder: &Path,
    checks: Option<ReadPlace>,
) -> Result<Vec<Setting>, GitFailure> {
    let ran = git
        .output(
            GitRun::new(
                folder,
                [
                    "--no-optional-locks",
                    "config",
                    "--list",
                    "--show-scope",
                    "-z",
                ],
            )
            .checked_as(checks),
        )
        .await?;
    Ok(parse_settings(&ran.output))
}

/// The settings that blank each filter driver whose program the
/// repository's own configuration names, as `-c` arguments for a read:
/// `filter.<driver>.clean=` and the same for `smudge` and `process`. An empty
/// program runs nothing.
pub(crate) fn blank_repository_filters(settings: &[Setting]) -> Vec<String> {
    let mut drivers: Vec<&str> = settings
        .iter()
        .filter(|setting| setting.scope.is_repository())
        .filter_map(|setting| setting.subsection_of("filter"))
        .filter(|(_driver, name)| matches!(*name, "clean" | "smudge" | "process"))
        .map(|(driver, _name)| driver)
        .collect();
    drivers.sort_unstable();
    drivers.dedup();
    let mut arguments = Vec::with_capacity(drivers.len() * 6);
    for driver in drivers {
        for name in ["clean", "smudge", "process"] {
            arguments.push("-c".to_owned());
            arguments.push(format!("filter.{driver}.{name}="));
        }
    }
    arguments
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mannie_git_testing::{HostGit, TestRepository};

    use super::{Scope, Setting, blank_repository_filters, parse_settings, read_settings};
    use crate::git::Git;

    #[tokio::test]
    async fn the_installed_git_s_settings_are_read_with_their_scopes() {
        let repository = TestRepository::new();
        repository.git(&["config", "filter.planted.clean", "run-me %f"]);
        repository.git(&["config", "core.bare", "false"]);
        let git = Git::new(Arc::new(HostGit::new()));

        let settings = read_settings(&git, repository.path(), None).await.unwrap();

        let planted = settings
            .iter()
            .find(|setting| setting.key == "filter.planted.clean")
            .expect("the planted filter is read");
        assert_eq!(planted.scope, Scope::Local);
        assert_eq!(planted.value.as_deref(), Some("run-me %f"));
        assert!(
            settings
                .iter()
                .any(|setting| setting.scope == Scope::Command)
        );
        assert_eq!(
            blank_repository_filters(&settings),
            [
                "-c",
                "filter.planted.clean=",
                "-c",
                "filter.planted.smudge=",
                "-c",
                "filter.planted.process="
            ]
        );
    }

    #[test]
    fn settings_are_read_with_their_scopes_and_values() {
        let output = b"global\0user.name\nJane Doe\0local\0core.bare\nfalse\0local\0filter.my.driver.clean\nrun-me %f\0command\0gc.auto\n0\0worktree\0core.sparsecheckout\0";

        let settings = parse_settings(output);

        assert_eq!(settings.len(), 5);
        assert_eq!(
            settings[0],
            Setting {
                scope: Scope::Global,
                key: "user.name".to_owned(),
                value: Some("Jane Doe".to_owned()),
            }
        );
        assert_eq!(
            settings[2].subsection_of("filter"),
            Some(("my.driver", "clean"))
        );
        assert_eq!(settings[4].value, None);
        assert_eq!(settings[4].scope, Scope::Worktree);
    }

    #[test]
    fn only_the_filters_of_the_repository_s_own_configuration_are_blanked() {
        let settings = parse_settings(
            b"global\0filter.lfs.process\ngit-lfs filter-process\0local\0filter.evil.smudge\nevil\0\
worktree\0filter.evil.clean\nevil\0local\0filter.other.required\ntrue\0",
        );

        assert_eq!(
            blank_repository_filters(&settings),
            [
                "-c",
                "filter.evil.clean=",
                "-c",
                "filter.evil.smudge=",
                "-c",
                "filter.evil.process="
            ]
        );
        assert_eq!(blank_repository_filters(&[]), Vec::<String>::new());
    }
}
