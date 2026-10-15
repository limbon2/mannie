//! Unix: a change opens its folder without following links, with `openat2`
//! and `RESOLVE_NO_SYMLINKS` on Linux, or one component at a time with
//! `O_NOFOLLOW`, and then works relative to the open folder with the `*at`
//! calls, which never follow a link in a name they create, rename or delete.

use std::ffi::{CString, OsStr};
use std::fs::File;
use std::io;
use std::os::fd::OwnedFd;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use mannie_types::tools::ToolFailure;
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};
use rustix::io::Errno;

use super::{EntryType, LinkFound};
use crate::paths::io_failure;

/// A pinned folder: its open descriptor.
#[derive(Debug)]
pub(super) struct Folder {
    path: PathBuf,
    fd: OwnedFd,
}

/// The permission bits of a file that its replacement keeps.
#[derive(Debug)]
pub(super) struct Permissions {
    mode: rustix::fs::RawMode,
}

/// Flags of a folder opened for `*at` calls and for flushing.
const FOLDER_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::CLOEXEC);

impl Folder {
    /// Opens a folder without following any link on its path.
    pub(super) fn open(path: &Path) -> Result<Self, ToolFailure> {
        let fd = open_folder(path).map_err(|error| {
            if super::is_link_error(&error) {
                ToolFailure::PathChanged
            } else {
                io_failure(&error, path)
            }
        })?;
        Ok(Self {
            path: path.to_path_buf(),
            fd,
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn identity(&self) -> io::Result<String> {
        let stat = rustix::fs::fstat(&self.fd)?;
        Ok(format!("{}:{}", stat.st_dev, stat.st_ino))
    }

    pub(super) fn entry(&self, name: &OsStr) -> io::Result<(String, EntryType)> {
        let stat = rustix::fs::statat(&self.fd, name, AtFlags::SYMLINK_NOFOLLOW)?;
        Ok((
            format!("{}:{}", stat.st_dev, stat.st_ino),
            entry_type(FileType::from_raw_mode(stat.st_mode)),
        ))
    }

    pub(super) fn child(self: &Arc<Self>, name: &OsStr) -> io::Result<Self> {
        Ok(Self {
            fd: open_child(&self.fd, name)?,
            path: self.path.join(name),
        })
    }

    pub(super) fn open_file(&self, name: &OsStr) -> io::Result<(File, Permissions)> {
        let fd = rustix::fs::openat(
            &self.fd,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(nofollow_error)?;
        let stat = rustix::fs::fstat(&fd)?;
        if FileType::from_raw_mode(stat.st_mode) == FileType::Directory {
            return Err(io::ErrorKind::IsADirectory.into());
        }
        Ok((
            File::from(fd),
            Permissions {
                mode: stat.st_mode & 0o777,
            },
        ))
    }

    pub(super) fn create_temporary(&self, name: &OsStr) -> io::Result<File> {
        let fd = rustix::fs::openat(
            &self.fd,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_bits_truncate(0o666),
        )?;
        Ok(File::from(fd))
    }

    /// Gives a temporary file a name that must not exist. Linux renames with
    /// `RENAME_NOREPLACE`; where a file system or the system lacks it, a hard
    /// link does the same, because linking fails when the name exists.
    pub(super) fn rename_new(&self, temporary: &OsStr, name: &OsStr) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        match rustix::fs::renameat_with(
            &self.fd,
            temporary,
            &self.fd,
            name,
            rustix::fs::RenameFlags::NOREPLACE,
        ) {
            Ok(()) => return Ok(()),
            Err(Errno::INVAL | Errno::NOSYS) => {}
            Err(errno) => return Err(errno.into()),
        }
        rustix::fs::linkat(&self.fd, temporary, &self.fd, name, AtFlags::empty())?;
        if let Err(error) = rustix::fs::unlinkat(&self.fd, temporary, AtFlags::empty()) {
            tracing::warn!(%error, "a temporary file of a change was not removed");
        }
        Ok(())
    }

    pub(super) fn rename_replace(&self, temporary: &OsStr, name: &OsStr) -> io::Result<()> {
        Ok(rustix::fs::renameat(&self.fd, temporary, &self.fd, name)?)
    }

    pub(super) fn create_folder(&self, name: &OsStr) -> io::Result<()> {
        Ok(rustix::fs::mkdirat(
            &self.fd,
            name,
            Mode::from_bits_truncate(0o777),
        )?)
    }

    /// Deletes a file or a link: `unlinkat` deletes a link itself.
    pub(super) fn remove_entry(&self, name: &OsStr) -> io::Result<()> {
        match rustix::fs::unlinkat(&self.fd, name, AtFlags::empty()) {
            Ok(()) => Ok(()),
            // Linux reports a folder with EISDIR, other systems with EPERM.
            Err(Errno::ISDIR) => Err(io::ErrorKind::IsADirectory.into()),
            Err(errno) => Err(errno.into()),
        }
    }

    pub(super) fn remove_folder(&self, name: &OsStr) -> io::Result<()> {
        Ok(rustix::fs::unlinkat(&self.fd, name, AtFlags::REMOVEDIR)?)
    }

    /// Deletes a folder with its contents, relative to open folders all the
    /// way down, so a link inside is deleted and never followed.
    pub(super) fn remove_tree(&self, name: &OsStr) -> io::Result<()> {
        let child = open_child(&self.fd, name)?;
        empty_folder(&child)?;
        Ok(rustix::fs::unlinkat(&self.fd, name, AtFlags::REMOVEDIR)?)
    }

    /// Moves an entry to a name that must not exist.
    pub(super) fn rename_to(
        &self,
        name: &OsStr,
        destination: &Self,
        new_name: &OsStr,
    ) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        match rustix::fs::renameat_with(
            &self.fd,
            name,
            &destination.fd,
            new_name,
            rustix::fs::RenameFlags::NOREPLACE,
        ) {
            Ok(()) => return Ok(()),
            Err(Errno::INVAL | Errno::NOSYS) => {}
            Err(errno) => return Err(errno.into()),
        }
        let stat = rustix::fs::statat(&self.fd, name, AtFlags::SYMLINK_NOFOLLOW)?;
        if FileType::from_raw_mode(stat.st_mode) == FileType::Directory {
            // A folder cannot be hard-linked: rename after a check.
            return match rustix::fs::statat(&destination.fd, new_name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(_) => Err(io::ErrorKind::AlreadyExists.into()),
                Err(Errno::NOENT) => Ok(rustix::fs::renameat(
                    &self.fd,
                    name,
                    &destination.fd,
                    new_name,
                )?),
                Err(errno) => Err(errno.into()),
            };
        }
        rustix::fs::linkat(&self.fd, name, &destination.fd, new_name, AtFlags::empty())?;
        if let Err(errno) = rustix::fs::unlinkat(&self.fd, name, AtFlags::empty()) {
            // Undo the link, so the move leaves one name as it found it.
            if let Err(error) = rustix::fs::unlinkat(&destination.fd, new_name, AtFlags::empty()) {
                tracing::warn!(%error, "an undone move left a second name");
            }
            return Err(errno.into());
        }
        Ok(())
    }

    pub(super) fn sync(&self) -> io::Result<()> {
        Ok(rustix::fs::fsync(&self.fd)?)
    }
}

/// Opens a folder without following any link: `openat2` with
/// `RESOLVE_NO_SYMLINKS` where Linux has it, else one component at a time.
fn open_folder(path: &Path) -> io::Result<OwnedFd> {
    #[cfg(target_os = "linux")]
    match rustix::fs::openat2(
        rustix::fs::CWD,
        path,
        FOLDER_FLAGS,
        Mode::empty(),
        rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
    ) {
        Ok(fd) => return Ok(fd),
        // Kernels before 5.6, and some sandboxes, lack openat2. A path that is
        // not a folder goes through the walk too, which tells a link apart.
        Err(Errno::NOSYS | Errno::PERM | Errno::NOTDIR) => {}
        Err(Errno::LOOP) => return Err(io::Error::other(LinkFound)),
        Err(errno) => return Err(errno.into()),
    }
    let mut fd = rustix::fs::open("/", FOLDER_FLAGS, Mode::empty())?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => fd = open_child(&fd, name)?,
            _ => return Err(io::ErrorKind::InvalidInput.into()),
        }
    }
    Ok(fd)
}

/// Opens a folder inside an open folder without following a link.
fn open_child(fd: &OwnedFd, name: &OsStr) -> io::Result<OwnedFd> {
    match rustix::fs::openat(fd, name, FOLDER_FLAGS | OFlags::NOFOLLOW, Mode::empty()) {
        Ok(child) => Ok(child),
        // With O_DIRECTORY, Linux reports a link as ENOTDIR before it checks
        // O_NOFOLLOW.
        Err(Errno::NOTDIR) if is_link_at(fd, name) => Err(io::Error::other(LinkFound)),
        Err(errno) => Err(nofollow_error(errno)),
    }
}

/// Whether a name in an open folder is a link.
fn is_link_at(fd: &OwnedFd, name: &OsStr) -> bool {
    rustix::fs::statat(fd, name, AtFlags::SYMLINK_NOFOLLOW)
        .is_ok_and(|stat| FileType::from_raw_mode(stat.st_mode) == FileType::Symlink)
}

/// The error of an open with `O_NOFOLLOW`. A link makes it fail with `ELOOP`
/// on Linux and `EMLINK` on FreeBSD.
fn nofollow_error(errno: Errno) -> io::Error {
    let is_link = errno == Errno::LOOP || (cfg!(target_os = "freebsd") && errno == Errno::MLINK);
    if is_link {
        io::Error::other(LinkFound)
    } else {
        errno.into()
    }
}

/// Deletes everything inside an open folder.
fn empty_folder(folder: &OwnedFd) -> io::Result<()> {
    // The names are read first: deleting while reading may skip entries.
    let mut entries: Vec<(CString, FileType)> = Vec::new();
    let mut directory = Dir::read_from(folder)?;
    while let Some(entry) = directory.read() {
        let entry = entry?;
        let name = entry.file_name();
        if name == c"." || name == c".." {
            continue;
        }
        entries.push((name.to_owned(), entry.file_type()));
    }
    for (name, file_type) in entries {
        let file_type = match file_type {
            FileType::Unknown => FileType::from_raw_mode(
                rustix::fs::statat(folder, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW)?.st_mode,
            ),
            known => known,
        };
        if file_type == FileType::Directory {
            let child = rustix::fs::openat(
                folder,
                name.as_c_str(),
                FOLDER_FLAGS | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .map_err(nofollow_error)?;
            empty_folder(&child)?;
            rustix::fs::unlinkat(folder, name.as_c_str(), AtFlags::REMOVEDIR)?;
        } else {
            rustix::fs::unlinkat(folder, name.as_c_str(), AtFlags::empty())?;
        }
    }
    Ok(())
}

/// The type of an entry.
fn entry_type(file_type: FileType) -> EntryType {
    match file_type {
        FileType::Symlink => EntryType::Link,
        FileType::Directory => EntryType::Folder,
        FileType::RegularFile => EntryType::File,
        _ => EntryType::Other,
    }
}

/// Gives a replacement the permission bits of the file it replaces, such as
/// the bits that make a script executable.
pub(super) fn apply_permissions(file: &File, permissions: &Permissions) -> io::Result<()> {
    Ok(rustix::fs::fchmod(
        file,
        Mode::from_raw_mode(permissions.mode),
    )?)
}
