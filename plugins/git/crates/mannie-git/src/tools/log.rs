//! `git_log` (plugins/git/docs/tools.md, tools that read): commits newest
//! first, narrowed by revisions, paths, an author, dates, the message, the
//! text that a change adds or removes, or a changed line's pattern; the
//! history of a range of a file's lines; or the entries of a reflog. It
//! pages with `limit` and `skip`.

use std::fmt::Write as _;

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::arguments::{check_revision, check_text, pathspecs, repository_path};
use crate::failure::GitFailure;
use crate::layout::Layout;
use crate::parse::diff::{Hunk, parse_hunks};
use crate::parse::log::{
    Commit, LINES_FORMAT, LOG_FORMAT, REFLOG_FORMAT, ReflogEntry, parse_line_history, parse_log,
    parse_reflog,
};
use crate::texts;
use crate::tools::{Answer, ReadTool, Reading, fitting_data};

/// The tool's name.
pub(crate) const GIT_LOG: &str = "git_log";

/// The commits that a call lists unless it names another limit.
const DEFAULT_LIMIT: u32 = 30;

/// The most commits that a call lists.
const MAX_LIMIT: u32 = 200;

/// The most commits that a call skips.
const MAX_SKIP: u32 = 1_000_000;

/// The arguments of `git_log`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LogArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// Revisions and ranges whose commits to list, such as main..feature,
    /// v1.0..HEAD, or main and ^origin/main together. HEAD when null. With
    /// reflog, the branch or HEAD whose reflog to list.
    revisions: Option<Vec<String>>,
    /// Whether to list the commits of every branch, tag and remote branch.
    all: Option<bool>,
    /// Only commits that change these files or folders, relative to the
    /// repository's top.
    paths: Option<Vec<String>>,
    /// Only commits whose author's name or email holds this text.
    author: Option<String>,
    /// Only commits after this date, such as 2026-10-01 or "2 weeks ago".
    since: Option<String>,
    /// Only commits before this date.
    until: Option<String>,
    /// Only commits whose message holds this text.
    message: Option<String>,
    /// Only commits whose change adds or removes this text, such as a
    /// function's name.
    added_or_removed: Option<String>,
    /// Only commits with an added or removed line that matches this POSIX
    /// extended regular expression.
    matching: Option<String>,
    /// Follow these lines of a file back through history, and show each
    /// commit's change to them. Not with paths.
    lines: Option<LineRange>,
    /// Whether to follow only the first parent of merges, which lists a
    /// branch's own commits and the merges into it.
    first_parent: Option<bool>,
    /// "only" lists only merge commits, "exclude" leaves them out.
    merges: Option<Merges>,
    /// Whether to list the entries of the reflog of the first revision
    /// instead of its history: where it pointed, newest first.
    reflog: Option<bool>,
    /// The most commits to list, from 1 to 200. 30 when null.
    limit: Option<u32>,
    /// The commits to skip before the first listed, to read the next page.
    skip: Option<u32>,
}

/// A range of a file's lines.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LineRange {
    /// The file, relative to the repository's top.
    path: String,
    /// The first line, from 1.
    start: u32,
    /// The last line.
    end: u32,
}

/// Which commits a list keeps by whether they merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Merges {
    Only,
    Exclude,
}

/// `git_log`.
#[derive(Debug)]
pub(crate) struct GitLog;

impl ReadTool for GitLog {
    type Arguments = LogArguments;

    const NAME: &'static str = GIT_LOG;
    const DESCRIPTION: &'static str = texts::LOG_DESCRIPTION;

    fn repository(arguments: &LogArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn check(arguments: &LogArguments, layout: &Layout) -> Result<(), GitFailure> {
        for revision in arguments.revisions.iter().flatten() {
            check_revision("revisions", revision)?;
        }
        pathspecs(
            &layout.top,
            "paths",
            arguments.paths.as_deref().unwrap_or_default(),
        )?;
        let texts = [
            ("author", &arguments.author),
            ("since", &arguments.since),
            ("until", &arguments.until),
            ("message", &arguments.message),
            ("added_or_removed", &arguments.added_or_removed),
            ("matching", &arguments.matching),
        ];
        for (argument, text) in texts {
            if let Some(text) = text {
                check_text(argument, text)?;
            }
        }
        if let Some(lines) = &arguments.lines {
            check_lines(lines, layout, arguments.paths.as_deref())?;
        }
        if arguments.reflog == Some(true) && arguments.all == Some(true) {
            return Err(invalid(
                "reflog lists one reflog, so it cannot go with all.",
            ));
        }
        if arguments.reflog == Some(true) && arguments.lines.is_some() {
            return Err(invalid("reflog cannot go with lines."));
        }
        let limit = arguments.limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(invalid("limit must be from 1 to 200."));
        }
        if arguments.skip.unwrap_or(0) > MAX_SKIP {
            return Err(invalid("skip must be at most 1000000."));
        }
        Ok(())
    }

    fn answer(
        reading: Reading,
        arguments: LogArguments,
    ) -> BoxFuture<'static, Result<Answer, GitFailure>> {
        Box::pin(async move { log(&reading, &arguments).await })
    }
}

fn invalid(detail: &str) -> GitFailure {
    GitFailure::InvalidArgument {
        detail: detail.to_owned(),
    }
}

fn check_lines(
    lines: &LineRange,
    layout: &Layout,
    paths: Option<&[String]>,
) -> Result<(), GitFailure> {
    if paths.is_some_and(|paths| !paths.is_empty()) {
        return Err(invalid(
            "lines follows one file, so it cannot go with paths.",
        ));
    }
    repository_path(&layout.top, "lines.path", &lines.path)?;
    if lines.start == 0 || lines.end < lines.start {
        return Err(invalid(
            "lines.start must be at least 1, and lines.end at least lines.start.",
        ));
    }
    Ok(())
}

/// The card's data, by what the call listed.
#[derive(Debug, Serialize)]
#[serde(
    tag = "mode",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum LogData<'a> {
    Commits {
        commits: &'a [Commit],
        total: usize,
        next_skip: Option<u32>,
    },
    Reflog {
        entries: &'a [ReflogEntry],
        total: usize,
        next_skip: Option<u32>,
    },
    Lines {
        path: &'a str,
        changes: Vec<LineChange<'a>>,
        total: usize,
        next_skip: Option<u32>,
    },
}

/// A commit's change to the lines that a call follows.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LineChange<'a> {
    commit: &'a Commit,
    hunks: Vec<Hunk>,
}

/// The options that narrow every mode, before the revisions.
fn filters(arguments: &LogArguments) -> Vec<String> {
    let mut options = Vec::new();
    let limit = arguments.limit.unwrap_or(DEFAULT_LIMIT);
    // One more than the limit tells whether another page follows.
    options.push(format!("--max-count={}", limit + 1));
    options.push(format!("--skip={}", arguments.skip.unwrap_or(0)));
    if arguments.first_parent == Some(true) {
        options.push("--first-parent".to_owned());
    }
    match arguments.merges {
        Some(Merges::Only) => options.push("--merges".to_owned()),
        Some(Merges::Exclude) => options.push("--no-merges".to_owned()),
        None => {}
    }
    if arguments.author.is_some() || arguments.message.is_some() {
        // The author and the message are text, never patterns.
        options.push("--fixed-strings".to_owned());
    }
    let valued = [
        ("--author=", &arguments.author),
        ("--grep=", &arguments.message),
        ("--since=", &arguments.since),
        ("--until=", &arguments.until),
        ("-S", &arguments.added_or_removed),
        ("-G", &arguments.matching),
    ];
    for (option, value) in valued {
        if let Some(value) = value {
            options.push(format!("{option}{value}"));
        }
    }
    options
}

/// The revisions after `--end-of-options`, HEAD when none.
fn revisions(arguments: &LogArguments) -> Vec<String> {
    let mut revisions: Vec<String> = arguments.revisions.clone().unwrap_or_default();
    if revisions.is_empty() && arguments.all != Some(true) {
        revisions.push("HEAD".to_owned());
    }
    revisions
}

async fn log(reading: &Reading, arguments: &LogArguments) -> Result<Answer, GitFailure> {
    if arguments.reflog == Some(true) {
        return reflog(reading, arguments).await;
    }
    if let Some(lines) = &arguments.lines {
        return line_history(reading, arguments, lines).await;
    }
    let mut command = vec!["log".to_owned(), "-z".to_owned(), LOG_FORMAT.to_owned()];
    command.extend(filters(arguments));
    if arguments.all == Some(true) {
        command.push("--all".to_owned());
    }
    command.push("--end-of-options".to_owned());
    command.extend(revisions(arguments));
    command.push("--".to_owned());
    command.extend(pathspecs(
        &reading.layout.top,
        "paths",
        arguments.paths.as_deref().unwrap_or_default(),
    )?);
    let ran = reading.git.output(reading.run(command)).await?;
    let mut commits =
        parse_log(&ran.output).map_err(|error| GitFailure::failed(&error.to_string()))?;
    let page = Page::of(&mut commits, arguments);
    let mut text = String::new();
    if commits.is_empty() {
        text.push_str("No commits match.\n");
    }
    for commit in &commits {
        text.push_str(&commit_line(commit));
        text.push('\n');
    }
    page.note(&mut text, "commits");
    let data = fitting_data(commits.len(), |kept| LogData::Commits {
        commits: &commits[..kept],
        total: commits.len(),
        next_skip: page.next_skip,
    });
    Ok(Answer { text, data })
}

async fn reflog(reading: &Reading, arguments: &LogArguments) -> Result<Answer, GitFailure> {
    let mut command = vec![
        "log".to_owned(),
        "-z".to_owned(),
        "--walk-reflogs".to_owned(),
        REFLOG_FORMAT.to_owned(),
    ];
    command.extend(filters(arguments));
    command.push("--end-of-options".to_owned());
    command.extend(revisions(arguments).into_iter().take(1));
    command.push("--".to_owned());
    command.extend(pathspecs(
        &reading.layout.top,
        "paths",
        arguments.paths.as_deref().unwrap_or_default(),
    )?);
    let ran = reading.git.output(reading.run(command)).await?;
    let mut entries =
        parse_reflog(&ran.output).map_err(|error| GitFailure::failed(&error.to_string()))?;
    let page = Page::of(&mut entries, arguments);
    let mut text = String::new();
    if entries.is_empty() {
        text.push_str("The reflog has no entries that match.\n");
    }
    for entry in &entries {
        let _written = writeln!(
            text,
            "{} {} {}: {}",
            entry.selector, entry.short, entry.action, entry.subject
        );
    }
    page.note(&mut text, "entries");
    let data = fitting_data(entries.len(), |kept| LogData::Reflog {
        entries: &entries[..kept],
        total: entries.len(),
        next_skip: page.next_skip,
    });
    Ok(Answer { text, data })
}

async fn line_history(
    reading: &Reading,
    arguments: &LogArguments,
    lines: &LineRange,
) -> Result<Answer, GitFailure> {
    let path = repository_path(&reading.layout.top, "lines.path", &lines.path)?;
    let mut command = vec![
        "log".to_owned(),
        LINES_FORMAT.to_owned(),
        "--no-ext-diff".to_owned(),
        "--no-textconv".to_owned(),
        format!("-L{},{}:{path}", lines.start, lines.end),
    ];
    command.extend(filters(arguments));
    command.push("--end-of-options".to_owned());
    command.extend(revisions(arguments));
    let ran = reading.git.output(reading.run(command)).await?;
    let mut history =
        parse_line_history(&ran.text()).map_err(|error| GitFailure::failed(&error.to_string()))?;
    let page = Page::of(&mut history, arguments);
    let mut text = String::new();
    if history.is_empty() {
        text.push_str("No commits changed these lines.\n");
    }
    for (commit, patch) in &history {
        text.push_str(&commit_line(commit));
        text.push('\n');
        text.push_str(patch.trim_end_matches('\n'));
        text.push_str("\n\n");
    }
    page.note(&mut text, "commits");
    let data = fitting_data(history.len(), |kept| LogData::Lines {
        path: &path,
        changes: history[..kept]
            .iter()
            .map(|(commit, patch)| LineChange {
                commit,
                hunks: parse_hunks(patch),
            })
            .collect(),
        total: history.len(),
        next_skip: page.next_skip,
    });
    Ok(Answer { text, data })
}

/// Whether another page follows a list, which git told by listing one more
/// item than the limit.
struct Page {
    next_skip: Option<u32>,
}

impl Page {
    /// Cuts `items` to the call's limit and finds the next page's skip.
    fn of<T>(items: &mut Vec<T>, arguments: &LogArguments) -> Self {
        let limit = arguments.limit.unwrap_or(DEFAULT_LIMIT);
        let shown = usize::try_from(limit).unwrap_or(usize::MAX);
        let more = items.len() > shown;
        items.truncate(shown);
        let next_skip = more.then(|| arguments.skip.unwrap_or(0) + limit);
        Self { next_skip }
    }

    /// Tells the model how to read the next page.
    fn note(&self, text: &mut String, things: &str) {
        if let Some(skip) = self.next_skip {
            let _written = writeln!(text, "More {things} follow: call git_log with skip {skip}.");
        }
    }
}

/// A commit in one line: its short hash, day, author and subject, the
/// branches and tags that point at it, and whether it merges.
fn commit_line(commit: &Commit) -> String {
    let mut line = format!(
        "{} {} {}: {}",
        commit.short,
        commit.day(),
        commit.author,
        commit.subject
    );
    if !commit.refs.is_empty() {
        let _written = write!(line, " ({})", commit.refs.join(", "));
    }
    if commit.is_merge() {
        line.push_str(" [merge]");
    }
    line
}

#[cfg(test)]
mod tests {
    use mannie_git_testing::TestRepository;
    use serde_json::json;

    use super::{GitLog, LogArguments, Merges, Page, commit_line, filters, revisions};
    use crate::parse::log::Commit;
    use crate::tools::testing::call;

    fn history() -> TestRepository {
        let repository = TestRepository::new();
        repository.write("a.txt", "one\n");
        repository.commit("Add a");
        repository.write("b.txt", "one\n");
        repository.commit("Add b");
        repository.write("a.txt", "two\n");
        repository.commit("Change a");
        repository
    }

    #[tokio::test]
    async fn the_log_pages_and_narrows_by_path_and_message() {
        let repository = history();

        let first = call::<GitLog>(&repository, json!({ "limit": 2 })).await;
        let next = call::<GitLog>(&repository, json!({ "limit": 2, "skip": 2 })).await;
        let of_a = call::<GitLog>(&repository, json!({ "paths": ["a.txt"] })).await;
        let added = call::<GitLog>(&repository, json!({ "message": "Add" })).await;

        assert!(
            first
                .output
                .contains("Test Author: Change a (HEAD -> main)\n"),
            "{}",
            first.output
        );
        assert!(
            first
                .output
                .ends_with("More commits follow: call git_log with skip 2.\n")
        );
        assert!(
            next.output.contains(": Add a\n") && !next.output.contains("More"),
            "{}",
            next.output
        );
        assert_eq!(of_a.output.lines().count(), 2, "{}", of_a.output);
        assert_eq!(added.output.lines().count(), 2, "{}", added.output);
        let data = first.data.expect("the card's data");
        assert_eq!(data["mode"], "commits");
        assert_eq!(data["nextSkip"], 2);
        assert_eq!(data["commits"][0]["subject"], "Change a");
    }

    #[tokio::test]
    async fn the_log_follows_lines_and_walks_the_reflog() {
        let repository = history();
        repository.git(&["reset", "--quiet", "--hard", "HEAD~1"]);

        let lines = call::<GitLog>(
            &repository,
            json!({ "lines": { "path": "a.txt", "start": 1, "end": 1 }, "revisions": ["HEAD@{1}"] }),
        )
        .await;
        let reflog = call::<GitLog>(&repository, json!({ "reflog": true, "limit": 1 })).await;

        assert!(lines.output.contains(": Change a\n"), "{}", lines.output);
        assert!(lines.output.contains("+two\n"), "{}", lines.output);
        assert_eq!(lines.data.as_ref().unwrap()["mode"], "lines");
        assert!(reflog.output.starts_with("HEAD@{0} "), "{}", reflog.output);
        assert!(
            reflog.output.contains("reset: moving to HEAD~1"),
            "{}",
            reflog.output
        );
    }

    #[tokio::test]
    async fn options_in_place_of_revisions_and_lines_with_paths_are_refused() {
        let repository = history();

        let option = call::<GitLog>(&repository, json!({ "revisions": ["--output=x"] })).await;
        let both = call::<GitLog>(
            &repository,
            json!({ "lines": { "path": "a.txt", "start": 1, "end": 1 }, "paths": ["a.txt"] }),
        )
        .await;
        let unknown = call::<GitLog>(&repository, json!({ "revisions": ["nope"] })).await;

        assert_eq!(
            option.failure.as_deref(),
            Some("invalid_argument"),
            "{}",
            option.output
        );
        assert_eq!(
            both.failure.as_deref(),
            Some("invalid_argument"),
            "{}",
            both.output
        );
        assert_eq!(
            unknown.failure.as_deref(),
            Some("git_failed"),
            "{}",
            unknown.output
        );
    }

    fn arguments(json: &str) -> LogArguments {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn filters_turn_each_argument_into_one_option_and_text_into_fixed_strings() {
        let options = filters(&arguments(
            r#"{"author": "Jane", "message": "fix (parser)", "since": "2 weeks ago",
                "added_or_removed": "parse(", "matching": "fn [a-z]+", "first_parent": true,
                "merges": "exclude", "limit": 10, "skip": 20}"#,
        ));

        assert_eq!(
            options,
            [
                "--max-count=11",
                "--skip=20",
                "--first-parent",
                "--no-merges",
                "--fixed-strings",
                "--author=Jane",
                "--grep=fix (parser)",
                "--since=2 weeks ago",
                "-Sparse(",
                "-Gfn [a-z]+",
            ]
        );
        assert_eq!(
            arguments(r#"{"merges": "only"}"#).merges,
            Some(Merges::Only)
        );
    }

    #[test]
    fn the_revisions_are_head_unless_the_call_names_some_or_all() {
        assert_eq!(revisions(&arguments("{}")), ["HEAD"]);
        assert_eq!(
            revisions(&arguments(r#"{"all": true}"#)),
            Vec::<String>::new()
        );
        assert_eq!(
            revisions(&arguments(r#"{"revisions": ["main..feature"]}"#)),
            ["main..feature"]
        );
    }

    #[test]
    fn a_page_keeps_the_limit_and_names_the_next_skip() {
        let call = arguments(r#"{"limit": 2, "skip": 4}"#);
        let mut items = vec![1, 2, 3];

        let page = Page::of(&mut items, &call);
        let mut text = String::new();
        page.note(&mut text, "commits");

        assert_eq!(items, [1, 2]);
        assert_eq!(text, "More commits follow: call git_log with skip 6.\n");
        let mut last = vec![1];
        assert_eq!(Page::of(&mut last, &call).next_skip, None);
    }

    #[test]
    fn a_commit_line_names_its_refs_and_whether_it_merges() {
        let commit = Commit {
            oid: "1".repeat(40),
            short: "1111111".to_owned(),
            parents: vec!["a".repeat(40), "b".repeat(40)],
            author: "Jane Doe".to_owned(),
            email: "jane@example.com".to_owned(),
            authored: "2026-10-19T14:02:11+02:00".to_owned(),
            committed: "2026-10-19T14:02:11+02:00".to_owned(),
            refs: vec!["HEAD -> main".to_owned(), "tag: v1.0".to_owned()],
            subject: "Merge branch 'feature'".to_owned(),
        };

        assert_eq!(
            commit_line(&commit),
            "1111111 2026-10-19 Jane Doe: Merge branch 'feature' (HEAD -> main, tag: v1.0) [merge]"
        );
    }
}
