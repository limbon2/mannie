//! Where a repository is (plugins/git/docs/tools.md, accesses; T-48): its
//! work tree's top, its git folder, the git folder that its worktrees share
//! and the object folders that it borrows, as git reports them for a folder
//! inside it. A read names every one of them that lies outside the top, so a
//! git folder elsewhere, such as a linked worktree's, asks like any other
//! folder there.

use std::path::{Path, PathBuf};

use mannie_plugin_sdk::contracts::AccessMode;
use mannie_plugin_sdk::points::tools::{PathAccess, ToolAccess};
use mannie_plugin_sdk::services::processes::ReadPlace;

use crate::failure::GitFailure;
use crate::git::{Git, GitRun};

/// A repository's folders, as absolute paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Layout {
    /// The top of the work tree.
    pub(crate) top: PathBuf,
    /// The repository's git folder, such as `<top>/.git`, or a linked
    /// worktree's folder inside the main git folder.
    pub(crate) git_dir: PathBuf,
    /// The git folder that every worktree of the repository shares.
    pub(crate) common_dir: PathBuf,
}

impl Layout {
    /// Finds the repository that holds `folder`. `checks` checks the runs'
    /// reads, for a mention.
    pub(crate) async fn discover(
        git: &Git,
        folder: &Path,
        checks: Option<ReadPlace>,
    ) -> Result<Self, GitFailure> {
        let not_a_repository = || GitFailure::NotARepository {
            folder: folder.display().to_string(),
        };
        if !folder.is_dir() {
            return Err(not_a_repository());
        }
        let folders = git
            .run(
                GitRun::new(
                    folder,
                    [
                        "rev-parse",
                        "--path-format=absolute",
                        "--absolute-git-dir",
                        "--git-common-dir",
                        "--is-bare-repository",
                    ],
                )
                .checked_as(checks.clone()),
            )
            .await?;
        if !folders.succeeded() {
            return Err(match folders.failure(folder) {
                GitFailure::Failed { .. } => not_a_repository(),
                other => other,
            });
        }
        let text = folders.text();
        let mut lines = text.lines();
        let (Some(git_dir), Some(common_dir), Some(bare)) =
            (lines.next(), lines.next(), lines.next())
        else {
            return Err(not_a_repository());
        };
        if bare.trim() == "true" {
            return Err(GitFailure::Bare {
                folder: folder.display().to_string(),
            });
        }
        let top = git
            .output(
                GitRun::new(
                    folder,
                    ["rev-parse", "--path-format=absolute", "--show-toplevel"],
                )
                .checked_as(checks),
            )
            .await?;
        let top = top.text();
        Ok(Self {
            top: PathBuf::from(top.trim_end_matches(['\r', '\n'])),
            git_dir: PathBuf::from(git_dir),
            common_dir: PathBuf::from(common_dir),
        })
    }

    /// The object folders that the repository borrows from others, which
    /// `objects/info/alternates` lists, one for each line, relative to the
    /// repository's object folder or absolute.
    pub(crate) fn alternates(&self) -> Vec<PathBuf> {
        let objects = self.common_dir.join("objects");
        let Ok(text) = std::fs::read_to_string(objects.join("info").join("alternates")) else {
            return Vec::new();
        };
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                let path = Path::new(line);
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    objects.join(path)
                }
            })
            .collect()
    }

    /// The folders that a run in the repository reads: the top, and each git
    /// folder and borrowed object folder that lies outside it.
    pub(crate) fn read_folders(&self) -> Vec<PathBuf> {
        let mut folders = vec![self.top.clone()];
        // The shared folder first, since a linked worktree's own git folder
        // lies inside it.
        let others = [self.common_dir.clone(), self.git_dir.clone()]
            .into_iter()
            .chain(self.alternates());
        for folder in others {
            let is_covered = folders.iter().any(|known| is_inside(&folder, known));
            if !is_covered {
                folders.push(folder);
            }
        }
        folders
    }

    /// The accesses of a read of the repository.
    pub(crate) fn read_accesses(&self) -> Vec<ToolAccess> {
        self.read_folders()
            .into_iter()
            .map(|folder| ToolAccess::Path(PathAccess::new(folder, AccessMode::Read)))
            .collect()
    }
}

/// Whether `path` is `folder` or lies below it, comparing names without
/// regard to case on Windows, as its file systems do.
pub(crate) fn is_inside(path: &Path, folder: &Path) -> bool {
    let mut components = path.components();
    folder.components().all(|folder_component| {
        components.next().is_some_and(|component| {
            let left = component.as_os_str().to_string_lossy();
            let right = folder_component.as_os_str().to_string_lossy();
            if cfg!(windows) {
                left.replace('\\', "/")
                    .eq_ignore_ascii_case(&right.replace('\\', "/"))
            } else {
                left == right
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Layout, is_inside};

    fn at(path: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!("D:{}", path.replace('/', "\\")))
        } else {
            PathBuf::from(path)
        }
    }

    #[test]
    fn a_read_names_the_top_and_each_git_folder_outside_it() {
        let inside = Layout {
            top: at("/work/app"),
            git_dir: at("/work/app/.git"),
            common_dir: at("/work/app/.git"),
        };
        let worktree = Layout {
            top: at("/work/app-feature"),
            git_dir: at("/work/app/.git/worktrees/app-feature"),
            common_dir: at("/work/app/.git"),
        };

        assert_eq!(inside.read_folders(), [at("/work/app")]);
        assert_eq!(
            worktree.read_folders(),
            [at("/work/app-feature"), at("/work/app/.git")]
        );
    }

    #[test]
    fn inside_compares_whole_names() {
        assert!(is_inside(&at("/work/app/src"), &at("/work/app")));
        assert!(is_inside(&at("/work/app"), &at("/work/app")));
        assert!(!is_inside(&at("/work/application"), &at("/work/app")));
        assert!(!is_inside(&at("/work"), &at("/work/app")));
    }
}
