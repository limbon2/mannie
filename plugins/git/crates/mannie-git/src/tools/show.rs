//! `git_show` (plugins/git/docs/tools.md, tools that read): a commit with
//! its whole message, the files that it changed and its patch against its
//! first parent; or a file's lines or a folder's entries at a revision.
//!
//! A merge's patch is against its first parent, which shows what the merge
//! brought into the branch, and a stash entry's is the changes that it
//! keeps. The patch runs without external diff programs and text
//! conversions, which the repository's configuration could name (T-44).

use std::fmt::Write as _;
use std::path::Path;

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::arguments::{check_revision, is_object_id, repository_path};
use crate::failure::GitFailure;
use crate::git::GitRun;
use crate::layout::Layout;
use crate::parse::diff::{FileStat, Hunk, parse_hunks, parse_numstat, split_patch};
use crate::parse::log::{CommitDetails, DETAILS_FORMAT, parse_details};
use crate::parse::tree::{TREE_FORMAT, TreeEntry, TreeEntryKind, parse_tree};
use crate::texts;
use crate::tools::diff::{PATCH_OPTIONS, stat_line};
use crate::tools::{Answer, NOTE_ROOM, ReadTool, Reading, count, cut_line, fitting_data};

/// The tool's name.
pub(crate) const GIT_SHOW: &str = "git_show";

/// The most bytes of a file that a call reads.
const MAX_FILE_BYTES: u32 = 16 * 1024 * 1024;

/// The bytes at a file's start in which a NUL marks it binary, as git
/// decides.
const BINARY_PROBE_BYTES: usize = 8000;

/// The arguments of `git_show`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShowArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// The commit, tag or stash entry, such as HEAD~1, v1.0, 1a2b3c4 or
    /// stash@{0}. HEAD when null.
    revision: Option<String>,
    /// A file or folder, relative to the repository's top, to show as it is
    /// at the revision instead of the commit.
    path: Option<String>,
    /// The first line of the file to show, from 1. The first when null.
    start_line: Option<u32>,
    /// The last line of the file to show. The last when null.
    end_line: Option<u32>,
    /// Whether to show the commit's patch. True when null; false shows its
    /// message and changed files only.
    patch: Option<bool>,
}

/// `git_show`.
#[derive(Debug)]
pub(crate) struct GitShow;

impl ReadTool for GitShow {
    type Arguments = ShowArguments;

    const NAME: &'static str = GIT_SHOW;
    const DESCRIPTION: &'static str = texts::SHOW_DESCRIPTION;

    fn repository(arguments: &ShowArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn check(arguments: &ShowArguments, layout: &Layout) -> Result<(), GitFailure> {
        if let Some(revision) = &arguments.revision {
            check_revision("revision", revision)?;
        }
        let has_lines = arguments.start_line.is_some() || arguments.end_line.is_some();
        match &arguments.path {
            Some(path) => {
                shown_path(&layout.top, path)?;
            }
            None if has_lines => {
                return Err(invalid("start_line and end_line need path."));
            }
            None => {}
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
        arguments: ShowArguments,
    ) -> BoxFuture<'static, Result<Answer, GitFailure>> {
        Box::pin(async move { show(&reading, &arguments).await })
    }
}

fn invalid(detail: &str) -> GitFailure {
    GitFailure::InvalidArgument {
        detail: detail.to_owned(),
    }
}

/// A file's patch, for the card.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FilePatch {
    pub(crate) path: String,
    pub(crate) hunks: Vec<Hunk>,
}

/// The card's data, by what the call showed.
#[derive(Debug, Serialize)]
#[serde(
    tag = "mode",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum ShowData<'a> {
    Commit {
        commit: &'a CommitDetails,
        files: &'a [FileStat],
        patches: &'a [FilePatch],
    },
    File {
        path: &'a str,
        revision: &'a str,
        first_line: usize,
        lines: &'a [String],
        total_lines: usize,
        binary: bool,
    },
    Folder {
        path: &'a str,
        revision: &'a str,
        entries: &'a [TreeEntry],
    },
}

async fn show(reading: &Reading, arguments: &ShowArguments) -> Result<Answer, GitFailure> {
    let revision = arguments.revision.as_deref().unwrap_or("HEAD");
    let commit = reading.commit("revision", revision).await?;
    match &arguments.path {
        Some(path) => {
            let path = shown_path(&reading.layout.top, path)?;
            show_path(reading, arguments, revision, &commit, &path).await
        }
        None => show_commit(reading, &commit, arguments.patch != Some(false)).await,
    }
}

/// The path that names the repository's top folder.
const TOP: &str = ".";

/// A path to show, from the repository's top, or [`TOP`] for the top's own
/// folder.
fn shown_path(top: &Path, path: &str) -> Result<String, GitFailure> {
    if matches!(path.trim(), "." | "./" | "/") {
        return Ok(TOP.to_owned());
    }
    repository_path(top, "path", path)
}

/// The run of a part of a commit: the options that keep the repository's
/// programs out, the first parent of a merge, then the commit.
fn commit_run(reading: &Reading, commit: &str, options: &[&str]) -> GitRun {
    let mut command = vec!["show".to_owned()];
    command.extend(options.iter().map(|option| (*option).to_owned()));
    command.extend(PATCH_OPTIONS.iter().map(|option| (*option).to_owned()));
    command.push("--diff-merges=first-parent".to_owned());
    command.push(commit.to_owned());
    reading.run(command)
}

/// A commit as a result shows it: its details, its changed files, the
/// patches of its files for the card, and the text for the model.
#[derive(Debug)]
pub(crate) struct ShownCommit {
    pub(crate) details: CommitDetails,
    pub(crate) files: Vec<FileStat>,
    pub(crate) patches: Vec<FilePatch>,
    pub(crate) text: String,
}

/// Reads a commit named by its full hash, with its patch when `with_patch`
/// says so.
pub(crate) async fn shown_commit(
    reading: &Reading,
    commit: &str,
    with_patch: bool,
) -> Result<ShownCommit, GitFailure> {
    let details = reading
        .git
        .output(reading.run(["show", "--no-patch", DETAILS_FORMAT, commit]))
        .await?;
    let details = parse_details(&details.text()).ok_or_else(|| {
        GitFailure::failed("git described the commit in a form that this version cannot read")
    })?;
    let numstat = reading
        .git
        .output(commit_run(
            reading,
            commit,
            &["--format=", "--numstat", "-z"],
        ))
        .await?;
    let files = parse_numstat(&numstat.output);
    let mut text = details_text(&details);
    text.push('\n');
    text.push_str(&files_text(&files));
    let mut patches = Vec::new();
    if with_patch && !files.is_empty() {
        let patch = reading
            .git
            .output(commit_run(reading, commit, &["--format=", "--patch"]))
            .await?;
        let patch_text = patch.text();
        let parts = split_patch(&patch_text);
        text.push('\n');
        for part in &parts {
            text.push_str(part);
        }
        if patch.cut {
            text.push_str(&texts::output_cut(4));
            text.push('\n');
        }
        patches = files
            .iter()
            .zip(&parts)
            .map(|(file, part)| FilePatch {
                path: file.path.clone(),
                hunks: parse_hunks(part),
            })
            .collect();
    }
    Ok(ShownCommit {
        details,
        files,
        patches,
        text,
    })
}

async fn show_commit(
    reading: &Reading,
    commit: &str,
    with_patch: bool,
) -> Result<Answer, GitFailure> {
    let ShownCommit {
        details,
        files,
        patches,
        text,
    } = shown_commit(reading, commit, with_patch).await?;
    let data = fitting_data(patches.len(), |kept| ShowData::Commit {
        commit: &details,
        files: &files,
        patches: &patches[..kept],
    })
    .or_else(|| {
        fitting_data(files.len(), |kept| ShowData::Commit {
            commit: &details,
            files: &files[..kept],
            patches: &[],
        })
    });
    Ok(Answer { text, data })
}

/// A commit's hashes, parents, people, dates, refs and message.
fn details_text(details: &CommitDetails) -> String {
    let mut text = format!("Commit {}", details.oid);
    if !details.refs.is_empty() {
        let _written = write!(text, " ({})", details.refs.join(", "));
    }
    text.push('\n');
    match details.short_parents.len() {
        0 => text.push_str("Parents: none, it is a root commit.\n"),
        1 => {
            let _written = writeln!(text, "Parent: {}", details.short_parents[0]);
        }
        _merge => {
            let _written = writeln!(
                text,
                "Parents: {}. It is a merge, so its patch is against the first parent.",
                details.short_parents.join(", ")
            );
        }
    }
    let _written = writeln!(
        text,
        "Author: {} <{}>, {}",
        details.author, details.email, details.authored
    );
    let same_committer = details.committer == details.author
        && details.committer_email == details.email
        && details.committed == details.authored;
    if !same_committer {
        let _written = writeln!(
            text,
            "Committer: {} <{}>, {}",
            details.committer, details.committer_email, details.committed
        );
    }
    text.push('\n');
    for line in details.message.lines() {
        let _written = writeln!(text, "    {line}");
    }
    text
}

/// The changed files with their line counts.
fn files_text(files: &[FileStat]) -> String {
    if files.is_empty() {
        return "No files changed.\n".to_owned();
    }
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
        "{} changed, +{added} -{removed}:\n",
        count(files.len(), "file", "files")
    );
    for file in files {
        let _written = writeln!(text, "  {}", stat_line(file));
    }
    text
}

async fn show_path(
    reading: &Reading,
    arguments: &ShowArguments,
    revision: &str,
    commit: &str,
    path: &str,
) -> Result<Answer, GitFailure> {
    // The top's own folder is the commit's tree.
    let object_name = if path == TOP {
        format!("{commit}^{{tree}}")
    } else {
        format!("{commit}:{path}")
    };
    let object = reading
        .git
        .run(reading.run([
            "rev-parse".to_owned(),
            "--verify".to_owned(),
            "--quiet".to_owned(),
            object_name,
        ]))
        .await?;
    let object_id = object.text().trim().to_owned();
    if !object.succeeded() || !is_object_id(&object_id) {
        return Err(GitFailure::InvalidArgument {
            detail: format!("{path} does not exist at {revision}."),
        });
    }
    // A submodule's commit is not an object of this repository.
    let kind = reading
        .git
        .run(reading.run(["cat-file", "-t", object_id.as_str()]))
        .await?;
    match kind.text().trim() {
        "blob" => show_file(reading, arguments, revision, path, &object_id).await,
        "tree" => show_folder(reading, revision, path, &object_id).await,
        _submodule => Ok(Answer {
            text: format!("{path} is a submodule at {revision}, at its commit {object_id}.\n"),
            data: None,
        }),
    }
}

async fn show_file(
    reading: &Reading,
    arguments: &ShowArguments,
    revision: &str,
    path: &str,
    object_id: &str,
) -> Result<Answer, GitFailure> {
    let blob = reading
        .git
        .output(
            reading
                .run(["cat-file", "blob", object_id])
                .with_output_limit(MAX_FILE_BYTES),
        )
        .await?;
    let probe = &blob.output[..blob.output.len().min(BINARY_PROBE_BYTES)];
    if probe.contains(&0) {
        let text = format!(
            "{path} at {revision} is a binary file of {} bytes, which this tool does not show.\n",
            blob.output.len()
        );
        let data = fitting_data(0, |_kept| ShowData::File {
            path,
            revision,
            first_line: 1,
            lines: &[],
            total_lines: 0,
            binary: true,
        });
        return Ok(Answer { text, data });
    }
    let content = blob.text();
    let all: Vec<&str> = content.lines().collect();
    if all.is_empty() {
        let data = fitting_data(0, |_kept| ShowData::File {
            path,
            revision,
            first_line: 1,
            lines: &[],
            total_lines: 0,
            binary: false,
        });
        return Ok(Answer {
            text: format!("{path} at {revision} is empty.\n"),
            data,
        });
    }
    let page = LinePage::of(all.len(), arguments.start_line, arguments.end_line);
    let budget = reading.room.saturating_sub(NOTE_ROOM);
    let mut body = String::new();
    let mut shown: Vec<String> = Vec::new();
    for (index, line) in all.iter().enumerate().skip(page.first - 1).take(page.shown) {
        let line = cut_line(line);
        let numbered = format!("{:>6}\t{line}\n", index + 1);
        // Lines stop where the room ends, and the model reads on from there.
        if !shown.is_empty() && body.len() + numbered.len() > budget {
            break;
        }
        body.push_str(&numbered);
        shown.push(line);
    }
    // A file with lines shows at least one of them.
    let last = page.first + shown.len() - 1;
    let mut text = format!(
        "{path} at {revision}, lines {}-{last} of {}:\n{body}",
        page.first,
        all.len()
    );
    let wanted_last = page.first + page.shown - 1;
    if last < wanted_last {
        let next = last + 1;
        let _written = writeln!(
            text,
            "Lines {next}-{wanted_last} follow: call git_show with start_line {next}."
        );
    }
    if blob.cut {
        text.push_str(&texts::output_cut(16));
        text.push('\n');
    }
    let data = fitting_data(shown.len(), |kept| ShowData::File {
        path,
        revision,
        first_line: page.first,
        lines: &shown[..kept],
        total_lines: all.len(),
        binary: false,
    });
    Ok(Answer { text, data })
}

/// The lines of a file that a call asks for: from the first, how many.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinePage {
    first: usize,
    shown: usize,
}

impl LinePage {
    fn of(total: usize, start: Option<u32>, end: Option<u32>) -> Self {
        let first = start.map_or(1, |start| usize::try_from(start).unwrap_or(usize::MAX));
        let last = end.map_or(total, |end| {
            usize::try_from(end).unwrap_or(usize::MAX).min(total)
        });
        let first = first.min(total.max(1));
        let shown = (last + 1).saturating_sub(first);
        Self { first, shown }
    }
}

async fn show_folder(
    reading: &Reading,
    revision: &str,
    path: &str,
    object_id: &str,
) -> Result<Answer, GitFailure> {
    let listed = reading
        .git
        .output(reading.run(["ls-tree", "-z", TREE_FORMAT, object_id]))
        .await?;
    let entries = parse_tree(&listed.output);
    let mut text = format!(
        "Folder {path} at {revision}, {}:\n",
        count(entries.len(), "entry", "entries")
    );
    for entry in &entries {
        let line = match (entry.kind, entry.size) {
            (TreeEntryKind::Folder, _size) => format!("{}/", entry.name),
            (TreeEntryKind::Submodule, _size) => format!("{} (submodule)", entry.name),
            (TreeEntryKind::File, Some(size)) => format!("{} ({size} bytes)", entry.name),
            (TreeEntryKind::File, None) => entry.name.clone(),
        };
        let _written = writeln!(text, "  {line}");
    }
    let data = fitting_data(entries.len(), |kept| ShowData::Folder {
        path,
        revision,
        entries: &entries[..kept],
    });
    Ok(Answer { text, data })
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::sync::Arc;

    use mannie_git_testing::{HostGit, TestRepository};
    use serde_json::json;

    use super::{GitShow, LinePage, details_text, files_text};
    use crate::parse::diff::FileStat;
    use crate::parse::log::CommitDetails;
    use crate::tools::testing::{call, call_with};

    fn repository() -> TestRepository {
        let repository = TestRepository::new();
        repository.write("src/lib.rs", "fn a() {}\nfn b() {}\n");
        repository.commit("Start\n\nThe body.");
        repository.write("src/lib.rs", "fn a() {}\nfn c() {}\n");
        repository.write("logo.bin", "\u{0}\u{1}binary");
        repository.commit("Rename b to c");
        repository
    }

    #[tokio::test]
    async fn a_commit_shows_its_message_its_files_and_its_patch() {
        let repository = repository();

        let head = call::<GitShow>(&repository, json!({})).await;
        let start =
            call::<GitShow>(&repository, json!({ "revision": "HEAD~1", "patch": false })).await;

        assert_eq!(head.failure, None, "{}", head.output);
        assert!(
            head.output.contains("\n    Rename b to c\n"),
            "{}",
            head.output
        );
        assert!(
            head.output
                .contains("2 files changed, +1 -1:\n  logo.bin (binary)\n  src/lib.rs +1 -1\n")
        );
        assert!(head.output.contains("+fn c() {}\n"), "{}", head.output);
        assert!(
            start
                .output
                .contains("Parents: none, it is a root commit.\n"),
            "{}",
            start.output
        );
        assert!(
            start.output.contains("    Start\n    \n    The body.\n"),
            "{}",
            start.output
        );
        assert!(!start.output.contains("diff --git"));
        let data = head.data.expect("the card's data");
        assert_eq!(data["mode"], "commit");
        assert_eq!(data["patches"][1]["path"], "src/lib.rs");
        assert_eq!(
            data["patches"][1]["hunks"][0]["lines"][1]["kind"],
            "removed"
        );
    }

    #[tokio::test]
    async fn a_path_shows_a_file_s_lines_a_folder_s_entries_or_a_binary_file_s_size() {
        let repository = repository();

        let file = call::<GitShow>(
            &repository,
            json!({ "revision": "HEAD~1", "path": "src/lib.rs", "start_line": 2 }),
        )
        .await;
        let top = call::<GitShow>(&repository, json!({ "path": "." })).await;
        let binary = call::<GitShow>(&repository, json!({ "path": "logo.bin" })).await;
        let missing = call::<GitShow>(&repository, json!({ "path": "nothing.txt" })).await;
        let unknown = call::<GitShow>(&repository, json!({ "revision": "nope" })).await;

        assert_eq!(
            file.output,
            "src/lib.rs at HEAD~1, lines 2-2 of 2:\n     2\tfn b() {}\n"
        );
        assert_eq!(
            top.output,
            "Folder . at HEAD, 2 entries:\n  logo.bin (8 bytes)\n  src/\n"
        );
        assert!(
            binary.output.contains("is a binary file of 8 bytes"),
            "{}",
            binary.output
        );
        assert_eq!(missing.failure.as_deref(), Some("invalid_argument"));
        assert_eq!(unknown.failure.as_deref(), Some("unknown_revision"));
    }

    #[tokio::test]
    async fn a_long_file_stops_where_the_room_ends_and_says_where_to_read_on() {
        let repository = TestRepository::new();
        let text = (1..=300).fold(String::new(), |mut text, number| {
            let _written = writeln!(text, "line {number}");
            text
        });
        repository.write("long.txt", &text);
        repository.commit("Long");

        let page = call_with::<GitShow>(
            &Arc::new(HostGit::new()),
            &repository,
            json!({ "path": "long.txt" }),
            2000,
        )
        .await;

        let last_line = page.output.lines().rev().nth(1).unwrap().trim().to_owned();
        let shown: usize = last_line.split('\t').next().unwrap().parse().unwrap();
        assert!(page.output.len() <= 2000, "{}", page.output.len());
        assert!(
            page.output.ends_with(&format!(
                "Lines {}-300 follow: call git_show with start_line {}.\n",
                shown + 1,
                shown + 1
            )),
            "{}",
            page.output
        );
    }

    #[test]
    fn a_line_page_stays_inside_the_file() {
        assert_eq!(
            LinePage::of(10, None, None),
            LinePage {
                first: 1,
                shown: 10
            }
        );
        assert_eq!(
            LinePage::of(10, Some(4), Some(6)),
            LinePage { first: 4, shown: 3 }
        );
        assert_eq!(
            LinePage::of(10, Some(8), Some(50)),
            LinePage { first: 8, shown: 3 }
        );
        assert_eq!(
            LinePage::of(10, Some(20), None),
            LinePage {
                first: 10,
                shown: 1
            }
        );
        assert_eq!(LinePage::of(0, None, None), LinePage { first: 1, shown: 0 });
    }

    #[test]
    fn a_commit_names_its_merge_parents_and_indents_its_message() {
        let details = CommitDetails {
            oid: "1".repeat(40),
            short: "1111111".to_owned(),
            parents: vec!["a".repeat(40), "b".repeat(40)],
            short_parents: vec!["aaaaaaa".to_owned(), "bbbbbbb".to_owned()],
            author: "Jane Doe".to_owned(),
            email: "jane@example.com".to_owned(),
            authored: "2026-10-19T14:02:11+02:00".to_owned(),
            committer: "Jane Doe".to_owned(),
            committer_email: "jane@example.com".to_owned(),
            committed: "2026-10-19T14:02:11+02:00".to_owned(),
            refs: vec!["HEAD -> main".to_owned()],
            message: "Merge branch 'feature'\n\nThe body.".to_owned(),
        };

        let text = details_text(&details);

        assert!(text.starts_with(&format!("Commit {} (HEAD -> main)\n", "1".repeat(40))));
        assert!(text.contains("Parents: aaaaaaa, bbbbbbb. It is a merge"));
        assert!(!text.contains("Committer:"));
        assert!(
            text.ends_with("    Merge branch 'feature'\n    \n    The body.\n"),
            "{text}"
        );
    }

    #[test]
    fn changed_files_are_counted_with_their_lines() {
        let files = [
            FileStat {
                path: "src/lib.rs".to_owned(),
                original: None,
                added: Some(8),
                removed: Some(1),
            },
            FileStat {
                path: "logo.png".to_owned(),
                original: None,
                added: None,
                removed: None,
            },
        ];

        assert_eq!(
            files_text(&files),
            "2 files changed, +8 -1:\n  src/lib.rs +8 -1\n  logo.png (binary)\n"
        );
        assert_eq!(files_text(&[]), "No files changed.\n");
    }
}
