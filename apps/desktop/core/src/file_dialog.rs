//! The system's file dialog as the SDK's service `file-dialog` (ADR-0034,
//! decision 5; plugins/images/docs/README.md, adding images). The core shows the
//! dialog over the main window and keeps the paths that the owner chose in a
//! table of the plugin's binding. The plugin gets a handle for each file and
//! reads it once through the service, so a path never crosses IPC or reaches
//! a plugin's code (ADR-0004, T-34). On Linux the dialog comes from the XDG
//! desktop portal.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::host::{BindRefusal, PluginBinding, ServiceProvider};
use mannie_plugin_sdk::service::{BoundService, Service};
use mannie_plugin_sdk::services::file_dialog::{
    ChosenFile, DialogError, FileDialog, FileDialogApi, FileFilter, FileHandle, MAX_CHOSEN_FILES,
};
use tauri::{AppHandle, Manager};

use crate::window::MAIN_WINDOW;

/// The longest label of a filter, in characters, which the dialog shows.
const MAX_LABEL_CHARACTERS: usize = 100;

/// The longest extension of a filter, in characters.
const MAX_EXTENSION_CHARACTERS: usize = 10;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every critical section only adds, takes or replaces entries.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The desktop app's provider of the service `file-dialog`, which gives each
/// start of a plugin a table of its own.
#[derive(Debug)]
pub(crate) struct FileDialogProvider {
    shared: Arc<Shared>,
}

#[derive(Debug)]
struct Shared {
    app: AppHandle,
    /// The next handle's number. Numbers never repeat while Mannie runs, so
    /// a handle of an earlier start names nothing in a later one.
    next_handle: AtomicU64,
    /// The answer of the next dialog in a validation build: the end-to-end
    /// tests cannot click in a dialog of the system.
    #[cfg(feature = "validation")]
    choice: ValidationChoice,
}

impl FileDialogProvider {
    /// The provider of the dialog over the main window of `app`.
    pub(crate) fn new(app: AppHandle) -> Self {
        Self {
            shared: Arc::new(Shared {
                app,
                next_handle: AtomicU64::new(1),
                #[cfg(feature = "validation")]
                choice: ValidationChoice::default(),
            }),
        }
    }

    /// Sets the files that the next dialog of a validation build gives
    /// instead of opening.
    #[cfg(feature = "validation")]
    pub(crate) fn set_choice(&self, files: Vec<String>) {
        self.shared.choice.set(files);
    }
}

impl ServiceProvider for FileDialogProvider {
    fn name(&self) -> &'static str {
        FileDialog::NAME
    }

    fn major(&self) -> u16 {
        FileDialog::MAJOR
    }

    fn bind(&self, _binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        let dialog: Arc<dyn FileDialogApi> = Arc::new(BoundDialog {
            shared: Arc::clone(&self.shared),
            chosen: Arc::default(),
        });
        Ok(BoundService::new::<FileDialog>(dialog))
    }
}

/// The dialog, bound to one start of one plugin.
#[derive(Debug)]
struct BoundDialog {
    shared: Arc<Shared>,
    /// The files that this binding's dialogs gave.
    chosen: Arc<Mutex<ChosenFiles>>,
}

impl FileDialogApi for BoundDialog {
    fn choose_files(
        &self,
        filter: FileFilter,
    ) -> BoxFuture<'static, Result<Vec<ChosenFile>, DialogError>> {
        let shared = Arc::clone(&self.shared);
        let chosen = Arc::clone(&self.chosen);
        Box::pin(async move {
            check_filter(&filter)?;
            let paths = shared.choose(&filter).await;
            // The paths join the table only once the dialog returned, inside
            // this future, so a call that was dropped leaves no handle.
            let mut table = lock(&chosen);
            let mut files = Vec::with_capacity(paths.len());
            for path in paths {
                let raw = shared.next_handle.fetch_add(1, Ordering::Relaxed);
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                table.add(raw, path);
                files.push(ChosenFile::new(FileHandle::from_raw(raw), name));
            }
            Ok(files)
        })
    }

    fn read(
        &self,
        file: FileHandle,
        max_bytes: u64,
    ) -> BoxFuture<'static, Result<Vec<u8>, DialogError>> {
        // The path leaves the table before the read starts, so a handle
        // reads once.
        let path = lock(&self.chosen).take(file.raw());
        Box::pin(async move {
            let path = path.ok_or(DialogError::UnknownFile)?;
            tauri::async_runtime::spawn_blocking(move || read_chosen(&path, max_bytes))
                .await
                .unwrap_or_else(|_unfinished| {
                    tracing::error!("a read of a chosen file did not finish");
                    Err(DialogError::Unreadable)
                })
        })
    }
}

impl Shared {
    /// The paths that the owner chose in the dialog, none when the owner
    /// closed it.
    async fn choose(&self, filter: &FileFilter) -> Vec<PathBuf> {
        #[cfg(feature = "validation")]
        if let Some(files) = self.choice.take() {
            return files.into_iter().map(PathBuf::from).collect();
        }
        let mut dialog = rfd::AsyncFileDialog::new().add_filter(&filter.label, &filter.extensions);
        if let Some(window) = self.app.get_webview_window(MAIN_WINDOW) {
            dialog = dialog.set_parent(&window);
        }
        dialog
            .pick_files()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|file| file.path().to_path_buf())
            .collect()
    }
}

/// The files that one binding's dialogs gave, by their handles' numbers, at
/// most [`MAX_CHOSEN_FILES`], the oldest first.
#[derive(Debug, Default)]
struct ChosenFiles {
    files: VecDeque<(u64, PathBuf)>,
}

impl ChosenFiles {
    /// Keeps a chosen file, and drops the oldest beyond the limit.
    fn add(&mut self, raw: u64, path: PathBuf) {
        self.files.push_back((raw, path));
        while self.files.len() > MAX_CHOSEN_FILES {
            self.files.pop_front();
        }
    }

    /// Takes the file of a handle out of the table.
    fn take(&mut self, raw: u64) -> Option<PathBuf> {
        let index = self
            .files
            .iter()
            .position(|(number, _path)| *number == raw)?;
        self.files.remove(index).map(|(_number, path)| path)
    }
}

/// Refuses a filter that the dialog should not show: a label that is blank,
/// longer than 100 characters or holds control characters, or an extension
/// that is not 1 to 10 ASCII letters and digits. The plugin checks its label
/// as well; the core checks again, as the dialog shows it, because the label
/// comes from the webview.
fn check_filter(filter: &FileFilter) -> Result<(), DialogError> {
    let label = &filter.label;
    let label_is_valid = !label.trim().is_empty()
        && label.chars().count() <= MAX_LABEL_CHARACTERS
        && !label.chars().any(char::is_control);
    let extensions_are_valid = !filter.extensions.is_empty()
        && filter.extensions.iter().all(|extension| {
            (1..=MAX_EXTENSION_CHARACTERS).contains(&extension.len())
                && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        });
    if label_is_valid && extensions_are_valid {
        Ok(())
    } else {
        Err(DialogError::InvalidFilter)
    }
}

/// Reads a chosen file, refusing one larger than `max_bytes` before reading
/// it: the size comes from the open file, and the read stops one byte past
/// the limit, so a file that grows meanwhile is refused as well (T-32). The
/// log names the kind of failure and never the path.
fn read_chosen(path: &Path, max_bytes: u64) -> Result<Vec<u8>, DialogError> {
    let unreadable = |error: io::Error| {
        tracing::info!(kind = %error.kind(), "a chosen file could not be read");
        DialogError::Unreadable
    };
    let file = File::open(path).map_err(unreadable)?;
    let size = file.metadata().map_err(unreadable)?.len();
    if size > max_bytes {
        return Err(DialogError::TooLarge { max_bytes });
    }
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(unreadable)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_bytes {
        return Err(DialogError::TooLarge { max_bytes });
    }
    Ok(bytes)
}

/// The answer of the next dialog in a validation build, which answers once.
#[cfg(feature = "validation")]
#[derive(Debug, Default)]
struct ValidationChoice {
    files: Mutex<Option<Vec<String>>>,
}

#[cfg(feature = "validation")]
impl ValidationChoice {
    fn set(&self, files: Vec<String>) {
        *lock(&self.files) = Some(files);
    }

    fn take(&self) -> Option<Vec<String>> {
        lock(&self.files).take()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use mannie_plugin_sdk::services::file_dialog::{DialogError, FileFilter, MAX_CHOSEN_FILES};

    use super::{ChosenFiles, check_filter, read_chosen};

    #[test]
    fn a_chosen_file_is_read_within_its_limit_and_refused_one_byte_over() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("photo.png");
        std::fs::write(&path, [7_u8; 10]).unwrap();

        let within = read_chosen(&path, 10);
        let over = read_chosen(&path, 9);

        assert_eq!(within.unwrap(), [7_u8; 10]);
        assert_eq!(over, Err(DialogError::TooLarge { max_bytes: 9 }));
    }

    #[test]
    fn a_large_sparse_file_is_refused_and_a_missing_one_is_unreadable() {
        let folder = tempfile::tempdir().unwrap();
        let large = folder.path().join("large.png");
        std::fs::File::create(&large)
            .unwrap()
            .set_len(21 * 1024 * 1024)
            .unwrap();
        let limit = 20 * 1024 * 1024;

        let too_large = read_chosen(&large, limit);
        let missing = read_chosen(&folder.path().join("gone.png"), limit);

        assert_eq!(too_large, Err(DialogError::TooLarge { max_bytes: limit }));
        assert_eq!(missing, Err(DialogError::Unreadable));
    }

    #[test]
    fn a_handle_reads_once_and_only_in_its_own_table() {
        let mut first = ChosenFiles::default();
        let mut second = ChosenFiles::default();
        first.add(1, PathBuf::from("/photos/a.png"));
        second.add(2, PathBuf::from("/photos/b.png"));

        let other_table = first.take(2);
        let once = first.take(1);
        let twice = first.take(1);

        assert_eq!(other_table, None);
        assert_eq!(once, Some(PathBuf::from("/photos/a.png")));
        assert_eq!(twice, None);
        assert_eq!(second.take(2), Some(PathBuf::from("/photos/b.png")));
    }

    #[test]
    fn the_table_drops_its_oldest_file_beyond_the_limit() {
        let mut table = ChosenFiles::default();
        let count = u64::try_from(MAX_CHOSEN_FILES).unwrap();
        for raw in 0..=count {
            table.add(raw, PathBuf::from(format!("/photos/{raw}.png")));
        }

        assert_eq!(table.files.len(), MAX_CHOSEN_FILES);
        assert_eq!(table.take(0), None);
        assert!(table.take(1).is_some());
    }

    #[test]
    fn a_filter_needs_a_short_label_and_short_ascii_extensions() {
        let valid = FileFilter::new("Images", ["png", "jpeg"]);
        let refused = [
            FileFilter::new("  ", ["png"]),
            FileFilter::new("Images\n", ["png"]),
            FileFilter::new("x".repeat(101), ["png"]),
            FileFilter::new("Images", Vec::<String>::new()),
            FileFilter::new("Images", ["p ng"]),
            FileFilter::new("Images", [".png"]),
            FileFilter::new("Images", ["abcdefghijk"]),
        ];

        assert_eq!(check_filter(&valid), Ok(()));
        for filter in refused {
            assert_eq!(
                check_filter(&filter),
                Err(DialogError::InvalidFilter),
                "{filter:?}"
            );
        }
    }

    #[cfg(feature = "validation")]
    #[test]
    fn a_validation_choice_answers_one_dialog() {
        let choice = super::ValidationChoice::default();
        choice.set(vec!["/photos/a.png".to_owned()]);

        let first = choice.take();
        let second = choice.take();

        assert_eq!(first, Some(vec!["/photos/a.png".to_owned()]));
        assert_eq!(second, None);
    }
}
