//! The handlers of the Git side panel's commands and channel
//! (plugins/git/docs/panel.md). Each reads the repository of a project,
//! with every run of git checked first as a read of the project's
//! conversations (ADR-0038, decision 10), with the same readers as the
//! tools, so the panel and the cards show the same things.

use futures_util::{StreamExt as _, stream};
use mannie_plugin_sdk::contracts::ProjectId;
use mannie_plugin_sdk::ipc::{Handlers, PluginStream};
use mannie_plugin_sdk::services::processes::{ProcessError, ReadPlace};
use tokio::sync::broadcast;

use crate::commands::{
    BranchList, CommitRef, CommitView, ConflictFileView, ConflictsView, FileDiff, FileRef,
    FollowRepositoryChannel, GitError, HISTORY_PAGE, History, HistoryRef, MAX_LISTED, ProjectRef,
    RepositoryBranchesCommand, RepositoryCommitCommand, RepositoryConflictsCommand,
    RepositoryFileDiffCommand, RepositoryHistoryCommand, RepositoryOverview,
    RepositoryOverviewCommand, RepositoryUpdate,
};
use crate::describe::{label_commits, operation_view};
use crate::git::Git;
use crate::layout::Layout;
use crate::outcome::head_commit;
use crate::parse::conflicts::{hunks, parse_markers};
use crate::parse::diff::{parse_hunks, parse_numstat};
use crate::parse::log::{LOG_FORMAT, parse_log};
use crate::parse::refs::{REF_FORMAT, RefKind, parse_refs};
use crate::tools::Reading;
use crate::tools::conflicts::conflicted_files;
use crate::tools::diff::PATCH_OPTIONS;
use crate::tools::show::shown_commit;
use crate::tools::status::read_status;

/// The room of a command's reads, which no model reads.
const ROOM: usize = 256 * 1024;

/// The most files of a commit whose patches a command returns.
const MAX_PATCHED_FILES: usize = 200;

/// The plugin's notes of its changes, which the channel carries.
#[derive(Debug, Clone)]
pub(crate) struct Changes {
    sender: broadcast::Sender<RepositoryUpdate>,
}

impl Changes {
    /// The notes that a start keeps for a channel that falls behind.
    const CAPACITY: usize = 16;

    pub(crate) fn new() -> Self {
        let (sender, _receiver) = broadcast::channel(Self::CAPACITY);
        Self { sender }
    }

    /// Tells the panels that a tool changed a repository.
    pub(crate) fn notify(&self) {
        // Without an open panel, nobody listens, which is no failure.
        if self.sender.send(RepositoryUpdate::Changed).is_err() {
            tracing::debug!("no panel follows the repositories");
        }
    }

    fn follow(&self) -> PluginStream<RepositoryUpdate> {
        stream::unfold(self.sender.subscribe(), |mut receiver| async move {
            match receiver.recv().await {
                Ok(update) => Some((update, receiver)),
                // A panel that fell behind reads once anyway.
                Err(broadcast::error::RecvError::Lagged(_missed)) => {
                    Some((RepositoryUpdate::Changed, receiver))
                }
                Err(broadcast::error::RecvError::Closed) => None,
            }
        })
        .boxed()
    }
}

/// The repository of a project, every run checked as the project's reads.
async fn project_reading(git: &Git, project: &ProjectId) -> Result<Reading, GitError> {
    let folder = git
        .project_folder(project.clone())
        .await
        .map_err(|error| match error {
            ProcessError::ProjectNotFound => GitError::ProjectNotFound,
            other => {
                tracing::warn!(error = %other, "the git panel could not find a project's folder");
                GitError::GitFailed
            }
        })?;
    let place = |folders| ReadPlace::Project {
        project: project.clone(),
        folders,
    };
    let layout = Layout::discover(git, &folder, Some(place(vec![folder.clone()]))).await?;
    let checks = place(layout.read_folders());
    Ok(Reading::open(git, layout, ROOM, Some(checks)).await?)
}

async fn overview(git: Git, input: ProjectRef) -> Result<RepositoryOverview, GitError> {
    let reading = project_reading(&git, &input.project_id).await?;
    let status = read_status(&reading).await?;
    let head = match &status.branch.oid {
        Some(oid) => label_commits(&reading, std::slice::from_ref(oid))
            .await?
            .into_iter()
            .next(),
        None => None,
    };
    let operation = operation_view(&reading).await?;
    let total = u32::try_from(status.entries.len()).unwrap_or(u32::MAX);
    Ok(RepositoryOverview {
        top: reading.layout.top.display().to_string(),
        branch: status.branch,
        head,
        operation,
        entries: status.entries.into_iter().take(MAX_LISTED).collect(),
        entries_total: total,
        stash: status.stash,
    })
}

async fn history(git: Git, input: HistoryRef) -> Result<History, GitError> {
    let reading = project_reading(&git, &input.project_id).await?;
    if head_commit(&reading).await?.is_none() {
        return Ok(History {
            commits: Vec::new(),
            more: false,
        });
    }
    let ran = reading
        .git
        .output(reading.run([
            "log".to_owned(),
            "-z".to_owned(),
            LOG_FORMAT.to_owned(),
            format!("--max-count={}", HISTORY_PAGE + 1),
            format!("--skip={}", input.skip),
            "HEAD".to_owned(),
        ]))
        .await?;
    let mut commits = parse_log(&ran.output).map_err(|_error| GitError::GitFailed)?;
    let page = usize::try_from(HISTORY_PAGE).unwrap_or(usize::MAX);
    let more = commits.len() > page;
    commits.truncate(page);
    Ok(History { commits, more })
}

async fn branches(git: Git, input: ProjectRef) -> Result<BranchList, GitError> {
    let reading = project_reading(&git, &input.project_id).await?;
    let ran = reading
        .git
        .output(reading.run([
            "for-each-ref",
            REF_FORMAT,
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ]))
        .await?;
    let refs = parse_refs(&ran.text());
    let of_kind = |kind: RefKind| {
        refs.iter()
            .filter(|found| found.kind == kind)
            .take(MAX_LISTED)
            .cloned()
            .collect()
    };
    Ok(BranchList {
        branches: of_kind(RefKind::Branch),
        remote_branches: of_kind(RefKind::RemoteBranch),
        tags: of_kind(RefKind::Tag),
    })
}

async fn commit(git: Git, input: CommitRef) -> Result<CommitView, GitError> {
    let reading = project_reading(&git, &input.project_id).await?;
    let oid = reading.commit("commit", &input.commit).await?;
    let shown = shown_commit(&reading, &oid, true).await?;
    Ok(CommitView {
        commit: shown.details,
        files: shown.files,
        patches: shown.patches.into_iter().take(MAX_PATCHED_FILES).collect(),
    })
}

async fn file_diff(git: Git, input: FileRef) -> Result<FileDiff, GitError> {
    let reading = project_reading(&git, &input.project_id).await?;
    let mut command = vec!["diff".to_owned()];
    if input.staged {
        command.push("--cached".to_owned());
    }
    command.extend(PATCH_OPTIONS.iter().map(|option| (*option).to_owned()));
    command.push("--ignore-submodules=dirty".to_owned());
    let pathspec = ["--".to_owned(), crate::arguments::pathspec(&input.path)];
    let stat = reading
        .git
        .output(
            reading.run(
                [
                    command.clone(),
                    vec!["--numstat".to_owned(), "-z".to_owned()],
                    pathspec.to_vec(),
                ]
                .concat(),
            ),
        )
        .await?;
    let patch = reading
        .git
        .output(reading.run([command, vec!["--patch".to_owned()], pathspec.to_vec()].concat()))
        .await?;
    Ok(FileDiff {
        stat: parse_numstat(&stat.output).into_iter().next(),
        hunks: parse_hunks(&patch.text()),
    })
}

async fn conflicts(git: Git, input: ProjectRef) -> Result<ConflictsView, GitError> {
    let reading = project_reading(&git, &input.project_id).await?;
    let operation = operation_view(&reading).await?;
    let head = match head_commit(&reading).await? {
        Some(head) => label_commits(&reading, &[head]).await?.into_iter().next(),
        None => None,
    };
    let mut files = Vec::new();
    for file in conflicted_files(&reading)
        .await?
        .into_iter()
        .take(MAX_LISTED)
    {
        let text = std::fs::read(reading.layout.top.join(&file.path))
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        let file_hunks = text
            .as_deref()
            .and_then(|text| parse_markers(text).ok())
            .map(|segments| hunks(&segments).into_iter().cloned().collect())
            .unwrap_or_default();
        files.push(ConflictFileView {
            file,
            hunks: file_hunks,
        });
    }
    Ok(ConflictsView {
        operation,
        head,
        files,
    })
}

/// The handlers of a start's commands and channel.
pub(crate) fn handlers(git: &Git, changes: &Changes) -> Handlers {
    let for_overview = git.clone();
    let for_history = git.clone();
    let for_branches = git.clone();
    let for_commit = git.clone();
    let for_diff = git.clone();
    let for_conflicts = git.clone();
    let following = changes.clone();
    Handlers::new()
        .command::<RepositoryOverviewCommand, _, _>(move |input| {
            overview(for_overview.clone(), input)
        })
        .command::<RepositoryHistoryCommand, _, _>(move |input| history(for_history.clone(), input))
        .command::<RepositoryBranchesCommand, _, _>(move |input| {
            branches(for_branches.clone(), input)
        })
        .command::<RepositoryCommitCommand, _, _>(move |input| commit(for_commit.clone(), input))
        .command::<RepositoryFileDiffCommand, _, _>(move |input| file_diff(for_diff.clone(), input))
        .command::<RepositoryConflictsCommand, _, _>(move |input| {
            conflicts(for_conflicts.clone(), input)
        })
        .channel::<FollowRepositoryChannel, _, _>(move |()| {
            std::future::ready(Ok::<_, GitError>(following.follow()))
        })
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use futures_util::StreamExt as _;
    use mannie_git_testing::{HostGit, TestRepository};
    use mannie_plugin_sdk::contracts::ProjectId;

    use super::{Changes, branches, commit, conflicts, file_diff, history, overview};
    use crate::commands::{
        CommitRef, FileRef, GitError, HISTORY_PAGE, HistoryRef, ProjectRef, RepositoryUpdate,
    };
    use crate::git::Git;
    use crate::operation::OperationKind;
    use crate::parse::diff::{Hunk, LineKind};
    use crate::parse::log::Commit;
    use crate::parse::refs::Tracking;

    fn project() -> ProjectId {
        ProjectId::parse("0199e0f2-0000-7000-8000-000000000005").unwrap()
    }

    fn of_project() -> ProjectRef {
        ProjectRef {
            project_id: project(),
        }
    }

    /// git for a core that finds the project in `folder`.
    fn git_for(folder: &Path) -> Git {
        Git::new(Arc::new(HostGit::new().with_project(project(), folder)))
    }

    fn started() -> TestRepository {
        let repository = TestRepository::new();
        repository.write("a.txt", "one\n");
        repository.commit("Add a");
        repository
    }

    #[tokio::test]
    async fn the_overview_names_the_branch_its_head_and_the_changed_files() {
        let repository = started();
        let head = repository.head();
        repository.write("a.txt", "two\n");
        repository.write("b.txt", "new\n");

        let shown = overview(git_for(repository.path()), of_project())
            .await
            .unwrap();

        assert_eq!(shown.branch.head.as_deref(), Some("main"));
        assert_eq!(shown.head.map(|label| label.oid), Some(head));
        let paths: Vec<&str> = shown
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect();
        assert_eq!(paths, ["a.txt", "b.txt"]);
        assert_eq!(shown.entries_total, 2);
        assert_eq!(shown.operation, None);
    }

    #[tokio::test]
    async fn the_commands_name_a_missing_project_a_plain_folder_and_a_read_that_would_ask() {
        let repository = started();
        let plain = repository.sibling("plain");
        let refusing = Git::new(Arc::new(
            HostGit::refusing_reads().with_project(project(), repository.path()),
        ));

        let missing = overview(Git::new(Arc::new(HostGit::new())), of_project()).await;
        let outside = overview(git_for(&plain), of_project()).await;
        let asks = branches(refusing, of_project()).await;

        assert_eq!(missing.err(), Some(GitError::ProjectNotFound));
        assert_eq!(outside.err(), Some(GitError::NotARepository));
        assert_eq!(asks.err(), Some(GitError::ReadNotAllowed));
    }

    #[tokio::test]
    async fn the_history_reads_pages_of_the_current_branch() {
        let unborn = TestRepository::new();
        let empty = history(
            git_for(unborn.path()),
            HistoryRef {
                project_id: project(),
                skip: 0,
            },
        )
        .await
        .unwrap();
        let repository = started();
        for _commit in 0..HISTORY_PAGE {
            repository.git(&["commit", "--quiet", "--allow-empty", "-m", "Empty"]);
        }
        let git = git_for(repository.path());

        let first = history(
            git.clone(),
            HistoryRef {
                project_id: project(),
                skip: 0,
            },
        )
        .await
        .unwrap();
        let last = history(
            git,
            HistoryRef {
                project_id: project(),
                skip: HISTORY_PAGE,
            },
        )
        .await
        .unwrap();

        assert_eq!(empty.commits, Vec::<Commit>::new());
        assert!(!empty.more);
        assert_eq!(first.commits.len(), usize::try_from(HISTORY_PAGE).unwrap());
        assert!(first.more);
        let subjects: Vec<&str> = last
            .commits
            .iter()
            .map(|commit| commit.subject.as_str())
            .collect();
        assert_eq!(subjects, ["Add a"]);
        assert!(!last.more);
    }

    #[tokio::test]
    async fn the_branches_list_their_upstreams_the_remote_branches_and_the_tags() {
        let repository = started();
        let remote = repository.sibling("remote.git");
        repository.git_in(
            &remote,
            &["init", "--quiet", "--bare", "--initial-branch=main"],
        );
        repository.git(&["remote", "add", "origin", remote.to_str().unwrap()]);
        repository.git(&["push", "--quiet", "--set-upstream", "origin", "main"]);
        repository.write("a.txt", "two\n");
        repository.commit("Change a");
        repository.git(&["branch", "feature"]);
        repository.git(&["tag", "v1.0"]);

        let listed = branches(git_for(repository.path()), of_project())
            .await
            .unwrap();

        let names: Vec<&str> = listed
            .branches
            .iter()
            .map(|found| found.name.as_str())
            .collect();
        assert_eq!(names, ["feature", "main"]);
        let main = &listed.branches[1];
        assert!(main.current);
        assert_eq!(main.upstream.as_deref(), Some("origin/main"));
        assert_eq!(
            main.tracking,
            Some(Tracking::Exists {
                ahead: 1,
                behind: 0
            })
        );
        let remote_names: Vec<&str> = listed
            .remote_branches
            .iter()
            .map(|found| found.name.as_str())
            .collect();
        assert_eq!(remote_names, ["origin/main"]);
        assert_eq!(listed.tags.len(), 1);
    }

    #[tokio::test]
    async fn a_commit_shows_its_message_files_and_patches_and_an_unknown_one_fails() {
        let repository = started();
        repository.write("a.txt", "two\n");
        let head = repository.commit("Change a\n\nWith a body.");
        let git = git_for(repository.path());

        let shown = commit(
            git.clone(),
            CommitRef {
                project_id: project(),
                commit: head[..7].to_owned(),
            },
        )
        .await
        .unwrap();
        let unknown = commit(
            git,
            CommitRef {
                project_id: project(),
                commit: "0000000".to_owned(),
            },
        )
        .await;

        assert_eq!(shown.commit.oid, head);
        assert!(
            shown.commit.message.contains("With a body."),
            "{}",
            shown.commit.message
        );
        assert_eq!(shown.files.len(), 1);
        assert_eq!(
            (shown.files[0].added, shown.files[0].removed),
            (Some(1), Some(1))
        );
        assert_eq!(shown.patches[0].path, "a.txt");
        assert_eq!(unknown.err(), Some(GitError::UnknownCommit));
    }

    #[tokio::test]
    async fn a_file_s_diff_is_its_staged_or_unstaged_change() {
        let repository = started();
        repository.write("a.txt", "two\n");
        repository.git(&["add", "a.txt"]);
        repository.write("a.txt", "three\n");
        let git = git_for(repository.path());
        let diff = |path: &str, staged: bool| {
            file_diff(
                git.clone(),
                FileRef {
                    project_id: project(),
                    path: path.to_owned(),
                    staged,
                },
            )
        };

        let staged = diff("a.txt", true).await.unwrap();
        let unstaged = diff("a.txt", false).await.unwrap();
        let unchanged = diff("b.txt", false).await.unwrap();

        let added = |hunks: &[Hunk]| -> Vec<String> {
            hunks
                .iter()
                .flat_map(|hunk| &hunk.lines)
                .filter(|line| line.kind == LineKind::Added)
                .map(|line| line.text.clone())
                .collect()
        };
        assert_eq!(added(&staged.hunks), ["two"]);
        assert_eq!(added(&unstaged.hunks), ["three"]);
        assert_eq!(staged.stat.map(|stat| stat.path), Some("a.txt".to_owned()));
        assert!(unchanged.stat.is_none());
        assert_eq!(unchanged.hunks, Vec::<Hunk>::new());
    }

    #[tokio::test]
    async fn the_conflicts_name_the_operation_and_each_file_s_hunks() {
        let repository = started();
        repository.git(&["switch", "--quiet", "-c", "feature"]);
        repository.write("a.txt", "incoming\n");
        repository.commit("Incoming");
        repository.git(&["switch", "--quiet", "main"]);
        repository.write("a.txt", "current\n");
        repository.commit("Current");
        assert!(!repository.git_may_fail(&["merge", "--quiet", "feature"]));

        let shown = conflicts(git_for(repository.path()), of_project())
            .await
            .unwrap();

        assert_eq!(
            shown.operation.map(|operation| operation.kind),
            Some(OperationKind::Merge)
        );
        assert_eq!(shown.files.len(), 1);
        let file = &shown.files[0];
        assert_eq!(file.file.path, "a.txt");
        assert_eq!(file.hunks.len(), 1);
        assert!(file.hunks[0].current.concat().contains("current"));
        assert!(file.hunks[0].incoming.concat().contains("incoming"));
    }

    #[tokio::test]
    async fn the_channel_carries_each_change_and_one_for_a_panel_that_fell_behind() {
        let changes = Changes::new();
        changes.notify();
        let mut following = changes.follow();
        let mut behind = changes.follow();

        changes.notify();
        let next = following.next().await;
        for _note in 0..Changes::CAPACITY * 2 {
            changes.notify();
        }
        let caught_up = behind.next().await;

        assert_eq!(next, Some(RepositoryUpdate::Changed));
        assert_eq!(caught_up, Some(RepositoryUpdate::Changed));
    }
}
