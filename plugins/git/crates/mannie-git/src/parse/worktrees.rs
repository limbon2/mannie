//! The output of `git worktree list --porcelain -z`: for each worktree of
//! the repository, its folder, its commit, its branch and its state, one
//! attribute in each NUL-ended field, with an empty field after each
//! worktree.

use serde::Serialize;

/// A worktree of the repository.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Worktree {
    /// Its folder, as git writes it.
    pub(crate) path: String,
    /// Its commit, or `None` for a bare repository's entry.
    pub(crate) head: Option<String>,
    /// The short name of the branch that it has checked out, or `None` for
    /// a detached HEAD or a bare repository.
    pub(crate) branch: Option<String>,
    /// Whether the entry is the bare repository that the worktrees share.
    pub(crate) bare: bool,
    /// Whether it is locked against removal.
    pub(crate) locked: bool,
    /// Whether git found its folder gone, so it can be pruned.
    pub(crate) prunable: bool,
}

/// Reads the output of `git worktree list --porcelain -z`. An attribute that
/// this version does not know is skipped, since git adds attributes.
pub(crate) fn parse_worktrees(output: &[u8]) -> Vec<Worktree> {
    let mut worktrees = Vec::new();
    let mut current: Option<Worktree> = None;
    for field in output.split(|byte| *byte == 0) {
        let field = String::from_utf8_lossy(field);
        if field.is_empty() {
            worktrees.extend(current.take());
            continue;
        }
        let (name, value) = field.split_once(' ').unwrap_or((&field, ""));
        if name == "worktree" {
            worktrees.extend(current.take());
            current = Some(Worktree {
                path: value.to_owned(),
                ..Worktree::default()
            });
            continue;
        }
        let Some(worktree) = current.as_mut() else {
            continue;
        };
        match name {
            "HEAD" => worktree.head = Some(value.to_owned()),
            "branch" => {
                let branch = value.strip_prefix("refs/heads/").unwrap_or(value);
                worktree.branch = Some(branch.to_owned());
            }
            "bare" => worktree.bare = true,
            "locked" => worktree.locked = true,
            "prunable" => worktree.prunable = true,
            _other => {}
        }
    }
    worktrees.extend(current);
    worktrees
}

#[cfg(test)]
mod tests {
    use super::{Worktree, parse_worktrees};

    #[test]
    fn each_worktree_is_read_with_its_branch_and_state() {
        let output = b"worktree D:/work/app\0HEAD aaaa\0branch refs/heads/main\0\0\
worktree D:/work/app feature\0HEAD bbbb\0detached\0locked reason here\0prunable gitdir file points to non-existent location\0\0";

        let worktrees = parse_worktrees(output);

        assert_eq!(
            worktrees,
            [
                Worktree {
                    path: "D:/work/app".to_owned(),
                    head: Some("aaaa".to_owned()),
                    branch: Some("main".to_owned()),
                    ..Worktree::default()
                },
                Worktree {
                    path: "D:/work/app feature".to_owned(),
                    head: Some("bbbb".to_owned()),
                    branch: None,
                    bare: false,
                    locked: true,
                    prunable: true,
                },
            ]
        );
        assert_eq!(parse_worktrees(b""), Vec::new());
    }
}
