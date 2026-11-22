//! The plugin's commands and channel, as specs of the plugin SDK (ADR-0034,
//! decision 8), for the Git side panel (plugins/git/docs/panel.md). Read
//! commands check every run of git as reads of a conversation of that project
//! (ADR-0038, decision 10). Owner action commands run through the core's
//! journaled owner-operation boundary (ADR-0078).

use mannie_plugin_sdk::contracts::{DiscardedWork, ProgramOperationId, ProjectId, UtcMillis};
use mannie_plugin_sdk::ipc::{
    ChannelSpec, CommandSpec, InputRefusal, Interface, PluginError, Validate,
};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::describe::{CommitLabel, OperationView};
use crate::failure::GitFailure;
use crate::parse::blame::{BlameCommit, BlameLine};
use crate::parse::conflicts::ConflictHunk;
use crate::parse::diff::{FileStat, Hunk};
use crate::parse::grep::Match;
use crate::parse::log::{Commit, CommitDetails, ReflogEntry};
use crate::parse::refs::Ref;
use crate::parse::remotes::Remote;
use crate::parse::status::{Branch, Entry};
use crate::parse::worktrees::Worktree;
use crate::tools::conflicts::ConflictedFile;
use crate::tools::fetch::Integrate;
use crate::tools::merge::MergeMode;
use crate::tools::operation::OperationAction;
use crate::tools::resolve::{HunkChoice, Side};
use crate::tools::show::FilePatch;

/// The commits of one page of the history.
pub(crate) const HISTORY_PAGE: u32 = 50;

/// The most commits one history query page may request.
pub(crate) const MAX_HISTORY_LIMIT: u32 = 200;

/// The most entries of each list that a command returns.
pub(crate) const MAX_LISTED: usize = 1000;

/// The most characters of a path that a command takes.
const MAX_PATH_CHARS: usize = 4096;

/// The most characters of a revision or ref name that a command takes.
const MAX_REVISION_CHARS: usize = 512;

/// The most characters of a search query or pattern.
const MAX_QUERY_CHARS: usize = 1024;

/// The most characters of a commit, tag or stash message.
const MAX_MESSAGE_CHARS: usize = 100_000;

/// The most bytes of a hunk patch sent from the UI.
const MAX_PATCH_BYTES: usize = 512 * 1024;

/// The most lines of diff context that a command takes.
const MAX_CONTEXT_LINES: u32 = 100;

/// The most files one batch action may list.
const MAX_ACTION_PATHS: usize = 500;

/// Checks a relative path inside the repository.
fn validate_repo_path(field: &'static str, path: &str) -> Result<(), InputRefusal> {
    let leaves = path.split('/').any(|part| part == "..");
    let is_relative = !path.starts_with('/') && !path.contains(':') && !path.contains('\\');
    let is_path = !path.is_empty()
        && path.chars().count() <= MAX_PATH_CHARS
        && !path.chars().any(char::is_control)
        && is_relative
        && !leaves;
    if !is_path {
        return Err(InputRefusal::new(field));
    }
    Ok(())
}

/// Checks a revision, branch or tag input from the UI.
fn validate_revision(field: &'static str, revision: &str) -> Result<(), InputRefusal> {
    let ok = !revision.is_empty()
        && revision.chars().count() <= MAX_REVISION_CHARS
        && !revision.starts_with('-')
        && !revision.chars().any(char::is_control)
        && !revision.contains("..");
    if !ok {
        return Err(InputRefusal::new(field));
    }
    Ok(())
}

/// Checks an optional short search string.
fn validate_search(field: &'static str, text: Option<&str>) -> Result<(), InputRefusal> {
    let Some(text) = text else {
        return Ok(());
    };
    if text.chars().count() > MAX_QUERY_CHARS || text.contains('\0') {
        return Err(InputRefusal::new(field));
    }
    Ok(())
}

/// Checks a free-form message from the UI.
fn validate_message(field: &'static str, message: Option<&str>) -> Result<(), InputRefusal> {
    let Some(message) = message else {
        return Ok(());
    };
    if message.chars().count() > MAX_MESSAGE_CHARS || message.contains('\0') {
        return Err(InputRefusal::new(field));
    }
    Ok(())
}

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

/// Which refs or log stream a history query reads.
#[derive(Debug, Clone, Default, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub(crate) enum HistoryScope {
    /// Commits reachable from HEAD.
    #[default]
    CurrentBranch,
    /// Commits reachable from all branches, remote branches and tags.
    AllBranches,
    /// Commits reachable from one specific ref or commit.
    Ref { name: String },
    /// Entries of HEAD's reflog.
    Reflog,
}

/// How a history query filters merge commits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MergeFilter {
    /// Include merge and non-merge commits.
    #[default]
    All,
    /// Include only merge commits.
    Only,
    /// Exclude merge commits.
    Exclude,
}

/// The argument of `repository_history_query`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HistoryQueryRef {
    pub(crate) project_id: ProjectId,
    #[serde(default)]
    pub(crate) scope: HistoryScope,
    /// Filter commits whose message contains this text.
    #[serde(default)]
    pub(crate) query: Option<String>,
    /// Filter commits whose author matches this text.
    #[serde(default)]
    pub(crate) author: Option<String>,
    /// Filter commits touching this file or folder (uses `--follow` for files).
    #[serde(default)]
    pub(crate) path: Option<String>,
    /// Filter commits adding or removing occurrences of this text (`-S`).
    #[serde(default)]
    pub(crate) pickaxe: Option<String>,
    #[serde(default)]
    pub(crate) merges: MergeFilter,
    #[serde(default)]
    pub(crate) first_parent: bool,
    #[serde(default)]
    pub(crate) skip: u32,
    #[serde(default)]
    pub(crate) limit: Option<u32>,
}

impl Validate for HistoryQueryRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        if let HistoryScope::Ref { name } = &self.scope {
            validate_revision("scope", name)?;
        }
        validate_search("query", self.query.as_deref())?;
        validate_search("author", self.author.as_deref())?;
        if let Some(path) = self.path.as_deref().filter(|path| !path.is_empty()) {
            validate_repo_path("path", path)?;
        }
        validate_search("pickaxe", self.pickaxe.as_deref())?;
        if self.skip > 1_000_000 {
            return Err(InputRefusal::new("skip"));
        }
        if let Some(limit) = self.limit
            && !(1..=MAX_HISTORY_LIMIT).contains(&limit)
        {
            return Err(InputRefusal::new("limit"));
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
    /// Whether to ignore whitespace-only changes (`-w`).
    #[serde(default)]
    pub(crate) ignore_whitespace: bool,
    /// Lines of context around each hunk, 0 to 100.
    #[serde(default)]
    pub(crate) context_lines: Option<u32>,
}

impl Validate for FileRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        validate_repo_path("path", &self.path)?;
        if let Some(context) = self.context_lines
            && context > MAX_CONTEXT_LINES
        {
            return Err(InputRefusal::new("contextLines"));
        }
        Ok(())
    }
}

/// The argument of `repository_stash_show`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StashShowRef {
    pub(crate) project_id: ProjectId,
    pub(crate) index: u32,
}

impl Validate for StashShowRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.index > 10_000 {
            return Err(InputRefusal::new("index"));
        }
        Ok(())
    }
}

/// How two revisions are compared.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CompareMode {
    /// Changes on head since it diverged from base (`base...head`).
    #[default]
    ThreeDot,
    /// Direct tree diff between base and head (`base..head`).
    TwoDot,
}

/// The argument of `repository_compare`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CompareRef {
    pub(crate) project_id: ProjectId,
    /// The base ref or commit.
    pub(crate) base: String,
    /// The target ref or commit, or `"WORKTREE"` to compare against the work tree.
    pub(crate) head: String,
    #[serde(default)]
    pub(crate) mode: CompareMode,
    #[serde(default)]
    pub(crate) ignore_whitespace: bool,
}

impl Validate for CompareRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        validate_revision("base", &self.base)?;
        if self.head != "WORKTREE" {
            validate_revision("head", &self.head)?;
        }
        Ok(())
    }
}

/// The argument of `repository_blame`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BlameRef {
    pub(crate) project_id: ProjectId,
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) revision: Option<String>,
    #[serde(default)]
    pub(crate) ignore_whitespace: bool,
    #[serde(default)]
    pub(crate) follow_moves: bool,
}

impl Validate for BlameRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        validate_repo_path("path", &self.path)?;
        if let Some(revision) = self.revision.as_deref().filter(|rev| !rev.is_empty()) {
            validate_revision("revision", revision)?;
        }
        Ok(())
    }
}

/// The argument of `repository_tree`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TreeQueryRef {
    pub(crate) project_id: ProjectId,
    #[serde(default)]
    pub(crate) revision: Option<String>,
    #[serde(default)]
    pub(crate) query: Option<String>,
}

impl Validate for TreeQueryRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        if let Some(revision) = self.revision.as_deref().filter(|rev| !rev.is_empty()) {
            validate_revision("revision", revision)?;
        }
        validate_search("query", self.query.as_deref())?;
        Ok(())
    }
}

/// The argument of `repository_grep`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GrepQueryRef {
    pub(crate) project_id: ProjectId,
    pub(crate) pattern: String,
    #[serde(default)]
    pub(crate) revision: Option<String>,
    #[serde(default)]
    pub(crate) case_sensitive: bool,
    #[serde(default)]
    pub(crate) fixed_strings: bool,
    #[serde(default)]
    pub(crate) path: Option<String>,
}

impl Validate for GrepQueryRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.pattern.is_empty()
            || self.pattern.chars().count() > MAX_QUERY_CHARS
            || self.pattern.contains('\0')
        {
            return Err(InputRefusal::new("pattern"));
        }
        if let Some(revision) = self.revision.as_deref().filter(|rev| !rev.is_empty()) {
            validate_revision("revision", revision)?;
        }
        if let Some(path) = self.path.as_deref().filter(|path| !path.is_empty()) {
            validate_repo_path("path", path)?;
        }
        Ok(())
    }
}

/// An interactive git action initiated by the owner from the side panel
/// (ADR-0078).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum OwnerGitAction {
    StageFiles {
        #[serde(default)]
        paths: Vec<String>,
        #[serde(default)]
        all: bool,
    },
    UnstageFiles {
        #[serde(default)]
        paths: Vec<String>,
        #[serde(default)]
        all: bool,
    },
    StageHunk {
        path: String,
        patch: String,
    },
    UnstageHunk {
        path: String,
        patch: String,
    },
    DiscardFiles {
        paths: Vec<String>,
        #[serde(default)]
        include_untracked: bool,
    },
    DiscardHunk {
        path: String,
        patch: String,
    },
    Commit {
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        amend: bool,
        #[serde(default)]
        all: bool,
    },
    SwitchBranch {
        target: String,
        #[serde(default)]
        create: bool,
        #[serde(default)]
        start: Option<String>,
        #[serde(default)]
        detach: bool,
    },
    CheckoutRemoteBranch {
        remote_ref: String,
        #[serde(default)]
        local_name: Option<String>,
    },
    CreateBranch {
        name: String,
        #[serde(default)]
        start: Option<String>,
        #[serde(default)]
        switch_to: bool,
    },
    RenameBranch {
        name: String,
        new_name: String,
    },
    DeleteBranch {
        name: String,
    },
    CreateTag {
        name: String,
        #[serde(default)]
        target: Option<String>,
        #[serde(default)]
        message: Option<String>,
    },
    DeleteTag {
        name: String,
    },
    StashPush {
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        include_untracked: bool,
    },
    StashApply {
        index: u32,
    },
    StashPop {
        index: u32,
    },
    StashDrop {
        index: u32,
    },
    Fetch {
        #[serde(default)]
        remote: Option<String>,
        #[serde(default)]
        prune: bool,
    },
    Pull {
        #[serde(default)]
        remote: Option<String>,
        integrate: Integrate,
        #[serde(default)]
        prune: bool,
    },
    Push {
        #[serde(default)]
        remote: Option<String>,
        #[serde(default)]
        branch: Option<String>,
        #[serde(default)]
        set_upstream: bool,
        #[serde(default)]
        force_with_lease: bool,
        #[serde(default)]
        tags: bool,
    },
    Merge {
        branch: String,
        #[serde(default)]
        mode: Option<MergeMode>,
        #[serde(default)]
        message: Option<String>,
    },
    Rebase {
        onto: String,
        #[serde(default)]
        autosquash: bool,
    },
    CherryPick {
        commit: String,
        #[serde(default)]
        no_commit: bool,
    },
    Revert {
        commit: String,
        #[serde(default)]
        no_commit: bool,
    },
    ResolveConflict {
        path: String,
        #[serde(default)]
        take: Option<Side>,
        #[serde(default)]
        hunks: Option<Vec<HunkChoice>>,
        #[serde(default)]
        deleted: Option<bool>,
        #[serde(default)]
        edited: bool,
    },
    OperationControl {
        action: OperationAction,
    },
}

impl OwnerGitAction {
    /// Short `snake_case` action name recorded in `tools_program_operations`.
    pub(crate) fn action_code(&self) -> &'static str {
        match self {
            Self::StageFiles { .. } => "stage_files",
            Self::UnstageFiles { .. } => "unstage_files",
            Self::StageHunk { .. } => "stage_hunk",
            Self::UnstageHunk { .. } => "unstage_hunk",
            Self::DiscardFiles { .. } => "discard_files",
            Self::DiscardHunk { .. } => "discard_hunk",
            Self::Commit { amend: true, .. } => "commit_amend",
            Self::Commit { amend: false, .. } => "commit",
            Self::SwitchBranch { .. } => "switch_branch",
            Self::CheckoutRemoteBranch { .. } => "checkout_remote_branch",
            Self::CreateBranch { .. } => "create_branch",
            Self::RenameBranch { .. } => "rename_branch",
            Self::DeleteBranch { .. } => "delete_branch",
            Self::CreateTag { .. } => "create_tag",
            Self::DeleteTag { .. } => "delete_tag",
            Self::StashPush { .. } => "stash_push",
            Self::StashApply { .. } => "stash_apply",
            Self::StashPop { .. } => "stash_pop",
            Self::StashDrop { .. } => "stash_drop",
            Self::Fetch { .. } => "fetch",
            Self::Pull { .. } => "pull",
            Self::Push { .. } => "push",
            Self::Merge { .. } => "merge",
            Self::Rebase { .. } => "rebase",
            Self::CherryPick { .. } => "cherry_pick",
            Self::Revert { .. } => "revert",
            Self::ResolveConflict { .. } => "resolve_conflict",
            Self::OperationControl { .. } => "operation_control",
        }
    }

    fn validate_action(&self) -> Result<(), InputRefusal> {
        match self {
            Self::StageFiles { .. }
            | Self::UnstageFiles { .. }
            | Self::StageHunk { .. }
            | Self::UnstageHunk { .. }
            | Self::DiscardHunk { .. }
            | Self::DiscardFiles { .. }
            | Self::Commit { .. }
            | Self::StashPush { .. }
            | Self::StashApply { .. }
            | Self::StashPop { .. }
            | Self::StashDrop { .. } => self.validate_working_tree_action(),
            _ => self.validate_branch_and_history_action(),
        }
    }

    fn validate_working_tree_action(&self) -> Result<(), InputRefusal> {
        match self {
            Self::StageFiles { paths, all } | Self::UnstageFiles { paths, all } => {
                if (!*all && paths.is_empty()) || (*all && !paths.is_empty()) {
                    return Err(InputRefusal::new("action"));
                }
                validate_path_list(paths, false)
            }
            Self::StageHunk { path, patch }
            | Self::UnstageHunk { path, patch }
            | Self::DiscardHunk { path, patch } => {
                validate_repo_path("path", path)?;
                if patch.is_empty() || patch.len() > MAX_PATCH_BYTES || patch.contains('\0') {
                    return Err(InputRefusal::new("patch"));
                }
                Ok(())
            }
            Self::DiscardFiles { paths, .. } => validate_path_list(paths, true),
            Self::Commit { message, .. } | Self::StashPush { message, .. } => {
                validate_message("message", message.as_deref())
            }
            Self::StashApply { index } | Self::StashPop { index } | Self::StashDrop { index } => {
                if *index > 10_000 {
                    return Err(InputRefusal::new("index"));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn validate_branch_and_history_action(&self) -> Result<(), InputRefusal> {
        match self {
            Self::SwitchBranch { target, start, .. } => {
                validate_revision("target", target)?;
                validate_optional_revision("start", start.as_deref())
            }
            Self::CheckoutRemoteBranch {
                remote_ref,
                local_name,
            } => {
                validate_revision("remoteRef", remote_ref)?;
                validate_optional_revision("localName", local_name.as_deref())
            }
            Self::CreateBranch { name, start, .. } => {
                validate_revision("name", name)?;
                validate_optional_revision("start", start.as_deref())
            }
            Self::RenameBranch { name, new_name } => {
                validate_revision("name", name)?;
                validate_revision("newName", new_name)
            }
            Self::DeleteBranch { name } | Self::DeleteTag { name } => {
                validate_revision("name", name)
            }
            Self::CreateTag {
                name,
                target,
                message,
            } => {
                validate_revision("name", name)?;
                validate_optional_revision("target", target.as_deref())?;
                validate_message("message", message.as_deref())
            }
            Self::Fetch { remote, .. } | Self::Pull { remote, .. } => {
                validate_optional_revision("remote", remote.as_deref())
            }
            Self::Push { remote, branch, .. } => {
                validate_optional_revision("remote", remote.as_deref())?;
                validate_optional_revision("branch", branch.as_deref())
            }
            Self::Merge {
                branch, message, ..
            } => {
                validate_revision("branch", branch)?;
                validate_message("message", message.as_deref())
            }
            Self::Rebase { onto, .. } => validate_revision("onto", onto),
            Self::CherryPick { commit, .. } | Self::Revert { commit, .. } => {
                validate_revision("commit", commit)
            }
            Self::ResolveConflict { path, hunks, .. } => {
                validate_repo_path("path", path)?;
                if let Some(hunks) = hunks {
                    if hunks.is_empty() || hunks.len() > MAX_ACTION_PATHS {
                        return Err(InputRefusal::new("hunks"));
                    }
                    for choice in hunks {
                        validate_message("hunks", choice.text.as_deref())?;
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

fn validate_path_list(paths: &[String], require_non_empty: bool) -> Result<(), InputRefusal> {
    if (require_non_empty && paths.is_empty()) || paths.len() > MAX_ACTION_PATHS {
        return Err(InputRefusal::new("paths"));
    }
    for path in paths {
        validate_repo_path("paths", path)?;
    }
    Ok(())
}

fn validate_optional_revision(
    field: &'static str,
    value: Option<&str>,
) -> Result<(), InputRefusal> {
    if let Some(rev) = value {
        validate_revision(field, rev)?;
    }
    Ok(())
}

/// The argument of `repository_action_preview`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ActionPreviewRef {
    pub(crate) project_id: ProjectId,
    pub(crate) action: OwnerGitAction,
}

impl Validate for ActionPreviewRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        self.action.validate_action()
    }
}

/// Confirmation supplied after reviewing an `ActionPreview`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ActionConfirmation {
    pub(crate) state: String,
    pub(crate) effects_digest: String,
}

/// The argument of `repository_action_run`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ActionRunRef {
    pub(crate) operation_id: ProgramOperationId,
    pub(crate) project_id: ProjectId,
    pub(crate) action: OwnerGitAction,
    #[serde(default)]
    pub(crate) confirmation: Option<ActionConfirmation>,
}

impl Validate for ActionRunRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        self.action.validate_action()?;
        if let Some(confirmation) = &self.confirmation {
            if confirmation.state.is_empty() || confirmation.state.len() > 2048 {
                return Err(InputRefusal::new("confirmation"));
            }
            if confirmation.effects_digest.is_empty() || confirmation.effects_digest.len() > 128 {
                return Err(InputRefusal::new("confirmation"));
            }
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
    /// Line stats for staged changes.
    pub(crate) staged_stats: Vec<FileStat>,
    /// Line stats for unstaged changes.
    pub(crate) unstaged_stats: Vec<FileStat>,
    /// Configured remotes with credentials redacted.
    pub(crate) remotes: Vec<Remote>,
    /// The effective push remote and credential-redacted push URL for the
    /// current branch, matching `GitPush::plan`, when one is configured.
    pub(crate) push_target: Option<Remote>,
}

/// A page of the current branch's history, newest first.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct History {
    pub(crate) commits: Vec<Commit>,
    /// Whether older commits follow.
    pub(crate) more: bool,
}

/// Result of `repository_history_query`.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryQueryResult {
    pub(crate) commits: Vec<Commit>,
    pub(crate) reflog: Vec<ReflogEntry>,
    pub(crate) head_oid: Option<String>,
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

/// Complete refs explorer overview: branches, remote branches, newest-first
/// tags, stashes, remotes and worktrees.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RefsOverview {
    pub(crate) branches: Vec<Ref>,
    pub(crate) remote_branches: Vec<Ref>,
    pub(crate) tags: Vec<Ref>,
    pub(crate) stashes: Vec<ReflogEntry>,
    pub(crate) remotes: Vec<Remote>,
    pub(crate) worktrees: Vec<Worktree>,
}

/// A commit with its files and their patches.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitView {
    pub(crate) commit: CommitDetails,
    pub(crate) files: Vec<FileStat>,
    pub(crate) patches: Vec<FilePatch>,
}

/// A stash entry with its changed files and patches.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StashView {
    pub(crate) entry: ReflogEntry,
    pub(crate) files: Vec<FileStat>,
    pub(crate) patches: Vec<FilePatch>,
}

/// Comparison between two revisions or between a revision and the work tree.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompareView {
    pub(crate) base: String,
    pub(crate) head: Option<String>,
    pub(crate) base_label: String,
    pub(crate) head_label: String,
    pub(crate) ahead: Vec<Commit>,
    pub(crate) behind: Vec<Commit>,
    pub(crate) files: Vec<FileStat>,
    pub(crate) patches: Vec<FilePatch>,
}

/// A blame run paired with its abbreviated commit hash.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BlameRunView {
    pub(crate) short: String,
    pub(crate) commit: BlameCommit,
    pub(crate) lines: Vec<BlameLine>,
}

/// Blame annotations of a file.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BlameView {
    pub(crate) path: String,
    pub(crate) revision: Option<String>,
    pub(crate) runs: Vec<BlameRunView>,
    pub(crate) min_timestamp: UtcMillis,
    pub(crate) max_timestamp: UtcMillis,
    pub(crate) total_lines: u32,
    pub(crate) binary: bool,
    pub(crate) truncated: bool,
}

/// Tracked files matching an optional filter query.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileTreeList {
    pub(crate) files: Vec<String>,
    pub(crate) total: u32,
}

/// Search matches across the repository.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GrepView {
    pub(crate) matches: Vec<Match>,
    pub(crate) truncated: bool,
}

/// A file's change in the work tree or the index.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileDiff {
    /// Its line counts, or none when it did not change.
    pub(crate) stat: Option<FileStat>,
    pub(crate) hunks: Vec<Hunk>,
    /// Whether this diff was synthesized from an untracked file.
    pub(crate) untracked: bool,
    /// Whether the file is binary.
    pub(crate) binary: bool,
    /// Whether the preview was truncated at the size limit.
    pub(crate) truncated: bool,
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

/// An effect of an owner-initiated git action shown in confirmation dialogs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub(crate) enum OwnerActionEffect {
    RunsFolderCode {
        name: String,
        source: String,
    },
    DiscardsWork {
        work: DiscardedWork,
        count: u32,
        items: Vec<String>,
        snapshot: bool,
    },
    RewritesPublished {
        count: u32,
        places: Vec<String>,
    },
    Network {
        endpoint: String,
    },
    DeletesRef {
        ref_kind: String,
        name: String,
    },
}

/// Preview of an owner action before running it.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActionPreview {
    pub(crate) state: String,
    pub(crate) effects: Vec<OwnerActionEffect>,
    pub(crate) effects_digest: String,
    pub(crate) needs_confirmation: bool,
}

/// Outcome of running an owner action.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(
    tag = "outcome",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub(crate) enum OwnerActionResult {
    Completed {
        head: Option<CommitLabel>,
        snapshot: Option<String>,
        count: Option<u32>,
    },
    Stopped {
        operation: Option<OperationView>,
        conflicts: u32,
    },
    NeedsConfirmation {
        preview: ActionPreview,
    },
    Refused {
        detail: String,
    },
}

/// What the channel `follow_repository` carries. More kinds of updates may
/// come, by addition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RepositoryUpdate {
    /// One of the plugin's tools or owner actions changed a repository, so a
    /// panel reads its own again.
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
    /// Changing a folder outside the project needs a write grant.
    #[error("changing a folder outside the project needs a write grant")]
    WriteNotAllowed,
    /// An owner operation with this ID was already journaled.
    #[error("this operation has already been run")]
    AlreadyRun,
    /// The repository changed after the preview was confirmed.
    #[error("the repository changed before the action ran")]
    RepositoryChanged,
    /// git needs the owner's name and email to create a commit.
    #[error("git has no user name or email configured")]
    IdentityMissing,
    /// git refused the operation and left the repository unchanged.
    #[error("git refused the operation")]
    OperationRefused,
    /// An argument to the command was invalid for the current repository state.
    #[error("the action's arguments were invalid")]
    InvalidArgument,
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
            Self::WriteNotAllowed => "git.write_not_allowed",
            Self::AlreadyRun => "git.already_run",
            Self::RepositoryChanged => "git.repository_changed",
            Self::IdentityMissing => "git.identity_missing",
            Self::OperationRefused => "git.operation_refused",
            Self::InvalidArgument => "git.invalid_argument",
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
            GitFailure::WriteNotAllowed { .. } => Self::WriteNotAllowed,
            GitFailure::AlreadyRun => Self::AlreadyRun,
            GitFailure::RepositoryChanged => Self::RepositoryChanged,
            GitFailure::IdentityMissing => Self::IdentityMissing,
            GitFailure::Refused { .. } => Self::OperationRefused,
            GitFailure::InvalidArgument { .. } => Self::InvalidArgument,
            GitFailure::UnknownRevision { .. } => Self::UnknownCommit,
            GitFailure::Locked { .. } => Self::Locked,
            GitFailure::TimedOut => Self::TimedOut,
            GitFailure::Failed { .. } => Self::GitFailed,
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

/// `repository_history_query`: filtered topological commit history or reflog.
#[derive(Debug)]
pub(crate) enum RepositoryHistoryQueryCommand {}

impl CommandSpec for RepositoryHistoryQueryCommand {
    const NAME: &'static str = "repository_history_query";
    const DOCS: &'static str =
        "Filtered commit history in topological order across branches, a ref, or the reflog.";
    type Input = HistoryQueryRef;
    type Output = HistoryQueryResult;
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

/// `repository_refs`: branches, remote branches, tags, stashes, remotes and worktrees.
#[derive(Debug)]
pub(crate) enum RepositoryRefsCommand {}

impl CommandSpec for RepositoryRefsCommand {
    const NAME: &'static str = "repository_refs";
    const DOCS: &'static str =
        "Branches, remote branches, newest-first tags, stashes, remotes and worktrees.";
    type Input = ProjectRef;
    type Output = RefsOverview;
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

/// `repository_file_diff`: a changed or untracked file's patch.
#[derive(Debug)]
pub(crate) enum RepositoryFileDiffCommand {}

impl CommandSpec for RepositoryFileDiffCommand {
    const NAME: &'static str = "repository_file_diff";
    const DOCS: &'static str = "A file's unstaged, staged or untracked change.";
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

/// `repository_stash_show`: a stash entry's changed files and patches.
#[derive(Debug)]
pub(crate) enum RepositoryStashShowCommand {}

impl CommandSpec for RepositoryStashShowCommand {
    const NAME: &'static str = "repository_stash_show";
    const DOCS: &'static str = "A stash entry with its changed files and their patches.";
    type Input = StashShowRef;
    type Output = StashView;
    type Error = GitError;
}

/// `repository_compare`: compare two revisions or a revision against the work tree.
#[derive(Debug)]
pub(crate) enum RepositoryCompareCommand {}

impl CommandSpec for RepositoryCompareCommand {
    const NAME: &'static str = "repository_compare";
    const DOCS: &'static str =
        "Compare two branches, tags or commits, or compare a revision against the work tree.";
    type Input = CompareRef;
    type Output = CompareView;
    type Error = GitError;
}

/// `repository_blame`: per-line blame annotations for a file.
#[derive(Debug)]
pub(crate) enum RepositoryBlameCommand {}

impl CommandSpec for RepositoryBlameCommand {
    const NAME: &'static str = "repository_blame";
    const DOCS: &'static str = "Per-line blame annotations for a file in the work tree or a ref.";
    type Input = BlameRef;
    type Output = BlameView;
    type Error = GitError;
}

/// `repository_tree`: tracked file paths matching a filter query.
#[derive(Debug)]
pub(crate) enum RepositoryTreeCommand {}

impl CommandSpec for RepositoryTreeCommand {
    const NAME: &'static str = "repository_tree";
    const DOCS: &'static str = "Tracked file paths in the repository, optionally filtered.";
    type Input = TreeQueryRef;
    type Output = FileTreeList;
    type Error = GitError;
}

/// `repository_grep`: search file contents in the work tree or at a revision.
#[derive(Debug)]
pub(crate) enum RepositoryGrepCommand {}

impl CommandSpec for RepositoryGrepCommand {
    const NAME: &'static str = "repository_grep";
    const DOCS: &'static str = "Search file contents in the work tree or at a revision.";
    type Input = GrepQueryRef;
    type Output = GrepView;
    type Error = GitError;
}

/// `repository_action_preview`: preview the effects and confirmation needs of an owner action.
#[derive(Debug)]
pub(crate) enum RepositoryActionPreviewCommand {}

impl CommandSpec for RepositoryActionPreviewCommand {
    const NAME: &'static str = "repository_action_preview";
    const DOCS: &'static str = "Preview an owner git action's hooks, discarded work and effects.";
    type Input = ActionPreviewRef;
    type Output = ActionPreview;
    type Error = GitError;
}

/// `repository_action_run`: execute a journaled owner git action (ADR-0078).
#[derive(Debug)]
pub(crate) enum RepositoryActionRunCommand {}

impl CommandSpec for RepositoryActionRunCommand {
    const NAME: &'static str = "repository_action_run";
    const DOCS: &'static str = "Run an owner git action from the side panel under core journaling.";
    type Input = ActionRunRef;
    type Output = OwnerActionResult;
    type Error = GitError;
}

/// `follow_repository`: a note after each change that the plugin's tools
/// or owner actions make.
#[derive(Debug)]
pub(crate) enum FollowRepositoryChannel {}

impl ChannelSpec for FollowRepositoryChannel {
    const NAME: &'static str = "follow_repository";
    const DOCS: &'static str = "A note after each change that the plugin's tools or owner actions make in a repository, until the plugin stops.";
    type Input = ();
    type Item = RepositoryUpdate;
    type Error = GitError;
}

/// The plugin's commands, its channel and the limits that the UI reads.
pub(crate) fn interface() -> Interface {
    Interface::new()
        .command::<RepositoryOverviewCommand>()
        .command::<RepositoryHistoryCommand>()
        .command::<RepositoryHistoryQueryCommand>()
        .command::<RepositoryBranchesCommand>()
        .command::<RepositoryRefsCommand>()
        .command::<RepositoryCommitCommand>()
        .command::<RepositoryFileDiffCommand>()
        .command::<RepositoryConflictsCommand>()
        .command::<RepositoryStashShowCommand>()
        .command::<RepositoryCompareCommand>()
        .command::<RepositoryBlameCommand>()
        .command::<RepositoryTreeCommand>()
        .command::<RepositoryGrepCommand>()
        .command::<RepositoryActionPreviewCommand>()
        .command::<RepositoryActionRunCommand>()
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

    use super::{
        ActionPreviewRef, CommitRef, CompareMode, CompareRef, FileRef, GrepQueryRef, OwnerGitAction,
    };

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
            ignore_whitespace: false,
            context_lines: None,
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

    #[test]
    fn compare_grep_and_owner_actions_validate_their_inputs() {
        let compare = CompareRef {
            project_id: project(),
            base: "main".to_owned(),
            head: "WORKTREE".to_owned(),
            mode: CompareMode::ThreeDot,
            ignore_whitespace: false,
        };
        assert_eq!(compare.validate(), Ok(()));

        let bad_compare = CompareRef {
            base: "--exec".to_owned(),
            ..compare
        };
        assert_eq!(bad_compare.validate().unwrap_err().field, "base");

        let bad_grep = GrepQueryRef {
            project_id: project(),
            pattern: String::new(),
            revision: None,
            case_sensitive: false,
            fixed_strings: true,
            path: None,
        };
        assert_eq!(bad_grep.validate().unwrap_err().field, "pattern");

        let empty_stage = ActionPreviewRef {
            project_id: project(),
            action: OwnerGitAction::StageFiles {
                paths: Vec::new(),
                all: false,
            },
        };
        assert_eq!(empty_stage.validate().unwrap_err().field, "action");

        let bad_hunk = ActionPreviewRef {
            project_id: project(),
            action: OwnerGitAction::StageHunk {
                path: "../escape.rs".to_owned(),
                patch: "@@ -1 +1 @@\n-a\n+b\n".to_owned(),
            },
        };
        assert_eq!(bad_hunk.validate().unwrap_err().field, "path");
    }
}
