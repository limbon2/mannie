//! The output of `git status --porcelain=v2 -z --branch --show-stash`: the
//! branch, its upstream with how far ahead and behind it is, the number of
//! stash entries, and each changed, renamed, conflicted, untracked or ignored
//! file. Each record ends with NUL, and a rename's original path follows its
//! record as one more field, so no path is quoted.

use serde::Serialize;

/// The branch part of a status.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Branch {
    /// The commit of HEAD, or `None` on a branch without commits.
    pub(crate) oid: Option<String>,
    /// The branch's name, or `None` for a detached HEAD.
    pub(crate) head: Option<String>,
    /// The upstream branch, such as `origin/main`.
    pub(crate) upstream: Option<String>,
    /// Commits on the branch that its upstream lacks, when it has one that
    /// exists.
    pub(crate) ahead: Option<u32>,
    /// Commits on the upstream that the branch lacks.
    pub(crate) behind: Option<u32>,
}

/// How a conflicted file conflicts, by what each side did to it. "Current"
/// is what HEAD holds and "incoming" what the operation applies, whatever git
/// calls ours and theirs during the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Conflict {
    /// Both sides changed the file.
    BothChanged,
    /// Both sides added a file of this name.
    BothAdded,
    /// Both sides deleted the file, which a rename on each side leaves.
    BothDeleted,
    /// The current side added the file.
    AddedCurrent,
    /// The incoming side added the file.
    AddedIncoming,
    /// The current side deleted the file, which the incoming side changed.
    DeletedCurrent,
    /// The incoming side deleted the file, which the current side changed.
    DeletedIncoming,
}

impl Conflict {
    /// The conflict that a status code of an unmerged path names, such as
    /// `UU`, where U means unmerged on that side, or `None` for another code.
    pub(crate) fn of_code(code: &str) -> Option<Self> {
        Some(match code {
            "UU" => Self::BothChanged,
            "AA" => Self::BothAdded,
            "DD" => Self::BothDeleted,
            "AU" => Self::AddedCurrent,
            "UA" => Self::AddedIncoming,
            "DU" => Self::DeletedCurrent,
            "UD" => Self::DeletedIncoming,
            _other => return None,
        })
    }
}

/// What a file of the status is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub(crate) enum EntryKind {
    /// A tracked file with changes in the index, the work tree or both.
    Changed,
    /// A file renamed or copied from another path, in the index or the work
    /// tree.
    Renamed {
        /// The path it came from.
        original: String,
        /// Whether git found a copy rather than a rename.
        copied: bool,
    },
    /// A file that an operation left conflicted.
    Conflicted {
        /// How it conflicts.
        conflict: Conflict,
    },
    /// A file that git does not track.
    Untracked,
    /// A file that the repository ignores.
    Ignored,
}

/// One file of the status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Entry {
    /// Its path from the repository's top, with forward slashes.
    pub(crate) path: String,
    /// What the index changed against HEAD: `M`, `T`, `A`, `D`, `R`, `C` or
    /// `.` for nothing.
    pub(crate) staged: char,
    /// What the work tree changed against the index, with the same letters.
    pub(crate) unstaged: char,
    #[serde(flatten)]
    pub(crate) kind: EntryKind,
    /// Whether the path is a submodule.
    pub(crate) submodule: bool,
}

impl Entry {
    /// Whether the index holds a change of the file.
    pub(crate) fn is_staged(&self) -> bool {
        let tracked = matches!(self.kind, EntryKind::Changed | EntryKind::Renamed { .. });
        tracked && self.staged != '.'
    }

    /// Whether the work tree holds a change of the file that the index lacks.
    pub(crate) fn is_unstaged(&self) -> bool {
        let tracked = matches!(self.kind, EntryKind::Changed | EntryKind::Renamed { .. });
        tracked && self.unstaged != '.'
    }
}

/// A repository's status.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Status {
    pub(crate) branch: Branch,
    pub(crate) entries: Vec<Entry>,
    /// The number of stash entries.
    pub(crate) stash: u32,
}

/// Why a status could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("git's status has a record that this version cannot read: {record}")]
pub(crate) struct StatusError {
    /// The start of the record.
    pub(crate) record: String,
}

/// Reads the output of `git status --porcelain=v2 -z --branch --show-stash`.
pub(crate) fn parse_status(output: &[u8]) -> Result<Status, StatusError> {
    let mut status = Status::default();
    let mut records = output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .map(String::from_utf8_lossy);
    while let Some(record) = records.next() {
        let refused = || StatusError {
            record: record.chars().take(80).collect(),
        };
        if let Some(header) = record.strip_prefix("# ") {
            read_header(&mut status, header).ok_or_else(refused)?;
            continue;
        }
        let entry = match record.as_bytes().first() {
            Some(b'1') => changed(&record).ok_or_else(refused)?,
            Some(b'2') => {
                let original = records.next().ok_or_else(refused)?;
                renamed(&record, &original).ok_or_else(refused)?
            }
            Some(b'u') => unmerged(&record).ok_or_else(refused)?,
            Some(b'?') => outside(&record, EntryKind::Untracked).ok_or_else(refused)?,
            Some(b'!') => outside(&record, EntryKind::Ignored).ok_or_else(refused)?,
            _other => return Err(refused()),
        };
        status.entries.push(entry);
    }
    Ok(status)
}

/// Reads one header, such as `branch.ab +2 -1`. A header that this version
/// does not know is skipped, since git adds headers.
fn read_header(status: &mut Status, header: &str) -> Option<()> {
    let (name, value) = header.split_once(' ').unwrap_or((header, ""));
    match name {
        "branch.oid" => {
            status.branch.oid = (value != "(initial)").then(|| value.to_owned());
        }
        "branch.head" => {
            status.branch.head = (value != "(detached)").then(|| value.to_owned());
        }
        "branch.upstream" => status.branch.upstream = Some(value.to_owned()),
        "branch.ab" => {
            let (ahead, behind) = value.split_once(' ')?;
            status.branch.ahead = Some(ahead.strip_prefix('+')?.parse().ok()?);
            status.branch.behind = Some(behind.strip_prefix('-')?.parse().ok()?);
        }
        "stash" => status.stash = value.parse().ok()?,
        _other => {}
    }
    Some(())
}

/// The fields of a record, split at single spaces, with the path, which may
/// hold spaces, as the rest after `fields` fields.
fn fields(record: &str, count: usize) -> Option<(Vec<&str>, &str)> {
    let mut parts = record.splitn(count + 1, ' ');
    let mut leading = Vec::with_capacity(count);
    for _ in 0..count {
        leading.push(parts.next()?);
    }
    let path = parts.next()?;
    (!path.is_empty()).then_some((leading, path))
}

/// The two letters of a status code, such as `M.`.
fn letters(code: &str) -> Option<(char, char)> {
    let mut characters = code.chars();
    let staged = characters.next()?;
    let unstaged = characters.next()?;
    characters.next().is_none().then_some((staged, unstaged))
}

/// Whether a record's submodule field, such as `N...` or `SC..`, names a
/// submodule.
fn is_submodule(field: &str) -> bool {
    field.starts_with('S')
}

/// `1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>`.
fn changed(record: &str) -> Option<Entry> {
    let (fields, path) = fields(record, 8)?;
    let (staged, unstaged) = letters(fields[1])?;
    Some(Entry {
        path: path.to_owned(),
        staged,
        unstaged,
        kind: EntryKind::Changed,
        submodule: is_submodule(fields[2]),
    })
}

/// `2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>`, followed by
/// the original path as its own field.
fn renamed(record: &str, original: &str) -> Option<Entry> {
    let (fields, path) = fields(record, 9)?;
    let (staged, unstaged) = letters(fields[1])?;
    let copied = fields[8].starts_with('C');
    Some(Entry {
        path: path.to_owned(),
        staged,
        unstaged,
        kind: EntryKind::Renamed {
            original: original.to_owned(),
            copied,
        },
        submodule: is_submodule(fields[2]),
    })
}

/// `u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>`.
fn unmerged(record: &str) -> Option<Entry> {
    let (fields, path) = fields(record, 10)?;
    let conflict = Conflict::of_code(fields[1])?;
    let (staged, unstaged) = letters(fields[1])?;
    Some(Entry {
        path: path.to_owned(),
        staged,
        unstaged,
        kind: EntryKind::Conflicted { conflict },
        submodule: is_submodule(fields[2]),
    })
}

/// `? <path>` or `! <path>`.
fn outside(record: &str, kind: EntryKind) -> Option<Entry> {
    let path = record.get(2..).filter(|path| !path.is_empty())?;
    Some(Entry {
        path: path.to_owned(),
        staged: '.',
        unstaged: '.',
        kind,
        submodule: false,
    })
}

#[cfg(test)]
mod tests {
    use super::{Branch, Conflict, Entry, EntryKind, Status, parse_status};

    /// A record of git's output, as `-z` ends it.
    fn records(lines: &[&str]) -> Vec<u8> {
        let mut output = Vec::new();
        for line in lines {
            output.extend_from_slice(line.as_bytes());
            output.push(0);
        }
        output
    }

    #[test]
    fn the_branch_its_upstream_and_the_stash_are_read_from_the_headers() {
        let output = records(&[
            "# branch.oid 1a2b3c4d5e6f7a8b9c0d1a2b3c4d5e6f7a8b9c0d",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -1",
            "# stash 3",
            "# branch.future something",
        ]);

        let status = parse_status(&output).unwrap();

        assert_eq!(
            status,
            Status {
                branch: Branch {
                    oid: Some("1a2b3c4d5e6f7a8b9c0d1a2b3c4d5e6f7a8b9c0d".to_owned()),
                    head: Some("main".to_owned()),
                    upstream: Some("origin/main".to_owned()),
                    ahead: Some(2),
                    behind: Some(1),
                },
                entries: Vec::new(),
                stash: 3,
            }
        );
    }

    #[test]
    fn a_new_repository_and_a_detached_head_have_no_commit_or_branch() {
        let initial = parse_status(&records(&["# branch.oid (initial)", "# branch.head main"]));
        let detached = parse_status(&records(&["# branch.oid 1a2b", "# branch.head (detached)"]));

        assert_eq!(initial.unwrap().branch.oid, None);
        let detached = detached.unwrap();
        assert_eq!(detached.branch.head, None);
        assert_eq!(detached.branch.oid.as_deref(), Some("1a2b"));
    }

    #[test]
    fn each_kind_of_record_is_read_with_paths_that_hold_spaces() {
        let output = records(&[
            "1 M. N... 100644 100644 100644 1111111 2222222 src/main file.rs",
            "1 .M N... 100644 100644 100644 1111111 1111111 README.md",
            "2 R. N... 100644 100644 100644 3333333 3333333 R100 docs/new name.md",
            "docs/old name.md",
            "u UU N... 100644 100644 100644 100644 4444444 5555555 6666666 src/lib.rs",
            "1 .M SC.. 160000 160000 160000 7777777 7777777 vendor/lib",
            "? notes/to do.txt",
            "! target/",
        ]);

        let status = parse_status(&output).unwrap();

        assert_eq!(
            status.entries,
            [
                Entry {
                    path: "src/main file.rs".to_owned(),
                    staged: 'M',
                    unstaged: '.',
                    kind: EntryKind::Changed,
                    submodule: false,
                },
                Entry {
                    path: "README.md".to_owned(),
                    staged: '.',
                    unstaged: 'M',
                    kind: EntryKind::Changed,
                    submodule: false,
                },
                Entry {
                    path: "docs/new name.md".to_owned(),
                    staged: 'R',
                    unstaged: '.',
                    kind: EntryKind::Renamed {
                        original: "docs/old name.md".to_owned(),
                        copied: false,
                    },
                    submodule: false,
                },
                Entry {
                    path: "src/lib.rs".to_owned(),
                    staged: 'U',
                    unstaged: 'U',
                    kind: EntryKind::Conflicted {
                        conflict: Conflict::BothChanged,
                    },
                    submodule: false,
                },
                Entry {
                    path: "vendor/lib".to_owned(),
                    staged: '.',
                    unstaged: 'M',
                    kind: EntryKind::Changed,
                    submodule: true,
                },
                Entry {
                    path: "notes/to do.txt".to_owned(),
                    staged: '.',
                    unstaged: '.',
                    kind: EntryKind::Untracked,
                    submodule: false,
                },
                Entry {
                    path: "target/".to_owned(),
                    staged: '.',
                    unstaged: '.',
                    kind: EntryKind::Ignored,
                    submodule: false,
                },
            ]
        );
        assert!(status.entries[0].is_staged() && !status.entries[0].is_unstaged());
        assert!(status.entries[1].is_unstaged() && !status.entries[1].is_staged());
        assert!(!status.entries[3].is_staged() && !status.entries[3].is_unstaged());
    }

    #[test]
    fn every_code_of_an_unmerged_path_names_its_conflict() {
        for (code, conflict) in [
            ("UU", Conflict::BothChanged),
            ("AA", Conflict::BothAdded),
            ("DD", Conflict::BothDeleted),
            ("AU", Conflict::AddedCurrent),
            ("UA", Conflict::AddedIncoming),
            ("DU", Conflict::DeletedCurrent),
            ("UD", Conflict::DeletedIncoming),
        ] {
            assert_eq!(Conflict::of_code(code), Some(conflict), "{code}");
        }
        assert_eq!(Conflict::of_code("MM"), None);
    }

    #[test]
    fn a_record_that_this_version_cannot_read_is_refused() {
        for lines in [
            &["X something"][..],
            &["1 M. N... 100644"][..],
            &["2 R. N... 100644 100644 100644 3 3 R100 new"][..],
            &["u ZZ N... 1 1 1 1 1 1 1 path"][..],
            &["# branch.ab two -1"][..],
        ] {
            assert!(parse_status(&records(lines)).is_err(), "{lines:?}");
        }
        assert_eq!(parse_status(b"").unwrap(), Status::default());
    }
}
