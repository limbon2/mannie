//! Commits as `git log -z --format=…` writes them with [`LOG_FORMAT`]: one
//! record for each commit, ended by NUL, whose fields are separated by the
//! unit separator, which no field of a commit holds.

use serde::Serialize;

/// The separator between the fields of a commit, which git writes for `%x1f`.
const FIELD: char = '\u{1f}';

/// The format of [`parse_log`]: the hash, the short hash, the parents, the
/// author's name and email, the author's and the committer's dates in strict
/// ISO 8601, the branches and tags that point at the commit, and the subject.
pub(crate) const LOG_FORMAT: &str =
    "--format=%H%x1f%h%x1f%P%x1f%an%x1f%ae%x1f%aI%x1f%cI%x1f%D%x1f%s";

/// A commit as `git log` describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Commit {
    /// The full hash.
    pub(crate) oid: String,
    /// The abbreviated hash, as the repository abbreviates it.
    pub(crate) short: String,
    /// The parents' full hashes; two or more for a merge.
    pub(crate) parents: Vec<String>,
    /// The author's name.
    pub(crate) author: String,
    /// The author's email.
    pub(crate) email: String,
    /// When the author wrote it, such as `2026-10-19T14:02:11+02:00`.
    pub(crate) authored: String,
    /// When it was committed.
    pub(crate) committed: String,
    /// The branches and tags that point at it, such as `HEAD -> main` or
    /// `tag: v1.0`.
    pub(crate) refs: Vec<String>,
    /// The first line of its message.
    pub(crate) subject: String,
}

impl Commit {
    /// Whether the commit merges two or more parents.
    pub(crate) fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }

    /// The day the author wrote it, such as `2026-10-19`.
    pub(crate) fn day(&self) -> &str {
        self.authored.get(..10).unwrap_or(&self.authored)
    }
}

/// Why a log could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("git's log has a record that this version cannot read: {record}")]
pub(crate) struct LogError {
    /// The start of the record.
    pub(crate) record: String,
}

/// Reads the output of `git log -z` with [`LOG_FORMAT`]. A record may start
/// with a line break, which git writes between records of some modes.
pub(crate) fn parse_log(output: &[u8]) -> Result<Vec<Commit>, LogError> {
    output
        .split(|byte| *byte == 0)
        .map(String::from_utf8_lossy)
        .filter(|record| !record.trim().is_empty())
        .map(|record| {
            parse_commit(record.trim_start_matches('\n')).ok_or_else(|| LogError {
                record: record.chars().take(80).collect(),
            })
        })
        .collect()
}

/// The format of [`parse_details`]: the hash and the short hash, the
/// parents' hashes and short hashes, the author's name, email and date, the
/// committer's name, email and date, the branches and tags that point at the
/// commit, and the whole message.
pub(crate) const DETAILS_FORMAT: &str =
    "--format=%H%x1f%h%x1f%P%x1f%p%x1f%an%x1f%ae%x1f%aI%x1f%cn%x1f%ce%x1f%cI%x1f%D%x1f%B";

/// A commit with its whole message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitDetails {
    pub(crate) oid: String,
    pub(crate) short: String,
    pub(crate) parents: Vec<String>,
    pub(crate) short_parents: Vec<String>,
    pub(crate) author: String,
    pub(crate) email: String,
    pub(crate) authored: String,
    pub(crate) committer: String,
    pub(crate) committer_email: String,
    pub(crate) committed: String,
    pub(crate) refs: Vec<String>,
    /// The message as the commit holds it, without trailing line breaks.
    pub(crate) message: String,
}

/// Reads the output of `git show --no-patch` with [`DETAILS_FORMAT`].
pub(crate) fn parse_details(output: &str) -> Option<CommitDetails> {
    let fields: Vec<&str> = output.splitn(12, FIELD).collect();
    let [
        oid,
        short,
        parents,
        short_parents,
        author_name,
        author_email,
        author_date,
        committer_name,
        committer_email,
        committer_date,
        refs,
        message,
    ] = fields.as_slice()
    else {
        return None;
    };
    let is_hash = !oid.is_empty() && oid.chars().all(|character| character.is_ascii_hexdigit());
    is_hash.then(|| CommitDetails {
        oid: (*oid).to_owned(),
        short: (*short).to_owned(),
        parents: words(parents),
        short_parents: words(short_parents),
        author: (*author_name).to_owned(),
        email: (*author_email).to_owned(),
        authored: (*author_date).to_owned(),
        committer: (*committer_name).to_owned(),
        committer_email: (*committer_email).to_owned(),
        committed: (*committer_date).to_owned(),
        refs: ref_names(refs),
        message: message.trim_end_matches(['\n', '\r']).to_owned(),
    })
}

/// The format of [`parse_reflog`]: the commit's hash and short hash, the
/// entry's selector and what moved the ref, the commit's date and subject.
/// `git stash list` writes its entries in it too.
pub(crate) const REFLOG_FORMAT: &str = "--format=%H%x1f%h%x1f%gd%x1f%gs%x1f%cI%x1f%s";

/// An entry of a reflog: where a ref pointed, and what moved it there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReflogEntry {
    pub(crate) oid: String,
    pub(crate) short: String,
    /// The entry's selector, such as `HEAD@{2}` or `stash@{0}`.
    pub(crate) selector: String,
    /// What moved the ref, such as `commit: Fix the parser` or `reset:
    /// moving to HEAD~1`, or a stash entry's message.
    pub(crate) action: String,
    pub(crate) committed: String,
    pub(crate) subject: String,
}

/// Reads the output of `git log -z --walk-reflogs` or `git stash list -z`
/// with [`REFLOG_FORMAT`].
pub(crate) fn parse_reflog(output: &[u8]) -> Result<Vec<ReflogEntry>, LogError> {
    output
        .split(|byte| *byte == 0)
        .map(String::from_utf8_lossy)
        .filter(|record| !record.trim().is_empty())
        .map(|record| {
            let record = record.trim_start_matches('\n');
            let fields: Vec<&str> = record.splitn(6, FIELD).collect();
            let refused = || LogError {
                record: record.chars().take(80).collect(),
            };
            let [oid, short, selector, action, committed, subject] = fields.as_slice() else {
                return Err(refused());
            };
            if oid.is_empty() || !oid.chars().all(|character| character.is_ascii_hexdigit()) {
                return Err(refused());
            }
            Ok(ReflogEntry {
                oid: (*oid).to_owned(),
                short: (*short).to_owned(),
                selector: (*selector).to_owned(),
                action: (*action).to_owned(),
                committed: (*committed).to_owned(),
                subject: subject.trim_end_matches('\n').to_owned(),
            })
        })
        .collect()
}

/// The format of [`parse_line_history`]: [`LOG_FORMAT`]'s fields after a
/// record separator, which starts each commit, since `git log -L` follows
/// each commit with the patch of the lines.
pub(crate) const LINES_FORMAT: &str =
    "--format=%x1e%H%x1f%h%x1f%P%x1f%an%x1f%ae%x1f%aI%x1f%cI%x1f%D%x1f%s";

/// Reads the output of `git log -L` with [`LINES_FORMAT`]: each commit with
/// the patch of the lines that it changed.
pub(crate) fn parse_line_history(output: &str) -> Result<Vec<(Commit, String)>, LogError> {
    output
        .split('\u{1e}')
        .filter(|record| !record.trim().is_empty())
        .map(|record| {
            let (header, patch) = record.split_once('\n').unwrap_or((record, ""));
            let commit = parse_commit(header).ok_or_else(|| LogError {
                record: record.chars().take(80).collect(),
            })?;
            Ok((commit, patch.trim_start_matches('\n').to_owned()))
        })
        .collect()
}

/// The words of a field, such as the hashes of the parents.
fn words(field: &str) -> Vec<String> {
    field.split_whitespace().map(str::to_owned).collect()
}

/// The branches and tags of a field of `%D`.
fn ref_names(field: &str) -> Vec<String> {
    field
        .split(", ")
        .filter(|reference| !reference.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Reads one commit's record.
pub(crate) fn parse_commit(record: &str) -> Option<Commit> {
    let fields: Vec<&str> = record.splitn(9, FIELD).collect();
    let [
        oid,
        short,
        parents,
        author,
        email,
        authored,
        committed,
        refs,
        subject,
    ] = fields.as_slice()
    else {
        return None;
    };
    let is_hash = !oid.is_empty() && oid.chars().all(|character| character.is_ascii_hexdigit());
    if !is_hash {
        return None;
    }
    Some(Commit {
        oid: (*oid).to_owned(),
        short: (*short).to_owned(),
        parents: words(parents),
        author: (*author).to_owned(),
        email: (*email).to_owned(),
        authored: (*authored).to_owned(),
        committed: (*committed).to_owned(),
        refs: ref_names(refs),
        subject: subject.trim_end_matches('\n').to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::{Commit, parse_details, parse_line_history, parse_log, parse_reflog};

    fn record(fields: &[&str]) -> Vec<u8> {
        let mut record = fields.join("\u{1f}").into_bytes();
        record.push(0);
        record
    }

    #[test]
    fn details_keep_the_whole_message_and_both_forms_of_the_parents() {
        let output = [
            "1a2b3c4d5e6f7a8b9c0d1a2b3c4d5e6f7a8b9c0d",
            "1a2b3c4",
            "aaaaaaaa bbbbbbbb",
            "aaaa bbbb",
            "Jane Doe",
            "jane@example.com",
            "2026-10-19T14:02:11+02:00",
            "Joe",
            "joe@example.com",
            "2026-10-19T15:00:00+02:00",
            "HEAD -> main",
            "Merge branch 'feature'\n\nWith a body\x1fthat holds a separator.\n\n",
        ]
        .join("\u{1f}");

        let details = parse_details(&output).unwrap();

        assert_eq!(details.short_parents, ["aaaa", "bbbb"]);
        assert_eq!(details.committer, "Joe");
        assert_eq!(details.refs, ["HEAD -> main"]);
        assert_eq!(
            details.message,
            "Merge branch 'feature'\n\nWith a body\u{1f}that holds a separator."
        );
        assert_eq!(parse_details("not a commit"), None);
    }

    #[test]
    fn reflog_entries_name_their_selector_and_what_moved_the_ref() {
        let mut output = record(&[
            "1a2b3c4d5e6f7a8b9c0d1a2b3c4d5e6f7a8b9c0d",
            "1a2b3c4",
            "HEAD@{0}",
            "reset: moving to HEAD~1",
            "2026-10-19T14:02:11+02:00",
            "Fix the parser",
        ]);
        output.extend(record(&[
            "aaaaaaaabbbbbbbbccccccccddddddddeeeeeeee",
            "aaaaaaa",
            "stash@{0}",
            "WIP on main: 1a2b3c4 Fix the parser",
            "2026-10-18T09:00:00Z",
            "WIP on main: 1a2b3c4 Fix the parser",
        ]));

        let entries = parse_reflog(&output).unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].selector, "HEAD@{0}");
        assert_eq!(entries[0].action, "reset: moving to HEAD~1");
        assert_eq!(entries[1].selector, "stash@{0}");
        let refused = parse_reflog(&record(&["xyz", "", "", "", "", ""])).unwrap_err();
        assert!(refused.record.starts_with("xyz"), "{refused}");
    }

    #[test]
    fn a_line_history_pairs_each_commit_with_its_patch() {
        let commit = [
            "1a2b3c4d5e6f7a8b9c0d1a2b3c4d5e6f7a8b9c0d",
            "1a2b3c4",
            "aaaa",
            "Jane Doe",
            "jane@example.com",
            "2026-10-19T14:02:11+02:00",
            "2026-10-19T14:02:11+02:00",
            "",
            "Fix the parser",
        ]
        .join("\u{1f}");
        let output = format!(
            "\u{1e}{commit}\n\ndiff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,1 +1,1 @@\n-a\n+b\n"
        );

        let history = parse_line_history(&output).unwrap();

        assert_eq!(history.len(), 1);
        assert_eq!(history[0].0.subject, "Fix the parser");
        assert!(history[0].1.starts_with("diff --git a/f.txt"));
        let refused = parse_line_history("\u{1e}broken\n").unwrap_err();
        assert!(refused.record.starts_with("broken"), "{refused}");
    }

    #[test]
    fn each_commit_is_read_with_its_parents_and_refs() {
        let mut output = record(&[
            "1a2b3c4d5e6f7a8b9c0d1a2b3c4d5e6f7a8b9c0d",
            "1a2b3c4",
            "aaaaaaaa bbbbbbbb",
            "Jane Doe",
            "jane@example.com",
            "2026-10-19T14:02:11+02:00",
            "2026-10-19T14:05:00+02:00",
            "HEAD -> main, origin/main, tag: v1.0",
            "Merge branch 'feature'",
        ]);
        output.extend(record(&[
            "aaaaaaaabbbbbbbbccccccccddddddddeeeeeeee",
            "aaaaaaa",
            "",
            "Joe",
            "joe@example.com",
            "2026-10-18T09:00:00Z",
            "2026-10-18T09:00:00Z",
            "",
            "Start: a subject\twith a tab",
        ]));

        let commits = parse_log(&output).unwrap();

        assert_eq!(commits.len(), 2);
        assert!(commits[0].is_merge());
        assert_eq!(
            commits[0].refs,
            ["HEAD -> main", "origin/main", "tag: v1.0"]
        );
        assert_eq!(commits[0].day(), "2026-10-19");
        assert_eq!(
            commits[1],
            Commit {
                oid: "aaaaaaaabbbbbbbbccccccccddddddddeeeeeeee".to_owned(),
                short: "aaaaaaa".to_owned(),
                parents: Vec::new(),
                author: "Joe".to_owned(),
                email: "joe@example.com".to_owned(),
                authored: "2026-10-18T09:00:00Z".to_owned(),
                committed: "2026-10-18T09:00:00Z".to_owned(),
                refs: Vec::new(),
                subject: "Start: a subject\twith a tab".to_owned(),
            }
        );
    }

    #[test]
    fn a_record_that_is_not_a_commit_is_refused_and_an_empty_log_is_empty() {
        let not_a_hash =
            parse_log(&record(&["not a hash", "x", "", "", "", "", "", "", ""])).unwrap_err();
        assert!(not_a_hash.record.starts_with("not a hash"), "{not_a_hash}");
        let too_few = parse_log(&record(&["1a2b", "1a2b"])).unwrap_err();
        assert!(too_few.record.starts_with("1a2b"), "{too_few}");
        assert_eq!(parse_log(b"").unwrap(), Vec::new());
        assert_eq!(parse_log(b"\n\0").unwrap(), Vec::new());
    }
}
