//! The identity of a file or folder: the volume serial number and file index
//! on Windows, the device and inode on Unix. A path whose file was replaced
//! keeps its text but changes its identity, so the digest of an operation
//! changes with it (docs/features/system-tools/operations-and-approvals.md,
//! accesses).

use std::path::Path;

use mannie_types::tools::ToolFailure;

use crate::paths::io_failure;

/// The identity of what is at `path`, without following a final link, or of
/// the nearest folder above it that exists.
pub(crate) fn identity(path: &Path) -> Result<String, ToolFailure> {
    let mut current = path;
    loop {
        match platform::identity(current) {
            Ok(identity) => return Ok(identity),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                current = current.parent().ok_or_else(|| io_failure(&error, path))?;
            }
            Err(error) => return Err(io_failure(&error, current)),
        }
    }
}

/// The identity of an open file or folder.
#[cfg(windows)]
pub(crate) fn of_file(file: &std::fs::File) -> std::io::Result<String> {
    platform::of_file(file)
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
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    /// The volume serial number and file index. The file is opened only to
    /// read its attributes, without following a reparse point, and shared
    /// fully, so the open disturbs no other program.
    pub(super) fn identity(path: &Path) -> io::Result<String> {
        let file = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        of_file(&file)
    }

    pub(super) fn of_file(file: &File) -> io::Result<String> {
        let information = winapi_util::file::information(file)?;
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

    use super::identity;

    #[test]
    fn a_replaced_file_has_another_identity() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("notes.txt");
        fs::write(&path, "first").unwrap();
        let first = identity(&path).unwrap();
        let other = folder.path().join("other.txt");
        fs::write(&other, "second").unwrap();

        fs::rename(&other, &path).unwrap();

        assert_ne!(identity(&path).unwrap(), first);
    }

    #[test]
    fn a_missing_path_has_the_identity_of_its_nearest_folder() {
        let folder = tempfile::tempdir().unwrap();

        assert_eq!(
            identity(&folder.path().join("missing").join("deeper.txt")).unwrap(),
            identity(folder.path()).unwrap()
        );
    }
}
