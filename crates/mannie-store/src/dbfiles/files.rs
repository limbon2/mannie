//! The checks of a file before the engine registers or opens it (ADR-0056,
//! decision 2), with what spikes S2 and S5 of M15 showed
//! (docs/roadmap/m15-validation.md, sections 3 and 6): SQLite reads a path
//! that starts with `file:` as a URI even without its flag, since the
//! bundled build sets `SQLITE_USE_URI`, and `SQLITE_OPEN_NOFOLLOW` opens a
//! link on Windows, so the engine refuses both itself.
//!
//! A file's identity is the volume serial number and file index on Windows,
//! the device and inode on Unix. It stays through writes in place and
//! changes when a program saves by replacing the file.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// The first 16 bytes of every SQLite database.
const HEADER: &[u8; 16] = b"SQLite format 3\0";

/// The suffixes of the files that SQLite keeps beside a database.
const COMPANIONS: &[&str] = &["-wal", "-shm", "-journal"];

/// Why the engine refuses a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FileRefusal {
    /// The path is not absolute.
    #[error("the path is not absolute")]
    NotAbsolute,
    /// The path starts with `file:`, which SQLite reads as a URI.
    #[error("the path is a URI")]
    Uri,
    /// Nothing is at the path.
    #[error("the file does not exist")]
    NotFound,
    /// The path names a folder or something else that is not a file.
    #[error("the path is not a file")]
    NotAFile,
    /// The path is a link, which the engine never follows.
    #[error("the path is a link")]
    Link,
    /// The file is on another computer, through a network path or a
    /// mapped drive.
    #[error("the file is on a network path")]
    Network,
    /// The file is one that SQLite keeps beside a database, such as its
    /// write-ahead log.
    #[error("the file belongs to another database")]
    Companion,
    /// The file is inside one of Mannie's own folders.
    #[error("the file is inside Mannie's folders")]
    MannieFolder,
    /// The file does not start with SQLite's header.
    #[error("the file is not a SQLite database")]
    NotADatabase,
    /// The file could not be read; the log has the cause.
    #[error("the file could not be read")]
    Unreadable,
    /// The file at the path is another file than the one that the owner
    /// registered, or the path now reaches it through a link.
    #[error("the file changed since it was registered")]
    Changed,
}

/// A file that passed the checks: its canonical path and its identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedFile {
    /// The canonical, absolute path, which the engine passes to SQLite.
    pub path: PathBuf,
    /// The file's identity.
    pub identity: String,
}

/// Maps an error of the file system to a refusal, and logs the cause of
/// one that is not a missing file.
fn refusal_of(error: &io::Error) -> FileRefusal {
    if error.kind() == io::ErrorKind::NotFound {
        return FileRefusal::NotFound;
    }
    tracing::warn!(%error, "a database file could not be checked");
    FileRefusal::Unreadable
}

/// Whether a canonical path reaches another computer. On Windows,
/// canonicalization turns a mapped network drive into its UNC path.
fn is_network(canonical: &Path) -> bool {
    let text = canonical.to_string_lossy();
    let extended_unc = text
        .get(..8)
        .is_some_and(|start| start.eq_ignore_ascii_case(r"\\?\UNC\"));
    let plain_unc =
        text.starts_with(r"\\") && !text.starts_with(r"\\?\") && !text.starts_with(r"\\.\");
    extended_unc || plain_unc
}

/// Whether `path` is inside `folder`, comparing their canonical forms, and
/// on Windows without regard to case.
fn is_inside(path: &Path, folder: &Path) -> bool {
    let Ok(folder) = fs::canonicalize(folder) else {
        return false;
    };
    if cfg!(windows) {
        let path = path.to_string_lossy().to_lowercase();
        let folder = folder.to_string_lossy().to_lowercase();
        Path::new(&path).starts_with(Path::new(&folder))
    } else {
        path.starts_with(&folder)
    }
}

/// Whether a file starts with SQLite's header.
fn has_header(path: &Path) -> Result<bool, FileRefusal> {
    let mut file = File::open(path).map_err(|error| refusal_of(&error))?;
    let mut start = [0_u8; 16];
    match file.read_exact(&mut start) {
        Ok(()) => Ok(&start == HEADER),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(refusal_of(&error)),
    }
}

/// Checks a file that the owner chose before the engine registers it: an
/// absolute path that is no URI, a file that is no link, on this computer,
/// that SQLite does not keep beside another database, outside `protected`,
/// Mannie's own folders, and that starts with SQLite's header.
pub fn check_new(path: &Path, protected: &[PathBuf]) -> Result<CheckedFile, FileRefusal> {
    if !path.is_absolute() {
        return Err(FileRefusal::NotAbsolute);
    }
    let text = path.to_string_lossy();
    let is_uri = text
        .get(..5)
        .is_some_and(|start| start.eq_ignore_ascii_case("file:"));
    if is_uri {
        return Err(FileRefusal::Uri);
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| refusal_of(&error))?;
    if metadata.file_type().is_symlink() {
        return Err(FileRefusal::Link);
    }
    if !metadata.is_file() {
        return Err(FileRefusal::NotAFile);
    }
    let canonical = fs::canonicalize(path).map_err(|error| refusal_of(&error))?;
    if is_network(&canonical) {
        return Err(FileRefusal::Network);
    }
    let name = canonical
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if COMPANIONS.iter().any(|suffix| name.ends_with(suffix)) {
        return Err(FileRefusal::Companion);
    }
    if protected.iter().any(|folder| is_inside(&canonical, folder)) {
        return Err(FileRefusal::MannieFolder);
    }
    if !has_header(&canonical)? {
        return Err(FileRefusal::NotADatabase);
    }
    let identity = platform::identity(&canonical).map_err(|error| refusal_of(&error))?;
    Ok(CheckedFile {
        path: canonical,
        identity,
    })
}

/// Checks a registered file before the engine opens it: the path still
/// leads, without a link, to the file with the registered identity.
pub fn check_registered(path: &Path, identity: &str) -> Result<(), FileRefusal> {
    let metadata = fs::symlink_metadata(path).map_err(|error| refusal_of(&error))?;
    if metadata.file_type().is_symlink() {
        return Err(FileRefusal::Changed);
    }
    let canonical = fs::canonicalize(path).map_err(|error| refusal_of(&error))?;
    if canonical != path {
        return Err(FileRefusal::Changed);
    }
    let current = platform::identity(path).map_err(|error| refusal_of(&error))?;
    if current != identity {
        return Err(FileRefusal::Changed);
    }
    Ok(())
}

#[cfg(unix)]
mod platform {
    use std::fs;
    use std::io;
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;

    /// The device and inode.
    pub(super) fn identity(path: &Path) -> io::Result<String> {
        let metadata = fs::symlink_metadata(path)?;
        Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
    }
}

#[cfg(windows)]
mod platform {
    use std::fs::OpenOptions;
    use std::io;
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE,
    };

    /// The volume serial number and file index. The file is opened only to
    /// read its attributes, without following a reparse point, and shared
    /// fully, so the open disturbs no other program.
    pub(super) fn identity(path: &Path) -> io::Result<String> {
        let file = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let information = winapi_util::file::information(&file)?;
        Ok(format!(
            "{:x}:{:x}",
            information.volume_serial_number(),
            information.file_index()
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{FileRefusal, check_new, check_registered, is_network};

    /// A file with SQLite's header and some bytes after it.
    fn database(path: &Path) {
        let mut bytes = b"SQLite format 3\0".to_vec();
        bytes.extend_from_slice(&[0; 84]);
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn a_database_passes_with_its_canonical_path_and_identity() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("shop.db");
        database(&path);

        let checked = check_new(&path, &[]).unwrap();

        assert_eq!(checked.path, fs::canonicalize(&path).unwrap());
        assert_ne!(checked.identity, "");
        assert_eq!(check_registered(&checked.path, &checked.identity), Ok(()));
    }

    #[test]
    fn the_engine_refuses_what_it_never_opens() {
        let folder = tempfile::tempdir().unwrap();
        let text = folder.path().join("notes.txt");
        fs::write(&text, "SQLite? No.").unwrap();
        let empty = folder.path().join("empty.db");
        fs::write(&empty, "").unwrap();
        let wal = folder.path().join("shop.db-wal");
        database(&wal);
        let mannie = folder.path().join("mannie");
        fs::create_dir(&mannie).unwrap();
        let state = mannie.join("mannie.sqlite");
        database(&state);

        assert_eq!(
            check_new(Path::new("shop.db"), &[]),
            Err(FileRefusal::NotAbsolute)
        );
        assert_eq!(
            check_new(&folder.path().join("missing.db"), &[]),
            Err(FileRefusal::NotFound)
        );
        assert_eq!(check_new(folder.path(), &[]), Err(FileRefusal::NotAFile));
        assert_eq!(check_new(&text, &[]), Err(FileRefusal::NotADatabase));
        assert_eq!(check_new(&empty, &[]), Err(FileRefusal::NotADatabase));
        assert_eq!(check_new(&wal, &[]), Err(FileRefusal::Companion));
        assert_eq!(
            check_new(&state, std::slice::from_ref(&mannie)),
            Err(FileRefusal::MannieFolder)
        );
    }

    #[test]
    fn a_path_that_sqlite_would_read_as_a_uri_is_refused() {
        let uri = if cfg!(windows) {
            PathBuf::from(r"file:C:\shop.db")
        } else {
            PathBuf::from("file:/shop.db")
        };

        // A relative path, as `file:` is on Windows, or a URI.
        assert!(matches!(
            check_new(&uri, &[]),
            Err(FileRefusal::Uri | FileRefusal::NotAbsolute)
        ));
    }

    #[test]
    fn network_paths_are_told_apart_from_local_ones() {
        assert!(is_network(Path::new(r"\\?\UNC\server\share\shop.db")));
        assert!(is_network(Path::new(r"\\server\share\shop.db")));
        assert!(!is_network(Path::new(r"\\?\D:\data\shop.db")));
        assert!(!is_network(Path::new("/home/owner/shop.db")));
    }

    #[test]
    fn a_replaced_file_is_another_file() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("shop.db");
        database(&path);
        let checked = check_new(&path, &[]).unwrap();
        let copy = folder.path().join("copy.db");
        database(&copy);

        fs::rename(&copy, &path).unwrap();

        assert_eq!(
            check_registered(&checked.path, &checked.identity),
            Err(FileRefusal::Changed)
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_refused_and_a_path_that_became_a_link_is_another_file() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("shop.db");
        database(&path);
        let link = folder.path().join("link.db");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        let checked = check_new(&path, &[]).unwrap();
        let moved = folder.path().join("moved.db");
        fs::rename(&path, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &path).unwrap();

        assert_eq!(check_new(&link, &[]), Err(FileRefusal::Link));
        assert_eq!(
            check_registered(&checked.path, &checked.identity),
            Err(FileRefusal::Changed)
        );
    }
}
