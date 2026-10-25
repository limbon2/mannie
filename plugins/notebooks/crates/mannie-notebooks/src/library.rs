//! The plugin while it runs (plugins/notebooks/docs/README.md): the
//! notebooks and their sources as the store keeps them, read again after
//! each change and followed by the channel `follow_notebooks`; the bytes of
//! the files that wait to be read, which only memory keeps; the embedding
//! model's current key; and the corpora of the notebooks that were searched.
//!
//! Every write goes to the store first; the library then reads the notebooks
//! and sources again, so what the channel sends is what the store holds.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use mannie_notebooks_documents::DocumentKind;
use mannie_plugin_sdk::context::HostProfile;
use mannie_plugin_sdk::contracts::{Clock, UtcMillis};
use mannie_plugin_sdk::services::file_dialog::{FileDialogApi, FileFilter};
use mannie_plugin_sdk::services::models::{ModelsApi, ModelsError, SearchModel, SearchModelKind};
use sha2::{Digest, Sha256};
use tokio::sync::{Notify, watch};

use crate::commands::{
    AddedSources, EXTENSIONS, MAX_SECTIONS_PER_READ, NotebookList, NotebookView, NotebooksError,
    PassagePlace, Preferences, ProfileOption, RefusedFile, SearchProfiles, SectionText, SourceKind,
    SourceRefusal, SourceState, SourceText, SourceView, UploadStarted,
};
use crate::search::Corpus;
use crate::store::{
    DUPLICATE, NAME_TAKEN, NewSource, NotebookRecord, NotebookStore, SourceRecord, WriteFailure,
};
use crate::uploads::Uploads;

/// The most memory that the corpora of searched notebooks take together.
const MAX_CORPUS_BYTES: usize = 256 * 1024 * 1024;

/// The waits before a source whose server failed is tried again, in a build
/// that Mannie ships (plugins/notebooks/docs/README.md, indexing).
const RETRY_DELAYS: [Duration; 5] = [
    Duration::from_mins(1),
    Duration::from_mins(2),
    Duration::from_mins(4),
    Duration::from_mins(8),
    Duration::from_mins(15),
];

/// The waits of a validation build, which the end-to-end tests drive.
const QUICK_RETRY_DELAYS: [Duration; 5] = [
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(150),
    Duration::from_millis(200),
    Duration::from_millis(250),
];

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every critical section only reads or replaces values.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The store's failure as the plugin's error, with a log line.
pub(crate) fn storage_failed(error: impl std::fmt::Display) -> NotebooksError {
    tracing::warn!(%error, "the notebooks could not be read or saved");
    NotebooksError::StorageFailed
}

/// The hexadecimal SHA-256 of a file's bytes.
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            // Writing to a String cannot fail.
            let _written = write!(hex, "{byte:02x}");
            hex
        })
}

fn source_kind(kind: DocumentKind) -> SourceKind {
    match kind {
        DocumentKind::Pdf => SourceKind::Pdf,
        DocumentKind::Markdown => SourceKind::Markdown,
        DocumentKind::Text => SourceKind::Text,
    }
}

/// A source as the UI shows it.
pub(crate) fn source_view(source: &SourceRecord) -> SourceView {
    SourceView {
        id: source.id.clone(),
        notebook_id: source.notebook_id.clone(),
        kind: source_kind(source.kind),
        name: source.name.clone(),
        bytes: u32::try_from(source.bytes).unwrap_or(u32::MAX),
        pages: source.pages,
        state: source.state,
        failure: source.failure.clone(),
        included: source.included,
        passages: source.passages,
        embedded: source.embedded,
        added_at: UtcMillis::from_millis(source.added_at),
        indexed_at: source.indexed_at.map(UtcMillis::from_millis),
    }
}

/// A file's name as the plugin keeps it: without folders, control
/// characters or white space at its ends, and at most 200 characters.
fn clean_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    base.chars()
        .filter(|character| !character.is_control())
        .take(200)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// How long the plugin waits before it tries a failed server again.
#[derive(Debug, Clone)]
pub(crate) struct Timing {
    pub(crate) retry_delays: Vec<Duration>,
}

impl Timing {
    /// The waits of a host of `profile`: short ones in a validation build.
    pub(crate) fn for_host(profile: HostProfile) -> Self {
        let delays = if profile == HostProfile::Validation {
            QUICK_RETRY_DELAYS
        } else {
            RETRY_DELAYS
        };
        Self {
            retry_delays: delays.to_vec(),
        }
    }
}

/// The notebooks and sources as the store holds them, with what searches and
/// the indexer need to know of the models.
#[derive(Debug, Default)]
struct Mirror {
    notebooks: Vec<NotebookRecord>,
    sources: Vec<SourceRecord>,
    preferences: Preferences,
    /// The key of the vectors of the embedding profile that the preferences
    /// name, or `None` without one, or when it is gone.
    embedding_key: Option<String>,
}

/// The corpora of searched notebooks, by notebook and the key of the vectors
/// that they hold, with when each was used last.
#[derive(Debug, Default)]
struct Corpora {
    entries: Vec<CorpusEntry>,
    uses: u64,
}

#[derive(Debug)]
struct CorpusEntry {
    notebook: String,
    key: String,
    corpus: Arc<Corpus>,
    used: u64,
}

/// The plugin's state while it runs, which its handlers, its indexer and its
/// context provider share.
#[derive(Debug)]
pub(crate) struct Library {
    pub(crate) store: NotebookStore,
    pub(crate) models: Arc<dyn ModelsApi>,
    dialog: Arc<dyn FileDialogApi>,
    clock: Arc<dyn Clock>,
    pub(crate) timing: Timing,
    mirror: Mutex<Mirror>,
    /// Counts the changes, which the channel follows.
    revisions: watch::Sender<u64>,
    /// Wakes the indexer.
    pub(crate) wake: Notify,
    /// The bytes of the files that wait to be read, by source.
    pending: Mutex<HashMap<String, Vec<u8>>>,
    /// The dropped files whose parts are arriving.
    uploads: Uploads,
    corpora: Mutex<Corpora>,
}

impl Library {
    /// The library of a start: the sources that were being read fail as
    /// interrupted, and the sources follow the embedding model.
    pub(crate) async fn load(
        store: NotebookStore,
        models: Arc<dyn ModelsApi>,
        dialog: Arc<dyn FileDialogApi>,
        clock: Arc<dyn Clock>,
        timing: Timing,
    ) -> Result<Arc<Self>, NotebooksError> {
        let interrupted = store
            .fail_interrupted()
            .await
            .map_err(|failure| storage_failed(format!("{failure:?}")))?;
        if interrupted > 0 {
            tracing::info!(
                interrupted,
                "sources that were being read when the plugin stopped failed"
            );
        }
        let preferences = store
            .preferences()
            .await
            .map_err(storage_failed)?
            .unwrap_or_default();
        let library = Arc::new(Self {
            store,
            models,
            dialog,
            clock,
            timing,
            mirror: Mutex::new(Mirror {
                preferences,
                ..Mirror::default()
            }),
            revisions: watch::Sender::new(0),
            wake: Notify::new(),
            pending: Mutex::default(),
            uploads: Uploads::default(),
            corpora: Mutex::default(),
        });
        library.follow_models().await?;
        Ok(library)
    }

    /// The time now, in milliseconds since the Unix epoch.
    pub(crate) fn now(&self) -> i64 {
        self.clock.now().as_millis()
    }

    /// Reads the notebooks and sources again and tells the channels.
    pub(crate) async fn refresh(&self) -> Result<(), NotebooksError> {
        let notebooks = self.store.notebooks().await.map_err(storage_failed)?;
        let sources = self.store.sources().await.map_err(storage_failed)?;
        {
            let mut mirror = lock(&self.mirror);
            mirror.notebooks = notebooks;
            mirror.sources = sources;
        }
        self.revisions.send_modify(|revision| *revision += 1);
        Ok(())
    }

    /// The changes of the notebooks, which a channel follows.
    pub(crate) fn follow(&self) -> watch::Receiver<u64> {
        self.revisions.subscribe()
    }

    /// Every notebook with its sources, the newest first.
    pub(crate) fn list(&self) -> NotebookList {
        let mirror = lock(&self.mirror);
        let notebooks = mirror
            .notebooks
            .iter()
            .map(|notebook| NotebookView {
                id: notebook.id.clone(),
                name: notebook.name.clone(),
                sources: mirror
                    .sources
                    .iter()
                    .filter(|source| source.notebook_id == notebook.id)
                    .map(source_view)
                    .collect(),
                created_at: UtcMillis::from_millis(notebook.created_at),
                updated_at: UtcMillis::from_millis(notebook.updated_at),
            })
            .collect();
        NotebookList { notebooks }
    }

    fn notebook(&self, id: &str) -> Option<NotebookRecord> {
        lock(&self.mirror)
            .notebooks
            .iter()
            .find(|notebook| notebook.id == id)
            .cloned()
    }

    pub(crate) fn source(&self, id: &str) -> Option<SourceRecord> {
        lock(&self.mirror)
            .sources
            .iter()
            .find(|source| source.id == id)
            .cloned()
    }

    /// The sources of the mirror, the oldest first.
    pub(crate) fn sources(&self) -> Vec<SourceRecord> {
        lock(&self.mirror).sources.clone()
    }

    /// The notebook's name, or `None` for one that does not exist.
    pub(crate) fn notebook_name(&self, id: &str) -> Option<String> {
        self.notebook(id).map(|notebook| notebook.name)
    }

    pub(crate) fn preferences(&self) -> Preferences {
        lock(&self.mirror).preferences.clone()
    }

    /// The embedding profile and the key of its vectors, while the
    /// preferences name one that exists.
    pub(crate) fn embedding(&self) -> Option<(String, String)> {
        let mirror = lock(&self.mirror);
        let profile = mirror.preferences.embedding_profile_id.clone()?;
        let key = mirror.embedding_key.clone()?;
        Some((profile, key))
    }

    fn notebook_view(&self, id: &str) -> Result<NotebookView, NotebooksError> {
        self.list()
            .notebooks
            .into_iter()
            .find(|notebook| notebook.id == id)
            .ok_or(NotebooksError::NotebookNotFound)
    }

    fn source_or_missing(&self, id: &str) -> Result<SourceRecord, NotebooksError> {
        self.source(id).ok_or(NotebooksError::SourceNotFound)
    }

    pub(crate) async fn create_notebook(&self, name: &str) -> Result<NotebookView, NotebooksError> {
        let now = self.now();
        let notebook = NotebookRecord {
            id: uuid::Uuid::now_v7().to_string(),
            name: name.trim().to_owned(),
            created_at: now,
            updated_at: now,
        };
        match self.store.insert_notebook(&notebook).await {
            Ok(()) => {}
            Err(WriteFailure::Constraint(NAME_TAKEN)) => return Err(NotebooksError::NameTaken),
            Err(failure) => return Err(storage_failed(format!("{failure:?}"))),
        }
        tracing::info!(notebook = %notebook.id, "a notebook was created");
        self.refresh().await?;
        self.notebook_view(&notebook.id)
    }

    pub(crate) async fn rename_notebook(
        &self,
        id: &str,
        name: &str,
    ) -> Result<NotebookView, NotebooksError> {
        match self
            .store
            .rename_notebook(id, name.trim(), self.now())
            .await
        {
            Ok(()) => {}
            Err(WriteFailure::Constraint(NAME_TAKEN)) => return Err(NotebooksError::NameTaken),
            Err(WriteFailure::Gone) => return Err(NotebooksError::NotebookNotFound),
            Err(failure) => return Err(storage_failed(format!("{failure:?}"))),
        }
        self.refresh().await?;
        self.notebook_view(id)
    }

    pub(crate) async fn delete_notebook(&self, id: &str) -> Result<(), NotebooksError> {
        let sources: Vec<String> = lock(&self.mirror)
            .sources
            .iter()
            .filter(|source| source.notebook_id == id)
            .map(|source| source.id.clone())
            .collect();
        let deleted = self
            .store
            .delete_notebook(id)
            .await
            .map_err(|failure| storage_failed(format!("{failure:?}")))?;
        if !deleted {
            return Err(NotebooksError::NotebookNotFound);
        }
        {
            let mut pending = lock(&self.pending);
            for source in &sources {
                pending.remove(source);
            }
        }
        self.uploads.forget_notebook(id);
        self.forget_corpus(id);
        tracing::info!(notebook = %id, sources = sources.len(), "a notebook was deleted");
        self.refresh().await
    }

    /// Shows the system's file dialog and adds the files that the owner chose
    /// to a notebook, each read once by its handle.
    pub(crate) async fn pick_sources(
        &self,
        notebook_id: &str,
        filter_name: &str,
    ) -> Result<AddedSources, NotebooksError> {
        if self.notebook(notebook_id).is_none() {
            return Err(NotebooksError::NotebookNotFound);
        }
        let filter = FileFilter::new(filter_name.trim(), EXTENSIONS.iter().copied());
        let chosen = self.dialog.choose_files(filter).await.map_err(|error| {
            tracing::warn!(error.code = error.code(), "the file dialog failed");
            NotebooksError::DialogFailed
        })?;
        let mut added = Vec::new();
        let mut refused = Vec::new();
        for file in chosen {
            let name = clean_name(&file.name);
            // A file of any size is read (plugin ADR-0002).
            let outcome = match self.dialog.read(file.file, u64::MAX).await {
                Ok(bytes) => self.add_file(notebook_id, &name, bytes).await,
                Err(error) => {
                    tracing::info!(error.code = error.code(), "a chosen file could not be read");
                    Err(NotebooksError::Refused {
                        reason: SourceRefusal::Unreadable,
                    })
                }
            };
            match outcome {
                Ok(source) => added.push(source),
                Err(NotebooksError::Refused { reason }) => {
                    refused.push(RefusedFile { name, reason });
                }
                Err(other) => return Err(other),
            }
        }
        Ok(AddedSources { added, refused })
    }

    /// Adds a file's bytes to a notebook as a source that waits to be read:
    /// the kind follows its extension, and a file that the notebook holds,
    /// byte for byte, is refused.
    pub(crate) async fn add_file(
        &self,
        notebook_id: &str,
        name: &str,
        bytes: Vec<u8>,
    ) -> Result<SourceView, NotebooksError> {
        let name = clean_name(name);
        let kind = DocumentKind::from_file_name(&name).ok_or(NotebooksError::Refused {
            reason: SourceRefusal::UnsupportedType,
        })?;
        self.add_source(notebook_id, kind, &name, bytes).await
    }

    /// Adds a text that the owner wrote or pasted, as a text file named after
    /// its title.
    pub(crate) async fn add_text(
        &self,
        notebook_id: &str,
        title: &str,
        text: &str,
    ) -> Result<SourceView, NotebooksError> {
        self.add_source(
            notebook_id,
            DocumentKind::Text,
            &clean_name(title),
            text.as_bytes().to_vec(),
        )
        .await
    }

    /// Starts the upload of a dropped file, after checking its notebook and
    /// its kind, so that a file that would be refused sends no part.
    pub(crate) fn start_upload(
        &self,
        notebook_id: &str,
        name: &str,
    ) -> Result<UploadStarted, NotebooksError> {
        if self.notebook(notebook_id).is_none() {
            return Err(NotebooksError::NotebookNotFound);
        }
        let name = clean_name(name);
        if DocumentKind::from_file_name(&name).is_none() {
            return Err(NotebooksError::Refused {
                reason: SourceRefusal::UnsupportedType,
            });
        }
        let upload_id = self.uploads.start(notebook_id, &name, self.now());
        Ok(UploadStarted { upload_id })
    }

    /// Adds the next part of an upload.
    pub(crate) fn add_upload_part(
        &self,
        upload_id: &str,
        index: u32,
        part: &[u8],
    ) -> Result<(), NotebooksError> {
        self.uploads.add(upload_id, index, part, self.now())
    }

    /// Adds an uploaded file to its notebook.
    pub(crate) async fn finish_upload(
        &self,
        upload_id: &str,
    ) -> Result<SourceView, NotebooksError> {
        let uploaded = self.uploads.finish(upload_id, self.now())?;
        self.add_file(&uploaded.notebook_id, &uploaded.name, uploaded.bytes)
            .await
    }

    /// Drops an upload. One that ended already is no error.
    pub(crate) fn cancel_upload(&self, upload_id: &str) {
        if self.uploads.cancel(upload_id) {
            tracing::info!(upload = %upload_id, "an upload was cancelled");
        }
    }

    async fn add_source(
        &self,
        notebook_id: &str,
        kind: DocumentKind,
        name: &str,
        bytes: Vec<u8>,
    ) -> Result<SourceView, NotebooksError> {
        if self.notebook(notebook_id).is_none() {
            return Err(NotebooksError::NotebookNotFound);
        }
        let source = NewSource {
            id: uuid::Uuid::now_v7().to_string(),
            notebook_id: notebook_id.to_owned(),
            kind,
            name: name.to_owned(),
            bytes: i64::try_from(bytes.len()).unwrap_or(i64::MAX),
            sha256: digest(&bytes),
        };
        // The bytes wait in memory before the row exists, so the indexer
        // finds them with the source.
        lock(&self.pending).insert(source.id.clone(), bytes);
        let inserted = self.store.insert_source(&source, self.now()).await;
        if let Err(failure) = inserted {
            lock(&self.pending).remove(&source.id);
            return Err(match failure {
                WriteFailure::Constraint(DUPLICATE) => NotebooksError::Refused {
                    reason: SourceRefusal::Duplicate,
                },
                WriteFailure::Gone => NotebooksError::NotebookNotFound,
                other => storage_failed(format!("{other:?}")),
            });
        }
        tracing::info!(notebook = %notebook_id, source = %source.id, kind = kind.as_str(), bytes = source.bytes, "a source was added");
        self.refresh().await?;
        self.wake.notify_one();
        self.source(&source.id)
            .map(|record| source_view(&record))
            .ok_or(NotebooksError::SourceNotFound)
    }

    /// The bytes of a source that waits to be read, which only the indexer
    /// takes.
    pub(crate) fn take_pending(&self, source: &str) -> Option<Vec<u8>> {
        lock(&self.pending).remove(source)
    }

    /// Whether a source's bytes wait to be read.
    pub(crate) fn has_pending(&self, source: &str) -> bool {
        lock(&self.pending).contains_key(source)
    }

    pub(crate) async fn remove_source(&self, id: &str) -> Result<(), NotebooksError> {
        let source = self.source_or_missing(id)?;
        let deleted = self
            .store
            .delete_source(id)
            .await
            .map_err(|failure| storage_failed(format!("{failure:?}")))?;
        if !deleted {
            return Err(NotebooksError::SourceNotFound);
        }
        lock(&self.pending).remove(id);
        self.forget_corpus(&source.notebook_id);
        tracing::info!(source = %id, "a source was removed");
        self.refresh().await
    }

    pub(crate) async fn retry_source(&self, id: &str) -> Result<SourceView, NotebooksError> {
        let source = self.source_or_missing(id)?;
        // A source whose text was read failed in its indexing; one that
        // failed while it was read has lost its bytes.
        let retries = source.state == SourceState::Failed && source.passages > 0;
        if !retries {
            return Err(NotebooksError::NothingToRetry);
        }
        let state = if self.embedding().is_some() {
            SourceState::Indexing
        } else {
            SourceState::Waiting
        };
        self.store
            .set_state(id, state, None)
            .await
            .map_err(|failure| storage_failed(format!("{failure:?}")))?;
        self.refresh().await?;
        self.wake.notify_one();
        self.source_or_missing(id)
            .map(|record| source_view(&record))
    }

    pub(crate) async fn set_included(
        &self,
        id: &str,
        included: bool,
    ) -> Result<SourceView, NotebooksError> {
        let source = self.source_or_missing(id)?;
        match self.store.set_included(id, included).await {
            Ok(()) => {}
            Err(WriteFailure::Gone) => return Err(NotebooksError::SourceNotFound),
            Err(failure) => return Err(storage_failed(format!("{failure:?}"))),
        }
        self.forget_corpus(&source.notebook_id);
        self.refresh().await?;
        self.source_or_missing(id)
            .map(|record| source_view(&record))
    }

    /// At most [`MAX_SECTIONS_PER_READ`] sections of a source's text from
    /// `first`.
    pub(crate) async fn read_source(
        &self,
        id: &str,
        first: u32,
    ) -> Result<SourceText, NotebooksError> {
        let source = self.source_or_missing(id)?;
        let (sections, total) = self
            .store
            .sections(id, first, MAX_SECTIONS_PER_READ)
            .await
            .map_err(storage_failed)?;
        Ok(SourceText {
            source: source_view(&source),
            sections: sections
                .into_iter()
                .map(|section| SectionText {
                    position: section.position,
                    place: section.place,
                    text: section.text,
                })
                .collect(),
            total,
        })
    }

    /// Where a passage stands in its source's text, with the offsets that
    /// the webview counts.
    pub(crate) async fn locate(&self, key: &str) -> Result<PassagePlace, NotebooksError> {
        let (source_id, number) = parse_passage_key(key).ok_or(NotebooksError::PassageNotFound)?;
        if self.source(source_id).is_none() {
            return Err(NotebooksError::PassageNotFound);
        }
        let (section, start, end) = self
            .store
            .passage(source_id, number)
            .await
            .map_err(storage_failed)?
            .ok_or(NotebooksError::PassageNotFound)?;
        let text = self
            .store
            .section_text(source_id, section)
            .await
            .map_err(storage_failed)?
            .ok_or(NotebooksError::PassageNotFound)?;
        let utf16 = |bytes: u32| -> Option<u32> {
            let offset = usize::try_from(bytes).ok()?;
            let prefix = text.get(..offset)?;
            u32::try_from(prefix.encode_utf16().count()).ok()
        };
        Ok(PassagePlace {
            source_id: source_id.to_owned(),
            section,
            start: utf16(start).ok_or(NotebooksError::PassageNotFound)?,
            end: utf16(end).ok_or(NotebooksError::PassageNotFound)?,
        })
    }

    /// Saves how the plugin searches. A profile that the preferences name
    /// must exist and be of its kind; a new embedding model indexes every
    /// source again.
    pub(crate) async fn save_preferences(
        &self,
        preferences: Preferences,
    ) -> Result<Preferences, NotebooksError> {
        let embeddings = self.profiles(SearchModelKind::Embeddings).await?;
        let rerank = self.profiles(SearchModelKind::Rerank).await?;
        let names_missing = |chosen: Option<&String>, offered: &[SearchModel]| {
            chosen.is_some_and(|id| !offered.iter().any(|profile| profile.id == *id))
        };
        if names_missing(preferences.embedding_profile_id.as_ref(), &embeddings)
            || names_missing(preferences.rerank_profile_id.as_ref(), &rerank)
        {
            return Err(NotebooksError::ProfileNotFound);
        }
        self.store
            .save_preferences(&preferences)
            .await
            .map_err(|failure| storage_failed(format!("{failure:?}")))?;
        lock(&self.mirror).preferences = preferences.clone();
        tracing::info!(
            embedding = preferences.embedding_profile_id.is_some(),
            rerank = preferences.rerank_profile_id.is_some(),
            "the preferences of the notebooks were saved"
        );
        self.follow_models().await?;
        Ok(preferences)
    }

    async fn profiles(&self, kind: SearchModelKind) -> Result<Vec<SearchModel>, NotebooksError> {
        self.models.search_profiles(kind).await.map_err(|error| {
            tracing::warn!(
                error.code = error.code(),
                "the search profiles could not be read"
            );
            NotebooksError::ProfilesUnavailable
        })
    }

    /// The owner's search profiles of each kind, which the settings offer.
    pub(crate) async fn search_profiles(&self) -> Result<SearchProfiles, NotebooksError> {
        let option = |profile: SearchModel| ProfileOption {
            id: profile.id,
            name: profile.name,
            model: profile.model,
        };
        Ok(SearchProfiles {
            embeddings: self
                .profiles(SearchModelKind::Embeddings)
                .await?
                .into_iter()
                .map(option)
                .collect(),
            rerank: self
                .profiles(SearchModelKind::Rerank)
                .await?
                .into_iter()
                .map(option)
                .collect(),
        })
    }

    /// Reads the key of the embedding profile's vectors again and moves the
    /// sources on to it: a source whose vectors are of another model is
    /// indexed again, and without a model a source waits. Called at the start,
    /// when the preferences change and when a search profile changes.
    pub(crate) async fn follow_models(&self) -> Result<(), NotebooksError> {
        let chosen = self.preferences().embedding_profile_id;
        let key = match chosen {
            Some(id) => match self
                .models
                .search_profiles(SearchModelKind::Embeddings)
                .await
            {
                Ok(profiles) => profiles
                    .into_iter()
                    .find(|profile| profile.id == id)
                    .map(|profile| profile.vector_key),
                Err(error) => {
                    tracing::warn!(
                        error.code = error.code(),
                        "the embedding profile could not be read"
                    );
                    None
                }
            },
            None => None,
        };
        let changed = {
            let mut mirror = lock(&self.mirror);
            let changed = mirror.embedding_key != key;
            mirror.embedding_key.clone_from(&key);
            changed
        };
        if changed {
            lock(&self.corpora).entries.clear();
            tracing::info!(
                model = key.is_some(),
                "the notebooks follow the embedding model"
            );
        }
        self.store
            .follow_model(key.as_deref())
            .await
            .map_err(|failure| storage_failed(format!("{failure:?}")))?;
        self.refresh().await?;
        self.wake.notify_one();
        Ok(())
    }

    /// Forgets the corpus of a notebook whose passages changed.
    pub(crate) fn forget_corpus(&self, notebook: &str) {
        lock(&self.corpora)
            .entries
            .retain(|entry| entry.notebook != notebook);
    }

    /// The corpus of a notebook's searched passages with the vectors of the
    /// model `key`: the one in memory, or one read now, which pushes out the
    /// corpora used least recently beyond [`MAX_CORPUS_BYTES`].
    pub(crate) async fn corpus(
        &self,
        notebook: &str,
        key: &str,
    ) -> Result<Arc<Corpus>, NotebooksError> {
        {
            let mut corpora = lock(&self.corpora);
            corpora.uses += 1;
            let uses = corpora.uses;
            if let Some(entry) = corpora
                .entries
                .iter_mut()
                .find(|entry| entry.notebook == notebook && entry.key == key)
            {
                entry.used = uses;
                return Ok(Arc::clone(&entry.corpus));
            }
        }
        let rows = self
            .store
            .corpus(notebook, Some(key))
            .await
            .map_err(storage_failed)?;
        let corpus = Arc::new(Corpus::new(rows));
        let mut corpora = lock(&self.corpora);
        corpora.uses += 1;
        let used = corpora.uses;
        corpora.entries.retain(|entry| entry.notebook != notebook);
        corpora.entries.push(CorpusEntry {
            notebook: notebook.to_owned(),
            key: key.to_owned(),
            corpus: Arc::clone(&corpus),
            used,
        });
        let mut total: usize = corpora
            .entries
            .iter()
            .map(|entry| entry.corpus.bytes())
            .sum();
        while total > MAX_CORPUS_BYTES && corpora.entries.len() > 1 {
            let Some(oldest) = corpora
                .entries
                .iter()
                .enumerate()
                .filter(|(_index, entry)| entry.notebook != notebook)
                .min_by_key(|(_index, entry)| entry.used)
                .map(|(index, _entry)| index)
            else {
                break;
            };
            total -= corpora.entries.remove(oldest).corpus.bytes();
        }
        Ok(corpus)
    }

    /// The notebooks of a request's keys that exist, in their order, without
    /// repetitions.
    pub(crate) fn existing_notebooks(&self, keys: &[String]) -> Vec<String> {
        let mirror = lock(&self.mirror);
        let mut seen = HashSet::new();
        keys.iter()
            .filter(|key| mirror.notebooks.iter().any(|notebook| notebook.id == **key))
            .filter(|key| seen.insert((*key).clone()))
            .cloned()
            .collect()
    }

    /// How many sources are being read or indexed, which turning the plugin
    /// off asks about.
    pub(crate) fn indexing_count(&self) -> u32 {
        let count = lock(&self.mirror)
            .sources
            .iter()
            .filter(|source| matches!(source.state, SourceState::Reading | SourceState::Indexing))
            .count();
        u32::try_from(count).unwrap_or(u32::MAX)
    }
}

/// The source and the number of a passage's key, `<source ID>:<number>`.
pub(crate) fn parse_passage_key(key: &str) -> Option<(&str, u32)> {
    let (source, number) = key.rsplit_once(':')?;
    if source.is_empty() {
        return None;
    }
    Some((source, number.parse().ok()?))
}

/// The plugin's code of a failure of the `models` service.
pub(crate) fn models_failure(error: &ModelsError) -> &'static str {
    match error {
        ModelsError::Unreachable => "server_unreachable",
        ModelsError::ProfileNotFound | ModelsError::WrongKind => "no_embedding_profile",
        ModelsError::InputTooLong | ModelsError::InvalidInputs => "input_too_long",
        _other => "server_error",
    }
}

#[cfg(test)]
mod tests {
    use super::{clean_name, digest, parse_passage_key};

    #[test]
    fn a_passage_key_names_its_source_and_its_number() {
        assert_eq!(parse_passage_key("0199-ab:12"), Some(("0199-ab", 12)));
        assert_eq!(parse_passage_key("a:b:3"), Some(("a:b", 3)));
        for invalid in [":3", "abc", "a:", "a:-1", "a:x"] {
            assert_eq!(parse_passage_key(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn a_file_s_name_loses_its_folders_and_control_characters() {
        assert_eq!(clean_name("C:\\Docs\\Optics.pdf"), "Optics.pdf");
        assert_eq!(clean_name("/home/me/notes.md"), "notes.md");
        assert_eq!(clean_name(" a\u{7}b.txt "), "ab.txt");
    }

    #[test]
    fn a_digest_is_the_hexadecimal_sha_256() {
        assert_eq!(
            digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
