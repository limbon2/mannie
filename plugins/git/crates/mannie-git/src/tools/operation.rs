//! `git_operation` (plugins/git/docs/conflicts-and-history.md, resolving):
//! continues the merge, rebase, cherry-pick or revert in progress once no
//! file is conflicted, skips its current step, or aborts it. Skipping and
//! aborting throw away the changes made since the operation stopped, and a
//! skip also the commit that the step applies, so they ask first. git cannot
//! keep a snapshot of a conflicted index, so they keep none.

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::changes::{Acting, ChangeTool, Changed, Plan, changed_paths};
use crate::describe::{CommitLabel, operation_view};
use crate::effects::{Change, discarded_changes, discarded_commits};
use crate::failure::GitFailure;
use crate::operation::{Operation, OperationKind};
use crate::outcome::{head_commit, operation_outcome, refusal};
use crate::texts;
use crate::tools::commit::check_identity;
use crate::tools::conflicts::conflicted_files;
use crate::tools::{Answer, Reading};

/// The tool's name.
pub(crate) const GIT_OPERATION: &str = "git_operation";

/// What to do with the operation in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OperationAction {
    Continue,
    Skip,
    Abort,
}

/// The arguments of `git_operation`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct OperationArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// continue, skip or abort.
    action: OperationAction,
}

/// What the run keeps: the operation, and the commit after which its new
/// commits come.
#[derive(Debug, Clone)]
pub(crate) struct Continuing {
    kind: OperationKind,
    since: Option<String>,
}

/// `git_operation`.
#[derive(Debug)]
pub(crate) struct GitOperation;

impl ChangeTool for GitOperation {
    type Arguments = OperationArguments;
    type Planned = Continuing;

    const NAME: &'static str = GIT_OPERATION;
    const DESCRIPTION: &'static str = texts::OPERATION_DESCRIPTION;

    fn repository(arguments: &OperationArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn plan<'a>(
        reading: &'a Reading,
        arguments: &'a OperationArguments,
    ) -> BoxFuture<'a, Result<Plan<Continuing>, GitFailure>> {
        Box::pin(plan(reading, arguments))
    }

    fn act<'a>(
        acting: &'a Acting,
        arguments: &'a OperationArguments,
        plan: Plan<Continuing>,
    ) -> BoxFuture<'a, Result<Answer, GitFailure>> {
        Box::pin(async move {
            let reading = &acting.reading;
            let name = plan.planned.kind.name();
            let ran = reading.git.run(acting.run(plan.command)).await?;
            if arguments.action != OperationAction::Abort {
                return operation_outcome(acting, name, plan.planned.since.as_deref(), &ran).await;
            }
            if !ran.succeeded() {
                return Err(refusal(&format!("{name} abort"), &ran));
            }
            let head = head_commit(reading).await?.unwrap_or_default();
            let short: String = head.chars().take(10).collect();
            Ok(Answer {
                text: format!("The {name} was aborted. HEAD is at {short}, where it started.\n"),
                data: None,
            })
        })
    }
}

fn invalid(detail: String) -> GitFailure {
    GitFailure::InvalidArgument { detail }
}

async fn plan(
    reading: &Reading,
    arguments: &OperationArguments,
) -> Result<Plan<Continuing>, GitFailure> {
    let operation = Operation::read(&reading.layout.git_dir).ok_or_else(|| {
        invalid("No merge, rebase, cherry-pick or revert is in progress.".to_owned())
    })?;
    let name = operation.kind.name();
    let since = match operation.kind {
        // A rebase's new commits come after its new base; the others' after
        // HEAD.
        OperationKind::Rebase => operation.onto.clone(),
        OperationKind::Merge | OperationKind::CherryPick | OperationKind::Revert => {
            head_commit(reading).await?
        }
    };
    let continuing = Continuing {
        kind: operation.kind,
        since,
    };
    let mut plan = match arguments.action {
        OperationAction::Continue => {
            let conflicted = conflicted_files(reading).await?;
            if !conflicted.is_empty() {
                return Err(invalid(format!(
                    "{} files are still conflicted. Resolve them with git_resolve first.",
                    conflicted.len()
                )));
            }
            check_identity(reading).await?;
            Plan::new(
                Change::Integrate,
                vec![name.to_owned(), "--continue".to_owned()],
                continuing,
            )
        }
        OperationAction::Skip => {
            if operation.kind == OperationKind::Merge {
                return Err(invalid(
                    "A merge has no steps to skip. Abort it instead.".to_owned(),
                ));
            }
            let step = operation_view(reading)
                .await?
                .and_then(|view| view.incoming);
            let skipped: Vec<CommitLabel> = step.into_iter().collect();
            let lost = changed_paths(reading, &[], Changed::Any).await?;
            Plan::new(
                Change::Integrate,
                vec![name.to_owned(), "--skip".to_owned()],
                continuing,
            )
            .with_effect(discarded_commits(&skipped))
            .with_effect(discarded_changes(&lost, false))
        }
        OperationAction::Abort => {
            let lost = changed_paths(reading, &[], Changed::Any).await?;
            Plan::new(
                Change::Abort,
                vec![name.to_owned(), "--abort".to_owned()],
                continuing,
            )
            .with_effect(discarded_changes(&lost, false))
        }
    };
    if let Some(incoming) = &operation.incoming {
        plan = plan.with_tip(incoming);
    }
    Ok(plan)
}
