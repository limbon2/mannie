//! Windows paths: the names that Windows treats specially, link targets in
//! their verbatim form, and the canonical form of paths.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf, Prefix};

use mannie_types::tools::{PathRefusal, ToolFailure};
use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION};

use super::io_failure;

/// Refuses UNC, device and verbatim paths, paths that depend on the current
/// drive, alternate data streams, names that end in a dot or a space, and
/// reserved device names.
pub(super) fn check_text(text: &str) -> Result<(), PathRefusal> {
    let is_separator = |character: char| character == '\\' || character == '/';
    let mut characters = text.chars();
    let first = characters.next();
    let second = characters.next();
    let third = characters.next();
    let fourth = characters.next();
    let starts_with_separator = first.is_some_and(is_separator);
    if starts_with_separator && second.is_some_and(is_separator) {
        // `\\?\` and `\\.\` are verbatim and device paths; every other path
        // that starts with two separators is a UNC path.
        let is_device = matches!(third, Some('?' | '.')) && fourth.is_none_or(is_separator);
        return Err(if is_device {
            PathRefusal::DevicePath
        } else {
            PathRefusal::NetworkPath
        });
    }
    if starts_with_separator {
        // `\??\` hands a path to the object manager unchanged. Any other path
        // that starts with one separator depends on the current drive.
        let is_object_manager =
            second == Some('?') && third == Some('?') && fourth.is_some_and(is_separator);
        return Err(if is_object_manager {
            PathRefusal::DevicePath
        } else {
            PathRefusal::DriveRelative
        });
    }
    let has_drive =
        first.is_some_and(|character| character.is_ascii_alphabetic()) && second == Some(':');
    let names = if has_drive {
        if !third.is_some_and(is_separator) {
            return Err(PathRefusal::DriveRelative);
        }
        // The drive letter and the colon are one byte each.
        &text[2..]
    } else {
        text
    };
    names.split(is_separator).try_for_each(check_name)
}

/// Checks one name of a path.
fn check_name(name: &str) -> Result<(), PathRefusal> {
    if name.is_empty() || name == "." || name == ".." {
        return Ok(());
    }
    if name.contains(':') {
        return Err(PathRefusal::AlternateStream);
    }
    // Windows removes trailing dots and spaces, so such a name would reach a
    // different file than the one the policy saw.
    if name.ends_with(['.', ' ']) {
        return Err(PathRefusal::TrailingDotOrSpace);
    }
    if is_reserved(name) {
        return Err(PathRefusal::ReservedName);
    }
    Ok(())
}

/// Whether a name is a device name, which Windows reserves with or without an
/// extension, such as `CON` or `com1.txt`.
fn is_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let mut characters = upper.chars();
    let device: String = characters.by_ref().take(3).collect();
    let number = characters.next();
    // The superscript digits one to three count as digits here.
    let is_port_number = number.is_some_and(|character| {
        character.is_ascii_digit() || matches!(character, '\u{b9}' | '\u{b2}' | '\u{b3}')
    });
    (device == "COM" || device == "LPT") && is_port_number && characters.next().is_none()
}

/// Checks a link's target as Windows stores it. The target of a junction, and
/// of most symbolic links, is a verbatim path such as `\\?\C:\folder`, which
/// becomes an ordinary path; verbatim UNC and device targets are refused.
pub(super) fn link_target(target: &Path) -> Result<PathBuf, PathRefusal> {
    let checked = match without_verbatim(target) {
        Some(plain) => plain?,
        None => target.to_path_buf(),
    };
    // A target that is not valid Unicode is checked in its lossy form: the
    // checks look only at separators, colons, dots, spaces and ASCII names.
    super::check_text(&checked.to_string_lossy())?;
    Ok(checked)
}

/// The ordinary form of a verbatim path, or `None` when the path is not
/// verbatim.
fn without_verbatim(path: &Path) -> Option<Result<PathBuf, PathRefusal>> {
    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return None;
    };
    let letter = match prefix.kind() {
        Prefix::VerbatimDisk(letter) => letter,
        Prefix::VerbatimUNC(..) | Prefix::UNC(..) => return Some(Err(PathRefusal::NetworkPath)),
        Prefix::Verbatim(_) | Prefix::DeviceNS(_) => return Some(Err(PathRefusal::DevicePath)),
        Prefix::Disk(_) => return None,
    };
    let mut plain = PathBuf::from(format!("{}:\\", char::from(letter)));
    for component in components {
        match component {
            Component::RootDir => {}
            // A verbatim path keeps `.` and `..` as names; in an ordinary path
            // they would move elsewhere.
            Component::Normal(name) if name != OsStr::new(".") && name != OsStr::new("..") => {
                plain.push(name);
            }
            _ => return Some(Err(PathRefusal::DevicePath)),
        }
    }
    Some(Ok(plain))
}

/// The canonical form of a walked path: the long names of its existing part,
/// in the case the file system stores them, without a verbatim prefix. A
/// short name such as `PROGRA~1` would otherwise hide which folder a path is
/// in. The missing names, and a final link that was kept, stay as written.
pub(super) fn canonical(
    path: &Path,
    missing: usize,
    keeps_link: bool,
) -> Result<PathBuf, ToolFailure> {
    let mut existing = path.to_path_buf();
    let mut rest: Vec<OsString> = Vec::new();
    for _ in 0..missing + usize::from(keeps_link) {
        if let Some(name) = existing.file_name() {
            rest.push(name.to_owned());
        }
        existing.pop();
    }
    let canonical = fs::canonicalize(&existing).map_err(|error| io_failure(&error, &existing))?;
    let mut result = match without_verbatim(&canonical) {
        Some(Ok(plain)) => {
            super::check_text(&plain.to_string_lossy())
                .map_err(|reason| ToolFailure::PathRefused { reason })?;
            plain
        }
        // A mapped network drive canonicalizes to its UNC path. The drive
        // letter that the owner mapped stays, so the path is not refused.
        Some(Err(PathRefusal::NetworkPath)) => existing,
        Some(Err(reason)) => return Err(ToolFailure::PathRefused { reason }),
        None => canonical,
    };
    for name in rest.into_iter().rev() {
        result.push(name);
    }
    Ok(result)
}

/// Whether two names are the same name. Windows compares names without
/// regard to case, with one-to-one case mappings like its file systems.
pub(super) fn same_name(left: &OsStr, right: &OsStr) -> bool {
    left.to_string_lossy()
        .chars()
        .map(fold_case)
        .eq(right.to_string_lossy().chars().map(fold_case))
}

/// The upper case of a character when it is a single character.
fn fold_case(character: char) -> char {
    let mut upper = character.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(single), None) => single,
        _ => character,
    }
}

/// Whether an error says that another program holds the file.
pub(super) fn is_sharing_violation(error: &io::Error) -> bool {
    error
        .raw_os_error()
        .and_then(|code| u32::try_from(code).ok())
        .is_some_and(|code| code == ERROR_SHARING_VIOLATION || code == ERROR_LOCK_VIOLATION)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use mannie_types::tools::PathRefusal;

    use super::{check_text, link_target, same_name};

    #[test]
    fn paths_that_windows_treats_specially_are_refused() {
        let refused = [
            (r"\\server\share\file.txt", PathRefusal::NetworkPath),
            ("//server/share/file.txt", PathRefusal::NetworkPath),
            (r"\/server\share", PathRefusal::NetworkPath),
            (r"\\", PathRefusal::NetworkPath),
            (r"\\?\C:\Windows", PathRefusal::DevicePath),
            (r"\\?\UNC\server\share", PathRefusal::DevicePath),
            (r"\\.\PhysicalDrive0", PathRefusal::DevicePath),
            ("//./pipe/name", PathRefusal::DevicePath),
            (r"\??\C:\Windows", PathRefusal::DevicePath),
            (r"\??\UNC\server\share", PathRefusal::DevicePath),
            (r"C:\work\notes.txt:hidden", PathRefusal::AlternateStream),
            (r"C:\work\notes.txt::$DATA", PathRefusal::AlternateStream),
            ("notes.txt:hidden", PathRefusal::AlternateStream),
            (r"C:\work\CON", PathRefusal::ReservedName),
            (r"C:\work\con.txt", PathRefusal::ReservedName),
            (r"C:\work\Com1.tar.gz", PathRefusal::ReservedName),
            (r"C:\work\LPT9", PathRefusal::ReservedName),
            ("C:\\work\\lpt\u{b9}", PathRefusal::ReservedName),
            (r"C:\work\nul .txt", PathRefusal::ReservedName),
            (r"C:\work\CONOUT$", PathRefusal::ReservedName),
            (r"work\aux\file", PathRefusal::ReservedName),
            (r"C:\work\notes.", PathRefusal::TrailingDotOrSpace),
            (r"C:\work\notes ", PathRefusal::TrailingDotOrSpace),
            (r"C:\work\...\file", PathRefusal::TrailingDotOrSpace),
            ("C:file.txt", PathRefusal::DriveRelative),
            ("C:", PathRefusal::DriveRelative),
            ("a:b", PathRefusal::DriveRelative),
            (r"\work\file.txt", PathRefusal::DriveRelative),
            ("/work/file.txt", PathRefusal::DriveRelative),
        ];
        for (path, reason) in refused {
            assert_eq!(check_text(path), Err(reason), "{path}");
        }
    }

    #[test]
    fn ordinary_paths_pass() {
        let accepted = [
            r"C:\work\notes.txt",
            "C:/work/notes.txt",
            r"C:\work\.\src\..\notes.txt",
            r"src\main.rs",
            r"..\other",
            r"C:\work\.gitignore",
            r"C:\work\console.txt",
            r"C:\work\COM10",
            r"C:\work\COMMON",
            r"C:\work\Program Files (x86)\app",
            r"C:\",
            "C:\\работа\\заметки.txt",
        ];
        for path in accepted {
            assert_eq!(check_text(path), Ok(()), "{path}");
        }
    }

    #[test]
    fn verbatim_link_targets_become_ordinary_paths_or_are_refused() {
        assert_eq!(
            link_target(Path::new(r"\\?\C:\work\target")),
            Ok(PathBuf::from(r"C:\work\target"))
        );
        assert_eq!(
            link_target(Path::new(r"..\target")),
            Ok(PathBuf::from(r"..\target"))
        );
        assert_eq!(
            link_target(Path::new(r"\\?\UNC\server\share\folder")),
            Err(PathRefusal::NetworkPath)
        );
        assert_eq!(
            link_target(Path::new(r"\\server\share\folder")),
            Err(PathRefusal::NetworkPath)
        );
        assert_eq!(
            link_target(Path::new(
                r"\\?\Volume{0b1c4e48-6c1b-4d39-9a64-3c5a1b2c3d4e}\folder"
            )),
            Err(PathRefusal::DevicePath)
        );
        assert_eq!(
            link_target(Path::new(r"\\?\C:\work\..\Windows")),
            Err(PathRefusal::DevicePath)
        );
        assert_eq!(
            link_target(Path::new(r"\\?\C:\work\notes.")),
            Err(PathRefusal::TrailingDotOrSpace)
        );
    }

    #[test]
    fn names_compare_without_regard_to_case() {
        assert!(same_name("Work".as_ref(), "WORK".as_ref()));
        assert!(same_name("Übung".as_ref(), "übung".as_ref()));
        // A mapping to two characters is not one that file systems make.
        assert!(!same_name("straße".as_ref(), "STRASSE".as_ref()));
        assert!(!same_name("work".as_ref(), "workshop".as_ref()));
    }
}
