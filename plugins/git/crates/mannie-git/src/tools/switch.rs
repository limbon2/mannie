//! `git_switch` (plugins/git/docs/tools.md, tools that change the
//! repository): switches the work tree to a branch, to a new branch, or to
//! a commit without a branch. git refuses when local changes would be
//! overwritten, and nothing changes.

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::arguments::check_branch;
use crate::changes::{Acting, ChangeTool, Plan, branch_tip, check_branch_name};
use crate::effects::Change;
use crate::failure::GitFailure;
use crate::outcome::refusal;
use crate::texts;
use crate::tools::status::{read_status, status_text};
use crate::tools::{Answer, Reading};

/// The tool's name.
pub(crate) const GIT_SWITCH: &str = "git_switch";

/// The arguments of `git_switch`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SwitchArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// The branch to switch to; with create, the new branch's name; with
    /// detach, the commit.
    target: String,
    /// Whether to create the branch target and switch to it.
    create: Option<bool>,
    /// For create: the commit to start the new branch at. HEAD when null.
    start: Option<String>,
    /// Whether to switch to the commit target without a branch.
    detach: Option<bool>,
}

/// `git_switch`.
#[derive(Debug)]
pub(crate) struct GitSwitch;

impl ChangeTool for GitSwitch {
    type Arguments = SwitchArguments;
    type Planned = ();

    const NAME: &'static str = GIT_SWITCH;
    const DESCRIPTION: &'static str = texts::SWITCH_DESCRIPTION;

    fn repository(arguments: &SwitchArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn plan<'a>(
        reading: &'a Reading,
        arguments: &'a SwitchArguments,
    ) -> BoxFuture<'a, Result<Plan<()>, GitFailure>> {
        Box::pin(plan(reading, arguments))
    }

    fn act<'a>(
        acting: &'a Acting,
        _arguments: &'a SwitchArguments,
        plan: Plan<()>,
    ) -> BoxFuture<'a, Result<Answer, GitFailure>> {
        Box::pin(async move {
            let ran = acting.reading.git.run(acting.run(plan.command)).await?;
            if !ran.succeeded() {
                return Err(refusal("switch", &ran));
            }
            let status = read_status(&acting.reading).await?;
            Ok(Answer {
                text: status_text(&status, None, None),
                data: None,
            })
        })
    }
}

fn invalid(detail: &str) -> GitFailure {
    GitFailure::InvalidArgument {
        detail: detail.to_owned(),
    }
}

async fn plan(reading: &Reading, arguments: &SwitchArguments) -> Result<Plan<()>, GitFailure> {
    let target = arguments.target.as_str();
    let create = arguments.create == Some(true);
    let detach = arguments.detach == Some(true);
    let owned = |parts: &[&str]| {
        parts
            .iter()
            .map(|part| (*part).to_owned())
            .collect::<Vec<_>>()
    };
    if create && detach {
        return Err(invalid("create and detach cannot go together."));
    }
    if arguments.start.is_some() && !create {
        return Err(invalid("start goes with create only."));
    }
    if detach {
        let commit = reading.commit("target", target).await?;
        return Ok(
            Plan::new(Change::Switch, owned(&["switch", "--detach", &commit]), ())
                .with_tip(&commit),
        );
    }
    if create {
        check_branch_name(reading, "target", target).await?;
        if branch_tip(reading, target).await?.is_some() {
            return Err(invalid(&format!("The branch {target} exists already.")));
        }
        let start = reading
            .commit("start", arguments.start.as_deref().unwrap_or("HEAD"))
            .await?;
        let command = owned(&["switch", "--no-track", "-c", target, &start]);
        return Ok(Plan::new(Change::Switch, command, ()).with_tip(&start));
    }
    // A branch's name holds no pattern, so the remotes' branches of that
    // name are the only ones that the pattern below matches.
    check_branch("target", target)?;
    if let Some(tip) = branch_tip(reading, target).await? {
        return Ok(Plan::new(Change::Switch, owned(&["switch", target]), ()).with_tip(&tip));
    }
    // A remote-tracking branch of the same name makes git create the branch,
    // tracking it.
    let remote = reading
        .git
        .output(reading.run([
            "for-each-ref".to_owned(),
            "--format=%(objectname)".to_owned(),
            format!("refs/remotes/*/{target}"),
        ]))
        .await?;
    let tips: Vec<String> = remote.text().lines().map(str::to_owned).collect();
    match tips.as_slice() {
        [tip] => Ok(Plan::new(Change::Switch, owned(&["switch", target]), ()).with_tip(tip)),
        [] => Err(GitFailure::UnknownRevision {
            revision: target.to_owned(),
        }),
        _several => Err(invalid(&format!(
            "Several remotes have a branch {target}. Create the branch with create and start, such as origin/{target}."
        ))),
    }
}
