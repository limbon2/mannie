//! Journaled owner-initiated Git actions for the side panel (ADR-0078).
//!
//! Every action plans through the same safety checks as the agent tools
//! (`hooks`, `configured_programs`, `discarded_changes`, `discarded_commits`,
//! `published`, `remote_endpoints`), enforces server-side confirmation under
//! [`RepositoryLocks`] whenever hooks, repo-configured programs, discarded
//! work, published rewrites, or ref deletions are involved, and journals the
//! run through `processes::begin_owner_operation` before executing git.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use mannie_plugin_sdk::contracts::{DiscardedWork, MAX_EFFECT_ITEMS, ProgramEffect, ProjectId};
use mannie_plugin_sdk::services::processes::{OwnerOperationOutcome, OwnerOperationRequest};

use crate::arguments::{is_object_id, pathspec, pathspecs, repository_path};
use crate::changes::{
    Acting, ChangeTool, Changed, Plan, RepositoryLocks, branch_tip, changed_paths,
    check_branch_name, fnv1a, repository_state,
};
use crate::commands::{
    ActionPreview, ActionPreviewRef, ActionRunRef, GitError, OwnerActionEffect, OwnerActionResult,
    OwnerGitAction,
};
use crate::describe::{label_commits, operation_view};
use crate::effects::{Change, configured_programs, discarded_changes, hooks};
use crate::failure::GitFailure;
use crate::git::{GIT, Git};
use crate::layout::is_inside;
use crate::outcome::{head_commit, refusal};
use crate::parse::diff::{parse_hunks, split_patch};
use crate::tools::Reading;
use crate::tools::apply::{ApplyArguments, GitCherryPick, GitRevert};
use crate::tools::branch::{BranchAction, BranchArguments, GitBranch};
use crate::tools::commit::{CommitArguments, GitCommit, check_identity, check_message};
use crate::tools::conflicts::conflicted_files;
use crate::tools::diff::PATCH_OPTIONS;
use crate::tools::fetch::{FetchArguments, GitFetch};
use crate::tools::merge::{GitMerge, MergeArguments};
use crate::tools::operation::{GitOperation, OperationArguments};
use crate::tools::push::{GitPush, PushArguments};
use crate::tools::rebase::{GitRebase, RebaseArguments};
use crate::tools::resolve::{GitResolve, ResolveArguments};
use crate::tools::stage::{GitStage, StageArguments};
use crate::tools::stash::{GitStash, StashAction, StashArguments};
use crate::tools::switch::{GitSwitch, SwitchArguments};

use super::{Changes, project_reading};

/// Preview an owner action's effects and whether confirmation is needed.
pub(crate) async fn action_preview(
    git: Git,
    input: ActionPreviewRef,
) -> Result<ActionPreview, GitError> {
    let reading = project_reading(&git, &input.project_id).await?;
    let planned = plan_action(&reading, &input.action).await?;
    let built = build_preview(&reading, &input.project_id, &input.action, &planned).await?;
    Ok(built.preview)
}

/// Run an owner action under `RepositoryLocks` and core owner-operation
/// journaling (ADR-0078).
pub(crate) async fn action_run(
    git: Git,
    locks: RepositoryLocks,
    changes: Changes,
    input: ActionRunRef,
) -> Result<OwnerActionResult, GitError> {
    let initial_reading = project_reading(&git, &input.project_id).await?;
    let _guard = locks.lock(&initial_reading.layout.common_dir).await;

    // Re-open reading under the repository lock so state and effects match
    // what executes.
    let reading = project_reading(&git, &input.project_id).await?;
    let planned = plan_action(&reading, &input.action).await?;
    let built = build_preview(&reading, &input.project_id, &input.action, &planned).await?;

    if built.preview.needs_confirmation {
        match &input.confirmation {
            None => {
                return Ok(OwnerActionResult::NeedsConfirmation {
                    preview: built.preview,
                });
            }
            Some(confirmation) => {
                let state_matches = confirmation.state == built.preview.state;
                let effects_match = confirmation.effects_digest == built.preview.effects_digest;
                if !state_matches || !effects_match {
                    return Err(GitError::RepositoryChanged);
                }
            }
        }
    }

    let write_folders = write_folders_for(&reading);
    let journal_effects = sanitize_effects_for_journal(&built.program_effects);
    let request = OwnerOperationRequest::new(
        input.operation_id,
        input.project_id,
        GIT,
        input.action.action_code(),
        write_folders,
    )
    .with_arguments(planned.command.clone())
    .with_effects(journal_effects);

    let session = git
        .begin_owner_operation(&reading.layout.top, request)
        .await?;
    let acting = Acting {
        reading,
        owner_operation: session.ticket(),
        last_exit_code: Some(session.exit_code_tracker()),
    };

    let executed = execute_planned(&acting, planned.runner).await;
    changes.notify();

    match executed {
        Ok(result) => {
            let fallback_code = match &result {
                OwnerActionResult::Stopped { .. } => 1,
                _ => 0,
            };
            let exit_code = session.last_exit_code().or(Some(fallback_code));
            session
                .finish(OwnerOperationOutcome::Completed { exit_code })
                .await;
            Ok(result)
        }
        Err(GitFailure::TimedOut) => {
            session.finish(OwnerOperationOutcome::Uncertain).await;
            Err(GitError::TimedOut)
        }
        Err(GitFailure::Refused { message, .. }) => {
            session
                .finish(OwnerOperationOutcome::Failed {
                    code: "operation_refused".to_owned(),
                })
                .await;
            Ok(OwnerActionResult::Refused { detail: message })
        }
        Err(failure) => {
            let code = failure.code().to_owned();
            session.finish(OwnerOperationOutcome::Failed { code }).await;
            Err(GitError::from(failure))
        }
    }
}

/// Folders that the core verifies write access for when journaling an owner
/// operation: the repository top plus any external git directories.
fn write_folders_for(reading: &Reading) -> Vec<PathBuf> {
    let layout = &reading.layout;
    let mut folders = vec![layout.top.clone()];
    for candidate in [&layout.git_dir, &layout.common_dir] {
        if !is_inside(candidate, &layout.top) && !folders.contains(candidate) {
            folders.push(candidate.clone());
        }
    }
    folders
}

/// Internal preview output pairing the UI `ActionPreview` with core
/// `ProgramEffect` records for the journal.
struct BuiltPreview {
    preview: ActionPreview,
    program_effects: Vec<ProgramEffect>,
}

async fn build_preview(
    reading: &Reading,
    project_id: &ProjectId,
    action: &OwnerGitAction,
    planned: &PlannedOwnerAction,
) -> Result<BuiltPreview, GitFailure> {
    let mut state = repository_state(reading, &planned.tips).await?;
    if !planned.fingerprint_paths.is_empty() {
        let files_digest =
            fingerprint_worktree_paths(&reading.layout.top, &planned.fingerprint_paths)?;
        state = format!("{state}; files {files_digest}");
    }

    let mut program_effects = hooks(reading, planned.change).await?;
    program_effects.extend(configured_programs(&reading.settings, planned.change));
    program_effects.extend(planned.effects.iter().cloned());

    let mut effects: Vec<OwnerActionEffect> = program_effects.iter().map(to_owner_effect).collect();
    effects.extend(planned.extra_owner_effects.iter().cloned());

    let digest_payload = serde_json::to_vec(&(project_id, action, &effects)).unwrap_or_default();
    let effects_digest = format!("{:016x}", fnv1a(&digest_payload));

    let requires_confirmation = planned.needs_confirmation_always
        || effects.iter().any(|effect| {
            matches!(
                effect,
                OwnerActionEffect::RunsFolderCode { .. }
                    | OwnerActionEffect::DiscardsWork { .. }
                    | OwnerActionEffect::RewritesPublished { .. }
                    | OwnerActionEffect::DeletesRef { .. }
            )
        });

    Ok(BuiltPreview {
        preview: ActionPreview {
            state,
            effects,
            effects_digest,
            needs_confirmation: requires_confirmation,
        },
        program_effects,
    })
}

/// Strips free-form commit subjects and stash messages from `ProgramEffect`
/// records before persisting them in the SQLite journal, keeping only object
/// IDs, selectors, paths, hook/program names, redacted endpoints, and counts.
fn sanitize_effects_for_journal(effects: &[ProgramEffect]) -> Vec<ProgramEffect> {
    effects
        .iter()
        .map(|effect| match effect {
            ProgramEffect::DiscardsWork {
                work: DiscardedWork::Commits,
                count,
                items,
                snapshot,
            } => ProgramEffect::DiscardsWork {
                work: DiscardedWork::Commits,
                count: *count,
                items: items
                    .iter()
                    .filter_map(|item| item.split_whitespace().next().map(str::to_owned))
                    .collect(),
                snapshot: *snapshot,
            },
            ProgramEffect::DiscardsWork {
                work: DiscardedWork::StashEntries,
                count,
                items,
                snapshot,
            } => ProgramEffect::DiscardsWork {
                work: DiscardedWork::StashEntries,
                count: *count,
                items: items
                    .iter()
                    .filter_map(|item| item.split(':').next().map(|sel| sel.trim().to_owned()))
                    .collect(),
                snapshot: *snapshot,
            },
            other => other.clone(),
        })
        .collect()
}

/// Streams the metadata and contents of affected worktree files into a 64-bit
/// FNV-1a digest so editing an untracked or modified file between preview and
/// confirmation invalidates the confirmation state token.
fn fingerprint_worktree_paths(top: &Path, paths: &[String]) -> Result<String, GitFailure> {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut sorted: Vec<&str> = paths.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.dedup();

    let canonical_top =
        std::fs::canonicalize(top).map_err(|_error| GitFailure::ReadNotAllowed {
            folder: top.display().to_string(),
        })?;

    let mut hash = OFFSET;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(PRIME);
        }
    };

    for relative in sorted {
        feed(relative.as_bytes());
        feed(&[0]);
        hash_single_worktree_path(top, &canonical_top, relative, &mut feed)?;
    }

    Ok(format!("{hash:016x}"))
}

fn hash_single_worktree_path(
    top: &Path,
    canonical_top: &Path,
    relative: &str,
    feed: &mut impl FnMut(&[u8]),
) -> Result<(), GitFailure> {
    let full = top.join(relative);
    let Ok(meta) = std::fs::symlink_metadata(&full) else {
        feed(b"missing\0");
        return Ok(());
    };
    if meta.file_type().is_symlink() {
        feed(b"symlink\0");
        if let Ok(target) = std::fs::read_link(&full) {
            feed(target.as_os_str().as_encoded_bytes());
        }
        feed(&[0]);
        return Ok(());
    }
    if !meta.is_file() {
        feed(b"other\0");
        return Ok(());
    }
    let Ok(canonical) = std::fs::canonicalize(&full) else {
        feed(b"unreadable\0");
        return Ok(());
    };
    if !is_inside(&canonical, canonical_top) {
        return Err(GitFailure::ReadNotAllowed {
            folder: full.display().to_string(),
        });
    }
    feed(b"file\0");
    feed(&meta.len().to_le_bytes());
    let mut file = std::fs::File::open(&canonical)
        .map_err(|error| GitFailure::failed(&format!("could not read {relative}: {error}")))?;
    let mut chunk = [0u8; 8192];
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|error| GitFailure::failed(&format!("could not read {relative}: {error}")))?;
        if read == 0 {
            break;
        }
        feed(&chunk[..read]);
    }
    feed(&[0]);
    Ok(())
}

fn to_owner_effect(effect: &ProgramEffect) -> OwnerActionEffect {
    match effect {
        ProgramEffect::RunsFolderCode { name, source } => OwnerActionEffect::RunsFolderCode {
            name: name.clone(),
            source: source.clone(),
        },
        ProgramEffect::DiscardsWork {
            work,
            count,
            items,
            snapshot,
        } => OwnerActionEffect::DiscardsWork {
            work: *work,
            count: *count,
            items: items.clone(),
            snapshot: *snapshot,
        },
        ProgramEffect::RewritesPublished { count, places } => {
            OwnerActionEffect::RewritesPublished {
                count: *count,
                places: places.clone(),
            }
        }
        ProgramEffect::Network { endpoint } => OwnerActionEffect::Network {
            endpoint: endpoint.clone(),
        },
    }
}

/// A planned owner action with its safety metadata and typed executor.
struct PlannedOwnerAction {
    change: Change,
    command: Vec<String>,
    effects: Vec<ProgramEffect>,
    extra_owner_effects: Vec<OwnerActionEffect>,
    tips: Vec<String>,
    /// Relative worktree paths whose exact metadata and contents are mixed
    /// into the preview state token so edits between preview and confirmation
    /// invalidate confirmation.
    fingerprint_paths: Vec<String>,
    needs_confirmation_always: bool,
    runner: Runner,
}

impl PlannedOwnerAction {
    fn from_tool_plan<P>(plan: Plan<P>, build: impl FnOnce(Plan<P>) -> Runner) -> Self {
        let change = plan.change;
        let command = plan.command.clone();
        let effects = plan.effects.clone();
        let tips = plan.tips.clone();
        Self {
            change,
            command,
            effects,
            extra_owner_effects: Vec::new(),
            tips,
            fingerprint_paths: Vec::new(),
            needs_confirmation_always: false,
            runner: build(plan),
        }
    }
}

/// Typed executor holding the exact planned change from [`plan_action`].
enum Runner {
    Stage(StageArguments, Plan<bool>),
    ApplyPatch {
        command: Vec<String>,
        patch: String,
        snapshot_label: Option<&'static str>,
    },
    DiscardFiles {
        tracked_specs: Vec<String>,
        untracked_specs: Vec<String>,
        snapshot_tracked: bool,
        count: u32,
    },
    Commit(CommitArguments, Plan<Option<String>>),
    Switch(SwitchArguments, Plan<()>),
    Branch(BranchArguments, Plan<()>),
    CreateTag {
        command: Vec<String>,
        message: Option<String>,
    },
    DeleteTag {
        command: Vec<String>,
    },
    Stash(StashArguments, Plan<()>),
    Fetch(FetchArguments, Plan<crate::tools::fetch::Fetching>),
    Push(PushArguments, Plan<crate::tools::push::Pushing>),
    Merge(MergeArguments, Plan<()>),
    Rebase(RebaseArguments, Plan<String>),
    CherryPick(ApplyArguments, Plan<()>),
    Revert(ApplyArguments, Plan<()>),
    Resolve(
        ResolveArguments,
        Plan<(String, crate::tools::resolve::Resolution)>,
    ),
    Operation(
        OperationArguments,
        Plan<crate::tools::operation::Continuing>,
    ),
}

/// Plans an owner action by delegating to the shared tool planners or
/// dedicated hunk/tag/discard planners.
async fn plan_action(
    reading: &Reading,
    action: &OwnerGitAction,
) -> Result<PlannedOwnerAction, GitFailure> {
    match action {
        OwnerGitAction::StageFiles { .. }
        | OwnerGitAction::UnstageFiles { .. }
        | OwnerGitAction::StageHunk { .. }
        | OwnerGitAction::UnstageHunk { .. }
        | OwnerGitAction::DiscardHunk { .. }
        | OwnerGitAction::DiscardFiles { .. }
        | OwnerGitAction::Commit { .. } => plan_working_tree_action(reading, action).await,
        OwnerGitAction::SwitchBranch { .. }
        | OwnerGitAction::CheckoutRemoteBranch { .. }
        | OwnerGitAction::CreateBranch { .. }
        | OwnerGitAction::RenameBranch { .. }
        | OwnerGitAction::DeleteBranch { .. }
        | OwnerGitAction::CreateTag { .. }
        | OwnerGitAction::DeleteTag { .. } => plan_branch_and_tag_action(reading, action).await,
        OwnerGitAction::StashPush { .. }
        | OwnerGitAction::StashApply { .. }
        | OwnerGitAction::StashPop { .. }
        | OwnerGitAction::StashDrop { .. }
        | OwnerGitAction::Fetch { .. }
        | OwnerGitAction::Pull { .. }
        | OwnerGitAction::Push { .. } => plan_stash_and_remote_action(reading, action).await,
        _ => plan_history_and_conflict_action(reading, action).await,
    }
}

async fn plan_working_tree_action(
    reading: &Reading,
    action: &OwnerGitAction,
) -> Result<PlannedOwnerAction, GitFailure> {
    match action {
        OwnerGitAction::StageFiles { paths, all } => {
            let args = StageArguments {
                repository: None,
                paths: (!*all).then(|| paths.clone()),
                all: (*all).then_some(true),
                unstage: Some(false),
            };
            let plan = GitStage::plan(reading, &args).await?;
            Ok(PlannedOwnerAction::from_tool_plan(plan, |p| {
                Runner::Stage(args, p)
            }))
        }
        OwnerGitAction::UnstageFiles { paths, all } => {
            let args = StageArguments {
                repository: None,
                paths: (!*all).then(|| paths.clone()),
                all: (*all).then_some(true),
                unstage: Some(true),
            };
            let plan = GitStage::plan(reading, &args).await?;
            Ok(PlannedOwnerAction::from_tool_plan(plan, |p| {
                Runner::Stage(args, p)
            }))
        }
        OwnerGitAction::StageHunk { path, patch } => {
            plan_hunk(reading, path, patch, HunkAction::Stage).await
        }
        OwnerGitAction::UnstageHunk { path, patch } => {
            plan_hunk(reading, path, patch, HunkAction::Unstage).await
        }
        OwnerGitAction::DiscardHunk { path, patch } => {
            plan_hunk(reading, path, patch, HunkAction::Discard).await
        }
        OwnerGitAction::DiscardFiles {
            paths,
            include_untracked,
        } => plan_discard_files(reading, paths, *include_untracked).await,
        OwnerGitAction::Commit {
            message,
            amend,
            all,
        } => {
            let args = CommitArguments {
                repository: None,
                message: non_empty_trimmed(message.as_deref()),
                amend: Some(*amend),
                all: Some(*all),
                allow_empty: Some(false),
                fixup: None,
            };
            let plan = GitCommit::plan(reading, &args).await?;
            Ok(PlannedOwnerAction::from_tool_plan(plan, |p| {
                Runner::Commit(args, p)
            }))
        }
        _ => unreachable!("handled by plan_action dispatch"),
    }
}

async fn plan_branch_and_tag_action(
    reading: &Reading,
    action: &OwnerGitAction,
) -> Result<PlannedOwnerAction, GitFailure> {
    match action {
        OwnerGitAction::SwitchBranch {
            target,
            create,
            start,
            detach,
        } => {
            let args = SwitchArguments {
                repository: None,
                target: target.clone(),
                create: Some(*create),
                start: start.clone(),
                detach: Some(*detach),
            };
            let plan = GitSwitch::plan(reading, &args).await?;
            Ok(PlannedOwnerAction::from_tool_plan(plan, |p| {
                Runner::Switch(args, p)
            }))
        }
        OwnerGitAction::CheckoutRemoteBranch {
            remote_ref,
            local_name,
        } => plan_checkout_remote_branch(reading, remote_ref, local_name.as_deref()).await,
        OwnerGitAction::CreateBranch {
            name,
            start,
            switch_to,
        } => {
            if *switch_to {
                let args = SwitchArguments {
                    repository: None,
                    target: name.clone(),
                    create: Some(true),
                    start: start.clone(),
                    detach: Some(false),
                };
                let plan = GitSwitch::plan(reading, &args).await?;
                Ok(PlannedOwnerAction::from_tool_plan(plan, |p| {
                    Runner::Switch(args, p)
                }))
            } else {
                let args = BranchArguments {
                    repository: None,
                    action: BranchAction::Create,
                    name: name.clone(),
                    start: start.clone(),
                    new_name: None,
                    upstream: None,
                };
                let plan = GitBranch::plan(reading, &args).await?;
                Ok(PlannedOwnerAction::from_tool_plan(plan, |p| {
                    Runner::Branch(args, p)
                }))
            }
        }
        OwnerGitAction::RenameBranch { name, new_name } => {
            let args = BranchArguments {
                repository: None,
                action: BranchAction::Rename,
                name: name.clone(),
                start: None,
                new_name: Some(new_name.clone()),
                upstream: None,
            };
            let plan = GitBranch::plan(reading, &args).await?;
            Ok(PlannedOwnerAction::from_tool_plan(plan, |p| {
                Runner::Branch(args, p)
            }))
        }
        OwnerGitAction::DeleteBranch { name } => {
            let args = BranchArguments {
                repository: None,
                action: BranchAction::Delete,
                name: name.clone(),
                start: None,
                new_name: None,
                upstream: None,
            };
            let plan = GitBranch::plan(reading, &args).await?;
            let mut planned = PlannedOwnerAction::from_tool_plan(plan, |p| Runner::Branch(args, p));
            planned.needs_confirmation_always = true;
            planned
                .extra_owner_effects
                .push(OwnerActionEffect::DeletesRef {
                    ref_kind: "branch".to_owned(),
                    name: name.clone(),
                });
            Ok(planned)
        }
        OwnerGitAction::CreateTag {
            name,
            target,
            message,
        } => plan_create_tag(reading, name, target.as_deref(), message.as_deref()).await,
        OwnerGitAction::DeleteTag { name } => plan_delete_tag(reading, name).await,
        _ => unreachable!("handled by plan_action dispatch"),
    }
}

async fn plan_stash_and_remote_action(
    reading: &Reading,
    action: &OwnerGitAction,
) -> Result<PlannedOwnerAction, GitFailure> {
    match action {
        OwnerGitAction::StashPush {
            message,
            include_untracked,
        } => {
            let args = StashArguments {
                repository: None,
                action: StashAction::Push,
                message: non_empty_trimmed(message.as_deref()),
                include_untracked: Some(*include_untracked),
                entry: None,
            };
            let plan = GitStash::plan(reading, &args).await?;
            Ok(PlannedOwnerAction::from_tool_plan(plan, |p| {
                Runner::Stash(args, p)
            }))
        }
        OwnerGitAction::StashApply { index }
        | OwnerGitAction::StashPop { index }
        | OwnerGitAction::StashDrop { index } => {
            let stash_action = match action {
                OwnerGitAction::StashApply { .. } => StashAction::Apply,
                OwnerGitAction::StashPop { .. } => StashAction::Pop,
                _ => StashAction::Drop,
            };
            let args = StashArguments {
                repository: None,
                action: stash_action,
                message: None,
                include_untracked: None,
                entry: Some(*index),
            };
            let plan = GitStash::plan(reading, &args).await?;
            let mut planned = PlannedOwnerAction::from_tool_plan(plan, |p| Runner::Stash(args, p));
            planned.needs_confirmation_always = matches!(action, OwnerGitAction::StashDrop { .. });
            Ok(planned)
        }
        OwnerGitAction::Fetch { remote, prune } => {
            let args = FetchArguments {
                repository: None,
                remote: remote.clone(),
                branch: None,
                prune: Some(*prune),
                integrate: None,
            };
            let plan = GitFetch::plan(reading, &args).await?;
            Ok(PlannedOwnerAction::from_tool_plan(plan, |p| {
                Runner::Fetch(args, p)
            }))
        }
        OwnerGitAction::Pull {
            remote,
            integrate,
            prune,
        } => {
            let args = FetchArguments {
                repository: None,
                remote: remote.clone(),
                branch: None,
                prune: Some(*prune),
                integrate: Some(*integrate),
            };
            let plan = GitFetch::plan(reading, &args).await?;
            Ok(PlannedOwnerAction::from_tool_plan(plan, |p| {
                Runner::Fetch(args, p)
            }))
        }
        OwnerGitAction::Push {
            remote,
            branch,
            set_upstream,
            force_with_lease,
            tags,
        } => {
            let args = PushArguments {
                repository: None,
                remote: remote.clone(),
                branch: branch.clone(),
                set_upstream: Some(*set_upstream),
                force_with_lease: Some(*force_with_lease),
                tags: Some(*tags),
            };
            let plan = GitPush::plan(reading, &args).await?;
            Ok(PlannedOwnerAction::from_tool_plan(plan, |p| {
                Runner::Push(args, p)
            }))
        }
        _ => unreachable!("handled by plan_action dispatch"),
    }
}

async fn plan_history_and_conflict_action(
    reading: &Reading,
    action: &OwnerGitAction,
) -> Result<PlannedOwnerAction, GitFailure> {
    match action {
        OwnerGitAction::Merge {
            branch,
            mode,
            message,
        } => {
            let args = MergeArguments {
                repository: None,
                branch: branch.clone(),
                mode: *mode,
                message: non_empty_trimmed(message.as_deref()),
            };
            let plan = GitMerge::plan(reading, &args).await?;
            Ok(PlannedOwnerAction::from_tool_plan(plan, |p| {
                Runner::Merge(args, p)
            }))
        }
        OwnerGitAction::Rebase { onto, autosquash } => {
            let args = RebaseArguments {
                repository: None,
                onto: onto.clone(),
                upstream: None,
                update_refs: Some(false),
                autosquash: Some(*autosquash),
            };
            let plan = GitRebase::plan(reading, &args).await?;
            Ok(PlannedOwnerAction::from_tool_plan(plan, |p| {
                Runner::Rebase(args, p)
            }))
        }
        OwnerGitAction::CherryPick { commit, no_commit } => {
            let args = ApplyArguments {
                repository: None,
                commits: vec![commit.clone()],
                mainline: None,
                no_commit: Some(*no_commit),
            };
            let plan = GitCherryPick::plan(reading, &args).await?;
            Ok(PlannedOwnerAction::from_tool_plan(plan, |p| {
                Runner::CherryPick(args, p)
            }))
        }
        OwnerGitAction::Revert { commit, no_commit } => {
            let args = ApplyArguments {
                repository: None,
                commits: vec![commit.clone()],
                mainline: None,
                no_commit: Some(*no_commit),
            };
            let plan = GitRevert::plan(reading, &args).await?;
            Ok(PlannedOwnerAction::from_tool_plan(plan, |p| {
                Runner::Revert(args, p)
            }))
        }
        OwnerGitAction::ResolveConflict {
            path,
            take,
            hunks,
            deleted,
            edited,
        } => {
            let args = ResolveArguments {
                repository: None,
                path: path.clone(),
                take: *take,
                hunks: hunks.clone(),
                content: None,
                deleted: *deleted,
                edited: (*edited).then_some(true),
            };
            let plan = GitResolve::plan(reading, &args).await?;
            Ok(PlannedOwnerAction::from_tool_plan(plan, |p| {
                Runner::Resolve(args, p)
            }))
        }
        OwnerGitAction::OperationControl { action } => {
            let args = OperationArguments {
                repository: None,
                action: *action,
            };
            let plan = GitOperation::plan(reading, &args).await?;
            Ok(PlannedOwnerAction::from_tool_plan(plan, |p| {
                Runner::Operation(args, p)
            }))
        }
        _ => unreachable!("handled by plan_action dispatch"),
    }
}

fn non_empty_trimmed(text: Option<&str>) -> Option<String> {
    text.filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

#[derive(Debug, Clone, Copy)]
enum HunkAction {
    Stage,
    Unstage,
    Discard,
}

async fn plan_hunk(
    reading: &Reading,
    raw_path: &str,
    submitted_patch: &str,
    action: HunkAction,
) -> Result<PlannedOwnerAction, GitFailure> {
    let file_path = repository_path(&reading.layout.top, "path", raw_path)?;
    let verified_patch =
        resolve_verified_hunk_patch(reading, &file_path, submitted_patch, action).await?;

    let mut command = vec!["apply".to_owned(), "--whitespace=nowarn".to_owned()];
    let mut effects = Vec::new();
    let mut fingerprint_paths = Vec::new();
    let (change, snapshot_label, needs_confirmation_always) = match action {
        HunkAction::Stage => {
            command.push("--cached".to_owned());
            (Change::Stage, None, false)
        }
        HunkAction::Unstage => {
            command.push("--cached".to_owned());
            command.push("--reverse".to_owned());
            let partly =
                changed_paths(reading, &[pathspec(&file_path)], Changed::PartlyStaged).await?;
            let snapshot = !partly.is_empty();
            effects.extend(discarded_changes(&partly, true));
            if snapshot {
                fingerprint_paths.push(file_path.clone());
            }
            (
                Change::Unstage { snapshot },
                snapshot.then_some("unstaging hunk"),
                snapshot,
            )
        }
        HunkAction::Discard => {
            command.push("--reverse".to_owned());
            effects.extend(discarded_changes(std::slice::from_ref(&file_path), true));
            fingerprint_paths.push(file_path.clone());
            (
                Change::Restore { snapshot: true },
                Some("discarding hunk"),
                true,
            )
        }
    };
    command.push("-".to_owned());

    Ok(PlannedOwnerAction {
        change,
        command: command.clone(),
        effects,
        extra_owner_effects: Vec::new(),
        tips: Vec::new(),
        fingerprint_paths,
        needs_confirmation_always,
        runner: Runner::ApplyPatch {
            command,
            patch: verified_patch,
            snapshot_label,
        },
    })
}

/// Resolves the single hunk requested by `submitted_patch` against Git's live
/// diff for `file_path` and returns a patch constructed exclusively from Git's
/// own output, so untrusted patch bytes from the webview never reach `git apply`.
async fn resolve_verified_hunk_patch(
    reading: &Reading,
    file_path: &str,
    submitted_patch: &str,
    action: HunkAction,
) -> Result<String, GitFailure> {
    let submitted_hunks = parse_hunks(submitted_patch);
    let [requested] = submitted_hunks.as_slice() else {
        return Err(GitFailure::InvalidArgument {
            detail: "patch must contain exactly one unified diff hunk.".to_owned(),
        });
    };

    let mut diff_cmd = vec!["diff".to_owned()];
    if matches!(action, HunkAction::Unstage) {
        diff_cmd.push("--cached".to_owned());
    }
    diff_cmd.extend(PATCH_OPTIONS.iter().map(|option| (*option).to_owned()));
    diff_cmd.push("--ignore-submodules=dirty".to_owned());
    diff_cmd.push("--patch".to_owned());
    diff_cmd.push("--".to_owned());
    diff_cmd.push(pathspec(file_path));

    let live_diff = reading.git.output(reading.run(diff_cmd)).await?;
    if live_diff.cut {
        return Err(GitFailure::InvalidArgument {
            detail: "The file diff is too large to apply a single hunk safely.".to_owned(),
        });
    }

    let text = live_diff.text();
    let parts = split_patch(&text);
    let [file_part] = parts.as_slice() else {
        return Err(GitFailure::RepositoryChanged);
    };

    let (file_header, raw_hunks) = split_file_hunks(file_part);
    for raw_hunk in raw_hunks {
        let parsed = parse_hunks(raw_hunk);
        if parsed.as_slice() == [requested.clone()] {
            let mut verified = format!("{file_header}{raw_hunk}");
            if !verified.ends_with('\n') {
                verified.push('\n');
            }
            return Ok(verified);
        }
    }

    Err(GitFailure::RepositoryChanged)
}

/// Splits one `diff --git` file patch into its header lines and each raw
/// `@@ ...` hunk block, preserving Git's exact bytes.
fn split_file_hunks(file_part: &str) -> (&str, Vec<&str>) {
    let mut starts: Vec<usize> = Vec::new();
    let mut offset = 0;
    for line in file_part.split_inclusive('\n') {
        if line.starts_with("@@ ") {
            starts.push(offset);
        }
        offset += line.len();
    }
    let Some(&first) = starts.first() else {
        return (file_part, Vec::new());
    };
    let header = &file_part[..first];
    let mut hunks = Vec::with_capacity(starts.len());
    for (index, &start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(file_part.len());
        hunks.push(&file_part[start..end]);
    }
    (header, hunks)
}

/// Plans checking out an explicit remote-tracking branch (`refs/remotes/<ref>`)
/// into a new local tracking branch, refusing when a local branch with the
/// target name already exists instead of silently switching to it.
async fn plan_checkout_remote_branch(
    reading: &Reading,
    remote_ref: &str,
    local_name: Option<&str>,
) -> Result<PlannedOwnerAction, GitFailure> {
    let full_remote_ref = if remote_ref.starts_with("refs/remotes/") {
        remote_ref.to_owned()
    } else {
        format!("refs/remotes/{remote_ref}")
    };
    let short_remote = full_remote_ref
        .strip_prefix("refs/remotes/")
        .unwrap_or(remote_ref);

    let verified = reading
        .git
        .run(reading.run(["rev-parse", "--verify", "--quiet", &full_remote_ref]))
        .await?;
    let tip = verified.text().trim().to_owned();
    if !verified.succeeded() || !is_object_id(&tip) {
        return Err(GitFailure::UnknownRevision {
            revision: remote_ref.to_owned(),
        });
    }

    let derived_local =
        if let Some(name) = local_name.map(str::trim).filter(|name| !name.is_empty()) {
            name.to_owned()
        } else {
            let Some((_remote, branch)) = short_remote.split_once('/') else {
                return Err(GitFailure::InvalidArgument {
                    detail: format!(
                        "Remote branch {remote_ref:?} must name its remote, such as origin/main."
                    ),
                });
            };
            branch.to_owned()
        };

    check_branch_name(reading, "localName", &derived_local).await?;
    if branch_tip(reading, &derived_local).await?.is_some() {
        return Err(GitFailure::InvalidArgument {
            detail: format!("The branch {derived_local} exists already."),
        });
    }

    let command = vec![
        "switch".to_owned(),
        "--track".to_owned(),
        "-c".to_owned(),
        derived_local.clone(),
        full_remote_ref,
    ];
    let plan = Plan::new(Change::Switch, command, ()).with_tip(&tip);
    Ok(PlannedOwnerAction::from_tool_plan(plan, |p| {
        Runner::Switch(
            SwitchArguments {
                repository: None,
                target: derived_local,
                create: Some(true),
                start: None,
                detach: Some(false),
            },
            p,
        )
    }))
}

async fn plan_discard_files(
    reading: &Reading,
    raw_paths: &[String],
    include_untracked: bool,
) -> Result<PlannedOwnerAction, GitFailure> {
    let specs = pathspecs(&reading.layout.top, "paths", raw_paths)?;
    let mut clean_paths = Vec::with_capacity(raw_paths.len());
    for raw in raw_paths {
        clean_paths.push(repository_path(&reading.layout.top, "paths", raw)?);
    }

    let untracked_found = list_untracked_matching(reading, &specs).await?;
    let mut tracked_specs = Vec::new();
    let mut untracked_specs = Vec::new();
    for (clean, spec) in clean_paths.iter().zip(&specs) {
        if untracked_found.iter().any(|found| found == clean) {
            if include_untracked {
                untracked_specs.push(spec.clone());
            }
        } else {
            tracked_specs.push(spec.clone());
        }
    }

    if tracked_specs.is_empty() && untracked_specs.is_empty() {
        return Err(GitFailure::InvalidArgument {
            detail: "No matching files to discard.".to_owned(),
        });
    }

    let mut effects = Vec::new();
    let mut snapshot_tracked = false;
    let mut tracked_count = 0usize;
    if !tracked_specs.is_empty() {
        let changed = changed_paths(reading, &tracked_specs, Changed::Unstaged).await?;
        tracked_count = changed.len();
        snapshot_tracked = !changed.is_empty();
        effects.extend(discarded_changes(&changed, true));
    }

    let untracked_items: Vec<String> = if untracked_specs.is_empty() {
        Vec::new()
    } else {
        untracked_found
            .into_iter()
            .filter(|found| clean_paths.contains(found))
            .collect()
    };
    if !untracked_items.is_empty() {
        effects.push(ProgramEffect::DiscardsWork {
            work: DiscardedWork::UncommittedChanges,
            count: u32::try_from(untracked_items.len()).unwrap_or(u32::MAX),
            items: untracked_items
                .iter()
                .take(MAX_EFFECT_ITEMS)
                .cloned()
                .collect(),
            snapshot: false,
        });
    }

    let command = discard_representative_command(&tracked_specs, &untracked_specs);
    let count = u32::try_from(tracked_count + untracked_items.len()).unwrap_or(u32::MAX);

    Ok(PlannedOwnerAction {
        change: Change::Restore {
            snapshot: snapshot_tracked,
        },
        command,
        effects,
        extra_owner_effects: Vec::new(),
        tips: Vec::new(),
        fingerprint_paths: clean_paths,
        needs_confirmation_always: true,
        runner: Runner::DiscardFiles {
            tracked_specs,
            untracked_specs,
            snapshot_tracked,
            count,
        },
    })
}

async fn list_untracked_matching(
    reading: &Reading,
    specs: &[String],
) -> Result<Vec<String>, GitFailure> {
    let untracked_ran = reading
        .git
        .output(
            reading.run(
                [
                    vec![
                        "ls-files".to_owned(),
                        "--others".to_owned(),
                        "--exclude-standard".to_owned(),
                        "-z".to_owned(),
                        "--".to_owned(),
                    ],
                    specs.to_vec(),
                ]
                .concat(),
            ),
        )
        .await?;
    Ok(untracked_ran
        .output
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect())
}

fn discard_representative_command(
    tracked_specs: &[String],
    untracked_specs: &[String],
) -> Vec<String> {
    let mut command = Vec::new();
    if tracked_specs.is_empty() {
        command.extend(["clean".to_owned(), "-f".to_owned(), "--".to_owned()]);
        command.extend(untracked_specs.iter().cloned());
    } else {
        command.extend([
            "restore".to_owned(),
            "--worktree".to_owned(),
            "--".to_owned(),
        ]);
        command.extend(tracked_specs.iter().cloned());
    }
    command
}

async fn plan_create_tag(
    reading: &Reading,
    name: &str,
    target: Option<&str>,
    message: Option<&str>,
) -> Result<PlannedOwnerAction, GitFailure> {
    let valid = reading
        .git
        .run(reading.run(["check-ref-format", &format!("refs/tags/{name}")]))
        .await?;
    if !valid.succeeded() {
        return Err(GitFailure::InvalidArgument {
            detail: format!("Tag {name:?} is not a valid git tag name."),
        });
    }

    let existing = reading
        .git
        .run(reading.run([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/tags/{name}"),
        ]))
        .await?;
    if existing.succeeded() {
        return Err(GitFailure::InvalidArgument {
            detail: format!("Tag {name} already exists."),
        });
    }

    let target_oid = reading.commit("target", target.unwrap_or("HEAD")).await?;
    let trimmed_message = message
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned);

    let command = if let Some(msg) = &trimmed_message {
        check_identity(reading).await?;
        check_message("message", msg)?;
        vec![
            "tag".to_owned(),
            "-a".to_owned(),
            "-F".to_owned(),
            "-".to_owned(),
            "--".to_owned(),
            name.to_owned(),
            target_oid.clone(),
        ]
    } else {
        vec![
            "tag".to_owned(),
            "--".to_owned(),
            name.to_owned(),
            target_oid.clone(),
        ]
    };

    Ok(PlannedOwnerAction {
        change: Change::CreateTag,
        command: command.clone(),
        effects: Vec::new(),
        extra_owner_effects: Vec::new(),
        tips: vec![target_oid],
        fingerprint_paths: Vec::new(),
        needs_confirmation_always: false,
        runner: Runner::CreateTag {
            command,
            message: trimmed_message,
        },
    })
}

async fn plan_delete_tag(reading: &Reading, name: &str) -> Result<PlannedOwnerAction, GitFailure> {
    let existing = reading
        .git
        .run(reading.run([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/tags/{name}"),
        ]))
        .await?;
    if !existing.succeeded() {
        return Err(GitFailure::UnknownRevision {
            revision: name.to_owned(),
        });
    }
    let tip = existing.text().trim().to_owned();
    let command = vec![
        "tag".to_owned(),
        "-d".to_owned(),
        "--".to_owned(),
        name.to_owned(),
    ];

    Ok(PlannedOwnerAction {
        change: Change::MoveRef,
        command: command.clone(),
        effects: Vec::new(),
        extra_owner_effects: vec![OwnerActionEffect::DeletesRef {
            ref_kind: "tag".to_owned(),
            name: name.to_owned(),
        }],
        tips: vec![tip],
        fingerprint_paths: Vec::new(),
        needs_confirmation_always: true,
        runner: Runner::DeleteTag { command },
    })
}

async fn execute_planned(acting: &Acting, runner: Runner) -> Result<OwnerActionResult, GitFailure> {
    match runner {
        Runner::Stage(..)
        | Runner::ApplyPatch { .. }
        | Runner::DiscardFiles { .. }
        | Runner::Commit(..)
        | Runner::CreateTag { .. }
        | Runner::DeleteTag { .. } => execute_direct_runner(acting, runner).await,
        _ => execute_branch_or_history_runner(acting, runner).await,
    }
}

async fn execute_direct_runner(
    acting: &Acting,
    runner: Runner,
) -> Result<OwnerActionResult, GitFailure> {
    match runner {
        Runner::Stage(args, plan) => {
            let will_snapshot = plan.planned;
            let _answer = GitStage::act(acting, &args, plan).await?;
            let snapshot = if will_snapshot {
                latest_stash_oid(&acting.reading).await
            } else {
                None
            };
            completed(acting, snapshot, None).await
        }
        Runner::ApplyPatch {
            command,
            patch,
            snapshot_label,
        } => {
            let snapshot = match snapshot_label {
                Some(label) => acting.snapshot(label).await?,
                None => None,
            };
            let ran = acting
                .reading
                .git
                .run(acting.run(command).with_input(patch.into_bytes()))
                .await?;
            if !ran.succeeded() {
                return Err(refusal("apply patch", &ran));
            }
            completed(acting, snapshot, None).await
        }
        Runner::DiscardFiles {
            tracked_specs,
            untracked_specs,
            snapshot_tracked,
            count,
        } => {
            execute_discard_files(
                acting,
                tracked_specs,
                untracked_specs,
                snapshot_tracked,
                count,
            )
            .await
        }
        Runner::Commit(args, plan) => {
            let _answer = GitCommit::act(acting, &args, plan).await?;
            completed(acting, None, None).await
        }
        Runner::CreateTag { command, message } => {
            let mut run = acting.run(command);
            if let Some(msg) = message {
                run = run.with_input(msg.into_bytes());
            }
            let ran = acting.reading.git.run(run).await?;
            if !ran.succeeded() {
                return Err(refusal("tag", &ran));
            }
            completed(acting, None, None).await
        }
        Runner::DeleteTag { command } => {
            let ran = acting.reading.git.run(acting.run(command)).await?;
            if !ran.succeeded() {
                return Err(refusal("tag delete", &ran));
            }
            completed(acting, None, None).await
        }
        _ => unreachable!("handled by execute_planned dispatch"),
    }
}

async fn execute_discard_files(
    acting: &Acting,
    tracked_specs: Vec<String>,
    untracked_specs: Vec<String>,
    snapshot_tracked: bool,
    count: u32,
) -> Result<OwnerActionResult, GitFailure> {
    let snapshot = if snapshot_tracked {
        acting.snapshot("restoring files").await?
    } else {
        None
    };
    if !tracked_specs.is_empty() {
        let mut cmd = vec![
            "restore".to_owned(),
            "--worktree".to_owned(),
            "--".to_owned(),
        ];
        cmd.extend(tracked_specs);
        let ran = acting.reading.git.run(acting.run(cmd)).await?;
        if !ran.succeeded() {
            return Err(refusal("restore", &ran));
        }
    }
    if !untracked_specs.is_empty() {
        let mut cmd = vec!["clean".to_owned(), "-f".to_owned(), "--".to_owned()];
        cmd.extend(untracked_specs);
        let ran = acting.reading.git.run(acting.run(cmd)).await?;
        if !ran.succeeded() {
            return Err(refusal("clean", &ran));
        }
    }
    completed(acting, snapshot, Some(count)).await
}

async fn execute_branch_or_history_runner(
    acting: &Acting,
    runner: Runner,
) -> Result<OwnerActionResult, GitFailure> {
    match runner {
        Runner::Switch(args, plan) => {
            let _answer = GitSwitch::act(acting, &args, plan).await?;
            completed(acting, None, None).await
        }
        Runner::Branch(args, plan) => {
            let _answer = GitBranch::act(acting, &args, plan).await?;
            completed(acting, None, None).await
        }
        Runner::Stash(args, plan) => {
            let may_conflict = matches!(args.action, StashAction::Apply | StashAction::Pop);
            let _answer = GitStash::act(acting, &args, plan).await?;
            if may_conflict {
                outcome_or_completed(acting).await
            } else {
                completed(acting, None, None).await
            }
        }
        Runner::Fetch(args, plan) => {
            let integrates = args.integrate.is_some();
            let _answer = GitFetch::act(acting, &args, plan).await?;
            if integrates {
                outcome_or_completed(acting).await
            } else {
                completed(acting, None, None).await
            }
        }
        Runner::Push(args, plan) => {
            let _answer = GitPush::act(acting, &args, plan).await?;
            completed(acting, None, None).await
        }
        Runner::Merge(args, plan) => {
            let _answer = GitMerge::act(acting, &args, plan).await?;
            outcome_or_completed(acting).await
        }
        Runner::Rebase(args, plan) => {
            let _answer = GitRebase::act(acting, &args, plan).await?;
            outcome_or_completed(acting).await
        }
        Runner::CherryPick(args, plan) => {
            let _answer = GitCherryPick::act(acting, &args, plan).await?;
            outcome_or_completed(acting).await
        }
        Runner::Revert(args, plan) => {
            let _answer = GitRevert::act(acting, &args, plan).await?;
            outcome_or_completed(acting).await
        }
        Runner::Resolve(args, plan) => {
            let _answer = GitResolve::act(acting, &args, plan).await?;
            let remaining = conflicted_files(&acting.reading).await?;
            completed(
                acting,
                None,
                Some(u32::try_from(remaining.len()).unwrap_or(u32::MAX)),
            )
            .await
        }
        Runner::Operation(args, plan) => {
            let _answer = GitOperation::act(acting, &args, plan).await?;
            outcome_or_completed(acting).await
        }
        _ => unreachable!("handled by execute_planned dispatch"),
    }
}

async fn completed(
    acting: &Acting,
    snapshot: Option<String>,
    count: Option<u32>,
) -> Result<OwnerActionResult, GitFailure> {
    let head = match head_commit(&acting.reading).await? {
        Some(oid) => label_commits(&acting.reading, &[oid])
            .await?
            .into_iter()
            .next(),
        None => None,
    };
    Ok(OwnerActionResult::Completed {
        head,
        snapshot,
        count,
    })
}

async fn outcome_or_completed(acting: &Acting) -> Result<OwnerActionResult, GitFailure> {
    let operation = operation_view(&acting.reading).await?;
    let conflicts = conflicted_files(&acting.reading).await?;
    if operation.is_some() || !conflicts.is_empty() {
        return Ok(OwnerActionResult::Stopped {
            operation,
            conflicts: u32::try_from(conflicts.len()).unwrap_or(u32::MAX),
        });
    }
    completed(acting, None, None).await
}

async fn latest_stash_oid(reading: &Reading) -> Option<String> {
    let ran = reading
        .git
        .run(reading.run(["rev-parse", "--verify", "--quiet", "refs/stash"]))
        .await
        .ok()?;
    ran.succeeded().then(|| ran.text().trim().to_owned())
}
