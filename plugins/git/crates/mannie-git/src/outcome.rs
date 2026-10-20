//! How a change that can stop ended (plugins/git/docs/conflicts-and-
//! history.md, operations that can stop): completed, with the new HEAD and
//! the commits that it made; stopped, with the operation's step and its
//! conflicted files; or refused, having changed nothing, with git's reason.
//! The operation's state stays git's own, so the owner's git continues it.

use std::fmt::Write as _;

use serde::Serialize;

use crate::changes::Acting;
use crate::describe::{CommitLabel, OperationView, label_commits, operation_view};
use crate::failure::GitFailure;
use crate::git::Ran;
use crate::parse::log::{LOG_FORMAT, parse_log};
use crate::tools::Answer;
use crate::tools::conflicts::{ConflictedFile, Markers, conflicted_files};
use crate::tools::status::conflict_words;
use crate::tools::{Reading, fitting_data};

/// The most commits that a completed operation lists.
const MAX_LISTED_COMMITS: usize = 50;

/// The card's data of an operation that can stop.
#[derive(Debug, Serialize)]
#[serde(
    tag = "outcome",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum OutcomeData<'a> {
    Completed {
        operation: &'a str,
        head: Option<&'a CommitLabel>,
        commits: &'a [CommitLabel],
    },
    Stopped {
        operation: &'a str,
        stopped: Option<&'a OperationView>,
        files: &'a [ConflictedFile],
    },
}

/// HEAD's commit, or `None` on a branch without commits.
pub(crate) async fn head_commit(reading: &Reading) -> Result<Option<String>, GitFailure> {
    let ran = reading
        .git
        .run(reading.run(["rev-parse", "--verify", "--quiet", "HEAD"]))
        .await?;
    Ok(ran.succeeded().then(|| ran.text().trim().to_owned()))
}

/// The commits after `since` up to HEAD, newest first, at most
/// [`MAX_LISTED_COMMITS`].
pub(crate) async fn new_commits(
    reading: &Reading,
    since: Option<&str>,
) -> Result<Vec<CommitLabel>, GitFailure> {
    let range = match since {
        Some(since) => format!("{since}..HEAD"),
        None => "HEAD".to_owned(),
    };
    let ran = reading
        .git
        .run(reading.run([
            "log".to_owned(),
            "-z".to_owned(),
            LOG_FORMAT.to_owned(),
            format!("--max-count={MAX_LISTED_COMMITS}"),
            range,
        ]))
        .await?;
    if !ran.succeeded() {
        return Ok(Vec::new());
    }
    let commits = parse_log(&ran.output).map_err(|error| GitFailure::failed(&error.to_string()))?;
    Ok(commits
        .into_iter()
        .map(|commit| CommitLabel {
            oid: commit.oid,
            short: commit.short,
            subject: commit.subject,
        })
        .collect())
}

/// How a run of `operation`, such as `merge`, ended, from what it left: an
/// operation that stopped, files that conflict, or a new HEAD. `since` is
/// the commit after which the commits that it made start, such as HEAD's
/// commit before a merge or the new base of a rebase.
pub(crate) async fn operation_outcome(
    acting: &Acting,
    operation: &str,
    since: Option<&str>,
    ran: &Ran,
) -> Result<Answer, GitFailure> {
    let reading = &acting.reading;
    let stopped = operation_view(reading).await?;
    let files = conflicted_files(reading).await?;
    if stopped.is_some() || !files.is_empty() {
        return Ok(stopped_answer(operation, stopped.as_ref(), &files, ran));
    }
    if !ran.succeeded() {
        return Err(refusal(operation, ran));
    }
    let head = match head_commit(reading).await? {
        Some(head) => label_commits(reading, &[head]).await?.into_iter().next(),
        None => None,
    };
    let commits = new_commits(reading, since).await?;
    let mut text = match &head {
        Some(head) => format!("The {operation} completed. HEAD is at {}.\n", head.text()),
        None => format!("The {operation} completed.\n"),
    };
    if commits.is_empty() {
        text.push_str("It made no new commits.\n");
    } else {
        let _written = writeln!(text, "New commits, newest first ({}):", commits.len());
        for commit in &commits {
            let _written = writeln!(text, "  {}", commit.text());
        }
    }
    let data = fitting_data(commits.len(), |kept| OutcomeData::Completed {
        operation,
        head: head.as_ref(),
        commits: &commits[..kept],
    });
    Ok(Answer { text, data })
}

/// What a run that git refused said, without changing anything.
pub(crate) fn refusal(operation: &str, ran: &Ran) -> GitFailure {
    let message = if ran.errors.trim().is_empty() {
        ran.text()
    } else {
        ran.errors.clone()
    };
    GitFailure::refused(operation, &message)
}

fn stopped_answer(
    operation: &str,
    stopped: Option<&OperationView>,
    files: &[ConflictedFile],
    ran: &Ran,
) -> Answer {
    let mut text = match stopped {
        Some(view) => format!("{}\n", view.text()),
        None => format!("The {operation} left conflicted files.\n"),
    };
    if files.is_empty() {
        // An operation also stops without conflicts, such as on a commit
        // that became empty, and git says why.
        let _written = writeln!(
            text,
            "It stopped without conflicts. git said: {}",
            ran.errors.trim()
        );
        text.push_str("Call git_operation with continue, skip or abort.\n");
    } else {
        let _written = writeln!(text, "Conflicted files ({}):", files.len());
        for file in files {
            let left = match &file.markers {
                Some(Markers::Left { hunks }) if *hunks > 0 => format!(", {hunks} conflicts"),
                _other => String::new(),
            };
            let _written = writeln!(
                text,
                "  {}: {}{left}",
                file.path,
                conflict_words(file.conflict)
            );
        }
        if stopped.is_some() {
            text.push_str(
                "Read them with git_conflicts, resolve each file with git_resolve or edit_file, then call git_operation with continue, or with abort to return to where it started.\n",
            );
        } else {
            text.push_str(
                "Read them with git_conflicts and resolve each file with git_resolve or edit_file. No operation waits to continue.\n",
            );
        }
    }
    let data = fitting_data(files.len(), |kept| OutcomeData::Stopped {
        operation,
        stopped,
        files: &files[..kept],
    });
    Answer { text, data }
}
