//! `git_diff` (plugins/git/docs/tools.md, tools that read): the changed
//! files with their line counts, then the patch of each file that fits the
//! result. A longer diff continues with `offset`, which skips the files
//! already read.
//!
//! The patch runs without external diff programs and text conversions,
//! which the repository's configuration could name, and shows submodules as
//! their commits only, so no git runs inside one (T-44).

use std::fmt::Write as _;

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::arguments::{check_revision, pathspecs};
use crate::failure::GitFailure;
use crate::git::Ran;
use crate::layout::Layout;
use crate::parse::diff::{FileStat, parse_hunks, parse_numstat, split_patch};
use crate::texts;
use crate::tools::show::FilePatch;
use crate::tools::{Answer, NOTE_ROOM, ReadTool, Reading, count, fitting_data};

/// The tool's name.
pub(crate) const GIT_DIFF: &str = "git_diff";

/// The options of every patch and line count: no external diff program or
/// text conversion, submodules as their commits, and renames found.
pub(crate) const PATCH_OPTIONS: [&str; 4] = [
    "--no-ext-diff",
    "--no-textconv",
    "--submodule=short",
    "--find-renames",
];

/// The most lines of context around a change.
const MAX_CONTEXT_LINES: u32 = 100;

/// The arguments of `git_diff`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiffArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// The revision to compare from, such as HEAD~3 or main, or a range such
    /// as main...feature for the changes on feature since it left main. The
    /// index when null, or HEAD with staged.
    from: Option<String>,
    /// The revision to compare to. The work tree when null, or the index
    /// with staged.
    to: Option<String>,
    /// Whether to compare the index, which holds the staged changes, instead
    /// of the work tree.
    staged: Option<bool>,
    /// Only these files or folders, relative to the repository's top.
    paths: Option<Vec<String>>,
    /// The lines of context around each change, from 0 to 100. 3 when null.
    context_lines: Option<u32>,
    /// Whether to list the changed files with their line counts only.
    stat_only: Option<bool>,
    /// The changed files to skip before the first patch, to read on in a
    /// long diff. 0 when null.
    offset: Option<u32>,
}

impl DiffArguments {
    fn staged(&self) -> bool {
        self.staged == Some(true)
    }

    /// Whether `from` is a range, which names both sides.
    fn compares_range(&self) -> bool {
        self.from.as_deref().is_some_and(|from| from.contains(".."))
    }

    /// Whether the diff reads the work tree.
    fn reads_work_tree(&self) -> bool {
        self.to.is_none() && !self.staged() && !self.compares_range()
    }

    /// What the diff compares, in words.
    fn compared(&self) -> String {
        match (&self.from, &self.to, self.staged()) {
            (None, _to, false) => "Unstaged changes, from the index to the work tree".to_owned(),
            (None, _to, true) => "Staged changes, from HEAD to the index".to_owned(),
            (Some(from), None, _staged) if self.compares_range() => format!("Changes in {from}"),
            (Some(from), None, false) => format!("Changes from {from} to the work tree"),
            (Some(from), None, true) => format!("Changes from {from} to the index"),
            (Some(from), Some(to), _staged) => format!("Changes from {from} to {to}"),
        }
    }
}

/// `git_diff`.
#[derive(Debug)]
pub(crate) struct GitDiff;

impl ReadTool for GitDiff {
    type Arguments = DiffArguments;

    const NAME: &'static str = GIT_DIFF;
    const DESCRIPTION: &'static str = texts::DIFF_DESCRIPTION;

    fn repository(arguments: &DiffArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn check(arguments: &DiffArguments, layout: &Layout) -> Result<(), GitFailure> {
        if let Some(from) = &arguments.from {
            check_revision("from", from)?;
        }
        if let Some(to) = &arguments.to {
            check_revision("to", to)?;
            if arguments.from.is_none() {
                return Err(invalid("to needs from."));
            }
            if arguments.compares_range() {
                return Err(invalid(
                    "from is a range, which names both sides, so it cannot go with to.",
                ));
            }
            if arguments.staged() {
                return Err(invalid(
                    "staged compares the index, so it cannot go with to.",
                ));
            }
        }
        if arguments.staged() && arguments.compares_range() {
            return Err(invalid(
                "from is a range of commits, so it cannot go with staged.",
            ));
        }
        if arguments.context_lines.unwrap_or(3) > MAX_CONTEXT_LINES {
            return Err(invalid("context_lines must be from 0 to 100."));
        }
        pathspecs(
            &layout.top,
            "paths",
            arguments.paths.as_deref().unwrap_or_default(),
        )?;
        Ok(())
    }

    fn answer(
        reading: Reading,
        arguments: DiffArguments,
    ) -> BoxFuture<'static, Result<Answer, GitFailure>> {
        Box::pin(async move { diff(&reading, &arguments).await })
    }
}

fn invalid(detail: &str) -> GitFailure {
    GitFailure::InvalidArgument {
        detail: detail.to_owned(),
    }
}

/// The card's data.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiffData<'a> {
    compared: &'a str,
    files: &'a [FileStat],
    /// The patches of the files from `offset` that the result shows.
    patches: &'a [FilePatch],
    offset: usize,
}

/// A changed file in one line, such as `src/lib.rs +8 -1`.
pub(crate) fn stat_line(file: &FileStat) -> String {
    let path = match &file.original {
        Some(original) => format!("{} (from {original})", file.path),
        None => file.path.clone(),
    };
    if file.is_binary() {
        return format!("{path} (binary)");
    }
    format!(
        "{path} +{} -{}",
        file.added.unwrap_or(0),
        file.removed.unwrap_or(0)
    )
}

/// The runs of a diff: its options, and what limits it, the revisions and
/// the paths after `--`.
struct DiffRun {
    options: Vec<String>,
    limits: Vec<String>,
}

impl DiffRun {
    async fn of(reading: &Reading, arguments: &DiffArguments) -> Result<Self, GitFailure> {
        let mut options = vec!["diff".to_owned()];
        if arguments.staged() {
            options.push("--cached".to_owned());
        }
        options.extend(PATCH_OPTIONS.iter().map(|option| (*option).to_owned()));
        if arguments.reads_work_tree() {
            options.push("--ignore-submodules=dirty".to_owned());
        }
        options.push(format!(
            "--unified={}",
            arguments.context_lines.unwrap_or(3)
        ));
        let mut limits = Vec::new();
        if let Some(from) = &arguments.from {
            limits.push(reading.range("from", from).await?);
        }
        if let Some(to) = &arguments.to {
            limits.push(reading.commit("to", to).await?);
        }
        limits.push("--".to_owned());
        limits.extend(pathspecs(
            &reading.layout.top,
            "paths",
            arguments.paths.as_deref().unwrap_or_default(),
        )?);
        Ok(Self { options, limits })
    }

    /// The run's arguments with `format`, such as `--patch`.
    fn with(&self, format: &[&str]) -> Vec<String> {
        let format = format.iter().map(|option| (*option).to_owned());
        self.options
            .iter()
            .cloned()
            .chain(format)
            .chain(self.limits.iter().cloned())
            .collect()
    }
}

async fn diff(reading: &Reading, arguments: &DiffArguments) -> Result<Answer, GitFailure> {
    let run = DiffRun::of(reading, arguments).await?;
    let ran = reading
        .git
        .output(reading.run(run.with(&["--numstat", "-z"])))
        .await?;
    let files = parse_numstat(&ran.output);
    let compared = arguments.compared();
    let offset = usize::try_from(arguments.offset.unwrap_or(0)).unwrap_or(usize::MAX);
    if files.is_empty() {
        let data = fitting_data(0, |_kept| DiffData {
            compared: &compared,
            files: &[],
            patches: &[],
            offset: 0,
        });
        let text = format!("{compared}: no changes.\n");
        return Ok(Answer { text, data });
    }
    if offset >= files.len() {
        return Err(GitFailure::InvalidArgument {
            detail: format!(
                "offset {offset} skips every file: the diff has {}.",
                count(files.len(), "changed file", "changed files")
            ),
        });
    }
    let mut text = summary(&compared, &files, offset);
    let mut patches = Vec::new();
    if arguments.stat_only != Some(true) {
        let patch = reading
            .git
            .output(reading.run(run.with(&["--patch"])))
            .await?;
        patches = patch_page(reading, &mut text, &files, offset, &patch);
    }
    let data = fitting_data(patches.len(), |kept| DiffData {
        compared: &compared,
        files: &files,
        patches: &patches[..kept],
        offset,
    })
    .or_else(|| {
        fitting_data(files.len(), |kept| DiffData {
            compared: &compared,
            files: &files[..kept],
            patches: &[],
            offset,
        })
    });
    Ok(Answer { text, data })
}

/// Adds to `text` the patches of the files from `offset` that fit the
/// result, after a note of how to read the rest, and returns them for the
/// card.
fn patch_page(
    reading: &Reading,
    text: &mut String,
    files: &[FileStat],
    offset: usize,
    patch: &Ran,
) -> Vec<FilePatch> {
    let patch_text = patch.text();
    let parts = split_patch(&patch_text);
    let budget = reading.room.saturating_sub(NOTE_ROOM + text.len());
    let mut body = String::new();
    let mut shown = 0;
    for part in parts.iter().skip(offset) {
        // A file's patch stays whole; the first one is shown even when it
        // does not fit, cut with the rest archived.
        if shown > 0 && body.len() + part.len() > budget {
            break;
        }
        body.push_str(part);
        shown += 1;
    }
    let next = offset + shown;
    if next < files.len() {
        // The note comes before the patches, so a cut patch keeps it.
        let _written = writeln!(
            text,
            "This result shows the patches of files {}-{next}; call git_diff with offset {next} for the next ones, or with paths for one file.",
            offset + 1
        );
    }
    if patch.cut {
        text.push_str(&texts::output_cut(4));
        text.push('\n');
    }
    text.push('\n');
    text.push_str(&body);
    files
        .iter()
        .zip(&parts)
        .skip(offset)
        .take(shown)
        .map(|(file, part)| FilePatch {
            path: file.path.clone(),
            hunks: parse_hunks(part),
        })
        .collect()
}

/// What the diff compares and its files: every file with its line counts on
/// the first page, the count on later ones.
fn summary(compared: &str, files: &[FileStat], offset: usize) -> String {
    let added: u64 = files
        .iter()
        .filter_map(|file| file.added)
        .map(u64::from)
        .sum();
    let removed: u64 = files
        .iter()
        .filter_map(|file| file.removed)
        .map(u64::from)
        .sum();
    let mut text = format!(
        "{compared}: {}, +{added} -{removed}.\n",
        count(files.len(), "file", "files")
    );
    if offset == 0 {
        for file in files {
            let _written = writeln!(text, "  {}", stat_line(file));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mannie_git_testing::{HostGit, TestRepository};
    use serde_json::json;

    use super::{DiffArguments, GitDiff, stat_line, summary};
    use crate::parse::diff::FileStat;
    use crate::tools::testing::{call, call_with};

    #[tokio::test]
    async fn the_diff_compares_what_the_call_names() {
        let repository = TestRepository::new();
        repository.write("a.txt", "one\n");
        repository.write("b.txt", "one\n");
        repository.commit("Start");
        repository.write("a.txt", "two\n");
        repository.git(&["add", "a.txt"]);
        repository.write("b.txt", "two\n");

        let unstaged = call::<GitDiff>(&repository, json!({})).await;
        let staged = call::<GitDiff>(&repository, json!({ "staged": true })).await;
        let both = call::<GitDiff>(&repository, json!({ "from": "HEAD", "stat_only": true })).await;
        let none = call::<GitDiff>(&repository, json!({ "from": "HEAD", "to": "HEAD" })).await;
        let lone_to = call::<GitDiff>(&repository, json!({ "to": "HEAD" })).await;

        assert!(
            unstaged.output.starts_with(
                "Unstaged changes, from the index to the work tree: 1 file, +1 -1.\n  b.txt +1 -1\n"
            ),
            "{}",
            unstaged.output
        );
        assert!(
            unstaged.output.contains("-one\n+two\n"),
            "{}",
            unstaged.output
        );
        assert!(
            staged.output.contains("  a.txt +1 -1\n"),
            "{}",
            staged.output
        );
        assert!(!staged.output.contains("b.txt"), "{}", staged.output);
        assert_eq!(
            both.output,
            "Changes from HEAD to the work tree: 2 files, +2 -2.\n  a.txt +1 -1\n  b.txt +1 -1\n"
        );
        assert_eq!(none.output, "Changes from HEAD to HEAD: no changes.\n");
        assert_eq!(lone_to.failure.as_deref(), Some("invalid_argument"));
        let data = unstaged.data.expect("the card's data");
        assert_eq!(data["files"][0]["path"], "b.txt");
        assert_eq!(data["patches"][0]["hunks"][0]["lines"][0]["text"], "one");
    }

    #[tokio::test]
    async fn a_diff_that_does_not_fit_continues_with_offset() {
        let repository = TestRepository::new();
        for number in 0..30 {
            repository.write(&format!("f{number:02}.txt"), &"line\n".repeat(20));
        }
        repository.commit("Start");
        for number in 0..30 {
            repository.write(&format!("f{number:02}.txt"), &"changed\n".repeat(20));
        }
        let host = Arc::new(HostGit::new());

        let first = call_with::<GitDiff>(&host, &repository, json!({}), 6000).await;
        let offset: u32 = first
            .output
            .split("call git_diff with offset ")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .and_then(|number| number.trim_end_matches(',').parse().ok())
            .expect("the first page names the next offset");
        let next =
            call_with::<GitDiff>(&host, &repository, json!({ "offset": offset }), 6000).await;

        assert!(offset > 0 && offset < 30, "{offset}");
        assert!(
            first.output.contains("  f29.txt +20 -20\n"),
            "the first page lists every file"
        );
        assert!(
            next.output
                .starts_with("Unstaged changes, from the index to the work tree: 30 files")
        );
        assert!(
            !next.output.contains("  f00.txt +20 -20\n"),
            "{}",
            next.output
        );
        assert!(
            next.output
                .contains(&format!("diff --git a/f{offset:02}.txt")),
            "{}",
            next.output
        );
        let past = call_with::<GitDiff>(&host, &repository, json!({ "offset": 30 }), 6000).await;
        assert_eq!(past.failure.as_deref(), Some("invalid_argument"));
    }

    fn arguments(json: &str) -> DiffArguments {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn each_combination_says_what_it_compares() {
        let cases = [
            (
                "{}",
                "Unstaged changes, from the index to the work tree",
                true,
            ),
            (
                r#"{"staged": true}"#,
                "Staged changes, from HEAD to the index",
                false,
            ),
            (
                r#"{"from": "main"}"#,
                "Changes from main to the work tree",
                true,
            ),
            (
                r#"{"from": "main", "staged": true}"#,
                "Changes from main to the index",
                false,
            ),
            (
                r#"{"from": "main...feature"}"#,
                "Changes in main...feature",
                false,
            ),
            (
                r#"{"from": "v1", "to": "v2"}"#,
                "Changes from v1 to v2",
                false,
            ),
        ];

        for (json, compared, reads_work_tree) in cases {
            let call = arguments(json);
            assert_eq!(call.compared(), compared, "{json}");
            assert_eq!(call.reads_work_tree(), reads_work_tree, "{json}");
        }
    }

    #[test]
    fn the_summary_lists_the_files_on_the_first_page_only() {
        let files = [
            FileStat {
                path: "docs/new.md".to_owned(),
                original: Some("docs/old.md".to_owned()),
                added: Some(1),
                removed: Some(0),
            },
            FileStat {
                path: "logo.png".to_owned(),
                original: None,
                added: None,
                removed: None,
            },
        ];

        assert_eq!(
            summary("Changes from main to the work tree", &files, 0),
            "Changes from main to the work tree: 2 files, +1 -0.\n  docs/new.md (from docs/old.md) +1 -0\n  logo.png (binary)\n"
        );
        assert_eq!(
            summary("Changes in a..b", &files, 1),
            "Changes in a..b: 2 files, +1 -0.\n"
        );
        assert_eq!(stat_line(&files[1]), "logo.png (binary)");
    }
}
