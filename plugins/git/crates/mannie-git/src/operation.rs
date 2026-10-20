//! The operation in progress in a repository (plugins/git/docs/
//! conflicts-and-history.md, operations that can stop): a merge, a rebase, a
//! cherry-pick or a revert that stopped, as git's state files in the git
//! folder record it. The state stays git's own, so the owner's git and IDE
//! see the same operation.

use std::fs;
use std::path::Path;

use serde::Serialize;

/// What kind of operation stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OperationKind {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

impl OperationKind {
    /// The operation's name in a sentence, such as `cherry-pick`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebase => "rebase",
            Self::CherryPick => "cherry-pick",
            Self::Revert => "revert",
        }
    }
}

/// An operation in progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Operation {
    pub(crate) kind: OperationKind,
    /// The step that stopped and the steps in all, for a rebase.
    pub(crate) step: Option<(u32, u32)>,
    /// The commit that the operation applies now: the merged commit, the
    /// picked or reverted commit, or the rebase's current commit.
    pub(crate) incoming: Option<String>,
    /// The branch that a rebase rewrites, such as `refs/heads/feature`.
    pub(crate) branch: Option<String>,
    /// The commit that a rebase replays onto.
    pub(crate) onto: Option<String>,
}

/// The first line of a state file, or `None` when it does not exist.
fn first_line(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_owned())
}

/// A number that a state file holds.
fn number(path: &Path) -> Option<u32> {
    first_line(path)?.parse().ok()
}

impl Operation {
    /// The operation in progress in the repository whose git folder is
    /// `git_dir`, or `None`.
    pub(crate) fn read(git_dir: &Path) -> Option<Self> {
        let rebase_merge = git_dir.join("rebase-merge");
        let rebase_apply = git_dir.join("rebase-apply");
        if rebase_merge.is_dir() {
            let step = number(&rebase_merge.join("msgnum")).zip(number(&rebase_merge.join("end")));
            return Some(Self {
                kind: OperationKind::Rebase,
                step,
                incoming: first_line(&rebase_merge.join("stopped-sha")),
                branch: first_line(&rebase_merge.join("head-name")),
                onto: first_line(&rebase_merge.join("onto")),
            });
        }
        if rebase_apply.is_dir() && !rebase_apply.join("applying").exists() {
            let step = number(&rebase_apply.join("next")).zip(number(&rebase_apply.join("last")));
            return Some(Self {
                kind: OperationKind::Rebase,
                step,
                incoming: first_line(&rebase_apply.join("original-commit")),
                branch: first_line(&rebase_apply.join("head-name")),
                onto: first_line(&rebase_apply.join("onto")),
            });
        }
        let simple = [
            ("MERGE_HEAD", OperationKind::Merge),
            ("CHERRY_PICK_HEAD", OperationKind::CherryPick),
            ("REVERT_HEAD", OperationKind::Revert),
        ];
        simple.into_iter().find_map(|(file, kind)| {
            let incoming = first_line(&git_dir.join(file))?;
            Some(Self {
                kind,
                step: None,
                incoming: Some(incoming),
                branch: None,
                onto: None,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{Operation, OperationKind};

    #[test]
    fn a_rebase_reads_its_step_its_commit_and_its_branch() {
        let git_dir = tempfile_dir();
        let state = git_dir.path().join("rebase-merge");
        fs::create_dir_all(&state).unwrap();
        for (file, text) in [
            ("msgnum", "3\n"),
            ("end", "7\n"),
            ("stopped-sha", "1a2b3c4\n"),
            ("head-name", "refs/heads/feature\n"),
            ("onto", "aaaa\n"),
        ] {
            fs::write(state.join(file), text).unwrap();
        }

        let operation = Operation::read(git_dir.path()).unwrap();

        assert_eq!(
            operation,
            Operation {
                kind: OperationKind::Rebase,
                step: Some((3, 7)),
                incoming: Some("1a2b3c4".to_owned()),
                branch: Some("refs/heads/feature".to_owned()),
                onto: Some("aaaa".to_owned()),
            }
        );
    }

    #[test]
    fn a_merge_a_pick_and_a_revert_name_their_incoming_commit() {
        for (file, kind) in [
            ("MERGE_HEAD", OperationKind::Merge),
            ("CHERRY_PICK_HEAD", OperationKind::CherryPick),
            ("REVERT_HEAD", OperationKind::Revert),
        ] {
            let git_dir = tempfile_dir();
            fs::write(git_dir.path().join(file), "bbbb\n").unwrap();

            let operation = Operation::read(git_dir.path()).unwrap();

            assert_eq!(operation.kind, kind);
            assert_eq!(operation.incoming.as_deref(), Some("bbbb"));
        }
        assert_eq!(Operation::read(tempfile_dir().path()), None);
    }

    fn tempfile_dir() -> mannie_git_testing::TempFolder {
        mannie_git_testing::TempFolder::new()
    }
}
