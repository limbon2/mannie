//! `git_branch` (plugins/git/docs/tools.md, tools that change the
//! repository): creates a branch at a commit, renames or deletes one, or
//! sets or unsets its upstream. Deleting a branch whose commits no other
//! branch, tag or remote-tracking branch holds throws them away, so it asks.

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::changes::{Acting, ChangeTool, Plan, branch_tip, check_branch_name, current_branch};
use crate::describe::label_commits;
use crate::effects::{Change, discarded_commits, unkept_commits};
use crate::failure::GitFailure;
use crate::outcome::refusal;
use crate::texts;
use crate::tools::{Answer, Reading};

/// The tool's name.
pub(crate) const GIT_BRANCH: &str = "git_branch";

/// What `git_branch` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BranchAction {
    Create,
    Rename,
    Delete,
    SetUpstream,
    UnsetUpstream,
}

/// The arguments of `git_branch`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BranchArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// What to do with the branch.
    action: BranchAction,
    /// The branch's name, such as feature/parser.
    name: String,
    /// For create: the commit to start the branch at. HEAD when null.
    start: Option<String>,
    /// For rename: the branch's new name.
    new_name: Option<String>,
    /// For `set_upstream`: the branch to track, such as `origin/main`.
    upstream: Option<String>,
}

/// `git_branch`.
#[derive(Debug)]
pub(crate) struct GitBranch;

impl ChangeTool for GitBranch {
    type Arguments = BranchArguments;
    type Planned = ();

    const NAME: &'static str = GIT_BRANCH;
    const DESCRIPTION: &'static str = texts::BRANCH_DESCRIPTION;

    fn repository(arguments: &BranchArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn plan<'a>(
        reading: &'a Reading,
        arguments: &'a BranchArguments,
    ) -> BoxFuture<'a, Result<Plan<()>, GitFailure>> {
        Box::pin(plan(reading, arguments))
    }

    fn act<'a>(
        acting: &'a Acting,
        arguments: &'a BranchArguments,
        plan: Plan<()>,
    ) -> BoxFuture<'a, Result<Answer, GitFailure>> {
        Box::pin(async move {
            let ran = acting.reading.git.run(acting.run(plan.command)).await?;
            if !ran.succeeded() {
                return Err(refusal("branch change", &ran));
            }
            let name = &arguments.name;
            let text = match arguments.action {
                BranchAction::Create => format!("Created the branch {name}.\n"),
                BranchAction::Rename => format!(
                    "Renamed the branch {name} to {}.\n",
                    arguments.new_name.as_deref().unwrap_or_default()
                ),
                BranchAction::Delete => format!("Deleted the branch {name}.\n"),
                BranchAction::SetUpstream => format!(
                    "The branch {name} now tracks {}.\n",
                    arguments.upstream.as_deref().unwrap_or_default()
                ),
                BranchAction::UnsetUpstream => {
                    format!("The branch {name} no longer tracks a branch.\n")
                }
            };
            Ok(Answer { text, data: None })
        })
    }
}

fn invalid(detail: &str) -> GitFailure {
    GitFailure::InvalidArgument {
        detail: detail.to_owned(),
    }
}

/// The commit of an existing branch, or a failure that names it.
async fn existing(reading: &Reading, name: &str) -> Result<String, GitFailure> {
    branch_tip(reading, name)
        .await?
        .ok_or_else(|| GitFailure::UnknownRevision {
            revision: name.to_owned(),
        })
}

async fn plan(reading: &Reading, arguments: &BranchArguments) -> Result<Plan<()>, GitFailure> {
    let name = arguments.name.as_str();
    check_branch_name(reading, "name", name).await?;
    let owned = |parts: &[&str]| {
        parts
            .iter()
            .map(|part| (*part).to_owned())
            .collect::<Vec<_>>()
    };
    match arguments.action {
        BranchAction::Create => {
            if branch_tip(reading, name).await?.is_some() {
                return Err(invalid(&format!("The branch {name} exists already.")));
            }
            let start = reading
                .commit("start", arguments.start.as_deref().unwrap_or("HEAD"))
                .await?;
            let command = [owned(&["branch", "--no-track", name]), vec![start.clone()]].concat();
            Ok(Plan::new(Change::MoveRef, command, ()).with_tip(&start))
        }
        BranchAction::Rename => {
            let tip = existing(reading, name).await?;
            let new_name = arguments
                .new_name
                .as_deref()
                .ok_or_else(|| invalid("rename needs new_name."))?;
            check_branch_name(reading, "new_name", new_name).await?;
            if branch_tip(reading, new_name).await?.is_some() {
                return Err(invalid(&format!("The branch {new_name} exists already.")));
            }
            Ok(Plan::new(
                Change::MoveRef,
                owned(&["branch", "-m", name, new_name]),
                (),
            )
            .with_tip(&tip))
        }
        BranchAction::Delete => {
            let tip = existing(reading, name).await?;
            if current_branch(reading).await?.as_deref() == Some(name) {
                return Err(invalid(&format!(
                    "{name} is the current branch. Switch to another branch first."
                )));
            }
            let unkept = unkept_commits(reading, &tip, Some(name), None).await?;
            let labels = label_commits(reading, &unkept).await?;
            // -d refuses a branch whose commits no other branch holds; the
            // owner allows throwing them away first.
            let flag = if unkept.is_empty() { "-d" } else { "-D" };
            Ok(
                Plan::new(Change::MoveRef, owned(&["branch", flag, name]), ())
                    .with_tip(&tip)
                    .with_effect(discarded_commits(&labels)),
            )
        }
        BranchAction::SetUpstream => {
            let tip = existing(reading, name).await?;
            let upstream = arguments
                .upstream
                .as_deref()
                .ok_or_else(|| invalid("set_upstream needs upstream."))?;
            reading.commit("upstream", upstream).await?;
            let command = vec![
                "branch".to_owned(),
                format!("--set-upstream-to={upstream}"),
                name.to_owned(),
            ];
            Ok(Plan::new(Change::Configure, command, ()).with_tip(&tip))
        }
        BranchAction::UnsetUpstream => {
            let tip = existing(reading, name).await?;
            Ok(Plan::new(
                Change::Configure,
                owned(&["branch", "--unset-upstream", name]),
                (),
            )
            .with_tip(&tip))
        }
    }
}
