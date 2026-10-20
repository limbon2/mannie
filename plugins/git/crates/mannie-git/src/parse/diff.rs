//! Diffs as git writes them: the changed files of `--numstat -z`, with
//! their added and removed lines, and the patch of `--patch`, split into one
//! part for each file and read into hunks and lines for a card's diff view.
//! Both runs list the files in the same order.

use serde::Serialize;

/// A changed file and how many lines it adds and removes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileStat {
    /// The path after the change, from the repository's top.
    pub(crate) path: String,
    /// The path before a rename or a copy.
    pub(crate) original: Option<String>,
    /// The lines that it adds, or `None` for a binary file.
    pub(crate) added: Option<u32>,
    /// The lines that it removes, or `None` for a binary file.
    pub(crate) removed: Option<u32>,
}

impl FileStat {
    /// Whether git found the file binary.
    pub(crate) fn is_binary(&self) -> bool {
        self.added.is_none()
    }
}

/// Reads the output of `git diff --numstat -z`: `<added>\t<removed>\t<path>`
/// for each file, or `<added>\t<removed>\t` followed by the original and the
/// new path as two more fields for a rename, with `-` for the counts of a
/// binary file.
pub(crate) fn parse_numstat(output: &[u8]) -> Vec<FileStat> {
    let mut fields = output
        .split(|byte| *byte == 0)
        .map(|field| String::from_utf8_lossy(field).into_owned());
    let mut stats = Vec::new();
    while let Some(field) = fields.next() {
        let field = field.trim_start_matches('\n');
        let mut parts = field.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let count = |text: &str| text.parse::<u32>().ok();
        let stat = if path.is_empty() {
            let (Some(original), Some(path)) = (fields.next(), fields.next()) else {
                break;
            };
            FileStat {
                path,
                original: Some(original),
                added: count(added),
                removed: count(removed),
            }
        } else {
            FileStat {
                path: path.to_owned(),
                original: None,
                added: count(added),
                removed: count(removed),
            }
        };
        stats.push(stat);
    }
    stats
}

/// Splits a patch into the part of each file, each starting with its
/// `diff --git` or `diff --cc` line. Text before the first file, such as a
/// commit's header, is dropped.
pub(crate) fn split_patch(patch: &str) -> Vec<&str> {
    let mut starts: Vec<usize> = Vec::new();
    let mut offset = 0;
    for line in patch.split_inclusive('\n') {
        let starts_file = line.starts_with("diff --git ")
            || line.starts_with("diff --cc ")
            || line.starts_with("diff --combined ");
        if starts_file {
            starts.push(offset);
        }
        offset += line.len();
    }
    let mut parts = Vec::with_capacity(starts.len());
    for (index, start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(patch.len());
        parts.push(&patch[*start..end]);
    }
    parts
}

/// What a line of a hunk is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LineKind {
    /// A line that both sides have.
    Context,
    /// A line that the change adds.
    Added,
    /// A line that the change removes.
    Removed,
}

/// One line of a hunk, with its numbers on the side or sides that have it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Line {
    pub(crate) kind: LineKind,
    /// Its number before the change, for a context or removed line.
    pub(crate) old: Option<u32>,
    /// Its number after the change, for a context or added line.
    pub(crate) new: Option<u32>,
    /// Its text, without the marker and the line break.
    pub(crate) text: String,
    /// Whether the side that has the line ends without a line break.
    pub(crate) no_newline: bool,
}

/// A hunk of a file's patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Hunk {
    /// Its header, such as `@@ -10,6 +10,8 @@ fn main()`.
    pub(crate) header: String,
    pub(crate) lines: Vec<Line>,
}

/// The first line numbers of a hunk header's two sides, such as `10` and
/// `12` for `@@ -10,6 +12,8 @@`. A combined diff's header, with more sides,
/// reads as its first and its last.
fn hunk_starts(header: &str) -> Option<(u32, u32)> {
    let ranges: Vec<&str> = header
        .split_whitespace()
        .skip(1)
        .take_while(|part| !part.starts_with("@@"))
        .collect();
    let old = ranges.first()?.strip_prefix('-')?;
    let new = ranges.last()?.strip_prefix('+')?;
    let start = |range: &str| range.split(',').next()?.parse::<u32>().ok();
    Some((start(old)?, start(new)?))
}

/// Reads the hunks of one file's part of a two-sided patch. A combined
/// diff of a merge, whose lines carry a marker for each parent, has no
/// hunks here, and its text stays the model's.
pub(crate) fn parse_hunks(part: &str) -> Vec<Hunk> {
    if part.starts_with("diff --cc ") || part.starts_with("diff --combined ") {
        return Vec::new();
    }
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut numbers = (0, 0);
    for line in part.lines() {
        if line.starts_with("@@ ") {
            let Some(starts) = hunk_starts(line) else {
                continue;
            };
            numbers = starts;
            hunks.push(Hunk {
                header: line.to_owned(),
                lines: Vec::new(),
            });
            continue;
        }
        let Some(hunk) = hunks.last_mut() else {
            continue;
        };
        // An empty line in a hunk is an empty context line whose space a
        // tool on the way removed.
        let (marker, text) = line.split_at_checked(1).unwrap_or((" ", ""));
        let kind = match marker {
            " " => LineKind::Context,
            "+" => LineKind::Added,
            "-" => LineKind::Removed,
            "\\" => {
                if let Some(last) = hunk.lines.last_mut() {
                    last.no_newline = true;
                }
                continue;
            }
            _other => continue,
        };
        let (old, new) = match kind {
            LineKind::Context => (Some(numbers.0), Some(numbers.1)),
            LineKind::Added => (None, Some(numbers.1)),
            LineKind::Removed => (Some(numbers.0), None),
        };
        if old.is_some() {
            numbers.0 += 1;
        }
        if new.is_some() {
            numbers.1 += 1;
        }
        hunk.lines.push(Line {
            kind,
            old,
            new,
            text: text.to_owned(),
            no_newline: false,
        });
    }
    hunks
}

#[cfg(test)]
mod tests {
    use super::{FileStat, LineKind, parse_hunks, parse_numstat, split_patch};

    #[test]
    fn numstat_reads_counts_binary_files_and_renames() {
        let output = b"3\t1\tsrc/main file.rs\0-\t-\tassets/logo.png\0\
0\t0\t\0docs/old.md\0docs/new.md\0";

        let stats = parse_numstat(output);

        assert_eq!(
            stats,
            [
                FileStat {
                    path: "src/main file.rs".to_owned(),
                    original: None,
                    added: Some(3),
                    removed: Some(1),
                },
                FileStat {
                    path: "assets/logo.png".to_owned(),
                    original: None,
                    added: None,
                    removed: None,
                },
                FileStat {
                    path: "docs/new.md".to_owned(),
                    original: Some("docs/old.md".to_owned()),
                    added: Some(0),
                    removed: Some(0),
                },
            ]
        );
        assert!(stats[1].is_binary());
        assert_eq!(parse_numstat(b""), Vec::new());
    }

    const PATCH: &str = "commit 1a2b\nAuthor: Jane\n\n    Subject\n\n\
diff --git a/src/lib.rs b/src/lib.rs\nindex 1111111..2222222 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n\
@@ -1,4 +1,5 @@ fn main()\n first\n-second\n+second, changed\n+inserted\n third\n\\ No newline at end of file\n\
diff --git a/README.md b/README.md\nnew file mode 100644\n--- /dev/null\n+++ b/README.md\n@@ -0,0 +1 @@\n+Hello\n";

    #[test]
    fn a_patch_splits_into_one_part_for_each_file_without_the_commit_header() {
        let parts = split_patch(PATCH);

        assert_eq!(parts.len(), 2);
        assert!(parts[0].starts_with("diff --git a/src/lib.rs"));
        assert!(parts[0].ends_with("\\ No newline at end of file\n"));
        assert!(parts[1].starts_with("diff --git a/README.md"));
        assert_eq!(split_patch("no diff here"), Vec::<&str>::new());
    }

    #[test]
    fn hunks_number_each_line_on_the_sides_that_have_it() {
        let parts = split_patch(PATCH);

        let hunks = parse_hunks(parts[0]);

        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].header, "@@ -1,4 +1,5 @@ fn main()");
        let lines: Vec<(LineKind, Option<u32>, Option<u32>, &str)> = hunks[0]
            .lines
            .iter()
            .map(|line| (line.kind, line.old, line.new, line.text.as_str()))
            .collect();
        assert_eq!(
            lines,
            [
                (LineKind::Context, Some(1), Some(1), "first"),
                (LineKind::Removed, Some(2), None, "second"),
                (LineKind::Added, None, Some(2), "second, changed"),
                (LineKind::Added, None, Some(3), "inserted"),
                (LineKind::Context, Some(3), Some(4), "third"),
            ]
        );
        assert!(hunks[0].lines[4].no_newline);
        let added = parse_hunks(parts[1]);
        assert_eq!(added[0].lines[0].new, Some(1));
    }

    #[test]
    fn a_combined_diff_has_no_hunks_of_its_own() {
        let combined =
            "diff --cc src/lib.rs\nindex 1,2..3\n@@@ -1,1 -1,1 +1,1 @@@\n- a\n -b\n++c\n";

        assert_eq!(parse_hunks(combined), Vec::new());
    }
}
