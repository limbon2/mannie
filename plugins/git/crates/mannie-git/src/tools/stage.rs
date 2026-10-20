//! `git_stage` (plugins/git/docs/tools.md, tools that change the
//! repository): stages files for the next commit, or takes them out of the
//! index. Taking a file out of the index throws away its staged version
//! when the work tree holds another, so that asks, after a snapshot.

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::changes::{Acting, ChangeTool, Changed, Plan, changed_paths};
use crate::effects::{Change, discarded_changes};
use crate::failure::GitFailure;
use crate::outcome::head_commit;
use crate::texts;
use crate::tools::status::{read_status, status_text};
use crate::tools::{Answer, Reading, pathspecs_or_all};

/// The tool's name.
pub(crate) const GIT_STAGE: &str = "git_stage";

/// The arguments of `git_stage`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct StageArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// The files or folders to stage or unstage, relative to the
    /// repository's top.
    paths: Option<Vec<String>>,
    /// Whether to stage or unstage every change instead of paths.
    all: Option<bool>,
    /// Whether to take the paths out of the index, keeping the work tree's
    /// files. False when null.
    unstage: Option<bool>,
}

/// `git_stage`.
#[derive(Debug)]
pub(crate) struct GitStage;

impl ChangeTool for GitStage {
    type Arguments = StageArguments;
    type Planned = bool;

    const NAME: &'static str = GIT_STAGE;
    const DESCRIPTION: &'static str = texts::STAGE_DESCRIPTION;

    fn repository(arguments: &StageArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn plan<'a>(
        reading: &'a Reading,
        arguments: &'a StageArguments,
    ) -> BoxFuture<'a, Result<Plan<bool>, GitFailure>> {
        Box::pin(async move {
            let specs = pathspecs_or_all(
                &reading.layout.top,
                arguments.paths.as_deref(),
                arguments.all == Some(true),
            )?;
            if arguments.unstage != Some(true) {
                let command = [
                    vec!["add".to_owned(), "--all".to_owned(), "--".to_owned()],
                    specs,
                ]
                .concat();
                return Ok(Plan::new(Change::Stage, command, false));
            }
            let lost = changed_paths(reading, &specs, Changed::PartlyStaged).await?;
            let snapshot = !lost.is_empty();
            // A branch without commits has no HEAD to restore the index from.
            let command = if head_commit(reading).await?.is_some() {
                [
                    vec!["restore".to_owned(), "--staged".to_owned(), "--".to_owned()],
                    specs,
                ]
                .concat()
            } else {
                let removal = ["rm", "--cached", "-r", "--quiet", "--"]
                    .map(str::to_owned)
                    .to_vec();
                [removal, specs].concat()
            };
            Ok(Plan::new(Change::Unstage { snapshot }, command, snapshot)
                .with_effect(discarded_changes(&lost, true)))
        })
    }

    fn act<'a>(
        acting: &'a Acting,
        _arguments: &'a StageArguments,
        plan: Plan<bool>,
    ) -> BoxFuture<'a, Result<Answer, GitFailure>> {
        Box::pin(async move {
            let snapshot = if plan.planned {
                acting.snapshot("unstaging").await?
            } else {
                None
            };
            let git = &acting.reading.git;
            git.output(acting.run(plan.command)).await?;
            let status = read_status(&acting.reading).await?;
            let mut text = status_text(&status, None, None);
            if let Some(snapshot) = snapshot {
                text.push_str(&texts::snapshot_kept("unstaging", &snapshot));
            }
            Ok(Answer { text, data: None })
        })
    }
}
