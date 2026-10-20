//! Branches, remote branches and tags as `git for-each-ref` writes them with
//! [`REF_FORMAT`]: one line for each ref, whose fields are separated by the
//! unit separator, which no ref name or subject holds.

use serde::Serialize;

/// The format of [`parse_refs`]: the full name, the commit or tag object,
/// the commit that a tag points at, the short forms of both, the upstream
/// with how far ahead and behind, whether HEAD is on it, the worktree that
/// has it checked out, the date of its newest commit and that commit's
/// subject.
pub(crate) const REF_FORMAT: &str = "--format=%(refname)%1f%(objectname)%1f%(*objectname)%1f\
%(objectname:short)%1f%(*objectname:short)%1f%(upstream:short)%1f\
%(upstream:track,nobracket)%1f%(HEAD)%1f%(worktreepath)%1f%(creatordate:iso-strict)%1f\
%(contents:subject)";

/// What kind of ref it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RefKind {
    Branch,
    RemoteBranch,
    Tag,
}

/// How a branch stands against its upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum Tracking {
    /// The upstream exists; the counts are the commits that only one side
    /// has.
    Exists { ahead: u32, behind: u32 },
    /// The upstream was deleted on the remote.
    Gone,
}

/// A branch, remote branch or tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Ref {
    pub(crate) kind: RefKind,
    /// Its short name, such as `main`, `origin/main` or `v1.0`.
    pub(crate) name: String,
    /// The commit that it points at, through an annotated tag too.
    pub(crate) commit: String,
    /// The commit's short hash.
    pub(crate) short: String,
    /// The upstream of a branch, such as `origin/main`.
    pub(crate) upstream: Option<String>,
    pub(crate) tracking: Option<Tracking>,
    /// Whether HEAD is on this branch.
    pub(crate) current: bool,
    /// The folder of the worktree that has the branch checked out.
    pub(crate) worktree: Option<String>,
    /// The date of the commit or the tag.
    pub(crate) date: String,
    pub(crate) subject: String,
}

/// The short name and the kind of a full ref name, or `None` for a ref of
/// another namespace, such as `refs/stash` or a remote's `HEAD`.
fn short_name(refname: &str) -> Option<(RefKind, String)> {
    if let Some(name) = refname.strip_prefix("refs/heads/") {
        return Some((RefKind::Branch, name.to_owned()));
    }
    if let Some(name) = refname.strip_prefix("refs/remotes/") {
        let is_symbolic_head = name.ends_with("/HEAD");
        return (!is_symbolic_head).then(|| (RefKind::RemoteBranch, name.to_owned()));
    }
    refname
        .strip_prefix("refs/tags/")
        .map(|name| (RefKind::Tag, name.to_owned()))
}

/// Reads `ahead 2, behind 1`, `ahead 2`, `behind 1`, an empty text for a
/// branch in step with its upstream, or `gone`.
fn parse_tracking(text: &str) -> Option<Tracking> {
    if text == "gone" {
        return Some(Tracking::Gone);
    }
    let mut ahead = 0;
    let mut behind = 0;
    for part in text.split(", ").filter(|part| !part.is_empty()) {
        let (name, count) = part.split_once(' ')?;
        let count = count.parse().ok()?;
        match name {
            "ahead" => ahead = count,
            "behind" => behind = count,
            _other => return None,
        }
    }
    Some(Tracking::Exists { ahead, behind })
}

/// Reads the output of `git for-each-ref` with [`REF_FORMAT`]. Refs of
/// other namespaces and lines that this version cannot read are skipped.
pub(crate) fn parse_refs(output: &str) -> Vec<Ref> {
    output.lines().filter_map(parse_ref).collect()
}

fn parse_ref(line: &str) -> Option<Ref> {
    let fields: Vec<&str> = line.splitn(11, '\u{1f}').collect();
    let [
        refname,
        object,
        peeled,
        short_object,
        short_peeled,
        upstream,
        track,
        head,
        worktree,
        date,
        subject,
    ] = fields.as_slice()
    else {
        return None;
    };
    let (kind, name) = short_name(refname)?;
    let (commit, short) = if peeled.is_empty() {
        (*object, *short_object)
    } else {
        (*peeled, *short_peeled)
    };
    let upstream = (!upstream.is_empty()).then(|| (*upstream).to_owned());
    let tracking = upstream
        .as_ref()
        .and_then(|_upstream| parse_tracking(track));
    Some(Ref {
        kind,
        name,
        commit: commit.to_owned(),
        short: short.to_owned(),
        upstream,
        tracking,
        current: *head == "*",
        worktree: (!worktree.is_empty()).then(|| (*worktree).to_owned()),
        date: (*date).to_owned(),
        subject: (*subject).to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::{Ref, RefKind, Tracking, parse_refs, parse_tracking};

    /// A line of [`REF_FORMAT`](super::REF_FORMAT)'s output, written with `|`
    /// between its fields for the tests' reading.
    fn line(fields: &str) -> String {
        fields.replace('|', "\u{1f}")
    }

    #[test]
    fn branches_remote_branches_and_tags_are_read_with_their_upstreams() {
        let output = [
            line(
                "refs/heads/main|aaaa||aa||origin/main|ahead 2, behind 1|*|D:/work/app|\
                 2026-10-19T14:02:11+02:00|Fix the parser",
            ),
            line("refs/heads/old|bbbb||bb||origin/old|gone| ||2026-01-01T00:00:00Z|Old work"),
            line("refs/remotes/origin/HEAD|aaaa||aa|||| |||"),
            line("refs/remotes/origin/main|cccc||cc|||| ||2026-10-18T09:00:00Z|Remote work"),
            line("refs/tags/v1.0|tagobject|dddd|ta|dd||| ||2026-10-01T00:00:00Z|Release 1.0"),
            line("refs/stash|eeee||ee|||| |||WIP"),
            "not a ref line".to_owned(),
        ]
        .join("\n");

        let refs = parse_refs(&output);

        assert_eq!(refs.len(), 4);
        assert_eq!(
            refs[0],
            Ref {
                kind: RefKind::Branch,
                name: "main".to_owned(),
                commit: "aaaa".to_owned(),
                short: "aa".to_owned(),
                upstream: Some("origin/main".to_owned()),
                tracking: Some(Tracking::Exists {
                    ahead: 2,
                    behind: 1
                }),
                current: true,
                worktree: Some("D:/work/app".to_owned()),
                date: "2026-10-19T14:02:11+02:00".to_owned(),
                subject: "Fix the parser".to_owned(),
            }
        );
        assert_eq!(refs[1].tracking, Some(Tracking::Gone));
        assert!(!refs[1].current);
        assert_eq!(refs[2].kind, RefKind::RemoteBranch);
        assert_eq!(refs[2].name, "origin/main");
        assert_eq!(refs[3].kind, RefKind::Tag);
        assert_eq!(refs[3].commit, "dddd", "an annotated tag names its commit");
        assert_eq!(refs[3].short, "dd");
    }

    #[test]
    fn tracking_reads_each_form_that_git_writes() {
        assert_eq!(
            parse_tracking(""),
            Some(Tracking::Exists {
                ahead: 0,
                behind: 0
            })
        );
        assert_eq!(
            parse_tracking("behind 4"),
            Some(Tracking::Exists {
                ahead: 0,
                behind: 4
            })
        );
        assert_eq!(parse_tracking("sideways 1"), None);
    }
}
