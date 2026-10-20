//! `git_commit` (plugins/git/docs/tools.md, tools that change the
//! repository): commits the staged changes, all tracked changes, or a
//! fixup of an older commit, or amends HEAD's commit. The message goes to
//! git on its standard input and is kept as written. The repository's
//! hooks run as in the owner's terminal, which the owner allows first, and
//! amending a commit that a remote holds asks too.

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::changes::{Acting, ChangeTool, Plan, current_branch};
use crate::effects::{Change, published};
use crate::failure::GitFailure;
use crate::outcome::{head_commit, refusal};
use crate::parse::diff::FileStat;
use crate::parse::log::CommitDetails;
use crate::parse::status::{Entry, EntryKind};
use crate::texts;
use crate::tools::show::shown_commit;
use crate::tools::status::read_status;
use crate::tools::{Answer, Reading, fitting_data};

/// The tool's name.
pub(crate) const GIT_COMMIT: &str = "git_commit";

/// The most characters of a commit's message.
const MAX_MESSAGE_CHARS: usize = 100_000;

/// The arguments of `git_commit`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CommitArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// The message: a subject line, then a blank line and the body. Needed
    /// unless amend keeps HEAD's message or fixup makes it.
    message: Option<String>,
    /// Whether to replace HEAD's commit instead of adding one.
    amend: Option<bool>,
    /// Whether to stage every change of tracked files first.
    all: Option<bool>,
    /// Whether to commit when nothing changed.
    allow_empty: Option<bool>,
    /// A commit of the branch, such as 1a2b3c4, that the staged changes fix.
    /// The commit's message is then "fixup!" and that commit's subject.
    fixup: Option<String>,
}

/// `git_commit`.
#[derive(Debug)]
pub(crate) struct GitCommit;

/// The message that a commit's run reads.
type Message = Option<String>;

impl ChangeTool for GitCommit {
    type Arguments = CommitArguments;
    type Planned = Message;

    const NAME: &'static str = GIT_COMMIT;
    const DESCRIPTION: &'static str = texts::COMMIT_DESCRIPTION;

    fn repository(arguments: &CommitArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn plan<'a>(
        reading: &'a Reading,
        arguments: &'a CommitArguments,
    ) -> BoxFuture<'a, Result<Plan<Message>, GitFailure>> {
        Box::pin(plan(reading, arguments))
    }

    fn act<'a>(
        acting: &'a Acting,
        _arguments: &'a CommitArguments,
        plan: Plan<Message>,
    ) -> BoxFuture<'a, Result<Answer, GitFailure>> {
        Box::pin(act(acting, plan))
    }
}

fn invalid(detail: &str) -> GitFailure {
    GitFailure::InvalidArgument {
        detail: detail.to_owned(),
    }
}

/// Checks that git knows who commits, which it reads from the owner's
/// configuration.
pub(crate) async fn check_identity(reading: &Reading) -> Result<(), GitFailure> {
    for variable in ["GIT_AUTHOR_IDENT", "GIT_COMMITTER_IDENT"] {
        let ran = reading.git.run(reading.run(["var", variable])).await?;
        if !ran.succeeded() {
            return Err(GitFailure::IdentityMissing);
        }
    }
    Ok(())
}

/// Checks a message that git reads as written.
pub(crate) fn check_message(argument: &str, message: &str) -> Result<(), GitFailure> {
    if message.trim().is_empty() {
        return Err(invalid(&format!("{argument} is empty.")));
    }
    if message.contains('\0') {
        return Err(invalid(&format!("{argument} holds a NUL character.")));
    }
    if message.chars().count() > MAX_MESSAGE_CHARS {
        return Err(invalid(&format!(
            "{argument} has more than 100,000 characters."
        )));
    }
    Ok(())
}

async fn plan(reading: &Reading, arguments: &CommitArguments) -> Result<Plan<Message>, GitFailure> {
    let amend = arguments.amend == Some(true);
    let all = arguments.all == Some(true);
    let allow_empty = arguments.allow_empty == Some(true);
    match (&arguments.message, &arguments.fixup) {
        (Some(_message), Some(_fixup)) => {
            return Err(invalid(
                "fixup makes the message, so it cannot go with message.",
            ));
        }
        (Some(message), None) => check_message("message", message)?,
        (None, None) if !amend => {
            return Err(invalid(
                "message is needed, unless amend keeps HEAD's message.",
            ));
        }
        (None, _fixup) => {}
    }
    if amend && arguments.fixup.is_some() {
        return Err(invalid(
            "fixup makes a new commit, so it cannot go with amend.",
        ));
    }
    check_identity(reading).await?;
    let head = head_commit(reading).await?;
    if amend && head.is_none() {
        return Err(invalid(
            "amend needs a commit to replace, and the branch has none.",
        ));
    }
    let fixup = match &arguments.fixup {
        Some(revision) => Some(reading.commit("fixup", revision).await?),
        None => None,
    };
    let status = read_status(reading).await?;
    let conflicted = status
        .entries
        .iter()
        .filter(|entry| matches!(entry.kind, EntryKind::Conflicted { .. }))
        .count();
    if conflicted > 0 {
        return Err(invalid(&format!(
            "{conflicted} files are conflicted. Resolve them with git_resolve first."
        )));
    }
    let has_staged = status.entries.iter().any(Entry::is_staged);
    let has_tracked_changes = status.entries.iter().any(Entry::is_unstaged);
    let has_changes = has_staged || (all && has_tracked_changes);
    if !amend && !allow_empty && !has_changes {
        return Err(invalid(
            "Nothing is staged. Stage changes with git_stage, pass all to commit every change of tracked files, or pass allow_empty.",
        ));
    }
    let mut command = vec!["commit".to_owned(), "--cleanup=whitespace".to_owned()];
    if arguments.message.is_some() {
        command.extend(["-F".to_owned(), "-".to_owned()]);
    }
    if amend {
        command.push("--amend".to_owned());
        if arguments.message.is_none() {
            command.push("--no-edit".to_owned());
        }
    }
    if all {
        command.push("--all".to_owned());
    }
    if allow_empty {
        command.push("--allow-empty".to_owned());
    }
    if let Some(fixup) = fixup {
        command.push(format!("--fixup={fixup}"));
    }
    let rewritten = match (&head, amend) {
        (Some(head), true) => published(reading, &format!("{head}^!")).await?,
        _other => None,
    };
    Ok(Plan::new(
        Change::Commit { amend, all },
        command,
        arguments.message.clone(),
    )
    .with_effect(rewritten))
}

/// The card's data: the new commit and its files.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CommitData<'a> {
    commit: &'a CommitDetails,
    branch: Option<&'a str>,
    files: &'a [FileStat],
}

async fn act(acting: &Acting, plan: Plan<Message>) -> Result<Answer, GitFailure> {
    let reading = &acting.reading;
    let mut run = acting.run(plan.command);
    if let Some(message) = plan.planned {
        run = run.with_input(message.into_bytes());
    }
    let ran = reading.git.run(run).await?;
    if !ran.succeeded() {
        return Err(refusal("commit", &ran));
    }
    let head = head_commit(reading)
        .await?
        .ok_or_else(|| GitFailure::failed("git committed, but HEAD names no commit"))?;
    let branch = current_branch(reading).await?;
    let shown = shown_commit(reading, &head, false).await?;
    let place = branch.as_deref().map_or_else(
        || " with HEAD detached".to_owned(),
        |branch| format!(" on {branch}"),
    );
    let text = format!("Committed {}{place}.\n{}", shown.details.short, shown.text);
    let data = fitting_data(shown.files.len(), |kept| CommitData {
        commit: &shown.details,
        branch: branch.as_deref(),
        files: &shown.files[..kept],
    });
    Ok(Answer { text, data })
}
