//! The arguments that the git tools share (plugins/git/docs/tools.md, every
//! tool; T-45): the repository's folder, revisions and paths. A revision
//! never starts with `-`, has no white space or control character, and
//! follows `--end-of-options`, so git never reads it as an option. A path is
//! relative to the repository's top and becomes a literal pathspec after
//! `--`, so no character in it is a pattern or a magic word.

use std::path::{Component, Path, PathBuf};

use crate::failure::GitFailure;

/// The most characters of a revision or a path.
const MAX_ARGUMENT_CHARS: usize = 4096;

fn invalid(detail: String) -> GitFailure {
    GitFailure::InvalidArgument { detail }
}

/// Whether a text has a control character, which no folder, revision or
/// path that a tool takes holds.
fn has_control(text: &str) -> bool {
    text.chars().any(char::is_control)
}

/// The folder that a call's `repository` names: absolute, or relative to
/// the workspace; the workspace itself without one. The core's policy
/// decides on the place that the folder's repository is.
pub(crate) fn repository_folder(
    workspace: Option<&Path>,
    repository: Option<&str>,
) -> Result<PathBuf, GitFailure> {
    let Some(text) = repository.map(str::trim).filter(|text| !text.is_empty()) else {
        return workspace.map(Path::to_path_buf).ok_or_else(|| {
            invalid(
                "This conversation has no project, so repository must name the repository's folder as an absolute path."
                    .to_owned(),
            )
        });
    };
    if has_control(text) || text.chars().count() > MAX_ARGUMENT_CHARS {
        return Err(invalid(format!(
            "repository {text:?} is not a folder's path."
        )));
    }
    let path = Path::new(text);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    match workspace {
        Some(workspace) => Ok(workspace.join(path)),
        None => Err(invalid(format!(
            "This conversation has no project, so repository must be an absolute path, not {text:?}."
        ))),
    }
}

/// Checks a revision, such as `main`, `HEAD~2`, `v1.0`, `abc1234` or
/// `main..feature`, named by `argument` in the failure.
pub(crate) fn check_revision(argument: &str, revision: &str) -> Result<(), GitFailure> {
    let refused = |why: &str| invalid(format!("{argument} {revision:?} {why}"));
    if revision.is_empty() {
        return Err(refused("is empty."));
    }
    if revision.starts_with('-') {
        return Err(refused("starts with -, which git would read as an option."));
    }
    if revision.chars().any(char::is_whitespace) || has_control(revision) {
        return Err(refused("holds white space or a control character."));
    }
    if revision.chars().count() > MAX_ARGUMENT_CHARS {
        return Err(refused("is too long."));
    }
    Ok(())
}

/// Checks a branch's short name, which the argument `argument` gave: a
/// revision without the parts that a revision reads as operators, so it
/// names one branch and nothing else.
pub(crate) fn check_branch(argument: &str, name: &str) -> Result<(), GitFailure> {
    check_revision(argument, name)?;
    let has_operator = name.contains("..")
        || name.contains("@{")
        || name
            .chars()
            .any(|character| matches!(character, ':' | '?' | '*' | '[' | '\\' | '^' | '~'));
    if has_operator {
        return Err(invalid(format!(
            "{argument} {name:?} is not a branch's name: it holds .., @{{ or one of : ? * [ \\ ^ ~."
        )));
    }
    Ok(())
}

/// A path from the repository's top, with forward slashes, from a path
/// that a call names: relative to the top, or absolute inside `top`. A path
/// that leaves the repository, or names no file, is refused.
pub(crate) fn repository_path(
    top: &Path,
    argument: &str,
    path: &str,
) -> Result<String, GitFailure> {
    let refused = |why: &str| invalid(format!("{argument} {path:?} {why}"));
    if path.trim().is_empty() {
        return Err(refused("is empty."));
    }
    if has_control(path) || path.chars().count() > MAX_ARGUMENT_CHARS {
        return Err(refused("is not a path."));
    }
    let given = Path::new(path);
    let relative = if given.is_absolute() {
        strip_top(top, given).ok_or_else(|| refused("is outside the repository."))?
    } else {
        given.to_path_buf()
    };
    let mut parts: Vec<String> = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir => return Err(refused("leaves the repository with ..")),
            Component::RootDir | Component::Prefix(_) => {
                return Err(refused("is not a path inside the repository."));
            }
        }
    }
    let joined = parts.join("/");
    if joined.is_empty() {
        return Err(refused("names the repository's top, not a file in it."));
    }
    Ok(joined)
}

/// An absolute path made relative to `top`, comparing names without regard
/// to case on Windows, or `None` for a path outside it.
fn strip_top(top: &Path, path: &Path) -> Option<PathBuf> {
    let mut path_components = path.components();
    for top_component in top.components() {
        let component = path_components.next()?;
        let top_name = top_component.as_os_str().to_string_lossy();
        let name = component.as_os_str().to_string_lossy();
        let same = if cfg!(windows) {
            top_name
                .replace('\\', "/")
                .eq_ignore_ascii_case(&name.replace('\\', "/"))
        } else {
            top_name == name
        };
        if !same {
            return None;
        }
    }
    Some(path_components.as_path().to_path_buf())
}

/// The literal pathspec of a path from the repository's top, for a run whose
/// working folder is the top.
pub(crate) fn pathspec(path: &str) -> String {
    format!(":(literal){path}")
}

/// Whether a text is an object's full hash, SHA-1 or SHA-256, such as git
/// writes in its state files. A text from a file that the repository holds
/// is passed to git only after this check, so it never reads as an option.
pub(crate) fn is_object_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Checks a text that a tool passes to git inside one argument, such as an
/// author or a date: no control character, and at most 4096 characters.
pub(crate) fn check_text(argument: &str, text: &str) -> Result<(), GitFailure> {
    if text.trim().is_empty() {
        return Err(invalid(format!("{argument} is empty.")));
    }
    if has_control(text) {
        return Err(invalid(format!("{argument} holds a control character.")));
    }
    if text.chars().count() > MAX_ARGUMENT_CHARS {
        return Err(invalid(format!("{argument} is too long.")));
    }
    Ok(())
}

/// The paths of a call, from the repository's top, as literal pathspecs.
pub(crate) fn pathspecs(
    top: &Path,
    argument: &str,
    paths: &[String],
) -> Result<Vec<String>, GitFailure> {
    paths
        .iter()
        .map(|path| repository_path(top, argument, path).map(|path| pathspec(&path)))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{
        check_revision, check_text, is_object_id, pathspec, pathspecs, repository_folder,
        repository_path,
    };
    use crate::failure::GitFailure;

    fn top() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"D:\work\app")
        } else {
            PathBuf::from("/work/app")
        }
    }

    #[test]
    fn the_repository_is_the_workspace_or_a_folder_relative_to_it_or_absolute() {
        let workspace = top();

        assert_eq!(
            repository_folder(Some(&workspace), None).unwrap(),
            workspace
        );
        assert_eq!(
            repository_folder(Some(&workspace), Some("  ")).unwrap(),
            workspace
        );
        assert_eq!(
            repository_folder(Some(&workspace), Some("crates/core")).unwrap(),
            workspace.join("crates/core")
        );
        let absolute = top().join("other");
        assert_eq!(
            repository_folder(None, Some(absolute.to_str().unwrap())).unwrap(),
            absolute
        );
        assert!(matches!(
            repository_folder(None, None),
            Err(GitFailure::InvalidArgument { .. })
        ));
        assert!(matches!(
            repository_folder(None, Some("relative")),
            Err(GitFailure::InvalidArgument { .. })
        ));
        assert!(matches!(
            repository_folder(Some(&workspace), Some("a\u{0}b")),
            Err(GitFailure::InvalidArgument { .. })
        ));
    }

    #[test]
    fn a_revision_never_reads_as_an_option() {
        for revision in [
            "main",
            "HEAD~2",
            "v1.0",
            "1a2b3c4",
            "main..feature",
            "HEAD@{1}",
            "origin/main",
        ] {
            assert_eq!(check_revision("revision", revision), Ok(()), "{revision}");
        }
        for revision in [
            "",
            "-n",
            "--output=/tmp/x",
            "--upload-pack=touch x",
            "main feature",
            "a\tb",
            "a\nb",
        ] {
            assert!(
                matches!(
                    check_revision("revision", revision),
                    Err(GitFailure::InvalidArgument { .. })
                ),
                "{revision:?}"
            );
        }
    }

    #[test]
    fn a_path_stays_inside_the_repository_and_becomes_a_literal_pathspec() {
        let top = top();
        let inside = top.join("src").join("main.rs");

        assert_eq!(
            repository_path(&top, "path", "src/main.rs").unwrap(),
            "src/main.rs"
        );
        assert_eq!(
            repository_path(&top, "path", "./src//lib.rs").unwrap(),
            "src/lib.rs"
        );
        assert_eq!(
            repository_path(&top, "path", inside.to_str().unwrap()).unwrap(),
            "src/main.rs"
        );
        assert_eq!(
            repository_path(&top, "path", ":(glob)*").unwrap(),
            ":(glob)*"
        );
        for refused in ["", "../outside", "src/../../x", ".", "a\u{1}b"] {
            assert!(
                matches!(
                    repository_path(&top, "path", refused),
                    Err(GitFailure::InvalidArgument { .. })
                ),
                "{refused:?}"
            );
        }
        let outside = if cfg!(windows) {
            r"D:\elsewhere\x"
        } else {
            "/elsewhere/x"
        };
        assert!(matches!(
            repository_path(&top, "path", outside),
            Err(GitFailure::InvalidArgument { .. })
        ));
        assert_eq!(pathspec(":(glob)*"), ":(literal):(glob)*");
        assert_eq!(pathspec("-rf"), ":(literal)-rf");
        assert!(Path::new(&top).is_absolute());
    }

    #[test]
    fn only_full_hashes_are_object_ids() {
        assert!(is_object_id("1a2b3c4d5e6f7a8b9c0d1a2b3c4d5e6f7a8b9c0d"));
        assert!(is_object_id(&"ab".repeat(32)));
        for refused in ["1a2b3c4", "--upload-pack=x", "", &"g".repeat(40)] {
            assert!(!is_object_id(refused), "{refused:?}");
        }
    }

    #[test]
    fn a_text_argument_has_no_control_character_and_paths_become_pathspecs() {
        assert_eq!(check_text("author", "Jane Doe"), Ok(()));
        assert!(matches!(
            check_text("author", " "),
            Err(GitFailure::InvalidArgument { .. })
        ));
        assert!(matches!(
            check_text("author", "a\nb"),
            Err(GitFailure::InvalidArgument { .. })
        ));
        assert!(matches!(
            check_text("author", &"x".repeat(5000)),
            Err(GitFailure::InvalidArgument { .. })
        ));
        let top = top();
        assert_eq!(
            pathspecs(&top, "paths", &["src".to_owned(), "a b.txt".to_owned()]).unwrap(),
            [":(literal)src", ":(literal)a b.txt"]
        );
        assert!(matches!(
            pathspecs(&top, "paths", &["../x".to_owned()]),
            Err(GitFailure::InvalidArgument { .. })
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_compares_the_top_without_regard_to_case() {
        let top = Path::new(r"D:\Work\App");

        assert_eq!(
            repository_path(top, "path", r"d:\work\app\SRC\Main.rs").unwrap(),
            "SRC/Main.rs"
        );
    }
}
