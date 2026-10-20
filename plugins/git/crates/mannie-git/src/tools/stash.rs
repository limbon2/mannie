//! `git_stash` (plugins/git/docs/tools.md, tools that change the
//! repository): saves the uncommitted changes as a stash entry, puts an
//! entry's changes back, or drops an entry, which throws it away and asks
//! first. Putting changes back can leave conflicted files, which
//! `git_resolve` resolves.

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{DiscardedWork, ProgramEffect};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::changes::{Acting, ChangeTool, Plan};
use crate::effects::Change;
use crate::failure::GitFailure;
use crate::outcome::{head_commit, operation_outcome, refusal};
use crate::parse::log::{REFLOG_FORMAT, parse_reflog};
use crate::texts;
use crate::tools::commit::check_message;
use crate::tools::{Answer, Reading};

/// The tool's name.
pub(crate) const GIT_STASH: &str = "git_stash";

/// What `git_stash` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StashAction {
    Push,
    Apply,
    Pop,
    Drop,
}

/// The arguments of `git_stash`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct StashArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// What to do.
    action: StashAction,
    /// For push: the entry's message.
    message: Option<String>,
    /// For push: whether to save untracked files too. False when null.
    include_untracked: Option<bool>,
    /// For apply, pop and drop: the entry's number, 0 for the newest. 0 when
    /// null.
    entry: Option<u32>,
}

/// `git_stash`.
#[derive(Debug)]
pub(crate) struct GitStash;

impl ChangeTool for GitStash {
    type Arguments = StashArguments;
    type Planned = ();

    const NAME: &'static str = GIT_STASH;
    const DESCRIPTION: &'static str = texts::STASH_DESCRIPTION;

    fn repository(arguments: &StashArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn plan<'a>(
        reading: &'a Reading,
        arguments: &'a StashArguments,
    ) -> BoxFuture<'a, Result<Plan<()>, GitFailure>> {
        Box::pin(plan(reading, arguments))
    }

    fn act<'a>(
        acting: &'a Acting,
        arguments: &'a StashArguments,
        plan: Plan<()>,
    ) -> BoxFuture<'a, Result<Answer, GitFailure>> {
        Box::pin(async move {
            let reading = &acting.reading;
            let before = head_commit(reading).await?;
            let ran = reading.git.run(acting.run(plan.command)).await?;
            match arguments.action {
                StashAction::Apply | StashAction::Pop => {
                    operation_outcome(acting, "stash apply", before.as_deref(), &ran).await
                }
                StashAction::Push | StashAction::Drop if !ran.succeeded() => {
                    Err(refusal("stash change", &ran))
                }
                StashAction::Push => Ok(Answer {
                    text: format!("{}\n", ran.text().trim()),
                    data: None,
                }),
                StashAction::Drop => Ok(Answer {
                    text: "Dropped the stash entry.\n".to_owned(),
                    data: None,
                }),
            }
        })
    }
}

/// An entry of the stash: its selector, its commit and its message.
async fn stash_entry(
    reading: &Reading,
    number: u32,
) -> Result<(String, String, String), GitFailure> {
    let ran = reading
        .git
        .output(reading.run(["stash", "list", "-z", REFLOG_FORMAT]))
        .await?;
    let entries =
        parse_reflog(&ran.output).map_err(|error| GitFailure::failed(&error.to_string()))?;
    let selector = format!("stash@{{{number}}}");
    entries
        .into_iter()
        .find(|entry| entry.selector == selector)
        .map(|entry| (selector.clone(), entry.oid, entry.action))
        .ok_or(GitFailure::UnknownRevision { revision: selector })
}

async fn plan(reading: &Reading, arguments: &StashArguments) -> Result<Plan<()>, GitFailure> {
    let owned = |parts: &[&str]| {
        parts
            .iter()
            .map(|part| (*part).to_owned())
            .collect::<Vec<_>>()
    };
    match arguments.action {
        StashAction::Push => {
            let mut command = owned(&["stash", "push"]);
            if arguments.include_untracked == Some(true) {
                command.push("--include-untracked".to_owned());
            }
            if let Some(message) = &arguments.message {
                check_message("message", message)?;
                command.extend(["-m".to_owned(), message.clone()]);
            }
            Ok(Plan::new(Change::Stash { applies: false }, command, ()))
        }
        StashAction::Apply | StashAction::Pop => {
            let (selector, commit, _message) =
                stash_entry(reading, arguments.entry.unwrap_or(0)).await?;
            let verb = if arguments.action == StashAction::Pop {
                "pop"
            } else {
                "apply"
            };
            Ok(Plan::new(
                Change::Stash { applies: true },
                owned(&["stash", verb, &selector]),
                (),
            )
            .with_tip(&commit))
        }
        StashAction::Drop => {
            let (selector, commit, message) =
                stash_entry(reading, arguments.entry.unwrap_or(0)).await?;
            let dropped = ProgramEffect::DiscardsWork {
                work: DiscardedWork::StashEntries,
                count: 1,
                items: vec![format!("{selector}: {message}")],
                snapshot: false,
            };
            Ok(Plan::new(
                Change::Stash { applies: false },
                owned(&["stash", "drop", &selector]),
                (),
            )
            .with_tip(&commit)
            .with_effect(Some(dropped)))
        }
    }
}
