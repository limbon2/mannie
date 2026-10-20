//! `git_rebase` (plugins/git/docs/conflicts-and-history.md, operations that
//! can stop): replays the current branch's commits onto a new base, and
//! ends completed, stopped on conflicts, or refused with nothing changed.
//! Autosquash runs git's interactive rebase with the todo that git writes,
//! which the sequence editor `:` keeps as it is. Rewriting commits that a
//! remote-tracking branch holds asks first.

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::changes::{Acting, ChangeTool, Plan};
use crate::effects::{Change, published};
use crate::failure::GitFailure;
use crate::outcome::operation_outcome;
use crate::texts;
use crate::tools::commit::check_identity;
use crate::tools::merge::no_operation;
use crate::tools::{Answer, Reading};

/// The tool's name.
pub(crate) const GIT_REBASE: &str = "git_rebase";

/// The arguments of `git_rebase`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RebaseArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// The commit to replay the branch onto, such as main or origin/main.
    onto: String,
    /// The commit after which the branch's commits are replayed. When null,
    /// the commits that onto lacks.
    upstream: Option<String>,
    /// Whether to also move the branches that point at the replayed commits.
    update_refs: Option<bool>,
    /// Whether to fold the commits that `git_commit` made with fixup into the
    /// commits that they fix.
    autosquash: Option<bool>,
}

/// `git_rebase`.
#[derive(Debug)]
pub(crate) struct GitRebase;

impl ChangeTool for GitRebase {
    type Arguments = RebaseArguments;
    /// The new base, after which the replayed commits come.
    type Planned = String;

    const NAME: &'static str = GIT_REBASE;
    const DESCRIPTION: &'static str = texts::REBASE_DESCRIPTION;

    fn repository(arguments: &RebaseArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn plan<'a>(
        reading: &'a Reading,
        arguments: &'a RebaseArguments,
    ) -> BoxFuture<'a, Result<Plan<String>, GitFailure>> {
        Box::pin(async move {
            no_operation(reading)?;
            check_identity(reading).await?;
            let onto = reading.commit("onto", &arguments.onto).await?;
            let upstream = match &arguments.upstream {
                Some(upstream) => Some(reading.commit("upstream", upstream).await?),
                None => None,
            };
            let mut command = vec!["rebase".to_owned()];
            if arguments.autosquash == Some(true) {
                command.extend(["--interactive".to_owned(), "--autosquash".to_owned()]);
            }
            if arguments.update_refs == Some(true) {
                command.push("--update-refs".to_owned());
            }
            match &upstream {
                Some(upstream) => {
                    command.extend(["--onto".to_owned(), onto.clone(), upstream.clone()]);
                }
                None => command.push(onto.clone()),
            }
            let base = upstream.clone().unwrap_or_else(|| onto.clone());
            let rewritten = published(reading, &format!("{base}..HEAD")).await?;
            let mut plan = Plan::new(Change::Integrate, command, onto.clone())
                .with_tip(&onto)
                .with_effect(rewritten);
            if let Some(upstream) = &upstream {
                plan = plan.with_tip(upstream);
            }
            Ok(plan)
        })
    }

    fn act<'a>(
        acting: &'a Acting,
        _arguments: &'a RebaseArguments,
        plan: Plan<String>,
    ) -> BoxFuture<'a, Result<Answer, GitFailure>> {
        Box::pin(async move {
            let ran = acting.reading.git.run(acting.run(plan.command)).await?;
            operation_outcome(acting, "rebase", Some(&plan.planned), &ran).await
        })
    }
}
