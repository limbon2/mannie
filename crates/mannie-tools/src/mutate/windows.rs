//! Windows: every folder of the path stays open while a change runs, opened
//! without following reparse points and without sharing the right to delete,
//! so no folder of the path can be renamed, deleted or replaced by a link.
//! Changes then work with paths, which can no longer move.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::windows::fs::{FileTypeExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use mannie_types::tools::ToolFailure;
use windows_sys::Win32::Foundation::{ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

use super::{EntryType, LinkFound};
use crate::identity;
use crate::paths::io_failure;

/// A pinned folder: its open handle, and its parent, which keeps the folders
/// above it open.
#[derive(Debug)]
pub(super) struct Folder {
    path: PathBuf,
    handle: File,
    _parent: Option<Arc<Self>>,
}

/// Windows keeps no permissions that a replacement must copy: a new file
/// gets the permissions of its folder, and read-only files are not replaced.
#[derive(Debug)]
pub(super) struct Permissions;

impl Folder {
    /// Pins a folder and every folder above it, below the root of its drive,
    /// which cannot be renamed or replaced.
    pub(super) fn open(path: &Path) -> Result<Self, ToolFailure> {
        let mut current = PathBuf::new();
        let mut pinned: Option<Self> = None;
        for component in path.components() {
            current.push(component);
            if !matches!(component, Component::Normal(_)) {
                continue;
            }
            let handle = open_folder(&current).map_err(|error| failure(&error, &current))?;
            pinned = Some(Self {
                path: current.clone(),
                handle,
                _parent: pinned.map(Arc::new),
            });
        }
        match pinned {
            Some(folder) => Ok(folder),
            // The root of a drive.
            None => Ok(Self {
                handle: open_folder(path).map_err(|error| failure(&error, path))?,
                path: path.to_path_buf(),
                _parent: None,
            }),
        }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn identity(&self) -> io::Result<String> {
        identity::of_file(&self.handle)
    }

    pub(super) fn entry(&self, name: &OsStr) -> io::Result<(String, EntryType)> {
        let file = open_entry(&self.path.join(name), FILE_READ_ATTRIBUTES)?;
        let metadata = file.metadata()?;
        Ok((identity::of_file(&file)?, entry_type(&metadata)))
    }

    pub(super) fn child(self: &Arc<Self>, name: &OsStr) -> io::Result<Self> {
        let path = self.path.join(name);
        Ok(Self {
            handle: open_folder(&path)?,
            path,
            _parent: Some(Arc::clone(self)),
        })
    }

    pub(super) fn open_file(&self, name: &OsStr) -> io::Result<(File, Permissions)> {
        // Backup semantics let the open reach a folder too, which is then
        // refused as one.
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.path.join(name))?;
        let metadata = file.metadata()?;
        match entry_type(&metadata) {
            EntryType::Link => Err(io::Error::other(LinkFound)),
            EntryType::Folder => Err(io::ErrorKind::IsADirectory.into()),
            EntryType::File | EntryType::Other => Ok((file, Permissions)),
        }
    }

    pub(super) fn create_temporary(&self, name: &OsStr) -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.path.join(name))
    }

    /// Gives a temporary file a name that must not exist: a hard link fails
    /// when the name exists, so another program's new file is never replaced.
    /// File systems without hard links, such as FAT, fall back to a check
    /// before the rename.
    pub(super) fn rename_new(&self, temporary: &OsStr, name: &OsStr) -> io::Result<()> {
        let from = self.path.join(temporary);
        let to = self.path.join(name);
        match fs::hard_link(&from, &to) {
            Ok(()) => {
                remove_leftover(&from);
                Ok(())
            }
            Err(error) if lacks_hard_links(&error) => rename_checked(&from, &to),
            Err(error) => Err(error),
        }
    }

    pub(super) fn rename_replace(&self, temporary: &OsStr, name: &OsStr) -> io::Result<()> {
        fs::rename(self.path.join(temporary), self.path.join(name))
    }

    pub(super) fn create_folder(&self, name: &OsStr) -> io::Result<()> {
        fs::create_dir(self.path.join(name))
    }

    /// Deletes a file or a link. A link to a folder, such as a junction, is
    /// deleted as a folder, which deletes the link and not its target.
    pub(super) fn remove_entry(&self, name: &OsStr) -> io::Result<()> {
        let path = self.path.join(name);
        if fs::symlink_metadata(&path)?.file_type().is_symlink_dir() {
            fs::remove_dir(path)
        } else {
            fs::remove_file(path)
        }
    }

    pub(super) fn remove_folder(&self, name: &OsStr) -> io::Result<()> {
        fs::remove_dir(self.path.join(name))
    }

    /// Deletes a folder with its contents. The standard library deletes
    /// through handles that it opens without following reparse points, so a
    /// link inside is deleted and its target kept.
    pub(super) fn remove_tree(&self, name: &OsStr) -> io::Result<()> {
        fs::remove_dir_all(self.path.join(name))
    }

    /// Moves an entry to a name that must not exist. A file moves through a
    /// hard link, which fails when the name exists; a folder or a link is
    /// renamed after a check, because Windows never replaces a folder.
    pub(super) fn rename_to(
        &self,
        name: &OsStr,
        destination: &Self,
        new_name: &OsStr,
    ) -> io::Result<()> {
        let from = self.path.join(name);
        let to = destination.path.join(new_name);
        let metadata = fs::symlink_metadata(&from)?;
        if entry_type(&metadata) != EntryType::File {
            return rename_checked(&from, &to);
        }
        match fs::hard_link(&from, &to) {
            Ok(()) => fs::remove_file(&from).inspect_err(|_| remove_leftover(&to)),
            Err(error) if lacks_hard_links(&error) => rename_checked(&from, &to),
            Err(error) => Err(error),
        }
    }

    /// Windows records folder entries in the file system's journal; flushing
    /// a folder needs write access to it, which the pinned handles lack.
    #[expect(
        clippy::unnecessary_wraps,
        clippy::unused_self,
        reason = "the same method flushes the folder on Unix"
    )]
    pub(super) fn sync(&self) -> io::Result<()> {
        Ok(())
    }
}

/// Opens a folder without following a reparse point, sharing reading and
/// writing but not deleting, and checks that it is a folder.
fn open_folder(path: &Path) -> io::Result<File> {
    let handle = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    match entry_type(&handle.metadata()?) {
        EntryType::Folder => Ok(handle),
        EntryType::Link => Err(io::Error::other(LinkFound)),
        EntryType::File | EntryType::Other => Err(io::ErrorKind::NotADirectory.into()),
    }
}

/// Opens any entry without following a reparse point, sharing everything.
fn open_entry(path: &Path, access: u32) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(access)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

/// The type of an entry from metadata that does not follow links.
fn entry_type(metadata: &fs::Metadata) -> EntryType {
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        EntryType::Link
    } else if file_type.is_dir() {
        EntryType::Folder
    } else if metadata.is_file() {
        EntryType::File
    } else {
        EntryType::Other
    }
}

/// Renames after checking that nothing has the new name.
fn rename_checked(from: &Path, to: &Path) -> io::Result<()> {
    match fs::symlink_metadata(to) {
        Ok(_) => Err(io::ErrorKind::AlreadyExists.into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::rename(from, to),
        Err(error) => Err(error),
    }
}

/// Whether an error says that the file system has no hard links.
fn lacks_hard_links(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::Unsupported
        || error
            .raw_os_error()
            .and_then(|code| u32::try_from(code).ok())
            .is_some_and(|code| code == ERROR_INVALID_FUNCTION || code == ERROR_NOT_SUPPORTED)
}

/// Removes a name that a finished or undone change left behind.
fn remove_leftover(path: &Path) {
    if let Err(error) = fs::remove_file(path) {
        tracing::warn!(%error, "a leftover name of a change was not removed");
    }
}

/// The failure of pinning a folder.
fn failure(error: &io::Error, path: &Path) -> ToolFailure {
    if super::is_link_error(error) {
        ToolFailure::PathChanged
    } else {
        io_failure(error, path)
    }
}

/// Copies no permissions: see [`Permissions`].
#[expect(
    clippy::unnecessary_wraps,
    reason = "the same function copies the mode on Unix"
)]
pub(super) fn apply_permissions(_file: &File, _permissions: &Permissions) -> io::Result<()> {
    Ok(())
}
