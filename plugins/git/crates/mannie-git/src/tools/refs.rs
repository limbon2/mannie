//! `git_refs` (plugins/git/docs/tools.md, tools that read): the branches
//! with their upstreams, the remote branches, the tags, the remotes with
//! their URLs without credentials (T-47), the stash entries and the
//! worktrees.

use std::fmt::Write as _;

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::describe::label_commits;
use crate::failure::GitFailure;
use crate::layout::Layout;
use crate::parse::log::{REFLOG_FORMAT, ReflogEntry, parse_reflog};
use crate::parse::refs::{REF_FORMAT, Ref, RefKind, Tracking, parse_refs};
use crate::parse::remotes::{Remote, parse_remotes};
use crate::parse::worktrees::{Worktree, parse_worktrees};
use crate::texts;
use crate::tools::{Answer, ReadTool, Reading, comparable, fitting_data};

/// The tool's name.
pub(crate) const GIT_REFS: &str = "git_refs";

/// What `git_refs` lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RefsKind {
    Branches,
    RemoteBranches,
    Tags,
    Remotes,
    Stashes,
    Worktrees,
}

/// The arguments of `git_refs`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RefsArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// What to list. All of them when null.
    kinds: Option<Vec<RefsKind>>,
}

impl RefsArguments {
    fn wants(&self, kind: RefsKind) -> bool {
        self.kinds
            .as_ref()
            .is_none_or(|kinds| kinds.contains(&kind))
    }
}

/// `git_refs`.
#[derive(Debug)]
pub(crate) struct GitRefs;

impl ReadTool for GitRefs {
    type Arguments = RefsArguments;

    const NAME: &'static str = GIT_REFS;
    const DESCRIPTION: &'static str = texts::REFS_DESCRIPTION;

    fn repository(arguments: &RefsArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn check(_arguments: &RefsArguments, _layout: &Layout) -> Result<(), GitFailure> {
        Ok(())
    }

    fn answer(
        reading: Reading,
        arguments: RefsArguments,
    ) -> BoxFuture<'static, Result<Answer, GitFailure>> {
        Box::pin(async move { refs(&reading, &arguments).await })
    }
}

/// A worktree with its commit's short hash, and whether it is the
/// repository that the call read.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ShownWorktree<'a> {
    #[serde(flatten)]
    worktree: &'a Worktree,
    short: Option<String>,
    current: bool,
}

/// What the call found.
#[derive(Debug, Default)]
struct Found<'a> {
    branches: Vec<&'a Ref>,
    remote_branches: Vec<&'a Ref>,
    tags: Vec<&'a Ref>,
    remotes: Vec<Remote>,
    stashes: Vec<ReflogEntry>,
    worktrees: Vec<ShownWorktree<'a>>,
}

/// The card's data: the lists that the call asked for, each cut to the same
/// length when the whole does not fit.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RefsData<'a> {
    branches: &'a [&'a Ref],
    remote_branches: &'a [&'a Ref],
    tags: &'a [&'a Ref],
    remotes: &'a [Remote],
    stashes: &'a [ReflogEntry],
    worktrees: &'a [ShownWorktree<'a>],
}

async fn refs(reading: &Reading, arguments: &RefsArguments) -> Result<Answer, GitFailure> {
    let namespaces: Vec<&str> = [
        (RefsKind::Branches, "refs/heads"),
        (RefsKind::RemoteBranches, "refs/remotes"),
        (RefsKind::Tags, "refs/tags"),
    ]
    .into_iter()
    .filter(|(kind, _namespace)| arguments.wants(*kind))
    .map(|(_kind, namespace)| namespace)
    .collect();
    let all_refs = if namespaces.is_empty() {
        Vec::new()
    } else {
        let mut command = vec!["for-each-ref", REF_FORMAT];
        command.extend(namespaces);
        let ran = reading.git.output(reading.run(command)).await?;
        parse_refs(&ran.text())
    };
    let of_kind = |kind: RefKind| -> Vec<&Ref> {
        all_refs.iter().filter(|found| found.kind == kind).collect()
    };
    let mut found = Found {
        branches: of_kind(RefKind::Branch),
        remote_branches: of_kind(RefKind::RemoteBranch),
        tags: of_kind(RefKind::Tag),
        ..Found::default()
    };
    if arguments.wants(RefsKind::Remotes) {
        let ran = reading.git.output(reading.run(["remote", "-v"])).await?;
        found.remotes = parse_remotes(&ran.text());
    }
    if arguments.wants(RefsKind::Stashes) {
        let ran = reading
            .git
            .output(reading.run(["stash", "list", "-z", REFLOG_FORMAT]))
            .await?;
        found.stashes =
            parse_reflog(&ran.output).map_err(|error| GitFailure::failed(&error.to_string()))?;
    }
    let worktrees = if arguments.wants(RefsKind::Worktrees) {
        let ran = reading
            .git
            .output(reading.run(["worktree", "list", "--porcelain", "-z"]))
            .await?;
        parse_worktrees(&ran.output)
    } else {
        Vec::new()
    };
    let heads: Vec<String> = worktrees
        .iter()
        .filter_map(|worktree| worktree.head.clone())
        .collect();
    let labels = label_commits(reading, &heads).await?;
    let top = comparable(&reading.layout.top);
    found.worktrees = worktrees
        .iter()
        .map(|worktree| ShownWorktree {
            worktree,
            short: worktree.head.as_ref().and_then(|head| {
                labels
                    .iter()
                    .find(|label| &label.oid == head)
                    .map(|label| label.short.clone())
            }),
            current: comparable(std::path::Path::new(&worktree.path)) == top,
        })
        .collect();
    let text = refs_text(&found, arguments);
    let longest = [
        found.branches.len(),
        found.remote_branches.len(),
        found.tags.len(),
        found.remotes.len(),
        found.stashes.len(),
        found.worktrees.len(),
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    let data = fitting_data(longest, |kept| RefsData {
        branches: first(&found.branches, kept),
        remote_branches: first(&found.remote_branches, kept),
        tags: first(&found.tags, kept),
        remotes: first(&found.remotes, kept),
        stashes: first(&found.stashes, kept),
        worktrees: first(&found.worktrees, kept),
    });
    Ok(Answer { text, data })
}

/// The first `kept` items of a list.
fn first<T>(items: &[T], kept: usize) -> &[T] {
    &items[..kept.min(items.len())]
}

/// What the call found, as the model reads it.
fn refs_text(found: &Found<'_>, arguments: &RefsArguments) -> String {
    let mut text = String::new();
    if arguments.wants(RefsKind::Branches) {
        heading(&mut text, "Branches", found.branches.len());
        for branch in &found.branches {
            let mark = if branch.current { "*" } else { " " };
            let _written = writeln!(
                text,
                "{mark} {} {}{}{}: {}",
                branch.name,
                branch.short,
                tracking_text(branch),
                worktree_text(branch),
                branch.subject
            );
        }
    }
    if arguments.wants(RefsKind::RemoteBranches) {
        heading(&mut text, "Remote branches", found.remote_branches.len());
        for branch in &found.remote_branches {
            let _written = writeln!(
                text,
                "  {} {}: {}",
                branch.name, branch.short, branch.subject
            );
        }
    }
    if arguments.wants(RefsKind::Tags) {
        heading(&mut text, "Tags", found.tags.len());
        for tag in &found.tags {
            let _written = writeln!(text, "  {} {}: {}", tag.name, tag.short, tag.subject);
        }
    }
    if arguments.wants(RefsKind::Remotes) {
        heading(&mut text, "Remotes", found.remotes.len());
        for remote in &found.remotes {
            let _written = writeln!(text, "  {}", remote_text(remote));
        }
    }
    if arguments.wants(RefsKind::Stashes) {
        heading(&mut text, "Stash entries", found.stashes.len());
        for entry in &found.stashes {
            let _written = writeln!(
                text,
                "  {} {}: {}",
                entry.selector, entry.short, entry.action
            );
        }
    }
    if arguments.wants(RefsKind::Worktrees) {
        heading(&mut text, "Worktrees", found.worktrees.len());
        for shown in &found.worktrees {
            let _written = writeln!(text, "  {}", worktree_line(shown));
        }
    }
    text
}

fn heading(text: &mut String, name: &str, count: usize) {
    if count == 0 {
        let _written = writeln!(text, "{name}: none.");
    } else {
        let _written = writeln!(text, "{name} ({count}):");
    }
}

/// How a branch stands against its upstream, such as `, tracking
/// origin/main: 2 ahead, 1 behind`.
fn tracking_text(branch: &Ref) -> String {
    let Some(upstream) = &branch.upstream else {
        return String::new();
    };
    match branch.tracking {
        Some(Tracking::Exists {
            ahead: 0,
            behind: 0,
        }) => format!(", up to date with {upstream}"),
        Some(Tracking::Exists { ahead, behind }) => {
            format!(", tracking {upstream}: {ahead} ahead, {behind} behind")
        }
        Some(Tracking::Gone) | None => format!(", tracking {upstream}, which no longer exists"),
    }
}

/// The worktree that has a branch checked out, other than the current one.
fn worktree_text(branch: &Ref) -> String {
    match (&branch.worktree, branch.current) {
        (Some(worktree), false) => format!(", checked out in {worktree}"),
        _other => String::new(),
    }
}

/// A remote with its URLs, once when both are the same.
fn remote_text(remote: &Remote) -> String {
    match (&remote.fetch, &remote.push) {
        (Some(fetch), Some(push)) if fetch == push => format!("{} {fetch}", remote.name),
        (Some(fetch), Some(push)) => {
            format!("{} fetches from {fetch}, pushes to {push}", remote.name)
        }
        (Some(url), None) | (None, Some(url)) => format!("{} {url}", remote.name),
        (None, None) => remote.name.clone(),
    }
}

/// A worktree with its branch or commit and its state.
fn worktree_line(shown: &ShownWorktree<'_>) -> String {
    let worktree = shown.worktree;
    let mut line = worktree.path.clone();
    if worktree.bare {
        line.push_str(", the bare repository");
    } else if let Some(branch) = &worktree.branch {
        let _written = write!(line, " on {branch}");
    } else if let Some(short) = &shown.short {
        let _written = write!(line, ", detached at {short}");
    }
    if shown.current {
        line.push_str(" (the one read)");
    }
    if worktree.locked {
        line.push_str(" (locked)");
    }
    if worktree.prunable {
        line.push_str(" (its folder is gone)");
    }
    line
}

#[cfg(test)]
mod tests {
    use mannie_git_testing::TestRepository;
    use serde_json::json;

    use super::{GitRefs, RefsArguments, RefsKind, remote_text, tracking_text};
    use crate::parse::refs::{Ref, RefKind, Tracking};
    use crate::parse::remotes::Remote;
    use crate::tools::testing::call;

    #[tokio::test]
    async fn refs_list_branches_tags_remotes_without_credentials_stashes_and_worktrees() {
        let repository = TestRepository::new();
        repository.write("a.txt", "one\n");
        repository.commit("Start");
        repository.git(&["tag", "-a", "v1.0", "-m", "Release"]);
        repository.git(&["branch", "feature"]);
        repository.git(&[
            "remote",
            "add",
            "origin",
            "https://me:token@example.com/app.git",
        ]);
        repository.write("a.txt", "two\n");
        repository.git(&["stash", "push", "--quiet", "-m", "saved work"]);
        let linked = repository.sibling("linked");
        repository.git(&[
            "worktree",
            "add",
            "--quiet",
            linked.to_str().unwrap(),
            "feature",
        ]);

        let all = call::<GitRefs>(&repository, json!({})).await;
        let tags = call::<GitRefs>(&repository, json!({ "kinds": ["tags"] })).await;

        let output = &all.output;
        assert_eq!(all.failure, None, "{output}");
        assert!(output.contains("Branches (2):\n  feature "), "{output}");
        assert!(output.contains(", checked out in "), "{output}");
        assert!(output.contains("* main "), "{output}");
        assert!(output.contains("Remote branches: none.\n"), "{output}");
        assert!(output.contains("Tags (1):\n  v1.0 "), "{output}");
        assert!(
            output.contains("Remotes (1):\n  origin https://example.com/app.git\n"),
            "{output}"
        );
        assert!(
            output.contains("Stash entries (1):\n  stash@{0} "),
            "{output}"
        );
        assert!(output.contains("saved work"), "{output}");
        assert!(output.contains("Worktrees (2):\n"), "{output}");
        assert!(output.contains(" on main (the one read)\n"), "{output}");
        assert!(!output.contains("token"));
        let data = all.data.expect("the card's data");
        assert!(!data.to_string().contains("token"));
        assert_eq!(data["tags"][0]["name"], "v1.0");
        assert_eq!(tags.output.lines().next(), Some("Tags (1):"));
    }

    fn branch(tracking: Option<Tracking>) -> Ref {
        Ref {
            kind: RefKind::Branch,
            name: "main".to_owned(),
            commit: "1".repeat(40),
            short: "1111111".to_owned(),
            upstream: Some("origin/main".to_owned()),
            tracking,
            current: true,
            worktree: None,
            date: String::new(),
            subject: "Fix".to_owned(),
        }
    }

    #[test]
    fn a_branch_says_how_it_stands_against_its_upstream() {
        assert_eq!(
            tracking_text(&branch(Some(Tracking::Exists {
                ahead: 0,
                behind: 0
            }))),
            ", up to date with origin/main"
        );
        assert_eq!(
            tracking_text(&branch(Some(Tracking::Exists {
                ahead: 2,
                behind: 1
            }))),
            ", tracking origin/main: 2 ahead, 1 behind"
        );
        assert_eq!(
            tracking_text(&branch(Some(Tracking::Gone))),
            ", tracking origin/main, which no longer exists"
        );
    }

    #[test]
    fn a_remote_names_its_url_once_when_both_directions_share_it() {
        let same = Remote {
            name: "origin".to_owned(),
            fetch: Some("https://example.com/a.git".to_owned()),
            push: Some("https://example.com/a.git".to_owned()),
        };
        let split = Remote {
            push: Some("ssh://git@example.com/a.git".to_owned()),
            ..same.clone()
        };

        assert_eq!(remote_text(&same), "origin https://example.com/a.git");
        assert_eq!(
            remote_text(&split),
            "origin fetches from https://example.com/a.git, pushes to ssh://git@example.com/a.git"
        );
    }

    #[test]
    fn kinds_default_to_all() {
        let all: RefsArguments = serde_json::from_str("{}").unwrap();
        let tags: RefsArguments = serde_json::from_str(r#"{"kinds": ["tags"]}"#).unwrap();

        assert!(all.wants(RefsKind::Worktrees));
        assert!(tags.wants(RefsKind::Tags));
        assert!(!tags.wants(RefsKind::Branches));
    }
}
