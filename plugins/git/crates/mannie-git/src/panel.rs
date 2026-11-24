//! The handlers of the Git side panel's commands and channel
//! (plugins/git/docs/panel.md). Read commands check every run of git as a
//! read of the project's conversations (ADR-0038, decision 10). Owner action
//! commands run under `RepositoryLocks` and the core's journaled
//! owner-operation boundary (ADR-0078).

mod actions;
mod queries;

use futures_util::{StreamExt as _, stream};
use mannie_plugin_sdk::contracts::ProjectId;
use mannie_plugin_sdk::ipc::{Handlers, PluginStream};
use mannie_plugin_sdk::services::processes::{ProcessError, ReadPlace};
use tokio::sync::broadcast;

use crate::changes::RepositoryLocks;
use crate::commands::{
    BranchList, CommitRef, CommitView, ConflictFileView, ConflictsView, FileDiff, FileRef,
    FollowRepositoryChannel, GitError, HISTORY_PAGE, History, HistoryRef, MAX_LISTED, ProjectRef,
    RepositoryActionPreviewCommand, RepositoryActionRunCommand, RepositoryBlameCommand,
    RepositoryBranchesCommand, RepositoryCommitCommand, RepositoryCompareCommand,
    RepositoryConflictsCommand, RepositoryFileDiffCommand, RepositoryGrepCommand,
    RepositoryHistoryCommand, RepositoryHistoryQueryCommand, RepositoryOverview,
    RepositoryOverviewCommand, RepositoryRefsCommand, RepositoryStashShowCommand,
    RepositoryTreeCommand, RepositoryUpdate,
};
use crate::describe::{label_commits, operation_view};
use crate::git::Git;
use crate::layout::Layout;
use crate::outcome::head_commit;
use crate::parse::conflicts::{hunks, parse_markers};
use crate::parse::diff::{FileStat, parse_hunks, parse_numstat};
use crate::parse::log::{LOG_FORMAT, parse_log};
use crate::parse::refs::{REF_FORMAT, RefKind, parse_refs};
use crate::tools::Reading;
use crate::tools::conflicts::conflicted_files;
use crate::tools::diff::PATCH_OPTIONS;
use crate::tools::push::effective_push_target;
use crate::tools::show::shown_commit;
use crate::tools::status::read_status_all;

/// The room of a command's reads, which no model reads.
pub(crate) const ROOM: usize = 256 * 1024;

/// The most files of a commit or diff whose patches a command returns.
pub(crate) const MAX_PATCHED_FILES: usize = 200;

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

    /// Tells the panels that a tool or owner action changed a repository.
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
pub(crate) async fn project_reading(git: &Git, project: &ProjectId) -> Result<Reading, GitError> {
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
    let status = read_status_all(&reading).await?;
    let head = match &status.branch.oid {
        Some(oid) => label_commits(&reading, std::slice::from_ref(oid))
            .await?
            .into_iter()
            .next(),
        None => None,
    };
    let operation = operation_view(&reading).await?;
    let staged_stats = queries::diff_stats(&reading, true).await?;
    let unstaged_stats = queries::diff_stats(&reading, false).await?;
    let remotes = queries::read_remotes(&reading).await?;
    let push_target = match status.branch.head.as_deref() {
        Some(branch) => effective_push_target(&reading, branch).await,
        None => None,
    };
    let total = u32::try_from(status.entries.len()).unwrap_or(u32::MAX);
    Ok(RepositoryOverview {
        top: reading.layout.top.display().to_string(),
        branch: status.branch,
        head,
        operation,
        entries: status.entries.into_iter().take(MAX_LISTED).collect(),
        entries_total: total,
        stash: status.stash,
        staged_stats,
        unstaged_stats,
        remotes,
        push_target,
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
            "--no-ext-diff".to_owned(),
            "--no-textconv".to_owned(),
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
    if !input.staged
        && let Some(untracked) = queries::untracked_file_diff(&reading, &input.path).await?
    {
        return Ok(untracked);
    }

    let mut command = vec!["diff".to_owned()];
    if input.staged {
        command.push("--cached".to_owned());
    }
    command.extend(PATCH_OPTIONS.iter().map(|option| (*option).to_owned()));
    command.push("--ignore-submodules=dirty".to_owned());
    if input.ignore_whitespace {
        command.push("-w".to_owned());
    }
    if let Some(context) = input.context_lines {
        command.push(format!("--unified={context}"));
    }
    let pathspec = ["--".to_owned(), crate::arguments::pathspec(&input.path)];
    let stat_ran = reading
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
    let patch_ran = reading
        .git
        .output(reading.run([command, vec!["--patch".to_owned()], pathspec.to_vec()].concat()))
        .await?;
    let stat = parse_numstat(&stat_ran.output).into_iter().next();
    let binary = stat.as_ref().is_some_and(FileStat::is_binary);
    Ok(FileDiff {
        stat,
        hunks: parse_hunks(&patch_ran.text()),
        untracked: false,
        binary,
        truncated: patch_ran.cut,
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
        let text = queries::safe_worktree_regular_file(&reading.layout.top, &file.path)
            .ok()
            .flatten()
            .and_then(|canonical| std::fs::read(canonical).ok())
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
pub(crate) fn handlers(git: &Git, locks: &RepositoryLocks, changes: &Changes) -> Handlers {
    let for_overview = git.clone();
    let for_history = git.clone();
    let for_history_query = git.clone();
    let for_branches = git.clone();
    let for_refs = git.clone();
    let for_commit = git.clone();
    let for_diff = git.clone();
    let for_conflicts = git.clone();
    let for_stash_show = git.clone();
    let for_compare = git.clone();
    let for_blame = git.clone();
    let for_tree = git.clone();
    let for_grep = git.clone();
    let for_preview = git.clone();
    let for_run_git = git.clone();
    let for_run_locks = locks.clone();
    let for_run_changes = changes.clone();
    let following = changes.clone();

    Handlers::new()
        .command::<RepositoryOverviewCommand, _, _>(move |input| {
            overview(for_overview.clone(), input)
        })
        .command::<RepositoryHistoryCommand, _, _>(move |input| history(for_history.clone(), input))
        .command::<RepositoryHistoryQueryCommand, _, _>(move |input| {
            queries::history_query(for_history_query.clone(), input)
        })
        .command::<RepositoryBranchesCommand, _, _>(move |input| {
            branches(for_branches.clone(), input)
        })
        .command::<RepositoryRefsCommand, _, _>(move |input| queries::refs(for_refs.clone(), input))
        .command::<RepositoryCommitCommand, _, _>(move |input| commit(for_commit.clone(), input))
        .command::<RepositoryFileDiffCommand, _, _>(move |input| file_diff(for_diff.clone(), input))
        .command::<RepositoryConflictsCommand, _, _>(move |input| {
            conflicts(for_conflicts.clone(), input)
        })
        .command::<RepositoryStashShowCommand, _, _>(move |input| {
            queries::stash_show(for_stash_show.clone(), input)
        })
        .command::<RepositoryCompareCommand, _, _>(move |input| {
            queries::compare(for_compare.clone(), input)
        })
        .command::<RepositoryBlameCommand, _, _>(move |input| {
            queries::blame(for_blame.clone(), input)
        })
        .command::<RepositoryTreeCommand, _, _>(move |input| queries::tree(for_tree.clone(), input))
        .command::<RepositoryGrepCommand, _, _>(move |input| queries::grep(for_grep.clone(), input))
        .command::<RepositoryActionPreviewCommand, _, _>(move |input| {
            actions::action_preview(for_preview.clone(), input)
        })
        .command::<RepositoryActionRunCommand, _, _>(move |input| {
            actions::action_run(
                for_run_git.clone(),
                for_run_locks.clone(),
                for_run_changes.clone(),
                input,
            )
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
    use mannie_plugin_sdk::contracts::{ProgramEffect, ProgramOperationId, ProjectId};
    use mannie_plugin_sdk::services::processes::{OwnerOperationOutcome, OwnerOperationRequest};

    use super::{
        Changes, actions, branches, commit, conflicts, file_diff, history, overview, queries,
    };
    use crate::changes::RepositoryLocks;
    use crate::commands::{
        ActionConfirmation, ActionPreview, ActionPreviewRef, ActionRunRef, BlameRef, CommitRef,
        CompareMode, CompareRef, FileRef, GitError, GrepQueryRef, HISTORY_PAGE, HistoryQueryRef,
        HistoryRef, HistoryScope, MergeFilter, OwnerActionEffect, OwnerActionResult,
        OwnerGitAction, ProjectRef, RepositoryUpdate, StashShowRef, TreeQueryRef,
    };
    use crate::git::Git;
    use crate::operation::OperationKind;
    use crate::parse::diff::{Hunk, LineKind};
    use crate::parse::log::Commit;
    use crate::parse::refs::Tracking;
    use crate::tools::operation::OperationAction;
    use crate::tools::resolve::Side;

    fn project() -> ProjectId {
        ProjectId::parse("0199e0f2-0000-7000-8000-000000000005").unwrap()
    }

    fn op_id(suffix: u32) -> ProgramOperationId {
        ProgramOperationId::parse(&format!("0199e0f2-0000-7000-8000-{suffix:012}")).unwrap()
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
        repository.write("nested/b.txt", "new\n");

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
        assert_eq!(paths, ["a.txt", "nested/b.txt"]);
        assert_eq!(shown.entries_total, 2);
        assert_eq!(shown.operation, None);
        assert_eq!(shown.unstaged_stats.len(), 1);
        assert_eq!(shown.unstaged_stats[0].path, "a.txt");
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
    async fn a_file_s_diff_handles_staged_unstaged_untracked_and_binary_files() {
        let repository = started();
        repository.write("a.txt", "two\n");
        repository.git(&["add", "a.txt"]);
        repository.write("a.txt", "three\n");
        repository.write("new.txt", "alpha\nbeta\n");
        std::fs::write(repository.path().join("bin.dat"), b"\0\x01\x02binary").unwrap();

        let git = git_for(repository.path());
        let diff = |path: &str, staged: bool| {
            file_diff(
                git.clone(),
                FileRef {
                    project_id: project(),
                    path: path.to_owned(),
                    staged,
                    ignore_whitespace: false,
                    context_lines: None,
                },
            )
        };

        let staged = diff("a.txt", true).await.unwrap();
        let unstaged = diff("a.txt", false).await.unwrap();
        let untracked = diff("new.txt", false).await.unwrap();
        let binary = diff("bin.dat", false).await.unwrap();
        let unchanged = diff("missing.txt", false).await.unwrap();

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
        assert!(untracked.untracked);
        assert!(!untracked.binary);
        assert_eq!(added(&untracked.hunks), ["alpha", "beta"]);
        assert_eq!(untracked.stat.as_ref().and_then(|s| s.added), Some(2));
        assert!(binary.untracked);
        assert!(binary.binary);
        assert_eq!(binary.hunks, Vec::<Hunk>::new());
        assert!(unchanged.stat.is_none());
        assert_eq!(unchanged.hunks, Vec::<Hunk>::new());
    }

    fn branched_query_repo() -> TestRepository {
        let repository = started();
        repository.git(&["tag", "-a", "v0.1.0", "-m", "first"]);
        repository.git(&["switch", "--quiet", "-c", "feature"]);
        repository.write("a.txt", "one\nfeature line\n");
        repository.commit("Feature change");
        repository.git(&["switch", "--quiet", "main"]);
        repository.write("b.txt", "search_token_here\n");
        repository.commit("Add b");
        repository.git(&["tag", "-a", "v0.2.0", "-m", "second"]);
        repository.write("a.txt", "stashed change\n");
        repository.git(&["stash", "push", "-m", "wip stash"]);
        repository.write("a.txt", "one\nuncommitted line\n");
        repository
    }

    #[tokio::test]
    async fn queries_cover_history_refs_and_stashes() {
        let repository = branched_query_repo();
        let git = git_for(repository.path());

        let all_hist = queries::history_query(
            git.clone(),
            HistoryQueryRef {
                project_id: project(),
                scope: HistoryScope::AllBranches,
                query: None,
                author: None,
                path: None,
                pickaxe: None,
                merges: MergeFilter::All,
                first_parent: false,
                skip: 0,
                limit: Some(20),
            },
        )
        .await
        .unwrap();
        assert_eq!(all_hist.commits.len(), 3);
        assert!(all_hist.head_oid.is_some());

        let filtered = queries::history_query(
            git.clone(),
            HistoryQueryRef {
                project_id: project(),
                scope: HistoryScope::AllBranches,
                query: Some("Feature".to_owned()),
                author: None,
                path: None,
                pickaxe: None,
                merges: MergeFilter::All,
                first_parent: false,
                skip: 0,
                limit: Some(20),
            },
        )
        .await
        .unwrap();
        assert_eq!(filtered.commits.len(), 1);
        assert_eq!(filtered.commits[0].subject, "Feature change");

        let reflog = queries::history_query(
            git.clone(),
            HistoryQueryRef {
                project_id: project(),
                scope: HistoryScope::Reflog,
                query: None,
                author: None,
                path: None,
                pickaxe: None,
                merges: MergeFilter::All,
                first_parent: false,
                skip: 0,
                limit: Some(10),
            },
        )
        .await
        .unwrap();
        assert_ne!(reflog.reflog.len(), 0);

        let all_refs = queries::refs(git.clone(), of_project()).await.unwrap();
        assert_eq!(all_refs.branches[0].name, "main");
        assert!(all_refs.branches[0].current);
        assert_eq!(all_refs.tags.len(), 2);
        assert_eq!(all_refs.stashes.len(), 1);
        assert_eq!(all_refs.worktrees.len(), 1);

        let stash = queries::stash_show(
            git,
            StashShowRef {
                project_id: project(),
                index: 0,
            },
        )
        .await
        .unwrap();
        assert_eq!(stash.files.len(), 1);
        assert_eq!(stash.files[0].path, "a.txt");
        assert_ne!(stash.patches.len(), 0);
    }

    #[tokio::test]
    async fn queries_cover_compare_blame_tree_and_grep() {
        let repository = branched_query_repo();
        let git = git_for(repository.path());

        let compared = queries::compare(
            git.clone(),
            CompareRef {
                project_id: project(),
                base: "main".to_owned(),
                head: "feature".to_owned(),
                mode: CompareMode::ThreeDot,
                ignore_whitespace: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(compared.ahead.len(), 1);
        assert_eq!(compared.behind.len(), 1);
        assert_eq!(compared.files.len(), 1);

        let compared_wt = queries::compare(
            git.clone(),
            CompareRef {
                project_id: project(),
                base: "HEAD".to_owned(),
                head: "WORKTREE".to_owned(),
                mode: CompareMode::TwoDot,
                ignore_whitespace: false,
            },
        )
        .await
        .unwrap();
        assert!(compared_wt.head.is_none());
        assert_eq!(compared_wt.files.len(), 1);

        let blamed = queries::blame(
            git.clone(),
            BlameRef {
                project_id: project(),
                path: "a.txt".to_owned(),
                revision: None,
                ignore_whitespace: false,
                follow_moves: false,
            },
        )
        .await
        .unwrap();
        assert!(!blamed.binary);
        assert_eq!(blamed.total_lines, 2);
        assert!(blamed.runs.iter().any(|run| run.commit.uncommitted));

        let files = queries::tree(
            git.clone(),
            TreeQueryRef {
                project_id: project(),
                revision: None,
                query: Some("b.".to_owned()),
            },
        )
        .await
        .unwrap();
        assert_eq!(files.files, ["b.txt"]);

        let searched = queries::grep(
            git,
            GrepQueryRef {
                project_id: project(),
                pattern: "search_token".to_owned(),
                revision: None,
                case_sensitive: true,
                fixed_strings: true,
                path: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(searched.matches.len(), 1);
        assert_eq!(searched.matches[0].path, "b.txt");
    }

    async fn run_owner_action(
        git: &Git,
        locks: &RepositoryLocks,
        changes: &Changes,
        id: u32,
        action: OwnerGitAction,
        confirmation: Option<ActionConfirmation>,
    ) -> Result<OwnerActionResult, GitError> {
        actions::action_run(
            git.clone(),
            locks.clone(),
            changes.clone(),
            ActionRunRef {
                operation_id: op_id(id),
                project_id: project(),
                action,
                confirmation,
            },
        )
        .await
    }

    async fn preview_owner_action(git: &Git, action: OwnerGitAction) -> ActionPreview {
        actions::action_preview(
            git.clone(),
            ActionPreviewRef {
                project_id: project(),
                action,
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn owner_actions_stage_commit_confirm_hooks_and_reject_duplicates() {
        let repository = started();
        let host = Arc::new(HostGit::new().with_project(project(), repository.path()));
        let cloned = Arc::clone(&host);
        let git = Git::new(cloned);
        let locks = RepositoryLocks::default();
        let changes = Changes::new();

        // 1. Stage directly when no hooks exist, and verify duplicate ID is refused.
        repository.write("a.txt", "one\ntwo\n");
        let stage_action = OwnerGitAction::StageFiles {
            paths: vec!["a.txt".to_owned()],
            all: false,
        };
        let staged = run_owner_action(&git, &locks, &changes, 1, stage_action.clone(), None)
            .await
            .unwrap();
        assert!(matches!(staged, OwnerActionResult::Completed { .. }));

        let duplicate = run_owner_action(&git, &locks, &changes, 1, stage_action, None).await;
        assert_eq!(duplicate.err(), Some(GitError::AlreadyRun));

        // 2. Add a pre-commit hook and verify Commit requires confirmation.
        repository.write_hook("pre-commit", "#!/bin/sh\nexit 0\n");
        let commit_action = OwnerGitAction::Commit {
            message: Some("Second commit".to_owned()),
            amend: false,
            all: false,
        };
        let needs_confirm =
            run_owner_action(&git, &locks, &changes, 2, commit_action.clone(), None)
                .await
                .unwrap();
        let OwnerActionResult::NeedsConfirmation { preview } = needs_confirm else {
            panic!("expected NeedsConfirmation, got {needs_confirm:?}");
        };
        assert!(preview.needs_confirmation);
        assert!(preview.effects.iter().any(|effect| {
            matches!(effect, OwnerActionEffect::RunsFolderCode { name, .. } if name == "pre-commit")
        }));

        // Stale state is refused with RepositoryChanged; valid confirmation runs.
        let stale = run_owner_action(
            &git,
            &locks,
            &changes,
            3,
            commit_action.clone(),
            Some(ActionConfirmation {
                state: "HEAD stale".to_owned(),
                effects_digest: preview.effects_digest.clone(),
            }),
        )
        .await;
        assert_eq!(stale.err(), Some(GitError::RepositoryChanged));

        let committed = run_owner_action(
            &git,
            &locks,
            &changes,
            4,
            commit_action,
            Some(ActionConfirmation {
                state: preview.state,
                effects_digest: preview.effects_digest,
            }),
        )
        .await
        .unwrap();
        assert!(matches!(
            committed,
            OwnerActionResult::Completed { head: Some(_), .. }
        ));
        assert_eq!(host.owner_operations().len(), 2);
    }

    #[tokio::test]
    async fn owner_actions_discard_tag_and_resolve_conflicts() {
        let repository = started();
        let host = Arc::new(HostGit::new().with_project(project(), repository.path()));
        let cloned = Arc::clone(&host);
        let git = Git::new(cloned);
        let locks = RepositoryLocks::default();
        let changes = Changes::new();

        // 1. Discard file requires confirmation and saves a snapshot.
        repository.write("a.txt", "dirty change\n");
        let discard_action = OwnerGitAction::DiscardFiles {
            paths: vec!["a.txt".to_owned()],
            include_untracked: false,
        };
        let discard_preview = preview_owner_action(&git, discard_action.clone()).await;
        assert!(discard_preview.needs_confirmation);
        let discarded = run_owner_action(
            &git,
            &locks,
            &changes,
            5,
            discard_action,
            Some(ActionConfirmation {
                state: discard_preview.state,
                effects_digest: discard_preview.effects_digest,
            }),
        )
        .await
        .unwrap();
        let OwnerActionResult::Completed { snapshot, .. } = discarded else {
            panic!("expected Completed with snapshot");
        };
        assert!(snapshot.is_some());
        assert_eq!(
            std::fs::read_to_string(repository.path().join("a.txt")).unwrap(),
            "one\n"
        );

        // 2. Create tag and delete tag (with confirmation).
        let created_tag = run_owner_action(
            &git,
            &locks,
            &changes,
            6,
            OwnerGitAction::CreateTag {
                name: "v1.0.0".to_owned(),
                target: None,
                message: Some("Release 1.0".to_owned()),
            },
            None,
        )
        .await
        .unwrap();
        assert!(matches!(created_tag, OwnerActionResult::Completed { .. }));

        let del_tag = OwnerGitAction::DeleteTag {
            name: "v1.0.0".to_owned(),
        };
        let del_preview = preview_owner_action(&git, del_tag.clone()).await;
        assert!(del_preview.needs_confirmation);
        let deleted_tag = run_owner_action(
            &git,
            &locks,
            &changes,
            7,
            del_tag,
            Some(ActionConfirmation {
                state: del_preview.state,
                effects_digest: del_preview.effects_digest,
            }),
        )
        .await
        .unwrap();
        assert!(matches!(deleted_tag, OwnerActionResult::Completed { .. }));
        assert_eq!(host.owner_operations().len(), 3);
    }

    #[tokio::test]
    async fn owner_actions_resolve_merge_conflict_and_continue() {
        let repository = started();
        let host = Arc::new(HostGit::new().with_project(project(), repository.path()));
        let cloned = Arc::clone(&host);
        let git = Git::new(cloned);
        let locks = RepositoryLocks::default();
        let changes = Changes::new();

        // Merge conflict + resolve + continue.
        repository.git(&["switch", "--quiet", "-c", "side"]);
        repository.write("a.txt", "side version\n");
        repository.commit("Side commit");
        repository.git(&["switch", "--quiet", "main"]);
        repository.write("a.txt", "main version\n");
        repository.commit("Main commit");

        let merge_action = OwnerGitAction::Merge {
            branch: "side".to_owned(),
            mode: None,
            message: None,
        };
        let merge_preview = preview_owner_action(&git, merge_action.clone()).await;
        let merged = run_owner_action(
            &git,
            &locks,
            &changes,
            8,
            merge_action,
            Some(ActionConfirmation {
                state: merge_preview.state,
                effects_digest: merge_preview.effects_digest,
            }),
        )
        .await
        .unwrap();
        assert!(matches!(
            merged,
            OwnerActionResult::Stopped { conflicts: 1, .. }
        ));

        let resolved = run_owner_action(
            &git,
            &locks,
            &changes,
            9,
            OwnerGitAction::ResolveConflict {
                path: "a.txt".to_owned(),
                take: Some(Side::Incoming),
                hunks: None,
                deleted: None,
                edited: false,
            },
            None,
        )
        .await
        .unwrap();
        assert!(matches!(
            resolved,
            OwnerActionResult::Completed { count: Some(0), .. }
        ));

        let cont_action = OwnerGitAction::OperationControl {
            action: OperationAction::Continue,
        };
        let cont_preview = preview_owner_action(&git, cont_action.clone()).await;
        let continued = run_owner_action(
            &git,
            &locks,
            &changes,
            10,
            cont_action,
            Some(ActionConfirmation {
                state: cont_preview.state,
                effects_digest: cont_preview.effects_digest,
            }),
        )
        .await
        .unwrap();
        assert!(matches!(continued, OwnerActionResult::Completed { .. }));
        assert_eq!(host.owner_operations().len(), 3);
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

    fn host_git(host: &Arc<HostGit>) -> Git {
        let cloned = Arc::clone(host);
        Git::new(cloned)
    }

    #[tokio::test]
    async fn history_pickaxe_disables_textconv() {
        let repository = started();
        let marker = repository.sibling("markers").join("textconv");
        let conv = TestRepository::marking_command(&marker, "textconv", "cat");
        repository.git(&["config", "diff.secret.textconv", &conv]);
        repository.write(".gitattributes", "*.txt diff=secret\n");
        repository.write("a.txt", "pickaxe_needle\n");
        repository.commit("Configure textconv");

        let result = queries::history_query(
            git_for(repository.path()),
            HistoryQueryRef {
                project_id: project(),
                scope: HistoryScope::CurrentBranch,
                query: None,
                author: None,
                path: None,
                pickaxe: Some("pickaxe_needle".to_owned()),
                merges: MergeFilter::All,
                first_parent: false,
                skip: 0,
                limit: Some(10),
            },
        )
        .await
        .unwrap();

        assert_eq!(result.commits.len(), 1);
        assert!(
            !marker.exists(),
            "pickaxe history query must not launch repository textconv"
        );
    }

    #[tokio::test]
    async fn stage_hunk_rejects_patch_targeting_another_file() {
        let repository = started();
        repository.write("b.txt", "before\n");
        repository.commit("Add b");
        repository.write("a.txt", "first\nchanged a\n");
        repository.write("b.txt", "before\nchanged b\n");

        let host = Arc::new(HostGit::new().with_project(project(), repository.path()));
        let git = host_git(&host);
        let locks = RepositoryLocks::default();
        let changes = Changes::new();

        let forged_patch = "diff --git a/b.txt b/b.txt\n--- a/b.txt\n+++ b/b.txt\n@@ -1 +1,2 @@\n before\n+changed b\n";
        let err = run_owner_action(
            &git,
            &locks,
            &changes,
            20,
            OwnerGitAction::StageHunk {
                path: "a.txt".to_owned(),
                patch: forged_patch.to_owned(),
            },
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(err, GitError::RepositoryChanged);

        let staged = repository.git(&["diff", "--cached", "--name-only"]);
        assert!(
            staged.trim().is_empty(),
            "b.txt must not be staged: {staged}"
        );
    }

    #[tokio::test]
    async fn discard_confirmation_invalidates_when_untracked_file_changes() {
        let repository = started();
        repository.write("scratch.txt", "draft one\n");

        let host = Arc::new(HostGit::new().with_project(project(), repository.path()));
        let git = host_git(&host);
        let locks = RepositoryLocks::default();
        let changes = Changes::new();

        let action = OwnerGitAction::DiscardFiles {
            paths: vec!["scratch.txt".to_owned()],
            include_untracked: true,
        };
        let preview = preview_owner_action(&git, action.clone()).await;
        assert!(preview.needs_confirmation);

        // Edit the already-untracked file between preview and confirmation.
        repository.write("scratch.txt", "important new work\n");

        let err = run_owner_action(
            &git,
            &locks,
            &changes,
            21,
            action,
            Some(ActionConfirmation {
                state: preview.state,
                effects_digest: preview.effects_digest,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err, GitError::RepositoryChanged);
        assert_eq!(
            std::fs::read_to_string(repository.path().join("scratch.txt")).unwrap(),
            "important new work\n"
        );
    }

    #[tokio::test]
    async fn create_tag_requires_confirmation_when_repo_configures_signer() {
        let repository = started();
        let marker = repository.sibling("markers").join("signer");
        let signer = TestRepository::marking_command(&marker, "signer", "true");
        repository.git(&["config", "tag.gpgSign", "true"]);
        repository.git(&["config", "gpg.program", &signer]);

        let host = Arc::new(HostGit::new().with_project(project(), repository.path()));
        let git = host_git(&host);
        let locks = RepositoryLocks::default();
        let changes = Changes::new();

        let action = OwnerGitAction::CreateTag {
            name: "v2.0.0".to_owned(),
            target: None,
            message: Some("Signed release".to_owned()),
        };
        let preview = preview_owner_action(&git, action.clone()).await;
        assert!(preview.needs_confirmation);
        assert!(preview.effects.iter().any(|effect| matches!(
            effect,
            OwnerActionEffect::RunsFolderCode { name, .. } if name == "gpg.program"
        )));

        let unconfirmed = run_owner_action(&git, &locks, &changes, 22, action, None)
            .await
            .unwrap();
        assert!(matches!(
            unconfirmed,
            OwnerActionResult::NeedsConfirmation { .. }
        ));
        assert!(!marker.exists(), "signer must not run before confirmation");
    }

    #[tokio::test]
    async fn blame_probe_skips_directories_and_external_symlinks() {
        let repository = started();
        std::fs::create_dir_all(repository.path().join("subdir")).unwrap();
        assert_eq!(
            queries::safe_worktree_regular_file(repository.path(), "subdir").unwrap(),
            None
        );

        let outside = repository.sibling("outside").join("outside_bin.dat");
        std::fs::write(&outside, b"\0\x01\x02binary").unwrap();
        let link_path = repository.path().join("ext_link");
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&outside, &link_path).is_ok();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(&outside, &link_path).is_ok();
        if linked {
            assert_eq!(
                queries::safe_worktree_regular_file(repository.path(), "ext_link").unwrap(),
                None
            );
            repository.git(&["add", "ext_link"]);
            repository.commit("Add symlink");
            let blamed = queries::blame(
                git_for(repository.path()),
                BlameRef {
                    project_id: project(),
                    path: "ext_link".to_owned(),
                    revision: None,
                    ignore_whitespace: false,
                    follow_moves: false,
                },
            )
            .await
            .unwrap();
            assert!(!blamed.binary, "symlink should be blamed as link text");
        }
    }

    #[tokio::test]
    async fn journal_effects_strip_commit_and_stash_messages() {
        let repository = started();
        repository.git(&["switch", "--quiet", "-c", "secret-branch"]);
        repository.write("s.txt", "secret\n");
        repository.commit("CONFIDENTIAL_COMMIT_SUBJECT_999");
        repository.git(&["switch", "--quiet", "main"]);
        repository.write("a.txt", "stashed change\n");
        repository.git(&["stash", "push", "-m", "CONFIDENTIAL_STASH_MESSAGE_888"]);

        let host = Arc::new(HostGit::new().with_project(project(), repository.path()));
        let git = host_git(&host);
        let locks = RepositoryLocks::default();
        let changes = Changes::new();

        let del_branch = OwnerGitAction::DeleteBranch {
            name: "secret-branch".to_owned(),
        };
        let branch_preview = preview_owner_action(&git, del_branch.clone()).await;
        let preview_json = serde_json::to_string(&branch_preview.effects).unwrap();
        assert!(preview_json.contains("CONFIDENTIAL_COMMIT_SUBJECT_999"));
        run_owner_action(
            &git,
            &locks,
            &changes,
            23,
            del_branch,
            Some(ActionConfirmation {
                state: branch_preview.state,
                effects_digest: branch_preview.effects_digest,
            }),
        )
        .await
        .unwrap();

        let drop_stash = OwnerGitAction::StashDrop { index: 0 };
        let stash_preview = preview_owner_action(&git, drop_stash.clone()).await;
        let stash_preview_json = serde_json::to_string(&stash_preview.effects).unwrap();
        assert!(stash_preview_json.contains("CONFIDENTIAL_STASH_MESSAGE_888"));
        run_owner_action(
            &git,
            &locks,
            &changes,
            24,
            drop_stash,
            Some(ActionConfirmation {
                state: stash_preview.state,
                effects_digest: stash_preview.effects_digest,
            }),
        )
        .await
        .unwrap();

        let requests = host.owner_requests();
        assert_eq!(requests.len(), 2);
        let branch_journal = serde_json::to_string(&requests[0].effects).unwrap();
        let stash_journal = serde_json::to_string(&requests[1].effects).unwrap();
        assert!(!branch_journal.contains("CONFIDENTIAL_COMMIT_SUBJECT_999"));
        assert!(!stash_journal.contains("CONFIDENTIAL_STASH_MESSAGE_888"));
        assert!(requests[1].effects.iter().any(|effect| matches!(
            effect,
            ProgramEffect::DiscardsWork { items, .. } if items == &["stash@{0}"]
        )));
    }

    #[tokio::test]
    async fn dropping_owner_session_marks_operation_uncertain() {
        let repository = started();
        let host = Arc::new(HostGit::new().with_project(project(), repository.path()));
        let git = host_git(&host);
        let op_id = ProgramOperationId::parse("01960000-0000-7000-8000-000000000025").unwrap();
        let request = OwnerOperationRequest::new(
            op_id.clone(),
            project(),
            "git",
            "stage_files",
            vec![repository.path().to_path_buf()],
        );
        {
            let _session = git
                .begin_owner_operation(repository.path(), request)
                .await
                .unwrap();
        }
        tokio::task::yield_now().await;
        let ops = host.owner_operations();
        assert_eq!(ops, vec![(op_id, Some(OwnerOperationOutcome::Uncertain))]);
    }

    #[tokio::test]
    async fn merge_conflict_records_nonzero_exit_and_timeout_records_uncertain() {
        let repository = started();
        repository.git(&["switch", "--quiet", "-c", "side"]);
        repository.write("a.txt", "side\n");
        repository.commit("Side");
        repository.git(&["switch", "--quiet", "main"]);
        repository.write("a.txt", "main\n");
        repository.commit("Main");

        let host = Arc::new(HostGit::new().with_project(project(), repository.path()));
        let git = host_git(&host);
        let locks = RepositoryLocks::default();
        let changes = Changes::new();

        let merged = run_owner_action(
            &git,
            &locks,
            &changes,
            26,
            OwnerGitAction::Merge {
                branch: "side".to_owned(),
                mode: None,
                message: None,
            },
            None,
        )
        .await
        .unwrap();
        assert!(matches!(merged, OwnerActionResult::Stopped { .. }));
        let ops = host.owner_operations();
        assert_eq!(
            ops[0].1,
            Some(OwnerOperationOutcome::Completed { exit_code: Some(1) })
        );

        let timeout_host = Arc::new(
            HostGit::new()
                .with_project(project(), repository.path())
                .timing_out_owner_runs(),
        );
        let timeout_git = host_git(&timeout_host);
        let err = run_owner_action(
            &timeout_git,
            &locks,
            &changes,
            27,
            OwnerGitAction::StageFiles {
                paths: vec!["a.txt".to_owned()],
                all: false,
            },
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(err, GitError::TimedOut);
        assert_eq!(
            timeout_host.owner_operations()[0].1,
            Some(OwnerOperationOutcome::Uncertain)
        );
    }

    #[tokio::test]
    async fn overview_push_target_honors_push_remote_and_push_url() {
        let repository = started();
        repository.git(&["remote", "add", "alpha", "https://alpha.example/repo.git"]);
        repository.git(&["remote", "add", "deploy", "https://fetch.example/repo.git"]);
        repository.git(&[
            "remote",
            "set-url",
            "--push",
            "deploy",
            "https://user:secret@push.example/repo.git",
        ]);
        repository.git(&["config", "branch.main.pushRemote", "deploy"]);

        let shown = overview(git_for(repository.path()), of_project())
            .await
            .unwrap();
        let target = shown.push_target.expect("push target should be resolved");
        assert_eq!(target.name, "deploy");
        assert_eq!(
            target.push.as_deref(),
            Some("https://push.example/repo.git")
        );
    }

    #[tokio::test]
    async fn checkout_remote_branch_preserves_selected_remote_and_refuses_local_collision() {
        let origin = started();
        origin.git(&["switch", "--quiet", "-c", "feature"]);
        origin.write("origin.txt", "from origin\n");
        origin.commit("Origin feature");
        origin.git(&["switch", "--quiet", "main"]);

        let fork = origin.sibling("fork");
        origin.git(&[
            "clone",
            "--quiet",
            origin.path().to_str().unwrap(),
            fork.to_str().unwrap(),
        ]);
        origin.git_in(&fork, &["config", "user.name", "Test"]);
        origin.git_in(&fork, &["config", "user.email", "test@example.com"]);
        origin.git_in(&fork, &["switch", "--quiet", "-c", "feature"]);
        std::fs::write(fork.join("fork.txt"), "from fork\n").unwrap();
        origin.git_in(&fork, &["add", "fork.txt"]);
        origin.git_in(&fork, &["commit", "--quiet", "-m", "Fork feature"]);
        let fork_tip = origin
            .git_in(&fork, &["rev-parse", "HEAD"])
            .trim()
            .to_owned();
        origin.git_in(&fork, &["switch", "--quiet", "main"]);

        let workspace = origin.sibling("workspace");
        origin.git(&[
            "clone",
            "--quiet",
            origin.path().to_str().unwrap(),
            workspace.to_str().unwrap(),
        ]);
        origin.git_in(
            &workspace,
            &["remote", "add", "fork", fork.to_str().unwrap()],
        );
        origin.git_in(&workspace, &["fetch", "--quiet", "origin"]);
        origin.git_in(&workspace, &["fetch", "--quiet", "fork"]);

        let host = Arc::new(HostGit::new().with_project(project(), &workspace));
        let git = host_git(&host);
        let locks = RepositoryLocks::default();
        let changes = Changes::new();

        // Local `main` already exists, so checking out `fork/main` as `main` must refuse.
        let collision = run_owner_action(
            &git,
            &locks,
            &changes,
            28,
            OwnerGitAction::CheckoutRemoteBranch {
                remote_ref: "fork/main".to_owned(),
                local_name: Some("main".to_owned()),
            },
            None,
        )
        .await;
        assert_eq!(collision.unwrap_err(), GitError::InvalidArgument);

        // Checking out `fork/feature` creates local `feature` at `fork`'s commit
        // tracking `fork`, even though `origin/feature` also exists.
        let checked_out = run_owner_action(
            &git,
            &locks,
            &changes,
            29,
            OwnerGitAction::CheckoutRemoteBranch {
                remote_ref: "fork/feature".to_owned(),
                local_name: Some("feature".to_owned()),
            },
            None,
        )
        .await
        .unwrap();
        assert!(matches!(checked_out, OwnerActionResult::Completed { .. }));
        assert_eq!(
            origin.git_in(&workspace, &["rev-parse", "HEAD"]).trim(),
            fork_tip
        );
        assert_eq!(
            origin
                .git_in(&workspace, &["config", "branch.feature.remote"])
                .trim(),
            "fork"
        );
    }
}
