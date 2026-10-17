//! The index of a workspace that the "@" list searches (docs/features/chat/
//! mentions.md, files and folders).
//!
//! The list shows every entry, also the ones that `.gitignore` excludes and
//! hidden ones, because hiding them confused models (owner, 2026-10-16). An
//! index holds at most [`MAX_INDEX_ENTRIES`], so it is built in two passes:
//! the first walks what `.gitignore` does not exclude, and the second adds
//! the rest breadth first, so a large ignored folder such as `node_modules`
//! never crowds out the project's own files. `.git` is listed, but nothing
//! inside it, and links are listed without being followed.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

/// The most entries that an index holds.
pub const MAX_INDEX_ENTRIES: usize = 100_000;

/// The folder whose contents an index never lists.
const GIT_FOLDER: &str = ".git";

/// An entry of the index.
#[derive(Debug, Clone)]
struct Entry {
    /// The path below the root, with `/` between names.
    path: String,
    /// The path in lower case, which queries match, with a `/` at the end of
    /// a folder's path.
    lower: String,
    /// Where the entry's name starts in `lower`.
    name_start: usize,
    /// Whether the entry is a folder.
    folder: bool,
    /// Whether `.gitignore` excludes the entry, or it lies in `.git`.
    ignored: bool,
    /// How many names the path has.
    depth: usize,
}

/// The files and folders below a workspace, as the "@" list searches them.
#[derive(Debug, Clone, Default)]
pub struct MentionIndex {
    entries: Vec<Entry>,
    incomplete: bool,
}

/// An entry that matches a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexMatch {
    /// The path below the workspace, with `/` between names.
    pub path: String,
    /// Whether it is a folder.
    pub folder: bool,
}

/// The entries that match a query, best first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexMatches {
    /// The matches.
    pub matches: Vec<IndexMatch>,
    /// Whether the workspace holds more entries than the index, so that a
    /// longer query may find files that this one does not.
    pub incomplete: bool,
}

/// An entry of a folder, before it joins the index.
struct Child {
    path: PathBuf,
    name: String,
    folder: bool,
}

/// An ignored entry that waits for the second pass, shallowest first.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Pending {
    depth: usize,
    relative: String,
    path: PathBuf,
    folder: bool,
}

impl MentionIndex {
    /// Indexes the files and folders below `root`, at most
    /// [`MAX_INDEX_ENTRIES`] of them.
    pub fn build(root: &Path) -> Self {
        Self::build_within(root, MAX_INDEX_ENTRIES)
    }

    /// Indexes at most `cap` entries below `root`.
    pub(crate) fn build_within(root: &Path, cap: usize) -> Self {
        let mut index = Self::default();
        let mut seen: HashSet<String> = HashSet::new();
        let mut folders: Vec<(PathBuf, String)> = vec![(root.to_path_buf(), String::new())];
        let walk = WalkBuilder::new(root)
            .hidden(false)
            .parents(true)
            .ignore(true)
            .git_ignore(true)
            // The owner's global ignore file lies outside the workspace.
            .git_global(false)
            .git_exclude(true)
            .require_git(false)
            .follow_links(false)
            .filter_entry(|entry| entry.file_name() != GIT_FOLDER)
            .sort_by_file_name(Ord::cmp)
            .build();
        // A folder that cannot be read is left out.
        for entry in walk.filter_map(Result::ok) {
            if entry.depth() == 0 {
                continue;
            }
            let Some(relative) = relative_text(root, entry.path()) else {
                continue;
            };
            if index.entries.len() == cap {
                index.incomplete = true;
                return index;
            }
            let folder = entry.file_type().is_some_and(|kind| kind.is_dir());
            if folder {
                folders.push((entry.path().to_path_buf(), relative.clone()));
            }
            seen.insert(relative.clone());
            index.push(relative, folder, false);
        }
        // The entries that the first pass left out are the roots of what
        // `.gitignore` excludes, together with `.git`.
        let mut pending = BinaryHeap::new();
        for (folder, relative) in &folders {
            for child in children(folder) {
                let child_relative = join(relative, &child.name);
                if !seen.contains(&child_relative) {
                    pending.push(Reverse(Pending {
                        depth: depth_of(&child_relative),
                        relative: child_relative,
                        path: child.path,
                        folder: child.folder,
                    }));
                }
            }
        }
        while let Some(Reverse(next)) = pending.pop() {
            if index.entries.len() == cap {
                index.incomplete = true;
                break;
            }
            let is_git = next.relative.rsplit('/').next() == Some(GIT_FOLDER);
            let expands = next.folder && !is_git;
            if expands {
                for child in children(&next.path) {
                    let child_relative = join(&next.relative, &child.name);
                    pending.push(Reverse(Pending {
                        depth: depth_of(&child_relative),
                        relative: child_relative,
                        path: child.path,
                        folder: child.folder,
                    }));
                }
            }
            index.push(next.relative, next.folder, true);
        }
        index
    }

    fn push(&mut self, path: String, folder: bool, ignored: bool) {
        let mut lower = path.to_lowercase();
        // `/` is ASCII, so lower case keeps it where it is.
        let name_start = lower.rfind('/').map_or(0, |slash| slash + 1);
        if folder {
            lower.push('/');
        }
        let depth = depth_of(&path);
        self.entries.push(Entry {
            path,
            lower,
            name_start,
            folder,
            ignored,
            depth,
        });
    }

    /// The entries that match what the owner typed after "@", best first, at
    /// most `limit` of them. Names compare without regard to case.
    pub fn suggest(&self, query: &str, limit: usize) -> IndexMatches {
        let query = normalize_query(query);
        let mut ranked: Vec<(Rank<'_>, &Entry)> = self
            .entries
            .iter()
            .filter_map(|entry| {
                tier(entry, &query).map(|tier| {
                    let rank = Rank {
                        tier,
                        ignored: entry.ignored,
                        depth: entry.depth,
                        length: entry.path.len(),
                        path: &entry.path,
                    };
                    (rank, entry)
                })
            })
            .collect();
        ranked.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        IndexMatches {
            matches: ranked
                .into_iter()
                .take(limit)
                .map(|(_rank, entry)| IndexMatch {
                    path: entry.path.clone(),
                    folder: entry.folder,
                })
                .collect(),
            incomplete: self.incomplete,
        }
    }

    /// How many entries the index holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index holds no entry.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// How well an entry matches, best first: the tier, then entries that are not
/// ignored, then fewer names deep, then shorter, then by path.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Rank<'a> {
    tier: u8,
    ignored: bool,
    depth: usize,
    length: usize,
    path: &'a str,
}

/// The query in lower case, with `/` between names and without a leading
/// `./`. Windows separates names with a backslash too.
fn normalize_query(query: &str) -> String {
    let query = query.to_lowercase();
    let query = if cfg!(windows) {
        query.replace('\\', "/")
    } else {
        query
    };
    query.strip_prefix("./").map(str::to_owned).unwrap_or(query)
}

/// How an entry matches the query, from 0 for the best, or `None`:
///
/// 0. the name or the path is the query;
/// 1. the name or the path starts with it;
/// 2. the name contains it;
/// 3. the path contains it;
/// 4. the path holds the query's characters in order.
///
/// An empty query matches every entry equally, so the shallowest come first.
/// A query that ends in `/` names a folder and matches what is inside it.
fn tier(entry: &Entry, query: &str) -> Option<u8> {
    if query.is_empty() {
        return Some(0);
    }
    let lower = entry.lower.as_str();
    let goes_inside = query.ends_with('/');
    if goes_inside && lower == query {
        return None;
    }
    let name = &lower[entry.name_start..];
    let bare_query = query.trim_end_matches('/');
    let bare_name = name.trim_end_matches('/');
    let bare_path = lower.trim_end_matches('/');
    if bare_name == bare_query || bare_path == bare_query {
        Some(0)
    } else if lower.starts_with(query) || name.starts_with(query) {
        Some(1)
    } else if name.contains(query) {
        Some(2)
    } else if lower.contains(query) {
        Some(3)
    } else if holds_in_order(lower, query) {
        Some(4)
    } else {
        None
    }
}

/// Whether `text` holds the characters of `query` in their order.
fn holds_in_order(text: &str, query: &str) -> bool {
    let mut characters = text.chars();
    query
        .chars()
        .all(|wanted| characters.any(|character| character == wanted))
}

/// The entries of a folder, sorted by name. A folder that cannot be read has
/// none, and a name that is not valid Unicode is left out, since a mention
/// names its path as text.
fn children(folder: &Path) -> Vec<Child> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut children: Vec<Child> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            // A link is listed and never followed, so it is no folder here.
            let folder = entry.file_type().is_ok_and(|kind| kind.is_dir());
            Some(Child {
                path: entry.path(),
                name,
                folder,
            })
        })
        .collect();
    children.sort_by(|left, right| left.name.cmp(&right.name));
    children
}

/// A path below `root` as text with `/` between names, or `None` for a path
/// that is not valid Unicode.
fn relative_text(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let names: Option<Vec<&str>> = relative
        .components()
        .map(|component| component.as_os_str().to_str())
        .collect();
    Some(names?.join("/"))
}

/// A name below a relative path.
fn join(relative: &str, name: &str) -> String {
    if relative.is_empty() {
        name.to_owned()
    } else {
        format!("{relative}/{name}")
    }
}

/// How many names a relative path has.
fn depth_of(relative: &str) -> usize {
    relative.split('/').count()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::{IndexMatch, MentionIndex};

    fn paths(index: &MentionIndex, query: &str) -> Vec<String> {
        index
            .suggest(query, 50)
            .matches
            .into_iter()
            .map(|found| {
                if found.folder {
                    format!("{}/", found.path)
                } else {
                    found.path
                }
            })
            .collect()
    }

    /// A project with hidden and ignored files, `.git`, and a large ignored
    /// folder.
    fn project(root: &Path) {
        fs::write(root.join(".gitignore"), "target/\n*.log\n").unwrap();
        fs::write(root.join(".env"), "TOKEN=1\n").unwrap();
        fs::write(root.join("debug.log"), "").unwrap();
        fs::create_dir_all(root.join("src").join("deep").join("deeper")).unwrap();
        fs::write(root.join("src").join("main.rs"), "").unwrap();
        fs::write(
            root.join("src").join("deep").join("deeper").join("leaf.rs"),
            "",
        )
        .unwrap();
        fs::create_dir_all(root.join(".git").join("objects")).unwrap();
        fs::write(root.join(".git").join("HEAD"), "").unwrap();
        fs::create_dir_all(root.join("target").join("debug")).unwrap();
        for number in 0..30 {
            fs::write(
                root.join("target")
                    .join("debug")
                    .join(format!("main{number}.rs")),
                "",
            )
            .unwrap();
        }
    }

    #[test]
    fn hidden_and_ignored_entries_are_listed_but_not_inside_git() {
        let folder = tempfile::tempdir().unwrap();
        project(folder.path());

        let index = MentionIndex::build(folder.path());
        let everything = paths(&index, "");

        for listed in [
            ".env",
            ".gitignore",
            "debug.log",
            "src/",
            "target/",
            ".git/",
        ] {
            assert!(
                everything.contains(&listed.to_owned()),
                "{listed}: {everything:?}"
            );
        }
        assert!(
            !paths(&index, "HEAD")
                .iter()
                .any(|path| path.starts_with(".git/"))
        );
        assert!(!index.suggest("", 50).incomplete);
    }

    #[test]
    fn project_files_rank_before_ignored_ones_and_stay_under_the_cap() {
        let folder = tempfile::tempdir().unwrap();
        project(folder.path());

        let index = MentionIndex::build(folder.path());
        let small = MentionIndex::build_within(folder.path(), 12);

        assert_eq!(paths(&index, "main")[0], "src/main.rs");
        assert!(paths(&index, "main").contains(&"target/debug/main7.rs".to_owned()));
        assert!(small.suggest("", 50).incomplete);
        assert_eq!(small.len(), 12);
        assert!(paths(&small, "leaf").contains(&"src/deep/deeper/leaf.rs".to_owned()));
        assert_eq!(paths(&small, "main7"), Vec::<String>::new());
    }

    #[test]
    fn matches_are_ranked_by_tier_then_depth_and_length() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        fs::create_dir_all(root.join("docs").join("adr")).unwrap();
        fs::write(root.join("docs").join("README.md"), "").unwrap();
        fs::write(root.join("docs").join("adr").join("readme-old.md"), "").unwrap();
        fs::write(root.join("README.md"), "").unwrap();
        fs::write(root.join("guide-reading.md"), "").unwrap();

        let index = MentionIndex::build(root);

        assert_eq!(
            paths(&index, "readme.md")[..2],
            ["README.md", "docs/README.md"],
            "exact names first, the shallowest first"
        );
        assert_eq!(
            paths(&index, "read"),
            [
                "README.md",
                "docs/README.md",
                "docs/adr/readme-old.md",
                "guide-reading.md"
            ]
        );
        assert_eq!(paths(&index, "dcsadr")[0], "docs/adr/");
        assert_eq!(paths(&index, "DOCS")[0], "docs/");
        assert_eq!(paths(&index, "zzz"), Vec::<String>::new());
    }

    #[test]
    fn a_folder_query_lists_what_is_inside_the_folder() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        fs::create_dir_all(root.join("docs").join("adr")).unwrap();
        fs::write(root.join("docs").join("README.md"), "").unwrap();
        fs::write(root.join("docs").join("adr").join("0001.md"), "").unwrap();

        let index = MentionIndex::build(root);

        assert_eq!(
            paths(&index, "docs/"),
            ["docs/adr/", "docs/README.md", "docs/adr/0001.md"]
        );
        assert_eq!(paths(&index, "./docs/adr/"), ["docs/adr/0001.md"]);
        assert_eq!(
            index.suggest("docs/adr/0001.md", 1).matches,
            [IndexMatch {
                path: "docs/adr/0001.md".to_owned(),
                folder: false
            }]
        );
    }

    #[cfg(unix)]
    #[test]
    fn links_are_listed_and_not_followed() {
        let folder = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "").unwrap();
        std::os::unix::fs::symlink(outside.path(), folder.path().join("elsewhere")).unwrap();

        let index = MentionIndex::build(folder.path());

        assert_eq!(paths(&index, ""), ["elsewhere"]);
        assert_eq!(paths(&index, "secret"), Vec::<String>::new());
    }

    #[cfg(windows)]
    #[test]
    fn backslashes_separate_names_in_a_query_on_windows() {
        let folder = tempfile::tempdir().unwrap();
        fs::create_dir_all(folder.path().join("src")).unwrap();
        fs::write(folder.path().join("src").join("main.rs"), "").unwrap();

        let index = MentionIndex::build(folder.path());

        assert_eq!(paths(&index, "src\\main.rs"), ["src/main.rs"]);
    }
}
