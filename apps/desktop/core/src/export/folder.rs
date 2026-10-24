//! Exclusive creation and cleanup confined to the current export's files.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use mannie_types::export::{ConversationRecord, ExportError};

pub(super) const MARKER: &str = ".incomplete";

/// Only files successfully created by this instance enter the cleanup list.
/// Dropping an unfinished export never recursively deletes a directory.
pub(super) struct Folder {
    path: PathBuf,
    files: Vec<PathBuf>,
    directories: Vec<PathBuf>,
    completed: bool,
    incomplete: String,
}

impl Folder {
    pub(super) fn create(path: PathBuf, incomplete: &str) -> Result<Self, ExportError> {
        fs::create_dir(&path).map_err(|_error| ExportError::WriteFailed)?;
        let mut folder = Self {
            path,
            files: Vec::new(),
            directories: Vec::new(),
            completed: false,
            incomplete: incomplete.to_owned(),
        };
        folder.write(MARKER, incomplete.as_bytes())?;
        for directory in ["tasks", "images", "outputs"] {
            let path = folder.path.join(directory);
            fs::create_dir(&path).map_err(|_error| ExportError::WriteFailed)?;
            folder.directories.push(path);
        }
        Ok(folder)
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn write(&mut self, relative: &str, bytes: &[u8]) -> Result<(), ExportError> {
        // Callers construct all names from validated UUIDs and known formats.
        // Refuse unsafe components again at the filesystem boundary.
        let relative_path = Path::new(relative);
        if relative_path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(ExportError::WriteFailed);
        }
        let path = self.path.join(relative_path);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_error| ExportError::WriteFailed)?;
        self.files.push(path);
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_error| ExportError::WriteFailed)
    }

    pub(super) fn assets(&mut self, record: &ConversationRecord) -> Result<(), ExportError> {
        for image in &record.images {
            self.write(
                &format!("images/{}.{}", image.part_id, image.format.as_str()),
                &image.data,
            )?;
        }
        // The images of a tool call's result sit beside the owner's, named by
        // the call's part and their position (ADR-0044, decision 7).
        for image in &record.tool_images {
            self.write(
                &format!(
                    "images/{}-{}.{}",
                    image.part_id,
                    image.position,
                    image.format.as_str()
                ),
                &image.data,
            )?;
        }
        for archive in &record.archives {
            self.write(
                &format!("outputs/{}.txt", archive.id),
                archive.content.as_bytes(),
            )?;
        }
        Ok(())
    }

    /// Only fully written documents and assets lose the incomplete marker.
    pub(super) fn seal(&mut self) -> Result<(), ExportError> {
        fs::remove_file(self.path.join(MARKER)).map_err(|_error| ExportError::WriteFailed)?;
        Ok(())
    }

    pub(super) fn keep(&mut self) {
        self.completed = true;
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        let path = self.path.clone();
        let files = std::mem::take(&mut self.files);
        let directories = std::mem::take(&mut self.directories);
        let incomplete = std::mem::take(&mut self.incomplete);
        // Dropping a failed or canceled async action must not block its
        // executor thread on cleanup. Synchronous tests have no runtime.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let _cleanup =
                runtime.spawn_blocking(move || cleanup(&path, &files, &directories, &incomplete));
        } else {
            cleanup(&path, &files, &directories, &incomplete);
        }
    }
}

fn cleanup(path: &Path, files: &[PathBuf], directories: &[PathBuf], incomplete: &str) {
    let marker = path.join(MARKER);
    // A failure after sealing still leaves an explicit marker if cleanup
    // encounters another process's files or a filesystem error.
    if !marker.exists()
        && let Err(error) = restore_marker(&marker, incomplete)
    {
        tracing::warn!(error.kind = ?error.kind(), "an incomplete export marker could not be restored");
    }
    let mut cleaned = true;
    for path in files.iter().rev().filter(|path| **path != marker) {
        if let Err(error) = fs::remove_file(path) {
            cleaned &= error.kind() == std::io::ErrorKind::NotFound;
        }
    }
    for directory in directories.iter().rev() {
        if let Err(error) = fs::remove_dir(directory) {
            cleaned &= error.kind() == std::io::ErrorKind::NotFound;
        }
    }
    // An unexpected file is never ours to delete. Keep the marker if
    // cleanup was incomplete, rather than make the folder look finished.
    if cleaned {
        cleaned = fs::read_dir(path).is_ok_and(|entries| {
            entries
                .into_iter()
                .all(|entry| entry.is_ok_and(|entry| entry.path() == marker))
        });
    }
    if cleaned {
        if let Err(error) = fs::remove_file(&marker)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return;
        }
        if let Err(error) = fs::remove_dir(path) {
            tracing::warn!(error.kind = ?error.kind(), "an incomplete export folder could not be removed");
        }
    }
}

fn restore_marker(marker: &Path, incomplete: &str) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(marker)?;
    file.write_all(incomplete.as_bytes())?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusive_creation_preserves_existing_content_and_cleanup_owns_only_its_files() {
        let parent = tempfile::tempdir().unwrap();
        let path = parent.path().join("export");
        let mut folder = Folder::create(path.clone(), "incomplete").unwrap();
        folder.write("conversation.md", b"first").unwrap();
        assert_eq!(
            folder.write("conversation.md", b"replacement"),
            Err(ExportError::WriteFailed)
        );
        assert_eq!(
            folder.write("../outside", b"bad"),
            Err(ExportError::WriteFailed)
        );
        assert!(matches!(
            Folder::create(path.clone(), "other"),
            Err(ExportError::WriteFailed)
        ));
        assert_eq!(fs::read(path.join("conversation.md")).unwrap(), b"first");
        fs::write(path.join("foreign-file"), b"keep me").unwrap();
        drop(folder);
        assert_eq!(fs::read(path.join("foreign-file")).unwrap(), b"keep me");
        // An unexpected root entry leaves the folder behind and must keep
        // its incomplete marker as well.
        assert!(path.join(MARKER).exists());
    }

    #[test]
    fn unfinished_exports_clean_up_and_finished_exports_keep_their_files() {
        let parent = tempfile::tempdir().unwrap();
        let unfinished = parent.path().join("unfinished");
        {
            let mut folder = Folder::create(unfinished.clone(), "incomplete").unwrap();
            folder.write("conversation.md", b"partial").unwrap();
        }
        assert!(!unfinished.exists());
        let finished = parent.path().join("finished");
        {
            let mut folder = Folder::create(finished.clone(), "incomplete").unwrap();
            folder.write("conversation.md", b"complete").unwrap();
            folder.seal().unwrap();
            folder.keep();
        }
        assert_eq!(
            fs::read(finished.join("conversation.md")).unwrap(),
            b"complete"
        );
        assert!(!finished.join(MARKER).exists());
    }
}
