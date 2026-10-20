//! `git_cherry_pick` and `git_revert` (plugins/git/docs/conflicts-and-
//! history.md, operations that can stop): apply commits onto the current
//! branch, or commits that undo them, in order, and end completed, stopped
//! on conflicts, or refused with nothing changed.

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::changes::{Acting, ChangeTool, Plan};
use crate::effects::Change;
use crate::failure::GitFailure;
use crate::outcome::{head_commit, operation_outcome};
use crate::texts;
use crate::tools::commit::check_identity;
use crate::tools::merge::no_operation;
use crate::tools::{Answer, Reading};

/// The name of `git_cherry_pick`.
pub(crate) const GIT_CHERRY_PICK: &str = "git_cherry_pick";

/// The name of `git_revert`.
pub(crate) const GIT_REVERT: &str = "git_revert";

/// The most commits that one call applies.
const MAX_COMMITS: usize = 100;

/// The arguments of `git_cherry_pick` and `git_revert`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplyArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// The commits, in the order to apply them, such as 1a2b3c4 and
    /// feature~2. At most 100.
    commits: Vec<String>,
    /// For a merge commit: the number of its parent, from 1, whose side the
    /// changes are taken against.
    mainline: Option<u32>,
    /// Whether to stage the changes without committing.
    no_commit: Option<bool>,
}

/// Which of the two tools a call is.
#[derive(Debug, Clone, Copy)]
enum Applying {
    CherryPick,
    Revert,
}

impl Applying {
    /// git's command, and the operation's name in a sentence.
    fn command(self) -> &'static str {
        match self {
            Self::CherryPick => "cherry-pick",
            Self::Revert => "revert",
        }
    }
}

async fn plan(
    reading: &Reading,
    arguments: &ApplyArguments,
    applying: Applying,
) -> Result<Plan<()>, GitFailure> {
    no_operation(reading)?;
    if arguments.commits.is_empty() || arguments.commits.len() > MAX_COMMITS {
        return Err(GitFailure::InvalidArgument {
            detail: "commits must name 1 to 100 commits.".to_owned(),
        });
    }
    if arguments.mainline == Some(0) {
        return Err(GitFailure::InvalidArgument {
            detail: "mainline counts parents from 1.".to_owned(),
        });
    }
    if arguments.no_commit != Some(true) {
        check_identity(reading).await?;
    }
    let mut command = vec![applying.command().to_owned()];
    if matches!(applying, Applying::Revert) {
        command.push("--no-edit".to_owned());
    }
    if let Some(mainline) = arguments.mainline {
        command.extend(["-m".to_owned(), mainline.to_string()]);
    }
    if arguments.no_commit == Some(true) {
        command.push("--no-commit".to_owned());
    }
    let mut plan = Plan::new(Change::Integrate, Vec::new(), ());
    for revision in &arguments.commits {
        let commit = reading.commit("commits", revision).await?;
        command.push(commit.clone());
        plan = plan.with_tip(&commit);
    }
    plan.command = command;
    Ok(plan)
}

async fn act(acting: &Acting, plan: Plan<()>, applying: Applying) -> Result<Answer, GitFailure> {
    let before = head_commit(&acting.reading).await?;
    let ran = acting.reading.git.run(acting.run(plan.command)).await?;
    operation_outcome(acting, applying.command(), before.as_deref(), &ran).await
}

/// `git_cherry_pick`.
#[derive(Debug)]
pub(crate) struct GitCherryPick;

impl ChangeTool for GitCherryPick {
    type Arguments = ApplyArguments;
    type Planned = ();

    const NAME: &'static str = GIT_CHERRY_PICK;
    const DESCRIPTION: &'static str = texts::CHERRY_PICK_DESCRIPTION;

    fn repository(arguments: &ApplyArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn plan<'a>(
        reading: &'a Reading,
        arguments: &'a ApplyArguments,
    ) -> BoxFuture<'a, Result<Plan<()>, GitFailure>> {
        Box::pin(plan(reading, arguments, Applying::CherryPick))
    }

    fn act<'a>(
        acting: &'a Acting,
        _arguments: &'a ApplyArguments,
        plan: Plan<()>,
    ) -> BoxFuture<'a, Result<Answer, GitFailure>> {
        Box::pin(act(acting, plan, Applying::CherryPick))
    }
}

/// `git_revert`.
#[derive(Debug)]
pub(crate) struct GitRevert;

impl ChangeTool for GitRevert {
    type Arguments = ApplyArguments;
    type Planned = ();

    const NAME: &'static str = GIT_REVERT;
    const DESCRIPTION: &'static str = texts::REVERT_DESCRIPTION;

    fn repository(arguments: &ApplyArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn plan<'a>(
        reading: &'a Reading,
        arguments: &'a ApplyArguments,
    ) -> BoxFuture<'a, Result<Plan<()>, GitFailure>> {
        Box::pin(plan(reading, arguments, Applying::Revert))
    }

    fn act<'a>(
        acting: &'a Acting,
        _arguments: &'a ApplyArguments,
        plan: Plan<()>,
    ) -> BoxFuture<'a, Result<Answer, GitFailure>> {
        Box::pin(act(acting, plan, Applying::Revert))
    }
}
