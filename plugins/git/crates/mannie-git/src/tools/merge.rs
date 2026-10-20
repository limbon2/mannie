//! `git_merge` (plugins/git/docs/conflicts-and-history.md, operations that
//! can stop): merges a branch or a commit into the current branch, and
//! ends completed, stopped on conflicts, or refused with nothing changed.
//! git's editor never opens: a merge commit takes git's message unless the
//! call gives one.

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::changes::{Acting, ChangeTool, Plan};
use crate::effects::Change;
use crate::failure::GitFailure;
use crate::operation::Operation;
use crate::outcome::{head_commit, operation_outcome};
use crate::texts;
use crate::tools::commit::{check_identity, check_message};
use crate::tools::{Answer, Reading};

/// The tool's name.
pub(crate) const GIT_MERGE: &str = "git_merge";

/// How a merge joins the histories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MergeMode {
    Default,
    FastForwardOnly,
    NoFastForward,
    Squash,
}

/// The arguments of `git_merge`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MergeArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// The branch or commit to merge, such as `feature` or `origin/main`.
    branch: String,
    /// `default`: a fast-forward when possible, else a merge commit;
    /// `fast_forward_only`: refuse anything else; `no_fast_forward`: always a
    /// merge commit; `squash`: stage the changes for `git_commit`. `default`
    /// when null.
    mode: Option<MergeMode>,
    /// The merge commit's message. git's when null.
    message: Option<String>,
}

/// `git_merge`.
#[derive(Debug)]
pub(crate) struct GitMerge;

impl ChangeTool for GitMerge {
    type Arguments = MergeArguments;
    type Planned = ();

    const NAME: &'static str = GIT_MERGE;
    const DESCRIPTION: &'static str = texts::MERGE_DESCRIPTION;

    fn repository(arguments: &MergeArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn plan<'a>(
        reading: &'a Reading,
        arguments: &'a MergeArguments,
    ) -> BoxFuture<'a, Result<Plan<()>, GitFailure>> {
        Box::pin(async move {
            no_operation(reading)?;
            check_identity(reading).await?;
            let tip = reading.commit("branch", &arguments.branch).await?;
            let mode = arguments.mode.unwrap_or(MergeMode::Default);
            let mut command = vec!["merge".to_owned(), "--no-stat".to_owned()];
            match mode {
                MergeMode::Default => {}
                MergeMode::FastForwardOnly => command.push("--ff-only".to_owned()),
                MergeMode::NoFastForward => command.push("--no-ff".to_owned()),
                MergeMode::Squash => command.push("--squash".to_owned()),
            }
            match &arguments.message {
                Some(message) => {
                    check_message("message", message)?;
                    command.extend(["-m".to_owned(), message.clone()]);
                }
                None => command.push("--no-edit".to_owned()),
            }
            // The name, checked and resolved above, gives git's message its
            // branch; the state holds its commit.
            command.extend(["--end-of-options".to_owned(), arguments.branch.clone()]);
            Ok(Plan::new(Change::Integrate, command, ()).with_tip(&tip))
        })
    }

    fn act<'a>(
        acting: &'a Acting,
        arguments: &'a MergeArguments,
        plan: Plan<()>,
    ) -> BoxFuture<'a, Result<Answer, GitFailure>> {
        Box::pin(async move {
            let before = head_commit(&acting.reading).await?;
            let ran = acting.reading.git.run(acting.run(plan.command)).await?;
            let mut answer = operation_outcome(acting, "merge", before.as_deref(), &ran).await?;
            if arguments.mode == Some(MergeMode::Squash) && ran.succeeded() {
                answer
                    .text
                    .push_str("The changes are staged; commit them with git_commit.\n");
            }
            Ok(answer)
        })
    }
}

/// Refuses a call while an operation is in progress, which git would also
/// refuse.
pub(crate) fn no_operation(reading: &Reading) -> Result<(), GitFailure> {
    match Operation::read(&reading.layout.git_dir) {
        Some(operation) => Err(GitFailure::InvalidArgument {
            detail: format!(
                "A {} is in progress. Finish it with git_operation first.",
                operation.kind.name()
            ),
        }),
        None => Ok(()),
    }
}
