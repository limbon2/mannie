//! The private folders of browser sessions (ADR-0064, decision 2).
//!
//! Each session of the Browser plugin keeps its engine's data in a folder of
//! its own under one root. The process that hosts the session holds the
//! folder's `owner.lock` for as long as the session's processes can run, so
//! a later start of Mannie removes only folders that nobody owns.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

const FOLDER_PREFIX: &str = "session-";
const OWNER_LOCK: &str = "owner.lock";

/// Removes only abandoned session folders. A live session holds the lock
/// file of its folder.
pub fn cleanup_browser_profiles(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_dir())
            || !entry
                .file_name()
                .to_string_lossy()
                .starts_with(FOLDER_PREFIX)
        {
            continue;
        }
        let path = entry.path();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.join(OWNER_LOCK));
        // A folder whose creation stopped before its lock existed has no
        // owner either.
        let abandoned = match &lock {
            Ok(lock) => lock.try_lock().is_ok(),
            Err(error) => error.kind() == io::ErrorKind::NotFound,
        };
        drop(lock);
        if abandoned {
            drop(fs::remove_dir_all(path));
        }
    }
}

/// A session's private folder. The lock stays open until the folder is
/// removed, which the owner does only after the session's processes ended.
#[derive(Debug)]
pub struct BrowserFolder {
    data: PathBuf,
    directory: Option<tempfile::TempDir>,
    lock: Option<File>,
}

impl BrowserFolder {
    /// Creates a new, empty folder under `root` and locks it.
    ///
    /// # Errors
    ///
    /// The file system's error when the folder or its lock cannot be made.
    pub fn create(root: &Path) -> io::Result<Self> {
        fs::create_dir_all(root)?;
        let mut builder = tempfile::Builder::new();
        builder.prefix(FOLDER_PREFIX);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            builder.permissions(fs::Permissions::from_mode(0o700));
        }
        let directory = builder.tempdir_in(root)?;
        let lock = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(directory.path().join(OWNER_LOCK))?;
        lock.try_lock().map_err(io::Error::other)?;
        let data = directory.path().join("data");
        fs::create_dir(&data)?;
        Ok(Self {
            data,
            directory: Some(directory),
            lock: Some(lock),
        })
    }

    /// The folder that the engine keeps its data in.
    #[must_use]
    pub fn data(&self) -> &Path {
        &self.data
    }

    /// Whether the engine opened a debugging port in this folder, which a
    /// session must never have (ADR-0064, decision 3).
    #[must_use]
    pub fn has_debugging_port(&self) -> bool {
        self.data
            .join("EBWebView")
            .join("DevToolsActivePort")
            .exists()
            || self.data.join("DevToolsActivePort").exists()
    }

    /// Removes the folder. Call it only after every process of the session
    /// has ended; a folder that cannot be removed now stays locked until
    /// this process ends, and the next start removes it.
    pub fn remove(mut self) {
        self.release();
    }

    fn release(&mut self) {
        let Some(directory) = self.directory.take() else {
            return;
        };
        let path = directory.keep();
        if fs::remove_dir_all(&path).is_err() {
            // Something still uses the folder. Keep it locked until this
            // process ends, so that only the next start removes it.
            if let Some(lock) = self.lock.take() {
                Box::leak(Box::new(lock));
            }
        }
    }
}

impl Drop for BrowserFolder {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::{BrowserFolder, cleanup_browser_profiles};

    #[test]
    fn a_folder_is_removed_with_its_lock() {
        let root = tempfile::tempdir().unwrap();
        let folder = BrowserFolder::create(root.path()).unwrap();
        let session = folder.data().parent().unwrap().to_path_buf();
        assert!(folder.data().is_dir());
        assert!(!folder.has_debugging_port());
        folder.remove();
        assert!(!session.exists());
    }

    #[test]
    fn cleanup_keeps_live_folders_and_removes_abandoned_ones() {
        let root = tempfile::tempdir().unwrap();
        let live = BrowserFolder::create(root.path()).unwrap();
        let abandoned = root.path().join("session-abandoned");
        std::fs::create_dir_all(abandoned.join("data")).unwrap();
        std::fs::write(abandoned.join("owner.lock"), b"").unwrap();
        let unrelated = root.path().join("other");
        std::fs::create_dir(&unrelated).unwrap();

        cleanup_browser_profiles(root.path());

        assert!(live.data().is_dir());
        assert!(!abandoned.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn a_debugging_port_file_is_detected() {
        let root = tempfile::tempdir().unwrap();
        let folder = BrowserFolder::create(root.path()).unwrap();
        std::fs::create_dir_all(folder.data().join("EBWebView")).unwrap();
        std::fs::write(
            folder.data().join("EBWebView").join("DevToolsActivePort"),
            b"9222\n",
        )
        .unwrap();
        assert!(folder.has_debugging_port());
    }
}
