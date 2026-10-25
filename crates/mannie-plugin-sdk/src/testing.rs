//! Stand-ins for the core's providers, for the tests of plugins and of the
//! host (feature `testing`; ADR-0034, decision 5). Never enabled in the
//! desktop app or the worker.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures_util::StreamExt as _;
use tokio::sync::{broadcast, watch};

use crate::contracts::Secret;
use crate::host::{BindRefusal, PluginBinding, ServiceProvider};
use crate::manifest::SecretsSection;
use crate::service::{BoundService, Service};
use crate::services::file_dialog::{
    ChosenFile, DialogError, FileDialog, FileDialogApi, FileFilter, FileHandle, MAX_CHOSEN_FILES,
};
use crate::services::models::{
    InputKind, Models, ModelsApi, ModelsError, SearchModel, SearchModelKind, inputs_fit,
};
use crate::services::secrets::{Secrets, SecretsApi, SecretsError};
use crate::{BoxFuture, BoxStream};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every critical section only reads or replaces a value.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The credential store in memory, as the desktop app's provider of
/// `secrets` binds it to each plugin: the plugin names an entry as its
/// manifest lists it, an entry of `entries` is kept as `<id>/<entry>`, a
/// legacy name is kept as it is, an entry of a keyed family as
/// `<id>/<family>/<key>`, and any other name is refused with `NotPermitted`
/// before the store is touched. The desktop app admits only the legacy names
/// of its frozen list, and this store admits every one that a manifest
/// lists.
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
        let api: Arc<dyn SecretsApi> = Arc::new(BoundSecrets {
            store: self.clone(),
            plugin: binding.id.as_str().to_owned(),
            section: binding.manifest.secrets.clone(),
        });
        Ok(BoundService::new::<Secrets>(api))
    }
}

/// The store, bound to one plugin.
#[derive(Debug)]
struct BoundSecrets {
    store: FakeSecrets,
    /// The plugin's ID.
    plugin: String,
    /// The entries that the plugin's manifest gives it.
    section: SecretsSection,
}

impl BoundSecrets {
    /// The name under which the store keeps `entry`. A name that the
    /// manifest does not give the plugin is refused before the store is
    /// touched.
    fn stored_name(&self, entry: &str) -> Result<String, SecretsError> {
        let name = self
            .section
            .stored_name(&self.plugin, entry)
            .ok_or(SecretsError::NotPermitted)?;
        if self.store.inner.unavailable.load(Ordering::SeqCst) {
            return Err(SecretsError::Unavailable);
        }
        Ok(name)
    }
}

impl SecretsApi for BoundSecrets {
    fn load(&self, entry: &str) -> BoxFuture<'static, Result<Option<Secret>, SecretsError>> {
        let loaded = self.stored_name(entry).map(|name| self.store.stored(&name));
        Box::pin(std::future::ready(loaded))
    }

    fn save(&self, entry: &str, secret: Secret) -> BoxFuture<'static, Result<(), SecretsError>> {
        let saved = self
            .stored_name(entry)
            .map(|name| self.store.keep(&name, secret));
        Box::pin(std::future::ready(saved))
    }

    fn delete(&self, entry: &str) -> BoxFuture<'static, Result<(), SecretsError>> {
        let deleted = self.stored_name(entry).map(|name| {
            lock(&self.store.inner.stored).remove(&name);
        });
        Box::pin(std::future::ready(deleted))
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

/// The embedding profile of [`FakeModels`].
pub const FAKE_EMBEDDING_PROFILE: &str = "fake-embeddings";

/// The rerank profile of [`FakeModels`].
pub const FAKE_RERANK_PROFILE: &str = "fake-rerank";

/// The dimensions of the vectors of [`FakeModels`].
pub const FAKE_DIMENSIONS: usize = 64;

/// The lower-case words of a text: its runs of letters and digits.
fn fake_words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
}

/// The vector of a text as [`FakeModels`] embeds it: how often each of its
/// lower-case words occurs, by a hash of the word into [`FAKE_DIMENSIONS`]
/// dimensions, scaled to a length of 1. Texts that share words are near.
pub fn fake_vector(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0_f32; FAKE_DIMENSIONS];
    for word in fake_words(text) {
        // FNV-1a, which keeps the vectors the same in every run.
        let hash = word.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
        let dimension = usize::try_from(hash % FAKE_DIMENSIONS as u64).unwrap_or(0);
        vector[dimension] += 1.0;
    }
    let length = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if length > 0.0 {
        for value in &mut vector {
            *value /= length;
        }
    }
    vector
}

/// The score of a passage as [`FakeModels`] reranks it: the share of the
/// query's distinct words that the passage holds, from 0 to 1.
pub fn fake_score(query: &str, passage: &str) -> f32 {
    let asked: BTreeSet<String> = fake_words(query).collect();
    if asked.is_empty() {
        return 0.0;
    }
    let held: BTreeSet<String> = fake_words(passage).collect();
    let shared = asked.intersection(&held).count();
    // Word counts stay far below the 24 bits of an f32's mantissa.
    #[expect(
        clippy::cast_precision_loss,
        reason = "the counts of a test's words fit an f32 exactly"
    )]
    let share = shared as f32 / asked.len() as f32;
    share
}

/// The models of the search profiles in memory, as the core's provider of
/// `models` binds them to a plugin: the embedding profile
/// [`FAKE_EMBEDDING_PROFILE`], whose vectors are [`fake_vector`]s, and the
/// rerank profile [`FAKE_RERANK_PROFILE`], whose scores are
/// [`fake_score`]s. A test makes the server unreachable, changes the key of
/// the embedding's vectors or removes a profile, which the stream of changes
/// tells, and reads the texts of each call.
///
/// Cloning is cheap; clones share everything.
#[derive(Debug, Clone)]
pub struct FakeModels {
    inner: Arc<ModelsInner>,
}

#[derive(Debug)]
struct ModelsInner {
    profiles: Mutex<Vec<SearchModel>>,
    unreachable: AtomicBool,
    /// The texts of each embedding, with their kind, in order.
    embedded: Mutex<Vec<(InputKind, Vec<String>)>>,
    /// The query and the passages of each rerank, in order.
    reranked: Mutex<Vec<(String, Vec<String>)>>,
    changes: broadcast::Sender<String>,
}

impl Default for FakeModels {
    fn default() -> Self {
        let profiles = vec![
            SearchModel::new(
                FAKE_EMBEDDING_PROFILE,
                "Embeddings",
                "fake-embedding-model",
                SearchModelKind::Embeddings,
                "fake-vectors-1",
            ),
            SearchModel::new(
                FAKE_RERANK_PROFILE,
                "Reranker",
                "fake-rerank-model",
                SearchModelKind::Rerank,
                "fake-scores-1",
            ),
        ];
        let (changes, _receiver) = broadcast::channel(64);
        Self {
            inner: Arc::new(ModelsInner {
                profiles: Mutex::new(profiles),
                unreachable: AtomicBool::new(false),
                embedded: Mutex::default(),
                reranked: Mutex::default(),
                changes,
            }),
        }
    }
}

impl FakeModels {
    /// The embedding profile and the rerank profile, reachable.
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes every call fail as a server that cannot be reached, or work
    /// again.
    pub fn set_unreachable(&self, unreachable: bool) {
        self.inner.unreachable.store(unreachable, Ordering::SeqCst);
    }

    /// Gives a profile's vectors a new key, as a change of its model does,
    /// and tells the change.
    pub fn set_vector_key(&self, profile: &str, key: &str) {
        for model in lock(&self.inner.profiles).iter_mut() {
            if model.id == profile {
                key.clone_into(&mut model.vector_key);
            }
        }
        self.tell(profile);
    }

    /// Deletes a profile, and tells the change.
    pub fn remove_profile(&self, profile: &str) {
        lock(&self.inner.profiles).retain(|model| model.id != profile);
        self.tell(profile);
    }

    /// The texts of each embedding so far, with their kind, in order.
    pub fn embedded(&self) -> Vec<(InputKind, Vec<String>)> {
        lock(&self.inner.embedded).clone()
    }

    /// The query and the passages of each rerank so far, in order.
    pub fn reranked(&self) -> Vec<(String, Vec<String>)> {
        lock(&self.inner.reranked).clone()
    }

    fn tell(&self, profile: &str) {
        // A test without a plugin that follows the changes has no receiver.
        if self.inner.changes.send(profile.to_owned()).is_err() {
            tracing::trace!("no plugin follows the changes of the fake models");
        }
    }

    /// The profile with this ID, if it is of `kind`.
    fn profile(&self, id: &str, kind: SearchModelKind) -> Result<(), ModelsError> {
        if self.inner.unreachable.load(Ordering::SeqCst) {
            return Err(ModelsError::Unreachable);
        }
        let profiles = lock(&self.inner.profiles);
        let model = profiles
            .iter()
            .find(|model| model.id == id)
            .ok_or(ModelsError::ProfileNotFound)?;
        if model.kind != kind {
            return Err(ModelsError::WrongKind);
        }
        Ok(())
    }
}

impl ServiceProvider for FakeModels {
    fn name(&self) -> &'static str {
        Models::NAME
    }

    fn major(&self) -> u16 {
        Models::MAJOR
    }

    fn bind(&self, _binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        let api: Arc<dyn ModelsApi> = Arc::new(self.clone());
        Ok(BoundService::new::<Models>(api))
    }
}

impl ModelsApi for FakeModels {
    fn search_profiles(
        &self,
        kind: SearchModelKind,
    ) -> BoxFuture<'static, Result<Vec<SearchModel>, ModelsError>> {
        let profiles = lock(&self.inner.profiles)
            .iter()
            .filter(|model| model.kind == kind)
            .cloned()
            .collect();
        Box::pin(std::future::ready(Ok(profiles)))
    }

    fn embed(
        &self,
        profile: &str,
        kind: InputKind,
        inputs: Vec<String>,
    ) -> BoxFuture<'static, Result<Vec<Vec<f32>>, ModelsError>> {
        let embedded = self
            .profile(profile, SearchModelKind::Embeddings)
            .and_then(|()| {
                if !inputs_fit(&inputs) {
                    return Err(ModelsError::InvalidInputs);
                }
                let vectors = inputs.iter().map(|input| fake_vector(input)).collect();
                lock(&self.inner.embedded).push((kind, inputs));
                Ok(vectors)
            });
        Box::pin(std::future::ready(embedded))
    }

    fn rerank(
        &self,
        profile: &str,
        query: String,
        documents: Vec<String>,
    ) -> BoxFuture<'static, Result<Vec<f32>, ModelsError>> {
        let reranked = self
            .profile(profile, SearchModelKind::Rerank)
            .and_then(|()| {
                if !inputs_fit(&documents) {
                    return Err(ModelsError::InvalidInputs);
                }
                let scores = documents
                    .iter()
                    .map(|document| fake_score(&query, document))
                    .collect();
                lock(&self.inner.reranked).push((query, documents));
                Ok(scores)
            });
        Box::pin(std::future::ready(reranked))
    }

    fn changes(&self) -> BoxStream<'static, String> {
        futures_util::stream::unfold(self.inner.changes.subscribe(), |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(profile) => return Some((profile, receiver)),
                    Err(broadcast::error::RecvError::Lagged(_missed)) => {}
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        })
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use futures_util::FutureExt;

    use super::{
        FAKE_EMBEDDING_PROFILE, FAKE_RERANK_PROFILE, FakeFile, FakeFileDialog, FakeModels,
        FakeSecrets, fake_score, fake_vector,
    };
    use crate::context::HostProfile;
    use crate::contracts::Secret;
    use crate::host::{PluginBinding, ServiceProvider};
    use crate::ids::PluginId;
    use crate::manifest::{CratesSection, Manifest, NativeSection, RustSection, SecretsSection};
    use crate::services::file_dialog::{DialogError, FileFilter, MAX_CHOSEN_FILES};
    use crate::services::models::{InputKind, ModelsApi, ModelsError, SearchModelKind};
    use crate::services::secrets::{Secrets, SecretsApi, SecretsError};
    use futures_util::StreamExt as _;

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
                keyed: vec!["host".to_owned()],
            },
            crates: CratesSection::default(),
            programs: Vec::new(),
            processes: BTreeMap::new(),
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
    fn an_entry_of_a_keyed_family_is_kept_below_the_plugins_id_until_the_plugin_deletes_it() {
        let store = FakeSecrets::new();
        let notes = bound(&store, "notes");

        notes
            .save("host/box-1", secret("box password"))
            .now_or_never()
            .unwrap()
            .unwrap();
        let kept = store.stored("notes/host/box-1");
        notes.delete("host/box-1").now_or_never().unwrap().unwrap();
        // Deleting what the store does not hold succeeds.
        notes.delete("host/box-1").now_or_never().unwrap().unwrap();
        let refused = notes.delete("host/box_1").now_or_never().unwrap();

        assert_eq!(kept.unwrap().expose(), "box password");
        assert!(store.stored("notes/host/box-1").is_none());
        assert_eq!(refused.unwrap_err(), SecretsError::NotPermitted);
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
    fn the_fake_models_embed_words_near_each_other_and_score_shared_words() {
        let models = FakeModels::new();
        let near = |left: &[f32], right: &[f32]| -> f32 {
            left.iter()
                .zip(right)
                .map(|(left, right)| left * right)
                .sum()
        };

        let vectors = models
            .embed(
                FAKE_EMBEDDING_PROFILE,
                InputKind::Document,
                vec![
                    "Light bends in water".to_owned(),
                    "Bread needs flour".to_owned(),
                ],
            )
            .now_or_never()
            .unwrap()
            .unwrap();
        let query = fake_vector("why does light bend in water");
        let scores = models
            .rerank(
                FAKE_RERANK_PROFILE,
                "light water".to_owned(),
                vec!["Light bends in water".to_owned(), "Bread".to_owned()],
            )
            .now_or_never()
            .unwrap()
            .unwrap();

        assert!(near(&query, &vectors[0]) > near(&query, &vectors[1]));
        assert_eq!(scores, [1.0, 0.0]);
        assert!((fake_score("a b", "a c") - 0.5).abs() < f32::EPSILON);
        assert_eq!(models.embedded().len(), 1);
        assert_eq!(models.reranked()[0].0, "light water");
    }

    #[test]
    fn the_fake_models_refuse_another_kind_an_unknown_profile_and_an_unreachable_server() {
        let models = FakeModels::new();
        let embed = |profile: &str| {
            models
                .embed(profile, InputKind::Query, vec!["light".to_owned()])
                .now_or_never()
                .unwrap()
        };

        let wrong = embed(FAKE_RERANK_PROFILE);
        let unknown = embed("nobody");
        let empty = models
            .embed(FAKE_EMBEDDING_PROFILE, InputKind::Query, Vec::new())
            .now_or_never()
            .unwrap();
        models.set_unreachable(true);
        let unreachable = embed(FAKE_EMBEDDING_PROFILE);

        assert_eq!(wrong, Err(ModelsError::WrongKind));
        assert_eq!(unknown, Err(ModelsError::ProfileNotFound));
        assert_eq!(empty, Err(ModelsError::InvalidInputs));
        assert_eq!(unreachable, Err(ModelsError::Unreachable));
    }

    #[test]
    fn a_change_of_the_fake_models_reaches_their_stream() {
        let models = FakeModels::new();
        let mut changes = models.changes();

        models.set_vector_key(FAKE_EMBEDDING_PROFILE, "fake-vectors-2");
        let told = changes.next().now_or_never().flatten();
        let profiles = models
            .search_profiles(SearchModelKind::Embeddings)
            .now_or_never()
            .unwrap()
            .unwrap();

        assert_eq!(told.as_deref(), Some(FAKE_EMBEDDING_PROFILE));
        assert_eq!(profiles[0].vector_key, "fake-vectors-2");
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
