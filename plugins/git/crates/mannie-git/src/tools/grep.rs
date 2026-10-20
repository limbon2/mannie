//! `git_grep` (plugins/git/docs/tools.md, tools that read): the lines of
//! the tracked files, in the work tree or at revisions, that hold a text or
//! match a regular expression. Binary files are skipped, and the files are
//! read as they are, without text conversions.

use std::fmt::Write as _;

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::arguments::{check_revision, check_text, pathspecs};
use crate::failure::GitFailure;
use crate::layout::Layout;
use crate::parse::grep::{Match, parse_grep};
use crate::texts;
use crate::tools::{Answer, ReadTool, Reading, fitting_data};

/// The tool's name.
pub(crate) const GIT_GREP: &str = "git_grep";

/// The matches that a call lists unless it names another limit.
const DEFAULT_LIMIT: u32 = 100;

/// The most matches that a call lists.
const MAX_LIMIT: u32 = 1000;

/// The most revisions that a call searches.
const MAX_REVISIONS: usize = 20;

/// The most characters of a matching line in a result, so a match in a
/// minified file does not fill it.
const MAX_MATCH_CHARS: usize = 400;

/// The bytes of git's output that a search reads, past which the matches
/// that follow are left out.
const OUTPUT_LIMIT: u32 = 2 * 1024 * 1024;

/// The arguments of `git_grep`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GrepArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// The text to find, or with regex, a POSIX extended regular expression.
    pattern: String,
    /// Whether pattern is a regular expression. False when null.
    regex: Option<bool>,
    /// Whether to ignore case.
    ignore_case: Option<bool>,
    /// The revisions whose files to search, such as main and v1.0. The work
    /// tree's tracked files when null.
    revisions: Option<Vec<String>>,
    /// Only these files or folders, relative to the repository's top.
    paths: Option<Vec<String>>,
    /// The most matches to list, from 1 to 1000. 100 when null.
    limit: Option<u32>,
}

/// `git_grep`.
#[derive(Debug)]
pub(crate) struct GitGrep;

impl ReadTool for GitGrep {
    type Arguments = GrepArguments;

    const NAME: &'static str = GIT_GREP;
    const DESCRIPTION: &'static str = texts::GREP_DESCRIPTION;

    fn repository(arguments: &GrepArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn check(arguments: &GrepArguments, layout: &Layout) -> Result<(), GitFailure> {
        check_text("pattern", &arguments.pattern)?;
        let revisions = arguments.revisions.as_deref().unwrap_or_default();
        if revisions.len() > MAX_REVISIONS {
            return Err(invalid("revisions may name at most 20 revisions."));
        }
        for revision in revisions {
            check_revision("revisions", revision)?;
        }
        pathspecs(
            &layout.top,
            "paths",
            arguments.paths.as_deref().unwrap_or_default(),
        )?;
        if !(1..=MAX_LIMIT).contains(&arguments.limit.unwrap_or(DEFAULT_LIMIT)) {
            return Err(invalid("limit must be from 1 to 1000."));
        }
        Ok(())
    }

    fn answer(
        reading: Reading,
        arguments: GrepArguments,
    ) -> BoxFuture<'static, Result<Answer, GitFailure>> {
        Box::pin(async move { grep(&reading, &arguments).await })
    }
}

fn invalid(detail: &str) -> GitFailure {
    GitFailure::InvalidArgument {
        detail: detail.to_owned(),
    }
}

/// A match with the revision that the call named.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ShownMatch<'a> {
    revision: Option<&'a str>,
    #[serde(flatten)]
    found: &'a Match,
}

/// The card's data.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GrepData<'a> {
    pattern: &'a str,
    matches: &'a [ShownMatch<'a>],
    more: bool,
}

async fn grep(reading: &Reading, arguments: &GrepArguments) -> Result<Answer, GitFailure> {
    let mut command = vec![
        "grep".to_owned(),
        "-z".to_owned(),
        "-n".to_owned(),
        "-I".to_owned(),
        "--no-color".to_owned(),
        "--full-name".to_owned(),
    ];
    let regex = arguments.regex == Some(true);
    command.push(if regex { "-E" } else { "-F" }.to_owned());
    if arguments.ignore_case == Some(true) {
        command.push("-i".to_owned());
    }
    // After -e, a pattern that starts with - stays a pattern.
    command.push("-e".to_owned());
    command.push(arguments.pattern.clone());
    // Each revision as its full hash, which names the matches without a
    // colon of its own.
    let mut revisions: Vec<(String, &str)> = Vec::new();
    for revision in arguments.revisions.iter().flatten() {
        let oid = reading.commit("revisions", revision).await?;
        command.push(oid.clone());
        revisions.push((oid, revision));
    }
    command.push("--".to_owned());
    command.extend(pathspecs(
        &reading.layout.top,
        "paths",
        arguments.paths.as_deref().unwrap_or_default(),
    )?);
    let ran = reading
        .git
        .run(reading.run(command).with_output_limit(OUTPUT_LIMIT))
        .await?;
    // grep exits with 1 when nothing matches.
    let found_nothing = ran.exit_code == Some(1) && ran.output.is_empty();
    if !ran.succeeded() && !found_nothing && !ran.cut {
        return Err(ran.failure(&reading.layout.top));
    }
    let at_revisions = !revisions.is_empty();
    let mut matches = parse_grep(&ran.text(), at_revisions);
    if ran.cut {
        // The output's last line may stop in the middle.
        matches.pop();
    }
    let limit = usize::try_from(arguments.limit.unwrap_or(DEFAULT_LIMIT)).unwrap_or(usize::MAX);
    let more = matches.len() > limit || ran.cut;
    matches.truncate(limit);
    let shown: Vec<ShownMatch<'_>> = matches
        .iter()
        .map(|found| ShownMatch {
            revision: found.commit.as_deref().map(|oid| {
                revisions
                    .iter()
                    .find(|(revision_oid, _given)| revision_oid == oid)
                    .map_or(oid, |(_oid, given)| *given)
            }),
            found,
        })
        .collect();
    let mut text = String::new();
    if shown.is_empty() {
        text.push_str("No line matches.\n");
    }
    for shown_match in &shown {
        let line = cut_text(&shown_match.found.text);
        let found = shown_match.found;
        match shown_match.revision {
            Some(revision) => {
                let _written = writeln!(text, "{revision}:{}:{}: {line}", found.path, found.line);
            }
            None => {
                let _written = writeln!(text, "{}:{}: {line}", found.path, found.line);
            }
        }
    }
    if more {
        let _written = writeln!(
            text,
            "More lines match: narrow the search with paths or a longer pattern, or raise limit."
        );
    }
    let data = fitting_data(shown.len(), |kept| GrepData {
        pattern: &arguments.pattern,
        matches: &shown[..kept],
        more,
    });
    Ok(Answer { text, data })
}

/// A matching line cut to [`MAX_MATCH_CHARS`] characters.
fn cut_text(line: &str) -> String {
    let total = line.chars().count();
    if total <= MAX_MATCH_CHARS {
        return line.to_owned();
    }
    let kept: String = line.chars().take(MAX_MATCH_CHARS).collect();
    format!("{kept} [{} more characters]", total - MAX_MATCH_CHARS)
}

#[cfg(test)]
mod tests {
    use mannie_git_testing::TestRepository;
    use serde_json::json;

    use super::{GitGrep, cut_text};
    use crate::tools::testing::call;

    #[tokio::test]
    async fn grep_finds_text_in_the_work_tree_and_at_revisions() {
        let repository = TestRepository::new();
        repository.write("src/a.rs", "fn parse() {}\n");
        repository.write("b.txt", "-n parse\n");
        repository.commit("Start");
        repository.write("src/a.rs", "fn parse_more() {}\n");

        let found = call::<GitGrep>(&repository, json!({ "pattern": "parse" })).await;
        let at_head = call::<GitGrep>(
            &repository,
            json!({ "pattern": "-n", "revisions": ["HEAD"] }),
        )
        .await;
        let regex =
            call::<GitGrep>(&repository, json!({ "pattern": "parse_?m", "regex": true })).await;
        let none = call::<GitGrep>(&repository, json!({ "pattern": "absent" })).await;
        let in_folder =
            call::<GitGrep>(&repository, json!({ "pattern": "parse", "paths": ["src"] })).await;

        assert_eq!(
            found.output,
            "b.txt:1: -n parse\nsrc/a.rs:1: fn parse_more() {}\n"
        );
        assert_eq!(at_head.output, "HEAD:b.txt:1: -n parse\n");
        assert_eq!(regex.output, "src/a.rs:1: fn parse_more() {}\n");
        assert_eq!(none.output, "No line matches.\n");
        assert_eq!(none.failure, None);
        assert_eq!(in_folder.output, "src/a.rs:1: fn parse_more() {}\n");
        let data = at_head.data.expect("the card's data");
        assert_eq!(data["matches"][0]["revision"], "HEAD");
        assert_eq!(data["matches"][0]["line"], 1);
    }

    #[test]
    fn a_long_match_is_cut_with_a_note() {
        assert_eq!(cut_text("short"), "short");
        let cut = cut_text(&"x".repeat(450));

        assert!(cut.ends_with(" [50 more characters]"), "{cut}");
    }
}
