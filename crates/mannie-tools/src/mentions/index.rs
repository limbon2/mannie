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
//!
//! The owner types a query key by key, so the index ranks its entries once,
//! when it is built, by everything but how they match. A query then reads
//! them in that order and keeps the first matches of each tier, without
//! sorting every match of a short query.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

/// The most entries that an index holds.
pub const MAX_INDEX_ENTRIES: usize = 100_000;

/// The folder whose contents an index never lists.
const GIT_FOLDER: &str = ".git";

/// How many tiers [`tier`] sorts matches into.
const TIERS: usize = 5;

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
    /// The entries, ranked by [`Entry::rank`].
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
                index.rank();
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
        index.rank();
        index
    }

    /// Sorts the entries by [`Entry::rank`], which [`suggest`](Self::suggest)
    /// reads them in.
    fn rank(&mut self) {
        self.entries
            .sort_unstable_by(|left, right| left.rank().cmp(&right.rank()));
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
    /// most `limit` of them: by [`tier`], then by [`Entry::rank`]. Names
    /// compare without regard to case.
    pub fn suggest(&self, query: &str, limit: usize) -> IndexMatches {
        let query = normalize_query(query);
        let mut tiers: [Vec<&Entry>; TIERS] = Default::default();
        // The entries come in the order of their rank, so the first matches
        // of a tier are its best ones, and a tier keeps at most `limit`. Once
        // the tiers above a tier hold `limit` matches, none of its matches
        // can be listed, so its checks, the slowest ones, stop.
        let mut worst = TIERS - 1;
        for entry in &self.entries {
            if tiers[0].len() >= limit {
                break;
            }
            let Some(found) = tier(entry, &query, worst) else {
                continue;
            };
            if tiers[found].len() < limit {
                tiers[found].push(entry);
            }
            while worst > 0 && tiers[..worst].iter().map(Vec::len).sum::<usize>() >= limit {
                worst -= 1;
            }
        }
        IndexMatches {
            matches: tiers
                .into_iter()
                .flatten()
                .take(limit)
                .map(|entry| IndexMatch {
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

impl Entry {
    /// How an entry ranks among those that match a query equally well:
    /// entries that are not ignored first, then fewer names deep, then
    /// shorter, then by path.
    fn rank(&self) -> (bool, usize, usize, &str) {
        (self.ignored, self.depth, self.path.len(), &self.path)
    }
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

/// How an entry matches the query, from 0 for the best, or `None` when it
/// matches no tier up to `worst`:
///
/// 0. the name or the path is the query;
/// 1. the name or the path starts with it;
/// 2. the name contains it;
/// 3. the path contains it;
/// 4. the path holds the query's characters in order.
///
/// An empty query matches every entry equally, so the shallowest come first.
/// A query that ends in `/` names a folder and matches what is inside it.
fn tier(entry: &Entry, query: &str, worst: usize) -> Option<usize> {
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
    (0..=worst).find(|tier| match tier {
        0 => bare_name == bare_query || bare_path == bare_query,
        1 => lower.starts_with(query) || name.starts_with(query),
        2 => name.contains(query),
        3 => lower.contains(query),
        _ => holds_in_order(lower, query),
    })
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

    use super::{IndexMatch, MentionIndex, TIERS, normalize_query, tier};

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

    /// The matches of every entry, sorted as a whole: the order that
    /// [`MentionIndex::suggest`] keeps without sorting.
    fn sorted_matches(index: &MentionIndex, query: &str, limit: usize) -> Vec<String> {
        let query = normalize_query(query);
        let mut ranked: Vec<_> = index
            .entries
            .iter()
            .filter_map(|entry| tier(entry, &query, TIERS - 1).map(|found| (found, entry.rank())))
            .collect();
        ranked.sort_unstable();
        ranked
            .into_iter()
            .take(limit)
            .map(|(_tier, (_ignored, _depth, _length, path))| path.to_owned())
            .collect()
    }

    #[test]
    fn a_query_lists_the_best_matches_of_every_tier_in_the_order_of_a_full_sort() {
        let names = ["src", "docs", "adr", "readme", "target", "Read Me"];
        let files = [
            "main.rs",
            "README.md",
            "readme-old.md",
            "lib.rs",
            "notes on reading.md",
        ];
        let mut index = MentionIndex::default();
        for first in names {
            let ignored = first == "target";
            index.push(first.to_owned(), true, ignored);
            for second in names {
                index.push(format!("{first}/{second}"), true, ignored);
                for file in files {
                    index.push(format!("{first}/{second}/{file}"), false, ignored);
                }
            }
            for file in files {
                index.push(format!("{first}/{file}"), false, ignored);
            }
        }
        index.rank();
        let queries = [
            "",
            "r",
            "re",
            "read",
            "readme.md",
            "README",
            "src/",
            "./docs/adr/",
            "main",
            "mn",
            "s/m",
            "read me",
            "zz",
        ];

        for query in queries {
            for limit in [0, 1, 3, 50, 10_000] {
                let listed: Vec<String> = index
                    .suggest(query, limit)
                    .matches
                    .into_iter()
                    .map(|found| found.path)
                    .collect();
                assert_eq!(
                    listed,
                    sorted_matches(&index, query, limit),
                    "{query:?}, {limit}"
                );
            }
        }
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
