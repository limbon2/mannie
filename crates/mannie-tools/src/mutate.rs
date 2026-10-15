//! Changes to the file system that never follow links (docs/features/
//! system-tools/tools-and-results.md, file safety).
//!
//! A change pins the folder it works in: on Windows, Mannie opens every folder
//! of the path without following reparse points and keeps them open without
//! sharing the right to delete, so none of them can be renamed, deleted or
//! replaced meanwhile; on Unix, it opens the folder without following links,
//! with `openat2` and `RESOLVE_NO_SYMLINKS` or one component at a time with
//! `O_NOFOLLOW`, and works relative to it. New content goes to a temporary
//! file in the same folder, which is flushed and then renamed into place.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mannie_types::tools::ToolFailure;

use crate::paths::io_failure;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
use windows as platform;

/// What a change finds at a name in a pinned folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryType {
    /// A file.
    File,
    /// A folder.
    Folder,
    /// A link, which a change never follows.
    Link,
    /// Something else, such as a socket.
    Other,
}

/// A folder that cannot change its place while the value lives.
#[derive(Debug, Clone)]
pub(crate) struct PinnedFolder {
    inner: Arc<platform::Folder>,
}

/// The content and permissions of a file that a change replaces.
#[derive(Debug)]
pub(crate) struct ExistingFile {
    /// The file's bytes.
    pub(crate) bytes: Vec<u8>,
    permissions: platform::Permissions,
}

impl PinnedFolder {
    /// Pins an existing folder. A link on the way, or at the folder itself,
    /// means that the path changed after it was checked.
    pub(crate) fn open(folder: &Path) -> Result<Self, ToolFailure> {
        Ok(Self {
            inner: Arc::new(platform::Folder::open(folder)?),
        })
    }

    /// The folder's path.
    pub(crate) fn path(&self) -> &Path {
        self.inner.path()
    }

    /// The path of a name in the folder, for results and errors.
    pub(crate) fn child_path(&self, name: &OsStr) -> PathBuf {
        self.inner.path().join(name)
    }

    /// The folder's identity, in the form of [`crate::identity`].
    pub(crate) fn identity(&self) -> Result<String, ToolFailure> {
        self.inner
            .identity()
            .map_err(|error| io_failure(&error, self.path()))
    }

    /// The identity and type of a name in the folder, without following a
    /// link, or `None` when nothing has the name.
    pub(crate) fn entry(&self, name: &OsStr) -> Result<Option<(String, EntryType)>, ToolFailure> {
        match self.inner.entry(name) {
            Ok(entry) => Ok(Some(entry)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_failure(&error, &self.child_path(name))),
        }
    }

    /// Pins a folder inside this one, which must not be a link.
    pub(crate) fn child(&self, name: &OsStr) -> Result<Self, ToolFailure> {
        let child = platform::Folder::child(&self.inner, name)
            .map_err(|error| self.failure(&error, name))?;
        Ok(Self {
            inner: Arc::new(child),
        })
    }

    /// Reads a file in the folder without following a link, with the
    /// permissions that its replacement keeps. Files larger than `limit`
    /// bytes are refused.
    pub(crate) fn read(&self, name: &OsStr, limit: u64) -> Result<ExistingFile, ToolFailure> {
        let (file, permissions) = self
            .inner
            .open_file(name)
            .map_err(|error| self.failure(&error, name))?;
        let mut bytes = Vec::new();
        file.take(limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| self.failure(&error, name))?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
            return Err(ToolFailure::TooLarge {
                limit_bytes: u32::try_from(limit).unwrap_or(u32::MAX),
            });
        }
        Ok(ExistingFile { bytes, permissions })
    }

    /// Opens a file in the folder for reading without following a link.
    pub(crate) fn open_file(&self, name: &OsStr) -> Result<File, ToolFailure> {
        self.inner
            .open_file(name)
            .map(|(file, _)| file)
            .map_err(|error| self.failure(&error, name))
    }

    /// A temporary file in the folder, which [`Self::commit_new`] or
    /// [`Self::commit_replace`] gives its name. Dropped without a commit, it
    /// is removed.
    pub(crate) fn draft(&self, name: &OsStr) -> Result<Draft, ToolFailure> {
        let temporary = temporary_name(name);
        let file = self
            .inner
            .create_temporary(&temporary)
            .map_err(|error| self.failure(&error, &temporary))?;
        Ok(Draft {
            file: Some(file),
            temporary,
            folder: Arc::clone(&self.inner),
        })
    }

    /// Flushes a draft to the disk and gives it `name`. Something that has the
    /// name already, even a link or a file another program just created,
    /// makes the commit fail with `already_exists`.
    pub(crate) fn commit_new(&self, mut draft: Draft, name: &OsStr) -> Result<(), ToolFailure> {
        draft.flush()?;
        self.inner
            .rename_new(&draft.temporary, name)
            .map_err(|error| self.failure(&error, name))?;
        draft.committed();
        self.sync()
    }

    /// Flushes a draft to the disk and replaces `name` with it, keeping the
    /// permissions of the file it replaces.
    pub(crate) fn commit_replace(
        &self,
        mut draft: Draft,
        name: &OsStr,
        like: &ExistingFile,
    ) -> Result<(), ToolFailure> {
        if let Some(file) = &draft.file {
            platform::apply_permissions(file, &like.permissions)
                .map_err(|error| self.failure(&error, name))?;
        }
        draft.flush()?;
        self.inner
            .rename_replace(&draft.temporary, name)
            .map_err(|error| self.failure(&error, name))?;
        draft.committed();
        self.sync()
    }

    /// Creates a folder in this one.
    pub(crate) fn create_folder(&self, name: &OsStr) -> Result<(), ToolFailure> {
        self.inner
            .create_folder(name)
            .map_err(|error| self.failure(&error, name))?;
        self.sync()
    }

    /// Deletes a file or a link, without following the link.
    pub(crate) fn remove_entry(&self, name: &OsStr) -> Result<(), ToolFailure> {
        self.inner
            .remove_entry(name)
            .map_err(|error| self.failure(&error, name))?;
        self.sync()
    }

    /// Deletes an empty folder.
    pub(crate) fn remove_empty_folder(&self, name: &OsStr) -> Result<(), ToolFailure> {
        self.inner
            .remove_folder(name)
            .map_err(|error| self.failure(&error, name))?;
        self.sync()
    }

    /// Deletes a folder with everything inside it. Links inside are deleted,
    /// never followed.
    pub(crate) fn remove_tree(&self, name: &OsStr) -> Result<(), ToolFailure> {
        self.inner
            .remove_tree(name)
            .map_err(|error| self.failure(&error, name))?;
        self.sync()
    }

    /// Moves `name` to `new_name` in `destination`, which must not exist.
    pub(crate) fn move_entry(
        &self,
        name: &OsStr,
        destination: &Self,
        new_name: &OsStr,
    ) -> Result<(), ToolFailure> {
        self.inner
            .rename_to(name, &destination.inner, new_name)
            .map_err(|error| destination.failure(&error, new_name))?;
        self.sync()?;
        destination.sync()
    }

    /// Flushes the folder's entries to the disk, where the system supports
    /// that.
    fn sync(&self) -> Result<(), ToolFailure> {
        self.inner
            .sync()
            .map_err(|error| io_failure(&error, self.path()))
    }

    /// The failure of an operation on a name in the folder. A link where the
    /// preparation found none means that the path changed.
    fn failure(&self, error: &io::Error, name: &OsStr) -> ToolFailure {
        if is_link_error(error) {
            return ToolFailure::PathChanged;
        }
        io_failure(error, &self.child_path(name))
    }
}

/// The error inside an [`io::Error`] for a link where a change allows none.
#[derive(Debug, thiserror::Error)]
#[error("the path is a link")]
struct LinkFound;

/// Whether an error is a link where a change allows none.
fn is_link_error(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(<dyn std::error::Error + Send + Sync>::is::<LinkFound>)
}

/// A temporary file that becomes a file of its folder when it is committed.
#[derive(Debug)]
pub(crate) struct Draft {
    file: Option<File>,
    temporary: OsString,
    folder: Arc<platform::Folder>,
}

impl Draft {
    /// Writes bytes to the draft.
    pub(crate) fn write(&mut self, bytes: &[u8]) -> Result<(), ToolFailure> {
        let path = self.folder.path().join(&self.temporary);
        match &mut self.file {
            Some(file) => file
                .write_all(bytes)
                .map_err(|error| io_failure(&error, &path)),
            None => Err(io_failure(&io::Error::other("the draft is closed"), &path)),
        }
    }

    /// Copies a reader into the draft and returns the bytes copied.
    pub(crate) fn copy_from(&mut self, reader: &mut impl Read) -> Result<u64, ToolFailure> {
        let path = self.folder.path().join(&self.temporary);
        match &mut self.file {
            Some(file) => io::copy(reader, file).map_err(|error| io_failure(&error, &path)),
            None => Err(io_failure(&io::Error::other("the draft is closed"), &path)),
        }
    }

    /// Flushes the content to the disk and closes the file.
    fn flush(&mut self) -> Result<(), ToolFailure> {
        if let Some(file) = self.file.take() {
            let path = self.folder.path().join(&self.temporary);
            file.sync_all().map_err(|error| io_failure(&error, &path))?;
        }
        Ok(())
    }

    /// Marks the draft as committed, so dropping it removes nothing.
    fn committed(&mut self) {
        self.temporary.clear();
    }
}

impl Drop for Draft {
    fn drop(&mut self) {
        // Close the file first: Windows removes no file that is open.
        self.file = None;
        if !self.temporary.is_empty()
            && let Err(error) = self.folder.remove_entry(&self.temporary)
        {
            tracing::warn!(%error, "a temporary file of a failed change was not removed");
        }
    }
}

/// The name of a temporary file for `name`: hidden on Unix, recognizable, and
/// random, so it never meets an existing name.
fn temporary_name(name: &OsStr) -> OsString {
    let mut temporary = OsString::from(".");
    let stem: String = name.to_string_lossy().chars().take(64).collect();
    temporary.push(stem);
    temporary.push(format!(".{:016x}.mannie-tmp", fastrand::u64(..)));
    temporary
}

#[cfg(test)]
mod tests;
