//! The plugin's commands and channel, as specs of the plugin SDK (ADR-0034,
//! decision 8), for the Git side panel (plugins/git/docs/panel.md). They
//! only read. Each names a project by its ID, and every run of git is
//! checked first as the reads of a conversation of that project (ADR-0038,
//! decision 10), so a repository whose git folder lies outside the project
//! needs a permission that the panel cannot ask for.

use mannie_plugin_sdk::contracts::ProjectId;
use mannie_plugin_sdk::ipc::{
    ChannelSpec, CommandSpec, InputRefusal, Interface, PluginError, Validate,
};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::describe::{CommitLabel, OperationView};
use crate::failure::GitFailure;
use crate::parse::conflicts::ConflictHunk;
use crate::parse::diff::{FileStat, Hunk};
use crate::parse::log::{Commit, CommitDetails};
use crate::parse::refs::Ref;
use crate::parse::status::{Branch, Entry};
use crate::tools::conflicts::ConflictedFile;
use crate::tools::show::FilePatch;

/// The commits of one page of the history.
pub(crate) const HISTORY_PAGE: u32 = 50;

/// The most entries of each list that a command returns.
pub(crate) const MAX_LISTED: usize = 1000;

/// The most characters of a path that a command takes.
const MAX_PATH_CHARS: usize = 4096;

/// A project, as a command names it.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProjectRef {
    /// The project.
    pub(crate) project_id: ProjectId,
}

impl Validate for ProjectRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// The argument of `repository_history`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HistoryRef {
    /// The project.
    pub(crate) project_id: ProjectId,
    /// The commits to skip, newest first, before the page.
    pub(crate) skip: u32,
}

impl Validate for HistoryRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.skip > 1_000_000 {
            return Err(InputRefusal::new("skip"));
        }
        Ok(())
    }
}

/// The argument of `repository_commit`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CommitRef {
    /// The project.
    pub(crate) project_id: ProjectId,
    /// The commit's hash, whole or its start, of 4 to 64 hexadecimal digits.
    pub(crate) commit: String,
}

impl Validate for CommitRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        let is_hash = (4..=64).contains(&self.commit.len())
            && self.commit.bytes().all(|byte| byte.is_ascii_hexdigit());
        if !is_hash {
            return Err(InputRefusal::new("commit"));
        }
        Ok(())
    }
}

/// The argument of `repository_file_diff`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FileRef {
    /// The project.
    pub(crate) project_id: ProjectId,
    /// The file, relative to the repository's top, as the overview lists it.
    pub(crate) path: String,
    /// Whether to show the staged change instead of the unstaged one.
    pub(crate) staged: bool,
}

impl Validate for FileRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        let path = self.path.as_str();
        let leaves = path.split('/').any(|part| part == "..");
        let is_relative = !path.starts_with('/') && !path.contains(':') && !path.contains('\\');
        let is_path = !path.is_empty()
            && path.chars().count() <= MAX_PATH_CHARS
            && !path.chars().any(char::is_control)
            && is_relative
            && !leaves;
        if !is_path {
            return Err(InputRefusal::new("path"));
        }
        Ok(())
    }
}

/// The repository of a project, as the panel's header and its Changes tab
/// show it.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RepositoryOverview {
    /// The repository's top folder.
    pub(crate) top: String,
    pub(crate) branch: Branch,
    pub(crate) head: Option<CommitLabel>,
    pub(crate) operation: Option<OperationView>,
    /// The changed, conflicted and untracked files, at most 1000.
    pub(crate) entries: Vec<Entry>,
    /// The files in all.
    pub(crate) entries_total: u32,
    /// The number of stash entries.
    pub(crate) stash: u32,
}

/// A page of the current branch's history, newest first.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct History {
    pub(crate) commits: Vec<Commit>,
    /// Whether older commits follow.
    pub(crate) more: bool,
}

/// The repository's branches, remote branches and tags.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BranchList {
    pub(crate) branches: Vec<Ref>,
    pub(crate) remote_branches: Vec<Ref>,
    pub(crate) tags: Vec<Ref>,
}

/// A commit with its files and their patches.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitView {
    pub(crate) commit: CommitDetails,
    pub(crate) files: Vec<FileStat>,
    pub(crate) patches: Vec<FilePatch>,
}

/// A file's change in the work tree or the index.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileDiff {
    /// Its line counts, or none when it did not change.
    pub(crate) stat: Option<FileStat>,
    pub(crate) hunks: Vec<Hunk>,
}

/// A conflicted file with its remaining hunks.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConflictFileView {
    pub(crate) file: ConflictedFile,
    pub(crate) hunks: Vec<ConflictHunk>,
}

/// The operation in progress and its conflicted files.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConflictsView {
    pub(crate) operation: Option<OperationView>,
    pub(crate) head: Option<CommitLabel>,
    pub(crate) files: Vec<ConflictFileView>,
}

/// What the channel `follow_repository` carries. More kinds of updates may
/// come, by addition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RepositoryUpdate {
    /// One of the plugin's tools changed a repository, so a panel reads its
    /// own again.
    Changed,
}

/// Why a command of the plugin failed. The host's refusals, such as
/// `plugin_off` and `invalid_input` with the refused field, come beside
/// these (ADR-0034, decision 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub(crate) enum GitError {
    /// The project does not exist.
    #[error("the project does not exist")]
    ProjectNotFound,
    /// The project's folder is not in a repository.
    #[error("the project's folder is not in a git repository")]
    NotARepository,
    /// The repository has no work tree.
    #[error("the repository has no work tree")]
    BareRepository,
    /// git is not on Mannie's `PATH`.
    #[error("git was not found")]
    GitMissing,
    /// The installed git is older than 2.40.
    #[error("the installed git is older than 2.40")]
    GitTooOld,
    /// git refuses the repository, because another user owns it.
    #[error("git refuses a repository that another user owns")]
    DubiousOwnership,
    /// Reading the repository needs the owner's permission.
    #[error("reading the repository needs the owner's permission")]
    ReadNotAllowed,
    /// The commit does not exist.
    #[error("the commit does not exist")]
    UnknownCommit,
    /// Another git process holds a lock of the repository.
    #[error("another git process holds a lock of the repository")]
    Locked,
    /// git did not finish in time.
    #[error("git did not finish in time")]
    TimedOut,
    /// git failed.
    #[error("git failed")]
    GitFailed,
}

impl PluginError for GitError {
    fn code(&self) -> &'static str {
        match self {
            Self::ProjectNotFound => "git.project_not_found",
            Self::NotARepository => "git.not_a_repository",
            Self::BareRepository => "git.bare_repository",
            Self::GitMissing => "git.git_missing",
            Self::GitTooOld => "git.git_too_old",
            Self::DubiousOwnership => "git.dubious_ownership",
            Self::ReadNotAllowed => "git.read_not_allowed",
            Self::UnknownCommit => "git.unknown_commit",
            Self::Locked => "git.locked",
            Self::TimedOut => "git.timed_out",
            Self::GitFailed => "git.git_failed",
        }
    }
}

impl From<GitFailure> for GitError {
    fn from(failure: GitFailure) -> Self {
        match failure {
            GitFailure::Missing => Self::GitMissing,
            GitFailure::TooOld { .. } => Self::GitTooOld,
            GitFailure::NotARepository { .. } => Self::NotARepository,
            GitFailure::Bare { .. } => Self::BareRepository,
            GitFailure::DubiousOwnership { .. } => Self::DubiousOwnership,
            GitFailure::ReadNotAllowed { .. } => Self::ReadNotAllowed,
            GitFailure::UnknownRevision { .. } => Self::UnknownCommit,
            GitFailure::Locked { .. } => Self::Locked,
            GitFailure::TimedOut => Self::TimedOut,
            GitFailure::InvalidArgument { .. }
            | GitFailure::RepositoryChanged
            | GitFailure::IdentityMissing
            | GitFailure::Refused { .. }
            | GitFailure::Failed { .. } => Self::GitFailed,
        }
    }
}

/// `repository_overview`: the branch, the operation in progress and the
/// changed files.
#[derive(Debug)]
pub(crate) enum RepositoryOverviewCommand {}

impl CommandSpec for RepositoryOverviewCommand {
    const NAME: &'static str = "repository_overview";
    const DOCS: &'static str =
        "The project's repository: its branch, the operation in progress and its changed files.";
    type Input = ProjectRef;
    type Output = RepositoryOverview;
    type Error = GitError;
}

/// `repository_history`: a page of the current branch's commits.
#[derive(Debug)]
pub(crate) enum RepositoryHistoryCommand {}

impl CommandSpec for RepositoryHistoryCommand {
    const NAME: &'static str = "repository_history";
    const DOCS: &'static str =
        "A page of the current branch's commits, newest first, after skip of them.";
    type Input = HistoryRef;
    type Output = History;
    type Error = GitError;
}

/// `repository_branches`: the branches, remote branches and tags.
#[derive(Debug)]
pub(crate) enum RepositoryBranchesCommand {}

impl CommandSpec for RepositoryBranchesCommand {
    const NAME: &'static str = "repository_branches";
    const DOCS: &'static str =
        "The repository's branches with their upstreams, its remote branches and its tags.";
    type Input = ProjectRef;
    type Output = BranchList;
    type Error = GitError;
}

/// `repository_commit`: a commit with its files and patches.
#[derive(Debug)]
pub(crate) enum RepositoryCommitCommand {}

impl CommandSpec for RepositoryCommitCommand {
    const NAME: &'static str = "repository_commit";
    const DOCS: &'static str = "A commit with its message, its changed files and their patches.";
    type Input = CommitRef;
    type Output = CommitView;
    type Error = GitError;
}

/// `repository_file_diff`: a changed file's patch.
#[derive(Debug)]
pub(crate) enum RepositoryFileDiffCommand {}

impl CommandSpec for RepositoryFileDiffCommand {
    const NAME: &'static str = "repository_file_diff";
    const DOCS: &'static str = "A file's unstaged or staged change.";
    type Input = FileRef;
    type Output = FileDiff;
    type Error = GitError;
}

/// `repository_conflicts`: the conflicted files with their hunks.
#[derive(Debug)]
pub(crate) enum RepositoryConflictsCommand {}

impl CommandSpec for RepositoryConflictsCommand {
    const NAME: &'static str = "repository_conflicts";
    const DOCS: &'static str =
        "The operation in progress and its conflicted files, each with its remaining hunks.";
    type Input = ProjectRef;
    type Output = ConflictsView;
    type Error = GitError;
}

/// `follow_repository`: a note after each change that the plugin's tools
/// make.
#[derive(Debug)]
pub(crate) enum FollowRepositoryChannel {}

impl ChannelSpec for FollowRepositoryChannel {
    const NAME: &'static str = "follow_repository";
    const DOCS: &'static str = "A note after each change that the plugin's tools make in a repository, until the plugin stops.";
    type Input = ();
    type Item = RepositoryUpdate;
    type Error = GitError;
}

/// The plugin's commands, its channel and the limits that the UI reads.
pub(crate) fn interface() -> Interface {
    Interface::new()
        .command::<RepositoryOverviewCommand>()
        .command::<RepositoryHistoryCommand>()
        .command::<RepositoryBranchesCommand>()
        .command::<RepositoryCommitCommand>()
        .command::<RepositoryFileDiffCommand>()
        .command::<RepositoryConflictsCommand>()
        .channel::<FollowRepositoryChannel>()
        .constant(
            "HISTORY_PAGE",
            "The commits of one page of the history.",
            HISTORY_PAGE,
        )
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::ProjectId;
    use mannie_plugin_sdk::ipc::Validate;

    use super::{CommitRef, FileRef};

    fn project() -> ProjectId {
        ProjectId::parse("0199e0f2-0000-7000-8000-000000000005").unwrap()
    }

    #[test]
    fn a_commit_is_a_hash_and_a_path_stays_inside_the_repository() {
        let commit = |commit: &str| CommitRef {
            project_id: project(),
            commit: commit.to_owned(),
        };
        let path = |path: &str| FileRef {
            project_id: project(),
            path: path.to_owned(),
            staged: false,
        };

        assert_eq!(commit("1a2b3c4").validate(), Ok(()));
        for refused in ["", "1a2", "--output", "main", &"a".repeat(65)] {
            assert_eq!(
                commit(refused).validate().unwrap_err().field,
                "commit",
                "{refused}"
            );
        }
        assert_eq!(path("src/main file.rs").validate(), Ok(()));
        for refused in ["", "../x", "/etc/passwd", "C:/x", "a\\b", "a\nb"] {
            assert_eq!(
                path(refused).validate().unwrap_err().field,
                "path",
                "{refused:?}"
            );
        }
    }
}
