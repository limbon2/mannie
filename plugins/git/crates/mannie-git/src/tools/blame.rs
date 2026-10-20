//! `git_blame` (plugins/git/docs/tools.md, tools that read): which commit
//! last changed each line of a range of a file, in the work tree or at a
//! revision, as runs of lines from one commit. The repository's
//! `.git-blame-ignore-revs` applies when it exists. Blame runs without text
//! conversions, which the repository's configuration could name (T-44).

use std::fmt::Write as _;

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::arguments::{check_revision, repository_path};
use crate::describe::label_commits;
use crate::failure::GitFailure;
use crate::layout::Layout;
use crate::parse::blame::{BlameRun, parse_blame};
use crate::texts;
use crate::tools::{Answer, NOTE_ROOM, ReadTool, Reading, cut_line, fitting_data};

/// The tool's name.
pub(crate) const GIT_BLAME: &str = "git_blame";

/// The file of revisions that blame skips, by the convention that GitHub
/// and GitLab follow.
const IGNORE_REVS_FILE: &str = ".git-blame-ignore-revs";

/// The arguments of `git_blame`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BlameArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// The file, relative to the repository's top.
    path: String,
    /// The revision whose file to blame, such as HEAD~5. The work tree's
    /// file, with its uncommitted lines, when null.
    revision: Option<String>,
    /// The first line, from 1. The first when null.
    start_line: Option<u32>,
    /// The last line. The last when null.
    end_line: Option<u32>,
    /// Whether to ignore changes of white space only.
    ignore_whitespace: Option<bool>,
    /// Whether to look through lines that moved or were copied from other
    /// files of the same commit, to find the commit that wrote them.
    follow_moves: Option<bool>,
}

/// `git_blame`.
#[derive(Debug)]
pub(crate) struct GitBlame;

impl ReadTool for GitBlame {
    type Arguments = BlameArguments;

    const NAME: &'static str = GIT_BLAME;
    const DESCRIPTION: &'static str = texts::BLAME_DESCRIPTION;

    fn repository(arguments: &BlameArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn check(arguments: &BlameArguments, layout: &Layout) -> Result<(), GitFailure> {
        repository_path(&layout.top, "path", &arguments.path)?;
        if let Some(revision) = &arguments.revision {
            check_revision("revision", revision)?;
        }
        if arguments.start_line == Some(0) {
            return Err(invalid("start_line must be at least 1."));
        }
        if let (Some(start), Some(end)) = (arguments.start_line, arguments.end_line)
            && end < start
        {
            return Err(invalid("end_line must be at least start_line."));
        }
        Ok(())
    }

    fn answer(
        reading: Reading,
        arguments: BlameArguments,
    ) -> BoxFuture<'static, Result<Answer, GitFailure>> {
        Box::pin(async move { blame(&reading, &arguments).await })
    }
}

fn invalid(detail: &str) -> GitFailure {
    GitFailure::InvalidArgument {
        detail: detail.to_owned(),
    }
}

/// A run with its commit's short hash.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ShownRun<'a> {
    short: &'a str,
    #[serde(flatten)]
    run: &'a BlameRun,
}

/// The card's data.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BlameData<'a> {
    path: &'a str,
    revision: Option<&'a str>,
    runs: &'a [ShownRun<'a>],
}

/// The run of `git blame` that a call asks for, of the file at `path`.
async fn blame_command(
    reading: &Reading,
    arguments: &BlameArguments,
    path: &str,
) -> Result<Vec<String>, GitFailure> {
    let mut command = vec![
        "blame".to_owned(),
        "--porcelain".to_owned(),
        "--no-textconv".to_owned(),
    ];
    if arguments.ignore_whitespace == Some(true) {
        command.push("-w".to_owned());
    }
    if arguments.follow_moves == Some(true) {
        command.push("-M".to_owned());
        command.push("-C".to_owned());
    }
    if reading.layout.top.join(IGNORE_REVS_FILE).is_file() {
        command.push(format!("--ignore-revs-file={IGNORE_REVS_FILE}"));
    }
    match (arguments.start_line, arguments.end_line) {
        (None, None) => {}
        (start, end) => {
            let start = start.unwrap_or(1);
            let end = end.map_or_else(String::new, |end| end.to_string());
            command.push(format!("-L{start},{end}"));
        }
    }
    if let Some(revision) = &arguments.revision {
        // A full hash, so blame never reads it as an option or a path.
        command.push(reading.commit("revision", revision).await?);
    }
    command.push("--".to_owned());
    command.push(path.to_owned());
    Ok(command)
}

/// The runs' lines under their commits, as many as `budget` bytes hold,
/// and the number of the last line written. The first line is written
/// whatever its length, so a call always shows something.
fn blame_lines(shown_runs: &[(String, &BlameRun)], budget: usize) -> (String, Option<u32>) {
    let mut body = String::new();
    let mut last_shown: Option<u32> = None;
    'runs: for (short, run) in shown_runs {
        let heading = if run.commit.uncommitted {
            "Not committed yet\n".to_owned()
        } else {
            format!(
                "{short} {} {}: {}\n",
                run.commit.day, run.commit.author, run.commit.summary
            )
        };
        let mut wrote_heading = false;
        for line in &run.lines {
            let numbered = format!("{:>6}\t{}\n", line.number, cut_line(&line.text));
            let needed = numbered.len() + if wrote_heading { 0 } else { heading.len() };
            if last_shown.is_some() && body.len() + needed > budget {
                break 'runs;
            }
            if !wrote_heading {
                body.push_str(&heading);
                wrote_heading = true;
            }
            body.push_str(&numbered);
            last_shown = Some(line.number);
        }
    }
    (body, last_shown)
}

async fn blame(reading: &Reading, arguments: &BlameArguments) -> Result<Answer, GitFailure> {
    let path = repository_path(&reading.layout.top, "path", &arguments.path)?;
    let command = blame_command(reading, arguments, &path).await?;
    let ran = reading.git.output(reading.run(command)).await?;
    let runs = parse_blame(&ran.text());
    let oids: Vec<String> = runs
        .iter()
        .filter(|run| !run.commit.uncommitted)
        .map(|run| run.commit.oid.clone())
        .collect();
    let labels = label_commits(reading, &oids).await?;
    let short_of = |run: &BlameRun| -> String {
        labels
            .iter()
            .find(|label| label.oid == run.commit.oid)
            .map_or_else(
                || run.commit.oid.chars().take(7).collect(),
                |label| label.short.clone(),
            )
    };
    let shown_runs: Vec<(String, &BlameRun)> =
        runs.iter().map(|run| (short_of(run), run)).collect();
    let revision = arguments.revision.as_deref();
    let place = revision.map_or_else(
        || "in the work tree".to_owned(),
        |revision| format!("at {revision}"),
    );
    let (body, last_shown) = blame_lines(&shown_runs, reading.room.saturating_sub(NOTE_ROOM));
    let first = runs
        .first()
        .and_then(|run| run.lines.first())
        .map(|line| line.number);
    let last = runs
        .last()
        .and_then(|run| run.lines.last())
        .map(|line| line.number);
    let mut text = match (first, last_shown) {
        (Some(first), Some(shown)) => format!("Blame of {path} {place}, lines {first}-{shown}:\n"),
        _empty => format!("Blame of {path} {place}: the file has no lines.\n"),
    };
    text.push_str(&body);
    if let (Some(shown), Some(last)) = (last_shown, last)
        && shown < last
    {
        let next = shown + 1;
        let _written = writeln!(
            text,
            "Lines {next}-{last} follow: call git_blame with start_line {next}."
        );
    }
    let data_runs: Vec<ShownRun<'_>> = shown_runs
        .iter()
        .map(|(short, run)| ShownRun { short, run })
        .collect();
    let data = fitting_data(data_runs.len(), |kept| BlameData {
        path: &path,
        revision,
        runs: &data_runs[..kept],
    });
    Ok(Answer { text, data })
}

#[cfg(test)]
mod tests {
    use mannie_git_testing::TestRepository;
    use serde_json::json;

    use super::GitBlame;
    use crate::tools::testing::call;

    #[tokio::test]
    async fn blame_names_the_commit_of_each_line_and_the_lines_not_committed_yet() {
        let repository = TestRepository::new();
        repository.write("a.txt", "one\ntwo\n");
        repository.commit("Start");
        repository.write("a.txt", "one\nTWO\nthree\n");
        repository.commit("Change two");
        repository.write("a.txt", "one\nTWO\nthree\nfour\n");

        let work_tree = call::<GitBlame>(&repository, json!({ "path": "a.txt" })).await;
        let earlier = call::<GitBlame>(
            &repository,
            json!({ "path": "a.txt", "revision": "HEAD~1", "start_line": 2, "end_line": 2 }),
        )
        .await;
        let outside = call::<GitBlame>(&repository, json!({ "path": "../a.txt" })).await;

        assert!(
            work_tree
                .output
                .starts_with("Blame of a.txt in the work tree, lines 1-4:\n"),
            "{}",
            work_tree.output
        );
        assert!(
            work_tree
                .output
                .contains("Test Author: Start\n     1\tone\n"),
            "{}",
            work_tree.output
        );
        assert!(
            work_tree
                .output
                .contains(": Change two\n     2\tTWO\n     3\tthree\n")
        );
        assert!(
            work_tree
                .output
                .ends_with("Not committed yet\n     4\tfour\n"),
            "{}",
            work_tree.output
        );
        assert!(
            earlier
                .output
                .starts_with("Blame of a.txt at HEAD~1, lines 2-2:\n"),
            "{}",
            earlier.output
        );
        assert!(earlier.output.contains("     2\ttwo\n"));
        assert_eq!(outside.failure.as_deref(), Some("invalid_argument"));
        let data = work_tree.data.expect("the card's data");
        assert_eq!(data["runs"][2]["commit"]["uncommitted"], true);
    }
}
