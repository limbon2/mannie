//! `git_restore` (plugins/git/docs/tools.md, tools that change the
//! repository): restores files of the work tree, the index or both, from
//! the index, HEAD or a revision. The uncommitted changes that it throws
//! away ask first and are kept as a stash entry before it runs (T-50).

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::arguments::pathspecs;
use crate::changes::{Acting, ChangeTool, Changed, Plan, changed_paths};
use crate::effects::{Change, discarded_changes};
use crate::failure::GitFailure;
use crate::outcome::refusal;
use crate::texts;
use crate::tools::{Answer, Reading, count};

/// The tool's name.
pub(crate) const GIT_RESTORE: &str = "git_restore";

/// The arguments of `git_restore`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RestoreArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// The files or folders to restore, relative to the repository's top.
    paths: Vec<String>,
    /// The revision to restore them from, such as HEAD~1. The index for the
    /// work tree, and HEAD for the index, when null.
    source: Option<String>,
    /// Whether to restore the index. False when null.
    staged: Option<bool>,
    /// Whether to restore the work tree. True when null, unless staged is
    /// true.
    worktree: Option<bool>,
}

/// What a restore keeps for its run: whether it keeps a snapshot first.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Restoring {
    snapshot: bool,
    files: usize,
}

/// `git_restore`.
#[derive(Debug)]
pub(crate) struct GitRestore;

impl ChangeTool for GitRestore {
    type Arguments = RestoreArguments;
    type Planned = Restoring;

    const NAME: &'static str = GIT_RESTORE;
    const DESCRIPTION: &'static str = texts::RESTORE_DESCRIPTION;

    fn repository(arguments: &RestoreArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn plan<'a>(
        reading: &'a Reading,
        arguments: &'a RestoreArguments,
    ) -> BoxFuture<'a, Result<Plan<Restoring>, GitFailure>> {
        Box::pin(plan(reading, arguments))
    }

    fn act<'a>(
        acting: &'a Acting,
        _arguments: &'a RestoreArguments,
        plan: Plan<Restoring>,
    ) -> BoxFuture<'a, Result<Answer, GitFailure>> {
        Box::pin(async move {
            let snapshot = if plan.planned.snapshot {
                acting.snapshot("restoring files").await?
            } else {
                None
            };
            let ran = acting.reading.git.run(acting.run(plan.command)).await?;
            if !ran.succeeded() {
                return Err(refusal("restore", &ran));
            }
            let mut text = format!(
                "Restored the paths; {} changed.\n",
                count(
                    plan.planned.files,
                    "file with changes",
                    "files with changes"
                )
            );
            if let Some(snapshot) = snapshot {
                text.push_str(&texts::snapshot_kept("restoring files", &snapshot));
            }
            Ok(Answer { text, data: None })
        })
    }
}

async fn plan(
    reading: &Reading,
    arguments: &RestoreArguments,
) -> Result<Plan<Restoring>, GitFailure> {
    if arguments.paths.is_empty() {
        return Err(GitFailure::InvalidArgument {
            detail: "paths names no file.".to_owned(),
        });
    }
    let specs = pathspecs(&reading.layout.top, "paths", &arguments.paths)?;
    let staged = arguments.staged == Some(true);
    let worktree = arguments.worktree.unwrap_or(!staged);
    if !staged && !worktree {
        return Err(GitFailure::InvalidArgument {
            detail: "Restore the work tree, the index, or both.".to_owned(),
        });
    }
    let mut command = vec!["restore".to_owned()];
    if let Some(source) = &arguments.source {
        command.push(format!(
            "--source={}",
            reading.commit("source", source).await?
        ));
    }
    if staged {
        command.push("--staged".to_owned());
    }
    if worktree {
        command.push("--worktree".to_owned());
    }
    command.push("--".to_owned());
    command.extend(specs.iter().cloned());
    // Restoring the work tree from the index throws its unstaged changes
    // away, and from a revision or with the index too, every change.
    // Restoring only the index throws the staged versions away where the
    // work tree holds others.
    let changed = match (worktree, staged || arguments.source.is_some()) {
        (true, false) => Changed::Unstaged,
        (true, true) => Changed::Any,
        (false, _staged) => Changed::PartlyStaged,
    };
    let discarded = changed_paths(reading, &specs, changed).await?;
    let snapshot = !discarded.is_empty();
    let restoring = Restoring {
        snapshot,
        files: discarded.len(),
    };
    Ok(Plan::new(Change::Restore { snapshot }, command, restoring)
        .with_effect(discarded_changes(&discarded, true)))
}
