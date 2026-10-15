//! Walking a folder the way `.gitignore` files say, for `find_files` and
//! `search_text`, and the glob patterns that filter the walk.

use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobMatcher};
use ignore::WalkBuilder;
use mannie_types::tools::ToolFailure;

/// A glob pattern as the tools match it: against the path below the folder,
/// or against the file name when the pattern has no slash, as `.gitignore`
/// patterns do.
#[derive(Debug)]
pub(crate) struct Pattern {
    matcher: GlobMatcher,
    names_only: bool,
}

impl Pattern {
    /// Compiles a pattern. Windows compares names without regard to case, and
    /// a backslash there separates names like a slash.
    pub(crate) fn new(pattern: &str) -> Result<Self, ToolFailure> {
        let pattern = if cfg!(windows) {
            pattern.replace('\\', "/")
        } else {
            pattern.to_owned()
        };
        let matcher = GlobBuilder::new(&pattern)
            .literal_separator(true)
            .case_insensitive(cfg!(windows))
            .build()
            .map_err(|error| ToolFailure::InvalidPattern {
                reason: error.kind().to_string(),
            })?
            .compile_matcher();
        Ok(Self {
            matcher,
            names_only: !pattern.contains('/'),
        })
    }

    /// Whether a path below the folder matches.
    pub(crate) fn matches(&self, relative: &Path) -> bool {
        if self.names_only {
            relative
                .file_name()
                .is_some_and(|name| self.matcher.is_match(name))
        } else {
            self.matcher.is_match(relative)
        }
    }
}

/// The files below a folder, as paths below it, sorted by name at each level.
/// Links are listed but never followed, so a walk stays inside the folder.
/// Hidden files and files that `.gitignore` excludes are skipped unless
/// `include_ignored` is set; `.git` folders are always skipped.
pub(crate) fn files(root: &Path, include_ignored: bool) -> impl Iterator<Item = PathBuf> {
    let filtered = !include_ignored;
    let root_path = root.to_path_buf();
    WalkBuilder::new(root)
        .hidden(filtered)
        .parents(filtered)
        .ignore(filtered)
        .git_ignore(filtered)
        // The owner's global ignore file lies outside the folder that the
        // policy authorized.
        .git_global(false)
        .git_exclude(filtered)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .sort_by_file_name(Ord::cmp)
        .build()
        // A folder that cannot be read is left out.
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| !kind.is_dir()))
        .filter_map(move |entry| {
            entry
                .path()
                .strip_prefix(&root_path)
                .ok()
                .map(Path::to_path_buf)
        })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use mannie_types::tools::ToolFailure;

    use super::{Pattern, files};

    #[test]
    fn patterns_without_a_slash_match_names_at_any_depth() {
        let pattern = Pattern::new("*.rs").unwrap();

        assert!(pattern.matches(Path::new("main.rs")));
        assert!(pattern.matches(&Path::new("src").join("main.rs")));
        assert!(!pattern.matches(Path::new("main.rs.bak")));
    }

    #[test]
    fn patterns_with_a_slash_match_the_path() {
        let pattern = Pattern::new("src/*.rs").unwrap();
        let deep = Pattern::new("**/tests/*.rs").unwrap();

        assert!(pattern.matches(&Path::new("src").join("main.rs")));
        assert!(!pattern.matches(&Path::new("src").join("bin").join("main.rs")));
        assert!(deep.matches(&Path::new("crates").join("tools").join("tests").join("a.rs")));
    }

    #[test]
    fn invalid_patterns_are_reported() {
        let failure = Pattern::new("src/[").unwrap_err();

        assert!(
            matches!(failure, ToolFailure::InvalidPattern { .. }),
            "{failure:?}"
        );
    }

    #[test]
    fn walks_skip_ignored_and_hidden_files_unless_asked() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        fs::write(root.join(".gitignore"), "target/\n*.log\n").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("src").join("main.rs"), "").unwrap();
        fs::write(root.join("target").join("out.bin"), "").unwrap();
        fs::write(root.join("debug.log"), "").unwrap();
        fs::write(root.join(".env"), "").unwrap();
        fs::write(root.join(".git").join("HEAD"), "").unwrap();

        let filtered: Vec<PathBuf> = files(root, false).collect();
        let everything: Vec<PathBuf> = files(root, true).collect();

        assert_eq!(filtered, [Path::new("src").join("main.rs")]);
        assert_eq!(
            everything,
            [
                PathBuf::from(".env"),
                PathBuf::from(".gitignore"),
                PathBuf::from("debug.log"),
                Path::new("src").join("main.rs"),
                Path::new("target").join("out.bin"),
            ]
        );
    }
}
