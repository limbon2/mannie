//! What the change tools share (docs/features/system-tools/
//! operations-and-approvals.md, the lifecycle): the last check right before
//! a change, under the machine's lock. The tool prepares again and compares
//! the accesses with the authorized ones, then pins the folder it changes and
//! checks that the path still names the file or folder that was authorized.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use mannie_types::tools::{Access, AccessMode, ToolFailure};

use crate::identity::identity;
use crate::mutate::PinnedFolder;
use crate::paths::{io_failure, path_text};
use crate::tool::{PrepareError, ToolContext, blocking};

/// Runs a change on a blocking thread, under the lock that makes file
/// changes run one at a time on the machine.
pub(crate) async fn locked<T, F>(context: &ToolContext, work: F) -> Result<T, ToolFailure>
where
    F: FnOnce() -> Result<T, ToolFailure> + Send + 'static,
    T: Send + 'static,
{
    let _guard = context.mutations.acquire().await;
    blocking(work).await
}

/// Checks that a fresh preparation found the accesses that were authorized.
/// Anything else means that the path changed since.
pub(crate) fn unchanged(
    authorized: &[Access],
    fresh: Result<Vec<Access>, PrepareError>,
) -> Result<(), ToolFailure> {
    match fresh {
        Ok(fresh) if fresh == authorized => Ok(()),
        // Only plugins' tools end a call in their preparation, so a change of
        // the core's that meets one has changed underneath, as one whose
        // accesses differ has.
        Ok(_) | Err(PrepareError::Ended(_)) => Err(ToolFailure::PathChanged),
        Err(PrepareError::Failed(failure)) => Err(failure),
        Err(PrepareError::InvalidArguments { detail }) => {
            Err(ToolFailure::IoError { kind: detail })
        }
    }
}

/// The path and the identity of a path access.
pub(crate) fn path_of(access: &Access) -> Result<(PathBuf, &str), ToolFailure> {
    match access {
        Access::Path { path, identity, .. } => Ok((PathBuf::from(path), identity.as_str())),
        Access::Command { .. }
        | Access::Project { .. }
        | Access::RemoteCommand { .. }
        | Access::Program { .. }
        | Access::ExternalTool { .. } => Err(ToolFailure::PathChanged),
    }
}

/// The folder above a path and the path's name.
fn split(path: &Path) -> Result<(&Path, OsString), ToolFailure> {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => Ok((parent, name.to_owned())),
        _ => Err(ToolFailure::IoError {
            kind: "the path has no folder above it".to_owned(),
        }),
    }
}

/// Pins the folder of an existing path and checks that the path still names
/// the file, folder or link with the authorized identity.
pub(crate) fn pin_existing(
    path: &Path,
    identity: &str,
) -> Result<(PinnedFolder, OsString), ToolFailure> {
    let (parent, name) = split(path)?;
    let folder = PinnedFolder::open(parent)?;
    match folder.entry(&name)? {
        Some((found, _)) if found == identity => Ok((folder, name)),
        _ => Err(ToolFailure::PathChanged),
    }
}

/// Pins the nearest existing folder above a path that does not exist yet,
/// checks that it is the authorized one, and returns it with the names of
/// the missing folders below it and, last, the path's own name.
pub(crate) fn pin_missing(
    path: &Path,
    identity: &str,
) -> Result<(PinnedFolder, Vec<OsString>), ToolFailure> {
    let mut names = Vec::new();
    let mut current = path;
    let existing = loop {
        let (parent, name) = split(current)?;
        names.push(name);
        match fs::symlink_metadata(parent) {
            Ok(_) => break parent,
            Err(error) if error.kind() == io::ErrorKind::NotFound => current = parent,
            Err(error) => return Err(io_failure(&error, parent)),
        }
    };
    names.reverse();
    let folder = PinnedFolder::open(existing)?;
    if folder.identity()? != identity {
        return Err(ToolFailure::PathChanged);
    }
    if let Some(first) = names.first()
        && folder.entry(first)?.is_some()
    {
        return Err(ToolFailure::PathChanged);
    }
    Ok((folder, names))
}

/// Creates the missing folders that `names` lists before its last name, and
/// returns the folder that holds the last name, with the number of folders
/// created.
pub(crate) fn create_folders(
    mut folder: PinnedFolder,
    names: &[OsString],
) -> Result<(PinnedFolder, u32), ToolFailure> {
    let mut created = 0_u32;
    if let Some((_, folders)) = names.split_last() {
        for name in folders {
            folder.create_folder(name)?;
            folder = folder.child(name)?;
            created += 1;
        }
    }
    Ok((folder, created))
}

/// The folders above a path that do not exist yet, from the outermost.
pub(crate) fn missing_folders(path: &Path) -> Result<Vec<PathBuf>, ToolFailure> {
    let mut missing = Vec::new();
    let mut current = path.parent();
    while let Some(folder) = current {
        match fs::symlink_metadata(folder) {
            Ok(metadata) if metadata.is_dir() => break,
            Ok(_) => {
                return Err(ToolFailure::NotAFolder {
                    path: folder.display().to_string(),
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(folder.to_path_buf());
                current = folder.parent();
            }
            Err(error) => return Err(io_failure(&error, folder)),
        }
    }
    missing.reverse();
    Ok(missing)
}

/// The access of a path that a change creates. Its identity is the one of
/// the nearest folder that exists.
pub(crate) fn create_access(path: &Path, folder: bool) -> Result<Access, ToolFailure> {
    Ok(Access::Path {
        path: path_text(path)?,
        mode: AccessMode::Create,
        folder,
        exists: false,
        identity: identity(path)?,
        entries: None,
        state: None,
    })
}

/// Whether the file is marked read-only. A change never replaces such a file.
pub(crate) fn refuse_read_only(path: &Path, metadata: &fs::Metadata) -> Result<(), ToolFailure> {
    if metadata.permissions().readonly() {
        return Err(ToolFailure::PermissionDenied {
            path: path.display().to_string(),
        });
    }
    Ok(())
}
