//! `git_conflicts` (plugins/git/docs/conflicts-and-history.md, reading
//! conflicts): the merge, rebase, cherry-pick or revert in progress, its
//! conflicted files with how each conflicts, and a file's remaining conflict
//! hunks, read from the markers in the work tree, so the hunks that the
//! agent resolved already no longer show.
//!
//! The sides are "current", what HEAD holds, and "incoming", what the
//! operation applies, in every operation; during a rebase git calls them
//! the other way around.

use std::fmt::Write as _;
use std::path::Path;

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::arguments::repository_path;
use crate::describe::{CommitLabel, OperationView, label_commits, operation_view};
use crate::failure::GitFailure;
use crate::layout::Layout;
use crate::parse::conflicts::{
    ConflictHunk, Segment, Stages, hunks, parse_markers, parse_unmerged,
};
use crate::parse::status::Conflict;
use crate::texts;
use crate::tools::status::conflict_words;
use crate::tools::{Answer, ReadTool, Reading, cut_line, fitting_data};

/// The tool's name.
pub(crate) const GIT_CONFLICTS: &str = "git_conflicts";

/// The most bytes of a conflicted file that the tool reads for its markers.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// The arguments of `git_conflicts`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConflictsArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// A conflicted file, relative to the repository's top, whose remaining
    /// conflicts to show. Every conflicted file is listed when null.
    path: Option<String>,
}

/// `git_conflicts`.
#[derive(Debug)]
pub(crate) struct GitConflicts;

impl ReadTool for GitConflicts {
    type Arguments = ConflictsArguments;

    const NAME: &'static str = GIT_CONFLICTS;
    const DESCRIPTION: &'static str = texts::CONFLICTS_DESCRIPTION;

    fn repository(arguments: &ConflictsArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn check(arguments: &ConflictsArguments, layout: &Layout) -> Result<(), GitFailure> {
        if let Some(path) = &arguments.path {
            repository_path(&layout.top, "path", path)?;
        }
        Ok(())
    }

    fn answer(
        reading: Reading,
        arguments: ConflictsArguments,
    ) -> BoxFuture<'static, Result<Answer, GitFailure>> {
        Box::pin(async move { conflicts(&reading, &arguments).await })
    }
}

/// What the markers of a conflicted file show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub(crate) enum Markers {
    /// The file has this many conflict hunks left.
    Left { hunks: u32 },
    /// The file's markers do not pair up, such as after a partial edit.
    Unbalanced { line: u32 },
    /// The file is not in the work tree, or is too large to read.
    Unread,
}

/// A conflicted file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConflictedFile {
    pub(crate) path: String,
    pub(crate) conflict: Conflict,
    /// What the markers show, for a file that both sides have.
    pub(crate) markers: Option<Markers>,
}

/// The card's data.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConflictsData<'a> {
    operation: Option<&'a OperationView>,
    head: Option<&'a CommitLabel>,
    files: &'a [ConflictedFile],
    /// The file that the call named, with its hunks.
    file: Option<FileHunks<'a>>,
}

/// A file's remaining hunks.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileHunks<'a> {
    path: &'a str,
    hunks: &'a [&'a ConflictHunk],
}

/// The conflicted files of the repository, from the index's stages and the
/// markers in the work tree.
pub(crate) async fn conflicted_files(reading: &Reading) -> Result<Vec<ConflictedFile>, GitFailure> {
    let ran = reading
        .git
        .output(reading.run(["ls-files", "-u", "-z"]))
        .await?;
    let stages = parse_unmerged(&ran.output);
    Ok(stages
        .iter()
        .map(|stages| ConflictedFile {
            path: stages.path.clone(),
            conflict: stages.conflict(),
            markers: has_both_sides(stages).then(|| markers_of(&reading.layout.top, &stages.path)),
        })
        .collect())
}

/// Whether the current and the incoming side both have the file, so its
/// conflicts are hunks of text.
fn has_both_sides(stages: &Stages) -> bool {
    stages.current.is_some() && stages.incoming.is_some()
}

/// A work tree file's text, when it exists and is not too large.
fn read_text(top: &Path, path: &str) -> Option<String> {
    let file = top.join(path);
    let size = std::fs::metadata(&file).ok()?.len();
    if size > MAX_FILE_BYTES {
        return None;
    }
    let bytes = std::fs::read(&file).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn markers_of(top: &Path, path: &str) -> Markers {
    let Some(text) = read_text(top, path) else {
        return Markers::Unread;
    };
    match parse_markers(&text) {
        Ok(segments) => Markers::Left {
            hunks: u32::try_from(hunks(&segments).len()).unwrap_or(u32::MAX),
        },
        Err(crate::parse::conflicts::MarkerError::Unbalanced { line }) => {
            Markers::Unbalanced { line }
        }
    }
}

async fn conflicts(
    reading: &Reading,
    arguments: &ConflictsArguments,
) -> Result<Answer, GitFailure> {
    let operation = operation_view(reading).await?;
    let files = conflicted_files(reading).await?;
    let head_oid = reading.commit("HEAD", "HEAD").await.ok();
    let head = match head_oid {
        Some(oid) => label_commits(reading, &[oid]).await?.into_iter().next(),
        None => None,
    };
    let mut text = String::new();
    match &operation {
        Some(operation) => {
            let _written = writeln!(text, "{}", operation.text());
        }
        None if files.is_empty() => {
            text.push_str("No merge, rebase, cherry-pick or revert is in progress, and no file is conflicted.\n");
        }
        None => text.push_str(
            "No merge, rebase, cherry-pick or revert is in progress, but files are conflicted, as applying a stash can leave them.\n",
        ),
    }
    if !files.is_empty() {
        sides_text(&mut text, head.as_ref(), operation.as_ref());
    }
    let named = match &arguments.path {
        Some(path) => Some(repository_path(&reading.layout.top, "path", path)?),
        None => None,
    };
    let segments = if let Some(path) = &named {
        Some(file_segments(reading, &files, path)?)
    } else {
        files_text(&mut text, &files);
        None
    };
    let file_hunks: Vec<&ConflictHunk> = segments.as_deref().map(hunks).unwrap_or_default();
    if let (Some(path), Some(segments)) = (&named, &segments) {
        file_text(
            &mut text,
            &files,
            path,
            segments,
            head.as_ref(),
            operation.as_ref(),
        );
    }
    let data = fitting_data(file_hunks.len().max(files.len()), |kept| ConflictsData {
        operation: operation.as_ref(),
        head: head.as_ref(),
        files: &files[..kept.min(files.len())],
        file: named.as_deref().map(|path| FileHunks {
            path,
            hunks: &file_hunks[..kept.min(file_hunks.len())],
        }),
    });
    Ok(Answer { text, data })
}

/// Which commit each side holds.
fn sides_text(text: &mut String, head: Option<&CommitLabel>, operation: Option<&OperationView>) {
    if let Some(head) = head {
        let _written = writeln!(text, "Current side: HEAD, {}.", head.text());
    }
    let incoming = operation.and_then(|operation| operation.incoming.as_ref());
    if let Some(incoming) = incoming {
        let _written = writeln!(text, "Incoming side: {}.", incoming.text());
    }
}

/// Every conflicted file with how it conflicts.
fn files_text(text: &mut String, files: &[ConflictedFile]) {
    if files.is_empty() {
        return;
    }
    let _written = writeln!(text, "Conflicted files ({}):", files.len());
    for file in files {
        let markers = match &file.markers {
            Some(Markers::Left { hunks: 0 }) => ", no conflict markers left".to_owned(),
            Some(Markers::Left { hunks: 1 }) => ", 1 conflict left".to_owned(),
            Some(Markers::Left { hunks }) => format!(", {hunks} conflicts left"),
            Some(Markers::Unbalanced { line }) => {
                format!(", its conflict markers do not pair up at line {line}")
            }
            Some(Markers::Unread) | None => String::new(),
        };
        let _written = writeln!(
            text,
            "  {}: {}{markers}",
            file.path,
            conflict_words(file.conflict)
        );
    }
    text.push_str("Call git_conflicts with a path to read that file's conflicts.\n");
}

/// The segments of the conflicted file that a call names.
fn file_segments(
    reading: &Reading,
    files: &[ConflictedFile],
    path: &str,
) -> Result<Vec<Segment>, GitFailure> {
    let Some(file) = files.iter().find(|file| file.path == path) else {
        return Err(GitFailure::InvalidArgument {
            detail: format!("{path} is not conflicted."),
        });
    };
    if file.markers.is_none() {
        return Ok(Vec::new());
    }
    let Some(text) = read_text(&reading.layout.top, path) else {
        return Err(GitFailure::InvalidArgument {
            detail: format!("{path} is not in the work tree, or is larger than 16 MiB."),
        });
    };
    parse_markers(&text).map_err(|error| GitFailure::InvalidArgument {
        detail: format!("{path}: {error}. Read the file with read_file and fix its markers."),
    })
}

/// A file's conflicts, each with its lines on each side.
fn file_text(
    text: &mut String,
    files: &[ConflictedFile],
    path: &str,
    segments: &[Segment],
    head: Option<&CommitLabel>,
    operation: Option<&OperationView>,
) {
    let Some(file) = files.iter().find(|file| file.path == path) else {
        return;
    };
    let file_hunks = hunks(segments);
    let words = conflict_words(file.conflict);
    if file.markers.is_none() {
        let _written = writeln!(
            text,
            "{path}: {words}. Its content does not conflict line by line: keep the file or delete it."
        );
        return;
    }
    if file_hunks.is_empty() {
        let _written = writeln!(
            text,
            "{path}: {words}, no conflict markers left. Once its content is right, mark it resolved."
        );
        return;
    }
    let _written = writeln!(
        text,
        "{path}: {words}, {} left.",
        count_conflicts(file_hunks.len())
    );
    let current = head.map_or_else(
        || "HEAD".to_owned(),
        |head| format!("HEAD, {}", head.text()),
    );
    let incoming = operation
        .and_then(|operation| operation.incoming.as_ref())
        .map_or_else(|| "what is applied".to_owned(), CommitLabel::text);
    for hunk in file_hunks {
        let _written = writeln!(
            text,
            "\nConflict {}, lines {}-{}:",
            hunk.number, hunk.first_line, hunk.last_line
        );
        side_text(text, &format!("Current ({current})"), &hunk.current);
        if let Some(base) = &hunk.base {
            side_text(text, "Base", base);
        }
        side_text(text, &format!("Incoming ({incoming})"), &hunk.incoming);
    }
}

fn count_conflicts(number: usize) -> String {
    if number == 1 {
        "1 conflict".to_owned()
    } else {
        format!("{number} conflicts")
    }
}

/// A side's lines, indented under its name.
fn side_text(text: &mut String, name: &str, lines: &[String]) {
    if lines.is_empty() {
        let _written = writeln!(text, "{name}: no lines.");
        return;
    }
    let _written = writeln!(text, "{name}:");
    for line in lines {
        let line = line.trim_end_matches(['\n', '\r']);
        let _written = writeln!(text, "    {}", cut_line(line));
    }
}

#[cfg(test)]
mod tests {
    use mannie_git_testing::TestRepository;
    use serde_json::json;

    use super::{ConflictedFile, GitConflicts, Markers, file_text, files_text};
    use crate::parse::conflicts::parse_markers;
    use crate::parse::status::Conflict;
    use crate::tools::testing::call;

    /// A repository where `feature` and `main` changed the same line of
    /// `a.txt` after `Start`.
    fn diverged() -> TestRepository {
        let repository = TestRepository::new();
        repository.write("a.txt", "one\n");
        repository.write("b.txt", "same\n");
        repository.commit("Start");
        repository.git(&["switch", "--quiet", "-c", "feature"]);
        repository.write("a.txt", "feature\n");
        repository.commit("Feature change");
        repository.git(&["switch", "--quiet", "main"]);
        repository.write("a.txt", "main\n");
        repository.commit("Main change");
        repository
    }

    #[tokio::test]
    async fn a_merge_s_conflicts_name_the_current_and_the_incoming_side() {
        let repository = diverged();
        assert!(!repository.git_may_fail(&[
            "-c",
            "merge.conflictStyle=zdiff3",
            "merge",
            "feature"
        ]));

        let list = call::<GitConflicts>(&repository, json!({})).await;
        let file = call::<GitConflicts>(&repository, json!({ "path": "a.txt" })).await;
        let clean = call::<GitConflicts>(&repository, json!({ "path": "b.txt" })).await;

        assert!(list.output.starts_with("A merge of "), "{}", list.output);
        assert!(
            list.output.contains(" Main change.\nIncoming side: "),
            "{}",
            list.output
        );
        assert!(
            list.output
                .contains("  a.txt: both changed, 1 conflict left\n"),
            "{}",
            list.output
        );
        assert!(
            file.output
                .contains(" Main change):\n    main\nBase:\n    one\nIncoming ("),
            "{}",
            file.output
        );
        assert!(
            file.output.contains(" Feature change):\n    feature\n"),
            "{}",
            file.output
        );
        assert_eq!(clean.failure.as_deref(), Some("invalid_argument"));
        let data = file.data.expect("the card's data");
        assert_eq!(data["file"]["hunks"][0]["incoming"][0], "feature\n");
    }

    #[tokio::test]
    async fn during_a_rebase_current_is_the_rebased_series_and_incoming_the_replayed_commit() {
        let repository = diverged();
        repository.git(&["switch", "--quiet", "feature"]);
        assert!(!repository.git_may_fail(&["-c", "merge.conflictStyle=zdiff3", "rebase", "main"]));

        let file = call::<GitConflicts>(&repository, json!({ "path": "a.txt" })).await;

        assert!(
            file.output.starts_with("A rebase of feature onto "),
            "{}",
            file.output
        );
        assert!(
            file.output.contains("stopped at step 1 of 1, applying "),
            "{}",
            file.output
        );
        assert!(
            file.output.contains(" Main change):\n    main\n"),
            "{}",
            file.output
        );
        assert!(
            file.output.contains(" Feature change):\n    feature\n"),
            "{}",
            file.output
        );
    }

    #[tokio::test]
    async fn without_an_operation_or_conflicts_the_tool_says_so() {
        let repository = diverged();

        let result = call::<GitConflicts>(&repository, json!({})).await;

        assert_eq!(
            result.output,
            "No merge, rebase, cherry-pick or revert is in progress, and no file is conflicted.\n"
        );
    }

    fn file(path: &str, conflict: Conflict, markers: Option<Markers>) -> ConflictedFile {
        ConflictedFile {
            path: path.to_owned(),
            conflict,
            markers,
        }
    }

    #[test]
    fn the_list_names_each_file_s_conflict_and_what_is_left() {
        let files = [
            file(
                "src/lib.rs",
                Conflict::BothChanged,
                Some(Markers::Left { hunks: 2 }),
            ),
            file(
                "src/done.rs",
                Conflict::BothChanged,
                Some(Markers::Left { hunks: 0 }),
            ),
            file("docs/a.md", Conflict::DeletedIncoming, None),
        ];
        let mut text = String::new();

        files_text(&mut text, &files);

        assert!(
            text.contains("  src/lib.rs: both changed, 2 conflicts left\n"),
            "{text}"
        );
        assert!(
            text.contains("  src/done.rs: both changed, no conflict markers left\n"),
            "{text}"
        );
        assert!(
            text.contains(
                "  docs/a.md: deleted on the incoming side, changed on the current side\n"
            ),
            "{text}"
        );
    }

    #[test]
    fn a_file_shows_each_side_of_each_hunk() {
        let files = [file(
            "src/lib.rs",
            Conflict::BothChanged,
            Some(Markers::Left { hunks: 1 }),
        )];
        let segments = parse_markers(
            "fn main() {\n<<<<<<< HEAD\n    let a = 1;\n||||||| base\n    let a = 0;\n=======\n>>>>>>> feature\n}\n",
        )
        .unwrap();
        let mut text = String::new();

        file_text(&mut text, &files, "src/lib.rs", &segments, None, None);

        assert!(
            text.starts_with("src/lib.rs: both changed, 1 conflict left.\n"),
            "{text}"
        );
        assert!(text.contains("Conflict 1, lines 2-7:\n"), "{text}");
        assert!(
            text.contains("Current (HEAD):\n        let a = 1;\n"),
            "{text}"
        );
        assert!(text.contains("Base:\n        let a = 0;\n"), "{text}");
        assert!(
            text.contains("Incoming (what is applied): no lines.\n"),
            "{text}"
        );
    }
}
