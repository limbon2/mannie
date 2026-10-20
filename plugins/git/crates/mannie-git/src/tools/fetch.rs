//! `git_fetch` (plugins/git/docs/tools.md, tools that reach a remote):
//! fetches from a remote, and with `integrate`, merges or rebases the
//! current branch onto what it fetched, as `git pull` does. Every run that
//! reaches a remote names its URL without credentials and asks the owner,
//! unless a grant, a rule for that URL or full access covers it (T-49).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::arguments::check_branch;
use crate::changes::{Acting, ChangeTool, Plan, current_branch};
use crate::effects::{Change, remote_endpoints};
use crate::failure::GitFailure;
use crate::outcome::{head_commit, operation_outcome, refusal};
use crate::texts;
use crate::tools::commit::check_identity;
use crate::tools::merge::no_operation;
use crate::tools::{Answer, Reading, fitting_data};

/// The tool's name.
pub(crate) const GIT_FETCH: &str = "git_fetch";

/// How a fetch joins what it fetched with the current branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Integrate {
    Merge,
    Rebase,
}

/// The arguments of `git_fetch`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FetchArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// The remote, such as `origin`. The current branch's remote, or
    /// `origin`, when null.
    remote: Option<String>,
    /// The remote's branch to fetch, such as `main`. Every branch when null,
    /// or with integrate, the current branch's upstream.
    branch: Option<String>,
    /// Whether to remove the remote-tracking branches whose branches the
    /// remote deleted.
    prune: Option<bool>,
    /// After fetching, merge or rebase the current branch onto the fetched
    /// branch, as git pull does.
    integrate: Option<Integrate>,
}

/// What a fetch keeps for its run.
#[derive(Debug, Clone)]
pub(crate) struct Fetching {
    remote: String,
    integrates: bool,
}

/// `git_fetch`.
#[derive(Debug)]
pub(crate) struct GitFetch;

impl ChangeTool for GitFetch {
    type Arguments = FetchArguments;
    type Planned = Fetching;

    const NAME: &'static str = GIT_FETCH;
    const DESCRIPTION: &'static str = texts::FETCH_DESCRIPTION;

    fn repository(arguments: &FetchArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn plan<'a>(
        reading: &'a Reading,
        arguments: &'a FetchArguments,
    ) -> BoxFuture<'a, Result<Plan<Fetching>, GitFailure>> {
        Box::pin(plan(reading, arguments))
    }

    fn act<'a>(
        acting: &'a Acting,
        _arguments: &'a FetchArguments,
        plan: Plan<Fetching>,
    ) -> BoxFuture<'a, Result<Answer, GitFailure>> {
        Box::pin(act(acting, plan))
    }
}

/// The value of the last setting of `key`.
pub(crate) fn setting<'a>(reading: &'a Reading, key: &str) -> Option<&'a str> {
    reading
        .settings
        .iter()
        .rev()
        .find(|setting| setting.key.eq_ignore_ascii_case(key))
        .and_then(|setting| setting.value.as_deref())
}

/// The remote of a call: the one that it names, else the branch's own,
/// else `origin`.
pub(crate) fn chosen_remote(
    reading: &Reading,
    named: Option<&str>,
    branch: Option<&str>,
) -> Result<String, GitFailure> {
    let remote = named
        .map(str::to_owned)
        .or_else(|| {
            branch.and_then(|branch| {
                setting(reading, &format!("branch.{branch}.remote")).map(str::to_owned)
            })
        })
        .unwrap_or_else(|| "origin".to_owned());
    check_branch("remote", &remote)?;
    Ok(remote)
}

/// The short name of the branch that `branch` tracks on its remote, from
/// its `merge` setting, such as `main` for `refs/heads/main`.
pub(crate) fn upstream_branch(reading: &Reading, branch: &str) -> Option<String> {
    setting(reading, &format!("branch.{branch}.merge")).map(|merge| {
        merge
            .strip_prefix("refs/heads/")
            .unwrap_or(merge)
            .to_owned()
    })
}

async fn plan(reading: &Reading, arguments: &FetchArguments) -> Result<Plan<Fetching>, GitFailure> {
    let current = current_branch(reading).await?;
    let remote = chosen_remote(reading, arguments.remote.as_deref(), current.as_deref())?;
    let endpoints = remote_endpoints(reading, &remote, false).await?;
    let integrates = arguments.integrate.is_some();
    let branch = match (&arguments.branch, integrates) {
        (Some(branch), _integrates) => Some(branch.clone()),
        (None, true) => {
            let current = current.as_deref().ok_or_else(|| {
                invalid("HEAD is detached, so there is no branch to integrate into.")
            })?;
            Some(upstream_branch(reading, current).ok_or_else(|| {
                invalid(
                    "The branch has no upstream. Pass branch, or set its upstream with git_branch.",
                )
            })?)
        }
        (None, false) => None,
    };
    if let Some(branch) = &branch {
        check_branch("branch", branch)?;
    }
    let mut command = match arguments.integrate {
        Some(integrate) => {
            no_operation(reading)?;
            check_identity(reading).await?;
            let how = match integrate {
                Integrate::Merge => "--no-rebase",
                Integrate::Rebase => "--rebase",
            };
            vec!["pull".to_owned(), how.to_owned()]
        }
        None => vec!["fetch".to_owned()],
    };
    if arguments.prune == Some(true) {
        command.push("--prune".to_owned());
    }
    command.push(remote.clone());
    command.extend(branch);
    let mut plan = Plan::new(
        Change::Fetch { integrates },
        command,
        Fetching { remote, integrates },
    );
    plan.effects.extend(endpoints);
    Ok(plan)
}

fn invalid(detail: &str) -> GitFailure {
    GitFailure::InvalidArgument {
        detail: detail.to_owned(),
    }
}

/// The commits of a remote's remote-tracking branches, by their short names,
/// such as `origin/main`. The remote's `HEAD`, which only names its default
/// branch and which git creates on a fetch, is not one of them.
pub(crate) async fn tracking(
    reading: &Reading,
    remote: &str,
) -> Result<BTreeMap<String, String>, GitFailure> {
    let ran = reading
        .git
        .output(reading.run([
            "for-each-ref".to_owned(),
            "--format=%(refname)%1f%(objectname)".to_owned(),
            format!("refs/remotes/{remote}/"),
        ]))
        .await?;
    Ok(ran
        .text()
        .lines()
        .filter_map(|line| line.split_once('\u{1f}'))
        .filter(|(name, _oid)| !name.ends_with("/HEAD"))
        .map(|(name, oid)| {
            let short = name.strip_prefix("refs/remotes/").unwrap_or(name);
            (short.to_owned(), oid.to_owned())
        })
        .collect())
}

/// How a remote-tracking branch moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Moved {
    pub(crate) name: String,
    pub(crate) from: Option<String>,
    pub(crate) to: Option<String>,
}

/// The remote-tracking branches that moved, came or went.
fn moved(before: &BTreeMap<String, String>, after: &BTreeMap<String, String>) -> Vec<Moved> {
    let mut names: Vec<&String> = before.keys().chain(after.keys()).collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .filter(|name| before.get(*name) != after.get(*name))
        .map(|name| Moved {
            name: name.clone(),
            from: before.get(name).cloned(),
            to: after.get(name).cloned(),
        })
        .collect()
}

/// A move in a line, such as `origin/main: 1a2b3c4d5e to 5d6e7f8a9b`.
fn moved_line(moved: &Moved) -> String {
    let short = |oid: &String| oid.chars().take(10).collect::<String>();
    match (&moved.from, &moved.to) {
        (Some(from), Some(to)) => format!("{}: {} to {}", moved.name, short(from), short(to)),
        (None, Some(to)) => format!("{}: new, at {}", moved.name, short(to)),
        (Some(_from), None) => format!("{}: removed", moved.name),
        (None, None) => moved.name.clone(),
    }
}

/// The card's data of a fetch.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FetchData<'a> {
    remote: &'a str,
    moved: &'a [Moved],
}

async fn act(acting: &Acting, plan: Plan<Fetching>) -> Result<Answer, GitFailure> {
    let reading = &acting.reading;
    let remote = plan.planned.remote.clone();
    let before = tracking(reading, &remote).await?;
    let head = head_commit(reading).await?;
    let ran = reading.git.run(acting.run(plan.command)).await?;
    let after = tracking(reading, &remote).await?;
    let changes = moved(&before, &after);
    let mut summary = if changes.is_empty() {
        format!("Fetched from {remote}: no remote-tracking branch moved.\n")
    } else {
        let mut text = format!("Fetched from {remote}:\n");
        for change in &changes {
            let _written = writeln!(text, "  {}", moved_line(change));
        }
        text
    };
    if plan.planned.integrates {
        let mut answer = operation_outcome(acting, "pull", head.as_deref(), &ran).await?;
        summary.push_str(&answer.text);
        answer.text = summary;
        return Ok(answer);
    }
    if !ran.succeeded() {
        return Err(refusal("fetch", &ran));
    }
    let data = fitting_data(changes.len(), |kept| FetchData {
        remote: &remote,
        moved: &changes[..kept],
    });
    Ok(Answer {
        text: summary,
        data,
    })
}
