//! Stand-ins for the core's providers, for the tests of plugins and of the
//! host (feature `testing`; ADR-0034, decision 5). Never enabled in the
//! desktop app or the worker.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::watch;

use crate::BoxFuture;
use crate::contracts::Secret;
use crate::host::{BindRefusal, PluginBinding, ServiceProvider};
use crate::service::{BoundService, Service};
use crate::services::file_dialog::{
    ChosenFile, DialogError, FileDialog, FileDialogApi, FileFilter, FileHandle, MAX_CHOSEN_FILES,
};
use crate::services::secrets::{Secrets, SecretsApi, SecretsError};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every critical section only reads or replaces a value.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The credential store in memory, as the desktop app's provider of
/// `secrets` binds it to each plugin: the plugin names an entry as its
/// manifest lists it, an entry of `entries` is kept as `<id>/<entry>`, a
/// legacy name is kept as it is, and any other name is refused with
/// `NotPermitted` before the store is touched. The desktop app admits only
/// the legacy names of its frozen list, and this store admits every one that
/// a manifest lists.
///
/// Cloning is cheap; clones share the entries.
#[derive(Debug, Clone, Default)]
pub struct FakeSecrets {
    inner: Arc<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// The secrets, by the names under which the store keeps them.
    stored: Mutex<BTreeMap<String, Secret>>,
    unavailable: AtomicBool,
}

impl FakeSecrets {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// The secret that the store keeps under `name`, as the store names it,
    /// such as `notes/token`.
    pub fn stored(&self, name: &str) -> Option<Secret> {
        lock(&self.inner.stored).get(name).cloned()
    }

    /// Keeps `secret` under `name`, as the store names it, such as an entry
    /// that an older version left.
    pub fn keep(&self, name: &str, secret: Secret) {
        lock(&self.inner.stored).insert(name.to_owned(), secret);
    }

    /// Makes the store unavailable to every plugin, or available again.
    pub fn set_unavailable(&self, unavailable: bool) {
        self.inner.unavailable.store(unavailable, Ordering::SeqCst);
    }
}

impl ServiceProvider for FakeSecrets {
    fn name(&self) -> &'static str {
        Secrets::NAME
    }

    fn major(&self) -> u16 {
        Secrets::MAJOR
    }

    fn bind(&self, binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        let listed = &binding.manifest.secrets;
        let mut names = BTreeMap::new();
        for entry in &listed.entries {
            names.insert(entry.clone(), format!("{}/{entry}", binding.id));
        }
        for legacy in &listed.legacy {
            names.insert(legacy.clone(), legacy.clone());
        }
        let api: Arc<dyn SecretsApi> = Arc::new(BoundSecrets {
            store: self.clone(),
            names,
        });
        Ok(BoundService::new::<Secrets>(api))
    }
}

/// The store, bound to one plugin.
#[derive(Debug)]
struct BoundSecrets {
    store: FakeSecrets,
    /// The name under which the store keeps each entry that the manifest
    /// lists, by the name that the plugin passes.
    names: BTreeMap<String, String>,
}

impl BoundSecrets {
    /// The name under which the store keeps `entry`. A name that the
    /// manifest does not list is refused before the store is touched.
    fn stored_name(&self, entry: &str) -> Result<&str, SecretsError> {
        let name = self.names.get(entry).ok_or(SecretsError::NotPermitted)?;
        if self.store.inner.unavailable.load(Ordering::SeqCst) {
            return Err(SecretsError::Unavailable);
        }
        Ok(name)
    }
}

impl SecretsApi for BoundSecrets {
    fn load(&self, entry: &str) -> BoxFuture<'static, Result<Option<Secret>, SecretsError>> {
        let loaded = self.stored_name(entry).map(|name| self.store.stored(name));
        Box::pin(std::future::ready(loaded))
    }

    fn save(&self, entry: &str, secret: Secret) -> BoxFuture<'static, Result<(), SecretsError>> {
        let saved = self
            .stored_name(entry)
            .map(|name| self.store.keep(name, secret));
        Box::pin(std::future::ready(saved))
    }
}

/// A file that the fake file dialog gives: its name, and its bytes, or none
/// for a file that cannot be read.
#[derive(Debug, Clone)]
pub struct FakeFile {
    name: String,
    bytes: Option<Vec<u8>>,
}

impl FakeFile {
    /// A file named `name` that holds `bytes`.
    pub fn new(name: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            name: name.into(),
            bytes: Some(bytes),
        }
    }

    /// A file named `name` that cannot be read, such as one that was
    /// deleted after the owner chose it.
    pub fn unreadable(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            bytes: None,
        }
    }
}

/// The system's file dialog in memory, as the desktop app binds it to each
/// plugin: each binding keeps a table of its own of the files that its
/// dialogs gave, a handle reads once, and a file larger than a read's limit
/// is refused without its bytes. A test sets what the next dialog answers;
/// a dialog without an answer was closed.
///
/// Cloning is cheap; clones share everything.
#[derive(Debug, Clone)]
pub struct FakeFileDialog {
    inner: Arc<DialogInner>,
}

#[derive(Debug)]
struct DialogInner {
    /// What the next dialogs answer, in order.
    answers: Mutex<VecDeque<Vec<FakeFile>>>,
    fail_next: AtomicBool,
    /// Whether dialogs and reads wait, until a gate opens.
    held: watch::Sender<bool>,
    opened: AtomicU32,
    filters: Mutex<Vec<FileFilter>>,
    /// Each read as it started: the file's name and the read's limit.
    reads: Mutex<Vec<(String, u64)>>,
    next_handle: AtomicU64,
}

impl Default for FakeFileDialog {
    fn default() -> Self {
        Self {
            inner: Arc::new(DialogInner {
                answers: Mutex::default(),
                fail_next: AtomicBool::new(false),
                held: watch::Sender::new(false),
                opened: AtomicU32::new(0),
                filters: Mutex::default(),
                reads: Mutex::default(),
                next_handle: AtomicU64::new(1),
            }),
        }
    }
}

/// Holds the fake dialog's dialogs and reads until it opens, or until it is
/// dropped.
#[derive(Debug)]
pub struct Gate {
    dialog: FakeFileDialog,
}

impl Gate {
    /// Lets the held dialogs and reads go on.
    pub fn open(self) {
        drop(self);
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        self.dialog.inner.held.send_replace(false);
    }
}

impl FakeFileDialog {
    /// A dialog that the owner closes until a test answers it.
    pub fn new() -> Self {
        Self::default()
    }

    /// The next dialog gives `files`.
    pub fn answer(&self, files: Vec<FakeFile>) {
        lock(&self.inner.answers).push_back(files);
    }

    /// The next dialog fails to show.
    pub fn fail_next(&self) {
        self.inner.fail_next.store(true, Ordering::SeqCst);
    }

    /// Holds every dialog and read until the gate opens.
    pub fn hold(&self) -> Gate {
        self.inner.held.send_replace(true);
        Gate {
            dialog: self.clone(),
        }
    }

    /// How many dialogs opened.
    pub fn opened(&self) -> u32 {
        self.inner.opened.load(Ordering::SeqCst)
    }

    /// The filters that the dialogs showed, in order.
    pub fn filters(&self) -> Vec<FileFilter> {
        lock(&self.inner.filters).clone()
    }

    /// Each read as it started, by the file's name and the read's limit.
    pub fn reads(&self) -> Vec<(String, u64)> {
        lock(&self.inner.reads).clone()
    }

    /// The dialog bound as for one start of a plugin, for the tests of a
    /// plugin's parts.
    pub fn api(&self) -> Arc<dyn FileDialogApi> {
        Arc::new(BoundDialog {
            dialog: self.clone(),
            chosen: Arc::default(),
        })
    }

    async fn wait_until_open(&self) {
        let mut held = self.inner.held.subscribe();
        // The fake owns the sender, so the wait ends only when the gate opens.
        drop(held.wait_for(|held| !*held).await);
    }
}

impl ServiceProvider for FakeFileDialog {
    fn name(&self) -> &'static str {
        FileDialog::NAME
    }

    fn major(&self) -> u16 {
        FileDialog::MAJOR
    }

    fn bind(&self, _binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        Ok(BoundService::new::<FileDialog>(self.api()))
    }
}

/// The fake dialog, bound to one start of one plugin.
#[derive(Debug)]
struct BoundDialog {
    dialog: FakeFileDialog,
    /// The files that this binding's dialogs gave, by their handles'
    /// numbers, the oldest first.
    chosen: Arc<Mutex<VecDeque<(u64, FakeFile)>>>,
}

impl FileDialogApi for BoundDialog {
    fn choose_files(
        &self,
        filter: FileFilter,
    ) -> BoxFuture<'static, Result<Vec<ChosenFile>, DialogError>> {
        let dialog = self.dialog.clone();
        let chosen = Arc::clone(&self.chosen);
        Box::pin(async move {
            dialog.inner.opened.fetch_add(1, Ordering::SeqCst);
            lock(&dialog.inner.filters).push(filter);
            dialog.wait_until_open().await;
            if dialog.inner.fail_next.swap(false, Ordering::SeqCst) {
                return Err(DialogError::Failed);
            }
            let files = lock(&dialog.inner.answers).pop_front().unwrap_or_default();
            // As the desktop's provider, the files join the table only once
            // the dialog returned.
            let mut table = lock(&chosen);
            let mut answered = Vec::with_capacity(files.len());
            for file in files {
                let raw = dialog.inner.next_handle.fetch_add(1, Ordering::SeqCst);
                answered.push(ChosenFile::new(
                    FileHandle::from_raw(raw),
                    file.name.clone(),
                ));
                table.push_back((raw, file));
                while table.len() > MAX_CHOSEN_FILES {
                    table.pop_front();
                }
            }
            Ok(answered)
        })
    }

    fn read(
        &self,
        file: FileHandle,
        max_bytes: u64,
    ) -> BoxFuture<'static, Result<Vec<u8>, DialogError>> {
        let taken = {
            let mut table = lock(&self.chosen);
            table
                .iter()
                .position(|(raw, _file)| *raw == file.raw())
                .and_then(|index| table.remove(index))
        };
        let dialog = self.dialog.clone();
        Box::pin(async move {
            let Some((_raw, file)) = taken else {
                return Err(DialogError::UnknownFile);
            };
            lock(&dialog.inner.reads).push((file.name.clone(), max_bytes));
            dialog.wait_until_open().await;
            let bytes = file.bytes.ok_or(DialogError::Unreadable)?;
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_bytes {
                return Err(DialogError::TooLarge { max_bytes });
            }
            Ok(bytes)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use futures_util::FutureExt;

    use super::{FakeFile, FakeFileDialog, FakeSecrets};
    use crate::context::HostProfile;
    use crate::contracts::Secret;
    use crate::host::{PluginBinding, ServiceProvider};
    use crate::ids::PluginId;
    use crate::manifest::{CratesSection, Manifest, NativeSection, RustSection, SecretsSection};
    use crate::services::file_dialog::{DialogError, FileFilter, MAX_CHOSEN_FILES};
    use crate::services::secrets::{Secrets, SecretsApi, SecretsError};

    fn manifest(id: &PluginId) -> Manifest {
        Manifest {
            id: id.clone(),
            sdk: 1,
            order: 10,
            rust: RustSection {
                entry: format!("mannie-{id}"),
            },
            services: BTreeMap::from([("secrets".to_owned(), 1)]),
            secrets: SecretsSection {
                entries: vec!["token".to_owned()],
                legacy: vec!["link/master-key".to_owned()],
            },
            crates: CratesSection::default(),
            programs: Vec::new(),
            native: NativeSection::default(),
        }
    }

    fn bound(store: &FakeSecrets, id: &str) -> std::sync::Arc<dyn SecretsApi> {
        let id = PluginId::parse(id).unwrap();
        let manifest = manifest(&id);
        store
            .bind(&PluginBinding::new(&id, &manifest, HostProfile::Release))
            .unwrap()
            .get::<Secrets>()
            .unwrap()
    }

    fn secret(value: &str) -> Secret {
        Secret::new(value.to_owned())
    }

    #[test]
    fn an_entry_is_kept_under_the_plugins_id_and_a_legacy_name_as_it_is() {
        let store = FakeSecrets::new();
        let notes = bound(&store, "notes");

        notes
            .save("token", secret("notes token"))
            .now_or_never()
            .unwrap()
            .unwrap();
        notes
            .save("link/master-key", secret("old key"))
            .now_or_never()
            .unwrap()
            .unwrap();
        let loaded = notes.load("token").now_or_never().unwrap().unwrap();

        assert_eq!(loaded.unwrap().expose(), "notes token");
        assert_eq!(store.stored("notes/token").unwrap().expose(), "notes token");
        assert_eq!(store.stored("link/master-key").unwrap().expose(), "old key");
        assert!(store.stored("token").is_none());
    }

    #[test]
    fn two_plugins_never_share_an_entry() {
        let store = FakeSecrets::new();
        let notes = bound(&store, "notes");
        let journal = bound(&store, "journal");

        notes
            .save("token", secret("notes token"))
            .now_or_never()
            .unwrap()
            .unwrap();
        let other = journal.load("token").now_or_never().unwrap().unwrap();

        assert!(other.is_none());
    }

    #[test]
    fn a_name_that_the_manifest_does_not_list_is_refused_before_the_store_is_touched() {
        let store = FakeSecrets::new();
        let notes = bound(&store, "notes");
        store.set_unavailable(true);

        let unlisted = notes.load("notes/token").now_or_never().unwrap();
        let unavailable = notes.load("token").now_or_never().unwrap();
        let refused = notes
            .save("provider/p/1", secret("credential"))
            .now_or_never()
            .unwrap();

        assert_eq!(unlisted.unwrap_err(), SecretsError::NotPermitted);
        assert_eq!(unavailable.unwrap_err(), SecretsError::Unavailable);
        assert_eq!(refused, Err(SecretsError::NotPermitted));
        assert!(store.stored("provider/p/1").is_none());
    }

    fn images() -> FileFilter {
        FileFilter::new("Images", ["png"])
    }

    #[test]
    fn a_chosen_file_reads_once_within_its_limit_and_only_in_its_binding() {
        let fake = FakeFileDialog::new();
        let first = fake.api();
        let second = fake.api();
        fake.answer(vec![
            FakeFile::new("a.png", vec![1, 2, 3]),
            FakeFile::new("large.png", vec![0; 10]),
            FakeFile::unreadable("gone.png"),
        ]);

        let mut chosen = first
            .choose_files(images())
            .now_or_never()
            .unwrap()
            .unwrap();
        let gone = chosen.pop().unwrap();
        let large = chosen.pop().unwrap();
        let photo = chosen.pop().unwrap();
        let names = [&photo.name, &large.name, &gone.name];
        // A handle of another binding names nothing in this one.
        let elsewhere = second.read(photo.file, 100).now_or_never().unwrap();

        assert_eq!(names, ["a.png", "large.png", "gone.png"]);
        assert_eq!(elsewhere, Err(DialogError::UnknownFile));
        assert_eq!(fake.opened(), 1);
        assert_eq!(fake.filters(), [images()]);
        let too_large = first.read(large.file, 9).now_or_never().unwrap();
        let unreadable = first.read(gone.file, 100).now_or_never().unwrap();
        assert_eq!(too_large, Err(DialogError::TooLarge { max_bytes: 9 }));
        assert_eq!(unreadable, Err(DialogError::Unreadable));
        assert_eq!(
            fake.reads(),
            [("large.png".to_owned(), 9), ("gone.png".to_owned(), 100)]
        );
    }

    #[test]
    fn a_handle_reads_once_and_a_failed_or_closed_dialog_gives_nothing() {
        let fake = FakeFileDialog::new();
        let dialog = fake.api();
        fake.answer(vec![FakeFile::new("a.png", vec![1])]);
        let mut chosen = dialog
            .choose_files(images())
            .now_or_never()
            .unwrap()
            .unwrap();
        let file = chosen.pop().unwrap();
        fake.fail_next();

        let read = dialog.read(file.file, 10).now_or_never().unwrap();
        let failed = dialog.choose_files(images()).now_or_never().unwrap();
        let closed = dialog.choose_files(images()).now_or_never().unwrap();

        assert_eq!(read, Ok(vec![1]));
        assert_eq!(failed.unwrap_err(), DialogError::Failed);
        assert_eq!(closed.unwrap().len(), 0);
        assert_eq!(fake.opened(), 3);
    }

    #[test]
    fn a_binding_keeps_its_newest_chosen_files() {
        let fake = FakeFileDialog::new();
        let dialog = fake.api();
        let files = (0..=MAX_CHOSEN_FILES)
            .map(|number| FakeFile::new(format!("{number}.png"), vec![1]))
            .collect();
        fake.answer(files);

        let mut chosen = dialog
            .choose_files(images())
            .now_or_never()
            .unwrap()
            .unwrap();
        let newest = chosen.pop().unwrap();
        let oldest = chosen.remove(0);

        assert_eq!(
            dialog.read(oldest.file, 10).now_or_never().unwrap(),
            Err(DialogError::UnknownFile)
        );
        assert_eq!(
            dialog.read(newest.file, 10).now_or_never().unwrap(),
            Ok(vec![1])
        );
    }

    #[test]
    fn a_held_dialog_answers_once_the_gate_opens() {
        let fake = FakeFileDialog::new();
        let dialog = fake.api();
        fake.answer(vec![FakeFile::new("a.png", vec![1])]);
        let gate = fake.hold();

        let mut choosing = dialog.choose_files(images());
        let while_held = (&mut choosing).now_or_never();
        gate.open();
        let chosen = choosing.now_or_never().unwrap().unwrap();

        assert!(while_held.is_none(), "the dialog waits while it is held");
        assert_eq!(chosen.len(), 1);
    }
}
