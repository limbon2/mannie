//! Conflicts as git leaves them (plugins/git/docs/conflicts-and-history.md):
//! the stages of each conflicted path in the index, as `git ls-files -u -z`
//! lists them, and the conflict markers in the work tree's file, which the
//! plugin's operations write in the `zdiff3` style, so each hunk has its
//! base. The markers are read from the file as it is now, so the hunks that
//! the agent resolved already no longer show.

use serde::Serialize;

use crate::parse::status::Conflict;

/// The stages of a conflicted path: the base, the current side and the
/// incoming side, each with its mode and blob, when it has one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Stages {
    pub(crate) path: String,
    pub(crate) base: Option<StageEntry>,
    pub(crate) current: Option<StageEntry>,
    pub(crate) incoming: Option<StageEntry>,
}

impl Stages {
    /// How the path conflicts, by the stages that it has: a side without a
    /// stage deleted the file, or never had it.
    pub(crate) fn conflict(&self) -> Conflict {
        match (
            self.base.is_some(),
            self.current.is_some(),
            self.incoming.is_some(),
        ) {
            (true, true, true) => Conflict::BothChanged,
            (false, true, true) => Conflict::BothAdded,
            (true, true, false) => Conflict::DeletedIncoming,
            (true, false, true) => Conflict::DeletedCurrent,
            (false, true, false) => Conflict::AddedCurrent,
            (false, false, true) => Conflict::AddedIncoming,
            (_base, false, false) => Conflict::BothDeleted,
        }
    }
}

/// One stage of a conflicted path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StageEntry {
    /// Its mode, such as `100644`.
    pub(crate) mode: String,
    pub(crate) blob: String,
}

/// Reads `git ls-files -u -z`: `<mode> <blob> <stage>\t<path>` for each
/// stage, which git lists path by path.
pub(crate) fn parse_unmerged(output: &[u8]) -> Vec<Stages> {
    let mut paths: Vec<Stages> = Vec::new();
    for record in output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let record = String::from_utf8_lossy(record);
        let Some((info, path)) = record.split_once('\t') else {
            continue;
        };
        let mut info = info.split(' ');
        let (Some(mode), Some(blob), Some(stage)) = (info.next(), info.next(), info.next()) else {
            continue;
        };
        let entry = StageEntry {
            mode: mode.to_owned(),
            blob: blob.to_owned(),
        };
        let known = paths.iter().position(|stages| stages.path == path);
        let index = known.unwrap_or_else(|| {
            paths.push(Stages {
                path: path.to_owned(),
                ..Stages::default()
            });
            paths.len() - 1
        });
        let stages = &mut paths[index];
        match stage {
            "1" => stages.base = Some(entry),
            "2" => stages.current = Some(entry),
            "3" => stages.incoming = Some(entry),
            _other => {}
        }
    }
    paths
}

/// The length of the markers that git writes unless an attribute sets
/// another.
const MARKER_LENGTH: usize = 7;

/// A conflict hunk of a file: the lines of each side, with their line
/// breaks, and where its markers stand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConflictHunk {
    /// Its number in the file, from 1.
    pub(crate) number: u32,
    /// The line of its opening marker, from 1.
    pub(crate) first_line: u32,
    /// The line of its closing marker.
    pub(crate) last_line: u32,
    pub(crate) current: Vec<String>,
    /// The base's lines, when the markers have the base's section.
    pub(crate) base: Option<Vec<String>>,
    pub(crate) incoming: Vec<String>,
}

/// A part of a file: text outside conflicts, or a conflict hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Segment {
    /// Lines outside conflicts, with their line breaks.
    Text(String),
    Conflict(ConflictHunk),
}

/// Why a file's markers could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum MarkerError {
    /// A hunk opens and never closes, or closes without opening.
    #[error("the conflict markers at line {line} are not balanced")]
    Unbalanced { line: u32 },
}

/// Whether a line, without its break, is a marker of `kind`, such as `<`,
/// at the length that git writes: the marker alone, or followed by a space
/// and a label.
fn is_marker(line: &str, kind: char) -> bool {
    let body = line.trim_end_matches(['\n', '\r']);
    let marker_end = body
        .char_indices()
        .find(|(_index, character)| *character != kind)
        .map_or(body.len(), |(index, _character)| index);
    let has_length = marker_end == MARKER_LENGTH;
    let rest = &body[marker_end..];
    has_length && (rest.is_empty() || rest.starts_with(' '))
}

/// Where a hunk being read stands.
enum Side {
    Current,
    Base,
    Incoming,
}

/// Reads a file into text and conflict hunks.
pub(crate) fn parse_markers(text: &str) -> Result<Vec<Segment>, MarkerError> {
    let mut segments: Vec<Segment> = Vec::new();
    let mut plain = String::new();
    let mut open: Option<(ConflictHunk, Side)> = None;
    let mut hunks: u32 = 0;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        // A file of more lines than u32 counts is not a source file.
        let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
        match open.as_mut() {
            None if is_marker(line, '<') => {
                if !plain.is_empty() {
                    segments.push(Segment::Text(std::mem::take(&mut plain)));
                }
                hunks += 1;
                open = Some((
                    ConflictHunk {
                        number: hunks,
                        first_line: number,
                        last_line: number,
                        current: Vec::new(),
                        base: None,
                        incoming: Vec::new(),
                    },
                    Side::Current,
                ));
            }
            None if is_marker(line, '>') || is_marker(line, '=') || is_marker(line, '|') => {
                return Err(MarkerError::Unbalanced { line: number });
            }
            None => plain.push_str(line),
            Some((hunk, side)) => {
                let closes = is_marker(line, '>') && matches!(side, Side::Incoming);
                if closes {
                    hunk.last_line = number;
                } else if is_marker(line, '|') && matches!(side, Side::Current) {
                    hunk.base = Some(Vec::new());
                    *side = Side::Base;
                } else if is_marker(line, '=') && !matches!(side, Side::Incoming) {
                    *side = Side::Incoming;
                } else if is_marker(line, '<') {
                    return Err(MarkerError::Unbalanced { line: number });
                } else {
                    let lines = match side {
                        Side::Current => &mut hunk.current,
                        Side::Base => hunk.base.get_or_insert_with(Vec::new),
                        Side::Incoming => &mut hunk.incoming,
                    };
                    lines.push(line.to_owned());
                }
                if closes && let Some((hunk, _side)) = open.take() {
                    segments.push(Segment::Conflict(hunk));
                }
            }
        }
    }
    if let Some((hunk, _side)) = open {
        return Err(MarkerError::Unbalanced {
            line: hunk.first_line,
        });
    }
    if !plain.is_empty() {
        segments.push(Segment::Text(plain));
    }
    Ok(segments)
}

/// The hunks of a file's segments.
pub(crate) fn hunks(segments: &[Segment]) -> Vec<&ConflictHunk> {
    segments
        .iter()
        .filter_map(|segment| match segment {
            Segment::Conflict(hunk) => Some(hunk),
            Segment::Text(_text) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        MarkerError, Segment, StageEntry, Stages, hunks, is_marker, parse_markers, parse_unmerged,
    };
    use crate::parse::status::Conflict;

    #[test]
    fn the_stages_of_each_path_are_gathered() {
        let output = b"100644 aaaa 1\tsrc/lib.rs\x00100644 bbbb 2\tsrc/lib.rs\x00100644 cccc 3\tsrc/lib.rs\x00\
100644 dddd 2\tdocs/only ours.md\x00";

        let paths = parse_unmerged(output);

        assert_eq!(paths.len(), 2);
        assert_eq!(
            paths[0].incoming,
            Some(StageEntry {
                mode: "100644".to_owned(),
                blob: "cccc".to_owned()
            })
        );
        assert_eq!(paths[1].path, "docs/only ours.md");
        assert!(paths[1].base.is_none() && paths[1].incoming.is_none());
        assert_eq!(paths[0].conflict(), Conflict::BothChanged);
        assert_eq!(paths[1].conflict(), Conflict::AddedCurrent);
    }

    #[test]
    fn the_stages_that_a_path_has_name_its_conflict_as_git_s_status_does() {
        let entry = || {
            Some(StageEntry {
                mode: "100644".to_owned(),
                blob: "aaaa".to_owned(),
            })
        };
        let stages = |base: bool, current: bool, incoming: bool| Stages {
            path: "f".to_owned(),
            base: base.then(entry).flatten(),
            current: current.then(entry).flatten(),
            incoming: incoming.then(entry).flatten(),
        };

        assert_eq!(stages(false, true, true).conflict(), Conflict::BothAdded);
        assert_eq!(stages(true, false, false).conflict(), Conflict::BothDeleted);
        assert_eq!(
            stages(true, true, false).conflict(),
            Conflict::DeletedIncoming
        );
        assert_eq!(
            stages(true, false, true).conflict(),
            Conflict::DeletedCurrent
        );
        assert_eq!(
            stages(false, false, true).conflict(),
            Conflict::AddedIncoming
        );
    }

    const FILE: &str = "fn main() {\r\n\
<<<<<<< HEAD\r\n    let a = 1;\r\n||||||| 1a2b3c4\r\n    let a = 0;\r\n=======\r\n    let a = 2;\r\n>>>>>>> feature\r\n\
    run(a);\r\n\
<<<<<<< HEAD\n=======\n    done();\n>>>>>>> feature\n}\n";

    #[test]
    fn hunks_keep_each_side_with_its_line_breaks_and_where_they_stand() {
        let segments = parse_markers(FILE).unwrap();

        assert_eq!(segments.len(), 5);
        assert_eq!(segments[0], Segment::Text("fn main() {\r\n".to_owned()));
        let found = hunks(&segments);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].number, 1);
        assert_eq!((found[0].first_line, found[0].last_line), (2, 8));
        assert_eq!(found[0].current, ["    let a = 1;\r\n"]);
        assert_eq!(
            found[0].base.as_deref(),
            Some(&["    let a = 0;\r\n".to_owned()][..])
        );
        assert_eq!(found[0].incoming, ["    let a = 2;\r\n"]);
        assert_eq!(found[1].current, Vec::<String>::new());
        assert_eq!(found[1].base, None);
        assert_eq!(found[1].incoming, ["    done();\n"]);
        assert_eq!(segments[4], Segment::Text("}\n".to_owned()));
    }

    #[test]
    fn markers_need_their_length_and_unbalanced_markers_are_refused() {
        assert!(is_marker("<<<<<<< HEAD\n", '<'));
        assert!(is_marker("=======\r\n", '='));
        assert!(!is_marker("<<<<<<<< too long\n", '<'));
        assert!(!is_marker("<<<<<<<HEAD\n", '<'));
        assert!(!is_marker("    =======\n", '='));
        assert_eq!(
            parse_markers("a\n<<<<<<< HEAD\nb\n"),
            Err(MarkerError::Unbalanced { line: 2 })
        );
        assert_eq!(
            parse_markers("a\n>>>>>>> feature\n"),
            Err(MarkerError::Unbalanced { line: 2 })
        );
        assert_eq!(
            parse_markers("no conflicts\n").unwrap(),
            [Segment::Text("no conflicts\n".to_owned())]
        );
    }
}
