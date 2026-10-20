//! `git_push` (plugins/git/docs/tools.md, tools that reach a remote):
//! pushes a branch to its branch of the same name on a remote, or to its
//! upstream there. It names the remote's push URL without credentials and
//! asks the owner, unless a grant, a rule for that URL or full access
//! covers it (T-49).
//!
//! A push never forces without a lease: with `force_with_lease`, git
//! replaces the remote's branch only while it still is at the commit that
//! the preparation read from the remote-tracking branch, and the commits
//! that it would replace ask as rewritten published work (T-50).

use std::fmt::Write as _;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::ProgramEffect;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::arguments::check_branch;
use crate::changes::{Acting, ChangeTool, Plan, branch_tip, current_branch};
use crate::effects::{Change, remote_endpoints};
use crate::failure::GitFailure;
use crate::outcome::refusal;
use crate::texts;
use crate::tools::fetch::{chosen_remote, setting, upstream_branch};
use crate::tools::{Answer, Reading, fitting_data};

/// The tool's name.
pub(crate) const GIT_PUSH: &str = "git_push";

/// The arguments of `git_push`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PushArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// The remote, such as `origin`. The branch's push remote or remote, or
    /// `origin`, when null.
    remote: Option<String>,
    /// The local branch to push. The current branch when null.
    branch: Option<String>,
    /// Whether to make the remote's branch the local branch's upstream.
    set_upstream: Option<bool>,
    /// Whether to replace the remote's branch even when it holds commits that
    /// the local branch lacks, but only while it is still where this call
    /// read it.
    force_with_lease: Option<bool>,
    /// Whether to also push the annotated tags that point into the pushed
    /// commits.
    tags: Option<bool>,
}

/// What a push keeps for its run.
#[derive(Debug, Clone)]
pub(crate) struct Pushing {
    remote: String,
    branch: String,
    destination: String,
}

/// `git_push`.
#[derive(Debug)]
pub(crate) struct GitPush;

impl ChangeTool for GitPush {
    type Arguments = PushArguments;
    type Planned = Pushing;

    const NAME: &'static str = GIT_PUSH;
    const DESCRIPTION: &'static str = texts::PUSH_DESCRIPTION;

    fn repository(arguments: &PushArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn plan<'a>(
        reading: &'a Reading,
        arguments: &'a PushArguments,
    ) -> BoxFuture<'a, Result<Plan<Pushing>, GitFailure>> {
        Box::pin(plan(reading, arguments))
    }

    fn act<'a>(
        acting: &'a Acting,
        _arguments: &'a PushArguments,
        plan: Plan<Pushing>,
    ) -> BoxFuture<'a, Result<Answer, GitFailure>> {
        Box::pin(act(acting, plan))
    }
}

fn invalid(detail: &str) -> GitFailure {
    GitFailure::InvalidArgument {
        detail: detail.to_owned(),
    }
}

async fn plan(reading: &Reading, arguments: &PushArguments) -> Result<Plan<Pushing>, GitFailure> {
    let branch = match &arguments.branch {
        Some(branch) => branch.clone(),
        None => current_branch(reading)
            .await?
            .ok_or_else(|| invalid("HEAD is detached. Pass the branch to push."))?,
    };
    check_branch("branch", &branch)?;
    let tip = branch_tip(reading, &branch)
        .await?
        .ok_or_else(|| GitFailure::UnknownRevision {
            revision: branch.clone(),
        })?;
    let push_remote = setting(reading, &format!("branch.{branch}.pushRemote"))
        .or_else(|| setting(reading, "remote.pushDefault"))
        .map(str::to_owned);
    let named = arguments.remote.clone().or(push_remote);
    let remote = chosen_remote(reading, named.as_deref(), Some(&branch))?;
    let endpoints = remote_endpoints(reading, &remote, true).await?;
    // The branch goes to its upstream when it tracks a branch of this remote,
    // and to a branch of its own name otherwise.
    let tracks_this_remote =
        setting(reading, &format!("branch.{branch}.remote")) == Some(remote.as_str());
    let destination = upstream_branch(reading, &branch)
        .filter(|_upstream| tracks_this_remote)
        .unwrap_or_else(|| branch.clone());
    check_branch("destination", &destination)?;
    let tracking = reading
        .commit(
            "remote branch",
            &format!("refs/remotes/{remote}/{destination}"),
        )
        .await
        .ok();
    let mut command = vec![
        "push".to_owned(),
        "--porcelain".to_owned(),
        "--recurse-submodules=no".to_owned(),
    ];
    if arguments.set_upstream == Some(true) {
        command.push("--set-upstream".to_owned());
    }
    if arguments.tags == Some(true) {
        command.push("--follow-tags".to_owned());
    }
    let mut rewritten = None;
    if arguments.force_with_lease == Some(true) {
        // The lease holds the commit that the call read, or that the branch
        // does not exist there yet.
        let expected = tracking.clone().unwrap_or_default();
        command.push(format!(
            "--force-with-lease=refs/heads/{destination}:{expected}"
        ));
        if let Some(tracking) = &tracking {
            let replaced = reading
                .git
                .output(reading.run(["rev-list".to_owned(), format!("{tip}..{tracking}")]))
                .await?;
            let count = replaced.text().lines().count();
            if count > 0 {
                rewritten = Some(ProgramEffect::RewritesPublished {
                    count: u32::try_from(count).unwrap_or(u32::MAX),
                    places: vec![format!("{remote}/{destination}")],
                });
            }
        }
    }
    command.push(remote.clone());
    command.push(format!("refs/heads/{branch}:refs/heads/{destination}"));
    let pushing = Pushing {
        remote,
        branch,
        destination,
    };
    let mut plan = Plan::new(Change::Push, command, pushing)
        .with_tip(&tip)
        .with_effect(rewritten);
    if let Some(tracking) = &tracking {
        plan = plan.with_tip(tracking);
    }
    plan.effects.extend(endpoints);
    Ok(plan)
}

/// A ref that a push updated, as `git push --porcelain` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Pushed {
    /// git's flag: a space for a fast-forward, `+` for a forced update, `*`
    /// for a new ref, `=` for one up to date, `!` for a rejected one.
    pub(crate) flag: char,
    pub(crate) destination: String,
    pub(crate) summary: String,
}

/// Reads the lines of `git push --porcelain` that name refs:
/// `<flag>\t<from>:<to>\t<summary>`.
pub(crate) fn parse_pushed(output: &str) -> Vec<Pushed> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let flag = fields.next()?.chars().next()?;
            let (_from, destination) = fields.next()?.split_once(':')?;
            let summary = fields.next().unwrap_or_default().trim().to_owned();
            Some(Pushed {
                flag,
                destination: destination.to_owned(),
                summary,
            })
        })
        .collect()
}

/// The card's data of a push.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PushData<'a> {
    remote: &'a str,
    branch: &'a str,
    pushed: &'a [Pushed],
}

async fn act(acting: &Acting, plan: Plan<Pushing>) -> Result<Answer, GitFailure> {
    let pushing = &plan.planned;
    let ran = acting.reading.git.run(acting.run(plan.command)).await?;
    let pushed = parse_pushed(&ran.text());
    let rejected = pushed.iter().any(|update| update.flag == '!');
    if !ran.succeeded() || rejected {
        return Err(refusal("push", &ran));
    }
    let mut text = format!(
        "Pushed {} to {} as {}.\n",
        pushing.branch, pushing.remote, pushing.destination
    );
    for update in &pushed {
        let how = match update.flag {
            '+' => "forced",
            '*' => "new",
            '=' => "already up to date",
            '-' => "deleted",
            _fast_forward => "fast-forward",
        };
        let _written = writeln!(text, "  {}: {} ({how})", update.destination, update.summary);
    }
    let data = fitting_data(pushed.len(), |kept| PushData {
        remote: &pushing.remote,
        branch: &pushing.branch,
        pushed: &pushed[..kept],
    });
    Ok(Answer { text, data })
}

#[cfg(test)]
mod tests {
    use super::{Pushed, parse_pushed};

    #[test]
    fn porcelain_lines_name_each_ref_with_its_flag() {
        let output = "To https://example.com/app.git\n \trefs/heads/main:refs/heads/main\t1a2b3c4..5d6e7f8\n\
!\trefs/heads/old:refs/heads/old\t[rejected] (fetch first)\nDone\n";

        assert_eq!(
            parse_pushed(output),
            [
                Pushed {
                    flag: ' ',
                    destination: "refs/heads/main".to_owned(),
                    summary: "1a2b3c4..5d6e7f8".to_owned(),
                },
                Pushed {
                    flag: '!',
                    destination: "refs/heads/old".to_owned(),
                    summary: "[rejected] (fetch first)".to_owned(),
                },
            ]
        );
    }
}
