//! What accounts can do with the worker's files (docs/features/workers/
//! service.md): the worker refuses to start when its own account could
//! change its configuration, since such an account could widen its own
//! ceiling; and a file that root writes into the data folder, such as a
//! pairing code, is handed to the service's account.

use std::io;
use std::path::Path;

/// Whether this process's account can change `file`: write it, or replace
/// it in its folder.
pub(crate) fn can_change(file: &Path) -> io::Result<bool> {
    let folder = file.parent().unwrap_or(file);
    Ok(can_write_file(file)? || can_add_to_folder(folder)?)
}

#[cfg(unix)]
fn can_write_file(file: &Path) -> io::Result<bool> {
    writable(file)
}

#[cfg(unix)]
fn can_add_to_folder(folder: &Path) -> io::Result<bool> {
    writable(folder)
}

/// Whether the account can write `path`, by its real IDs, which are the
/// service's account. Root can write anything, so a worker that runs as
/// root can always change its configuration.
#[cfg(unix)]
fn writable(path: &Path) -> io::Result<bool> {
    use rustix::fs::{Access, AtFlags, CWD, accessat};
    use rustix::io::Errno;

    match accessat(CWD, path, Access::WRITE_OK, AtFlags::empty()) {
        Ok(()) => Ok(true),
        Err(Errno::ACCESS | Errno::PERM | Errno::ROFS) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(windows)]
fn can_write_file(file: &Path) -> io::Result<bool> {
    // Opening for writing changes nothing until something is written.
    allowed(std::fs::OpenOptions::new().write(true).open(file))
}

#[cfg(windows)]
fn can_add_to_folder(folder: &Path) -> io::Result<bool> {
    use std::os::windows::fs::OpenOptionsExt;

    use windows_sys::Win32::Storage::FileSystem::{FILE_ADD_FILE, FILE_FLAG_BACKUP_SEMANTICS};

    // A folder opens only with backup semantics, and asking for nothing but
    // the right to add a file says whether the account has that right.
    allowed(
        std::fs::OpenOptions::new()
            .access_mode(FILE_ADD_FILE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(folder),
    )
}

#[cfg(windows)]
fn allowed(opened: io::Result<std::fs::File>) -> io::Result<bool> {
    match opened {
        Ok(_file) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => Ok(false),
        Err(error) => Err(error),
    }
}

/// Gives `path` to the owner of `folder`, so that a file that root wrote
/// into the data folder belongs to the service's account.
#[cfg(unix)]
pub(crate) fn give_to_owner_of(path: &Path, folder: &Path) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    let owner = std::fs::metadata(folder)?;
    std::os::unix::fs::chown(path, Some(owner.uid()), Some(owner.gid()))
}

#[cfg(test)]
mod tests;
