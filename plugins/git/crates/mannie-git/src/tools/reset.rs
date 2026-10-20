//! `git_reset` (plugins/git/docs/tools.md, tools that change the
//! repository): moves the current branch to a commit. The commits that the
//! branch leaves behind and no other ref holds ask first, and `hard`, which
//! throws uncommitted changes away, keeps them as a stash entry first
//! (T-50).

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::changes::{Acting, ChangeTool, Changed, Plan, changed_paths, current_branch};
use crate::describe::label_commits;
use crate::effects::{Change, discarded_changes, discarded_commits, unkept_commits};
use crate::failure::GitFailure;
use crate::outcome::{head_commit, refusal};
use crate::texts;
use crate::tools::{Answer, Reading};

/// The tool's name.
pub(crate) const GIT_RESET: &str = "git_reset";

/// What a reset keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResetMode {
    Soft,
    Mixed,
    Keep,
    Hard,
}

/// The arguments of `git_reset`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResetArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// The commit to move the current branch to, such as HEAD~1.
    target: String,
    /// What the reset keeps. mixed when null.
    mode: Option<ResetMode>,
}

/// `git_reset`.
#[derive(Debug)]
pub(crate) struct GitReset;

impl ChangeTool for GitReset {
    type Arguments = ResetArguments;
    type Planned = bool;

    const NAME: &'static str = GIT_RESET;
    const DESCRIPTION: &'static str = texts::RESET_DESCRIPTION;

    fn repository(arguments: &ResetArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn plan<'a>(
        reading: &'a Reading,
        arguments: &'a ResetArguments,
    ) -> BoxFuture<'a, Result<Plan<bool>, GitFailure>> {
        Box::pin(plan(reading, arguments))
    }

    fn act<'a>(
        acting: &'a Acting,
        _arguments: &'a ResetArguments,
        plan: Plan<bool>,
    ) -> BoxFuture<'a, Result<Answer, GitFailure>> {
        Box::pin(async move {
            let snapshot = if plan.planned {
                acting.snapshot("a hard reset").await?
            } else {
                None
            };
            let ran = acting.reading.git.run(acting.run(plan.command)).await?;
            if !ran.succeeded() {
                return Err(refusal("reset", &ran));
            }
            let head = head_commit(&acting.reading).await?.unwrap_or_default();
            let label = label_commits(&acting.reading, &[head]).await?;
            let mut text = match label.first() {
                Some(head) => format!("HEAD is now at {}.\n", head.text()),
                None => "The reset completed.\n".to_owned(),
            };
            if let Some(snapshot) = snapshot {
                text.push_str(&texts::snapshot_kept("a hard reset", &snapshot));
            }
            Ok(Answer { text, data: None })
        })
    }
}

async fn plan(reading: &Reading, arguments: &ResetArguments) -> Result<Plan<bool>, GitFailure> {
    let mode = arguments.mode.unwrap_or(ResetMode::Mixed);
    let target = reading.commit("target", &arguments.target).await?;
    let head = head_commit(reading)
        .await?
        .ok_or_else(|| GitFailure::InvalidArgument {
            detail: "The branch has no commit to reset from.".to_owned(),
        })?;
    let branch = current_branch(reading).await?;
    let unkept = unkept_commits(reading, &head, branch.as_deref(), Some(&target)).await?;
    let left_behind = label_commits(reading, &unkept).await?;
    let thrown_away = if mode == ResetMode::Hard {
        changed_paths(reading, &[], Changed::Any).await?
    } else {
        Vec::new()
    };
    let snapshot = !thrown_away.is_empty();
    let flag = match mode {
        ResetMode::Soft => "--soft",
        ResetMode::Mixed => "--mixed",
        ResetMode::Keep => "--keep",
        ResetMode::Hard => "--hard",
    };
    let change = Change::Reset {
        index: mode != ResetMode::Soft,
        work_tree: matches!(mode, ResetMode::Keep | ResetMode::Hard),
        snapshot,
    };
    let command = vec![
        "reset".to_owned(),
        "--quiet".to_owned(),
        flag.to_owned(),
        target.clone(),
    ];
    Ok(Plan::new(change, command, snapshot)
        .with_tip(&target)
        .with_effect(discarded_commits(&left_behind))
        .with_effect(discarded_changes(&thrown_away, true)))
}
