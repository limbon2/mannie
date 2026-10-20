//! `git_status` (plugins/git/docs/tools.md, tools that read): the branch or
//! the detached commit, its upstream, the operation in progress, and the
//! conflicted, staged, unstaged and untracked files, with the number of
//! stash entries.
//!
//! Submodules show only whether their commit changed: a look inside one
//! would run git there under the submodule's own configuration (T-44).

use std::fmt::Write as _;

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::describe::{CommitLabel, OperationView, label_commits, operation_view};
use crate::failure::GitFailure;
use crate::layout::Layout;
use crate::parse::status::{Branch, Conflict, Entry, EntryKind, Status, parse_status};
use crate::texts;
use crate::tools::{Answer, ReadTool, Reading, fitting_data};

/// The tool's name.
pub(crate) const GIT_STATUS: &str = "git_status";

/// The arguments of `git_status`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatusArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
}

/// `git_status`.
#[derive(Debug)]
pub(crate) struct GitStatus;

impl ReadTool for GitStatus {
    type Arguments = StatusArguments;

    const NAME: &'static str = GIT_STATUS;
    const DESCRIPTION: &'static str = texts::STATUS_DESCRIPTION;

    fn repository(arguments: &StatusArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn check(_arguments: &StatusArguments, _layout: &Layout) -> Result<(), GitFailure> {
        Ok(())
    }

    fn answer(
        reading: Reading,
        _arguments: StatusArguments,
    ) -> BoxFuture<'static, Result<Answer, GitFailure>> {
        Box::pin(async move { status(&reading).await })
    }
}

/// The card's data.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusData<'a> {
    branch: &'a Branch,
    head: Option<&'a CommitLabel>,
    operation: Option<&'a OperationView>,
    entries: &'a [Entry],
    /// The files in all, of which `entries` may hold the first only.
    entries_total: usize,
    stash: u32,
}

/// Reads the repository's status.
pub(crate) async fn read_status(reading: &Reading) -> Result<Status, GitFailure> {
    let ran = reading
        .git
        .output(reading.run([
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--show-stash",
            "--untracked-files=normal",
            "--ignore-submodules=dirty",
        ]))
        .await?;
    parse_status(&ran.output).map_err(|error| GitFailure::failed(&error.to_string()))
}

async fn status(reading: &Reading) -> Result<Answer, GitFailure> {
    let status = read_status(reading).await?;
    let operation = operation_view(reading).await?;
    let head_oid: Vec<String> = status.branch.oid.iter().cloned().collect();
    let head = label_commits(reading, &head_oid).await?.into_iter().next();
    let text = status_text(&status, head.as_ref(), operation.as_ref());
    let data = fitting_data(status.entries.len(), |kept| StatusData {
        branch: &status.branch,
        head: head.as_ref(),
        operation: operation.as_ref(),
        entries: &status.entries[..kept],
        entries_total: status.entries.len(),
        stash: status.stash,
    });
    Ok(Answer { text, data })
}

/// The status as the model reads it.
pub(crate) fn status_text(
    status: &Status,
    head: Option<&CommitLabel>,
    operation: Option<&OperationView>,
) -> String {
    let mut text = branch_line(&status.branch, head);
    text.push('\n');
    let conflicted: Vec<&Entry> = status
        .entries
        .iter()
        .filter(|entry| matches!(entry.kind, EntryKind::Conflicted { .. }))
        .collect();
    if let Some(operation) = operation {
        text.push_str(&operation.text());
        if conflicted.is_empty() {
            text.push_str(" No file is conflicted, so it can continue.\n");
        } else {
            text.push_str(" Resolve the conflicted files, then continue it.\n");
        }
    }
    let staged: Vec<&Entry> = status
        .entries
        .iter()
        .filter(|entry| entry.is_staged())
        .collect();
    let unstaged: Vec<&Entry> = status
        .entries
        .iter()
        .filter(|entry| entry.is_unstaged())
        .collect();
    let untracked: Vec<&Entry> = status
        .entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Untracked)
        .collect();
    if staged.is_empty() && unstaged.is_empty() && untracked.is_empty() && conflicted.is_empty() {
        text.push_str("Nothing to commit: the work tree and the index match HEAD.\n");
    }
    section(&mut text, "Conflicted", &conflicted, |entry| {
        let EntryKind::Conflicted { conflict } = &entry.kind else {
            return entry.path.clone();
        };
        format!("{}: {}", conflict_words(*conflict), entry.path)
    });
    section(&mut text, "Staged", &staged, |entry| {
        change_line(entry, entry.staged)
    });
    section(&mut text, "Not staged", &unstaged, |entry| {
        change_line(entry, entry.unstaged)
    });
    section(&mut text, "Untracked", &untracked, |entry| {
        entry.path.clone()
    });
    if status.stash > 0 {
        let _written = writeln!(text, "Stash entries: {}.", status.stash);
    }
    text
}

/// The first line: the branch or the detached commit, and its upstream.
fn branch_line(branch: &Branch, head: Option<&CommitLabel>) -> String {
    let at = head.map_or_else(String::new, |head| format!(" at {}", head.text()));
    let Some(name) = &branch.head else {
        return format!("HEAD is detached{at}.");
    };
    if branch.oid.is_none() {
        return format!("On branch {name}, which has no commits yet.");
    }
    let tracking = match (&branch.upstream, branch.ahead, branch.behind) {
        (None, _ahead, _behind) => ", without an upstream".to_owned(),
        (Some(upstream), Some(0), Some(0)) => format!(", up to date with {upstream}"),
        (Some(upstream), Some(ahead), Some(behind)) => {
            format!(", tracking {upstream}: {ahead} ahead, {behind} behind")
        }
        (Some(upstream), _ahead, _behind) => {
            format!(", tracking {upstream}, which no longer exists")
        }
    };
    format!("On branch {name}{at}{tracking}.")
}

/// A list of files under a heading, such as `Staged (2):`.
fn section(text: &mut String, heading: &str, entries: &[&Entry], line: impl Fn(&Entry) -> String) {
    if entries.is_empty() {
        return;
    }
    let _written = writeln!(text, "{heading} ({}):", entries.len());
    for entry in entries {
        let submodule = if entry.submodule { " (submodule)" } else { "" };
        let _written = writeln!(text, "  {}{submodule}", line(entry));
    }
}

/// A changed file with what changed, such as `modified: src/lib.rs`.
fn change_line(entry: &Entry, letter: char) -> String {
    let what = match letter {
        'M' => "modified",
        'T' => "type changed",
        'A' => "added",
        'D' => "deleted",
        'R' => "renamed",
        'C' => "copied",
        _other => "changed",
    };
    match &entry.kind {
        EntryKind::Renamed { original, .. } if matches!(letter, 'R' | 'C') => {
            format!("{what}: {} (from {original})", entry.path)
        }
        _other => format!("{what}: {}", entry.path),
    }
}

/// How a file conflicts, in words.
pub(crate) fn conflict_words(conflict: Conflict) -> &'static str {
    match conflict {
        Conflict::BothChanged => "both changed",
        Conflict::BothAdded => "both added",
        Conflict::BothDeleted => "both deleted",
        Conflict::AddedCurrent => "added on the current side only",
        Conflict::AddedIncoming => "added on the incoming side only",
        Conflict::DeletedCurrent => "deleted on the current side, changed on the incoming side",
        Conflict::DeletedIncoming => "deleted on the incoming side, changed on the current side",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mannie_git_testing::{HostGit, TestRepository};
    use serde_json::json;

    use super::{GitStatus, branch_line, status_text};
    use crate::describe::CommitLabel;
    use crate::parse::status::{Branch, Conflict, Entry, EntryKind, Status};
    use crate::tools::testing::{ROOM, call, call_with};

    /// A repository whose merge of `feature` into `main` stopped on a
    /// conflict in `a.txt`, with the markers of zdiff3.
    fn stopped_merge() -> TestRepository {
        let repository = TestRepository::new();
        repository.write("a.txt", "one\n");
        repository.commit("Start");
        repository.git(&["switch", "--quiet", "-c", "feature"]);
        repository.write("a.txt", "feature\n");
        repository.commit("Feature change");
        repository.git(&["switch", "--quiet", "main"]);
        repository.write("a.txt", "main\n");
        repository.commit("Main change");
        let merged =
            repository.git_may_fail(&["-c", "merge.conflictStyle=zdiff3", "merge", "feature"]);
        assert!(!merged, "the merge stops on the conflict");
        repository
    }

    #[tokio::test]
    async fn the_status_names_the_merge_that_stopped_and_every_kind_of_file() {
        let repository = stopped_merge();
        repository.write("new.txt", "new\n");

        let result = call::<GitStatus>(&repository, json!({})).await;

        assert_eq!(result.failure, None, "{}", result.output);
        assert!(
            result.output.starts_with("On branch main at "),
            "{}",
            result.output
        );
        assert!(
            result
                .output
                .contains(" Main change, without an upstream.\n"),
            "{}",
            result.output
        );
        assert!(
            result
                .output
                .contains("Feature change is in progress. Resolve the conflicted files")
        );
        assert!(
            result
                .output
                .contains("Conflicted (1):\n  both changed: a.txt\n")
        );
        assert!(result.output.contains("Untracked (1):\n  new.txt\n"));
        let data = result.data.expect("the card's data");
        assert_eq!(data["branch"]["head"], "main");
        assert_eq!(data["operation"]["kind"], "merge");
        assert_eq!(data["operation"]["incoming"]["subject"], "Feature change");
        assert_eq!(data["entriesTotal"], 2);
    }

    #[tokio::test]
    async fn a_folder_outside_a_repository_and_a_missing_git_fail_with_their_codes() {
        let repository = TestRepository::new();
        let plain = repository.sibling("plain");

        let outside = call::<GitStatus>(&repository, json!({ "repository": plain })).await;
        let missing =
            call_with::<GitStatus>(&Arc::new(HostGit::missing()), &repository, json!({}), ROOM)
                .await;
        let unknown = call::<GitStatus>(&repository, json!({ "folder": "x" })).await;

        assert_eq!(
            outside.failure.as_deref(),
            Some("not_a_repository"),
            "{}",
            outside.output
        );
        assert_eq!(
            missing.failure.as_deref(),
            Some("git_missing"),
            "{}",
            missing.output
        );
        assert_eq!(
            unknown.failure.as_deref(),
            Some("invalid_argument"),
            "{}",
            unknown.output
        );
    }

    fn head() -> CommitLabel {
        CommitLabel {
            oid: "1".repeat(40),
            short: "1111111".to_owned(),
            subject: "Fix the parser".to_owned(),
        }
    }

    fn entry(path: &str, staged: char, unstaged: char, kind: EntryKind) -> Entry {
        Entry {
            path: path.to_owned(),
            staged,
            unstaged,
            kind,
            submodule: false,
        }
    }

    #[test]
    fn the_branch_line_names_the_upstream_and_how_far_apart_they_are() {
        let mut branch = Branch {
            oid: Some("1".repeat(40)),
            head: Some("main".to_owned()),
            upstream: Some("origin/main".to_owned()),
            ahead: Some(2),
            behind: Some(1),
        };

        assert_eq!(
            branch_line(&branch, Some(&head())),
            "On branch main at 1111111 Fix the parser, tracking origin/main: 2 ahead, 1 behind."
        );
        branch.ahead = None;
        branch.behind = None;
        assert!(
            branch_line(&branch, None).ends_with("tracking origin/main, which no longer exists.")
        );
        branch.head = None;
        assert_eq!(
            branch_line(&branch, Some(&head())),
            "HEAD is detached at 1111111 Fix the parser."
        );
        let unborn = Branch {
            head: Some("main".to_owned()),
            ..Branch::default()
        };
        assert_eq!(
            branch_line(&unborn, None),
            "On branch main, which has no commits yet."
        );
    }

    #[test]
    fn files_are_grouped_by_what_happened_to_them() {
        let status = Status {
            branch: Branch {
                oid: Some("1".repeat(40)),
                head: Some("main".to_owned()),
                ..Branch::default()
            },
            entries: vec![
                entry(
                    "src/lib.rs",
                    'U',
                    'U',
                    EntryKind::Conflicted {
                        conflict: Conflict::BothChanged,
                    },
                ),
                entry("src/main.rs", 'M', 'M', EntryKind::Changed),
                entry(
                    "docs/new.md",
                    'R',
                    '.',
                    EntryKind::Renamed {
                        original: "docs/old.md".to_owned(),
                        copied: false,
                    },
                ),
                entry("notes/", '.', '.', EntryKind::Untracked),
            ],
            stash: 2,
        };

        let text = status_text(&status, Some(&head()), None);

        assert!(
            text.contains("Conflicted (1):\n  both changed: src/lib.rs\n"),
            "{text}"
        );
        assert!(
            text.contains(
                "Staged (2):\n  modified: src/main.rs\n  renamed: docs/new.md (from docs/old.md)\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("Not staged (1):\n  modified: src/main.rs\n"),
            "{text}"
        );
        assert!(text.contains("Untracked (1):\n  notes/\n"), "{text}");
        assert!(text.ends_with("Stash entries: 2.\n"), "{text}");
        let clean = Status {
            entries: Vec::new(),
            stash: 0,
            ..status
        };
        assert!(status_text(&clean, None, None).contains("Nothing to commit"));
    }
}
