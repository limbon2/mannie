//! The plugin's tables (plugins/notebooks/docs/README.md, data): the
//! notebooks, their sources, the text of each source's pages or sections,
//! its passages, their vectors and the preferences, in one row.
//!
//! The plugin keeps the text of a document and never its file (plugin
//! ADR-0001, decision 4). Deleting a notebook deletes its sources, and
//! deleting a source its sections, passages and vectors, through the tables'
//! foreign keys, in one statement that erases: the store overwrites the freed
//! pages and empties its log, so the text leaves every file (T-67).

use std::sync::Arc;

use mannie_notebooks_documents::{CHUNKER_VERSION, DocumentKind};
use mannie_plugin_sdk::services::store::{
    Constraint, Durability, Expect, Migration, ModuleStoreApi, Row, SqlValue, Statement,
    StoreError, StoreModule, WriteBatch,
};
use serde::Deserialize;

use crate::commands::{DEFAULT_PASSAGES, PASSAGE_RANGE, Preferences, SectionPlace, SourceState};

/// The prefix of the plugin's tables, which names its module of the store.
pub(crate) const PREFIX: &str = "notebooks";

const MIGRATIONS: &[Migration] = &[
    Migration::new(
        1,
        "create the tables of the notebooks, their sources, text, passages and vectors",
        "CREATE TABLE notebooks_notebooks (
        id TEXT PRIMARY KEY NOT NULL,
        name TEXT NOT NULL UNIQUE COLLATE NOCASE,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE notebooks_sources (
        id TEXT PRIMARY KEY NOT NULL,
        notebook_id TEXT NOT NULL REFERENCES notebooks_notebooks (id) ON DELETE CASCADE,
        kind TEXT NOT NULL CHECK (kind IN ('pdf', 'markdown', 'text')),
        name TEXT NOT NULL,
        bytes INTEGER NOT NULL CHECK (bytes >= 0),
        sha256 TEXT NOT NULL,
        pages INTEGER CHECK (pages IS NULL OR pages >= 0),
        state TEXT NOT NULL CHECK (state IN ('reading', 'waiting', 'indexing', 'ready', 'failed')),
        failure TEXT,
        included INTEGER NOT NULL CHECK (included IN (0, 1)),
        passages INTEGER NOT NULL CHECK (passages >= 0),
        embedded INTEGER NOT NULL CHECK (embedded >= 0),
        vector_key TEXT,
        added_at INTEGER NOT NULL,
        indexed_at INTEGER,
        UNIQUE (notebook_id, sha256)
    ) STRICT;
    CREATE INDEX notebooks_sources_by_notebook ON notebooks_sources (notebook_id);
    CREATE TABLE notebooks_sections (
        source_id TEXT NOT NULL REFERENCES notebooks_sources (id) ON DELETE CASCADE,
        position INTEGER NOT NULL CHECK (position >= 0),
        place TEXT NOT NULL CHECK (json_valid(place)),
        text TEXT NOT NULL,
        PRIMARY KEY (source_id, position)
    ) STRICT;
    CREATE TABLE notebooks_passages (
        source_id TEXT NOT NULL REFERENCES notebooks_sources (id) ON DELETE CASCADE,
        number INTEGER NOT NULL CHECK (number >= 0),
        section INTEGER NOT NULL CHECK (section >= 0),
        start_offset INTEGER NOT NULL CHECK (start_offset >= 0),
        end_offset INTEGER NOT NULL CHECK (end_offset > start_offset),
        text TEXT NOT NULL,
        PRIMARY KEY (source_id, number)
    ) STRICT;
    CREATE TABLE notebooks_vectors (
        source_id TEXT NOT NULL,
        number INTEGER NOT NULL,
        vector_key TEXT NOT NULL,
        vector BLOB NOT NULL,
        PRIMARY KEY (source_id, number),
        FOREIGN KEY (source_id, number) REFERENCES notebooks_passages (source_id, number)
            ON DELETE CASCADE
    ) STRICT;
    CREATE TABLE notebooks_preferences (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        document TEXT NOT NULL CHECK (json_valid(document))
    ) STRICT",
    ),
    // The version of the splitter that made a source's passages, so that a
    // later version splits the stored text again (ADR-0049, context).
    Migration::new(
        2,
        "record the version of the passages of each source",
        "ALTER TABLE notebooks_sources ADD COLUMN chunker INTEGER NOT NULL DEFAULT 1 \
         CHECK (chunker >= 1)",
    ),
];

const SELECT_NOTEBOOKS: &str = "SELECT id, name, created_at, updated_at FROM notebooks_notebooks \
    ORDER BY created_at DESC, id DESC";

const SELECT_SOURCES: &str = "SELECT id, notebook_id, kind, name, bytes, sha256, pages, state, \
    failure, included, passages, embedded, vector_key, added_at, indexed_at \
    FROM notebooks_sources ORDER BY added_at, id";

const INSERT_NOTEBOOK: &str = "INSERT INTO notebooks_notebooks (id, name, created_at, updated_at) \
    VALUES (?1, ?2, ?3, ?3)";

const RENAME_NOTEBOOK: &str =
    "UPDATE notebooks_notebooks SET name = ?2, updated_at = ?3 WHERE id = ?1";

const TOUCH_NOTEBOOK: &str = "UPDATE notebooks_notebooks SET updated_at = ?2 WHERE id = ?1";

const DELETE_NOTEBOOK: &str = "DELETE FROM notebooks_notebooks WHERE id = ?1";

const INSERT_SOURCE: &str = "INSERT INTO notebooks_sources (id, notebook_id, kind, name, bytes, \
    sha256, pages, state, failure, included, passages, embedded, vector_key, added_at, \
    indexed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 'reading', NULL, 1, 0, 0, NULL, ?7, NULL)";

const DELETE_SOURCE: &str = "DELETE FROM notebooks_sources WHERE id = ?1";

const SET_INCLUDED: &str = "UPDATE notebooks_sources SET included = ?2 WHERE id = ?1";

const SET_STATE: &str = "UPDATE notebooks_sources SET state = ?2, failure = ?3 WHERE id = ?1";

// A source that is indexed keeps its state while its server fails, with the
// reason, until a batch is stored or the waits are used up.
const SET_PAUSED: &str =
    "UPDATE notebooks_sources SET failure = ?2 WHERE id = ?1 AND state = 'indexing'";

const INSERT_SECTION: &str =
    "INSERT INTO notebooks_sections (source_id, position, place, text) VALUES (?1, ?2, ?3, ?4)";

const INSERT_PASSAGE: &str = "INSERT INTO notebooks_passages (source_id, number, section, \
    start_offset, end_offset, text) VALUES (?1, ?2, ?3, ?4, ?5, ?6)";

const SET_TEXT_READ: &str = "UPDATE notebooks_sources SET state = ?2, failure = NULL, pages = ?3, \
    passages = ?4, embedded = 0, chunker = ?5 WHERE id = ?1 AND state = 'reading'";

// The sources whose text was read and split by an earlier version.
const SELECT_OUTDATED_PASSAGES: &str = "SELECT id FROM notebooks_sources \
    WHERE chunker < ?1 AND passages > 0 ORDER BY added_at, id";

const DELETE_PASSAGES: &str = "DELETE FROM notebooks_passages WHERE source_id = ?1";

// A source split again is embedded again, so its vectors and readiness go.
const SET_SPLIT_AGAIN: &str = "UPDATE notebooks_sources SET state = ?2, failure = NULL, \
    passages = ?3, embedded = 0, vector_key = NULL, indexed_at = NULL, chunker = ?4 \
    WHERE id = ?1 AND passages > 0";

const SELECT_PASSAGES_TO_EMBED: &str = "SELECT p.number, p.text, s.place \
    FROM notebooks_passages p \
    JOIN notebooks_sections s ON s.source_id = p.source_id AND s.position = p.section \
    LEFT JOIN notebooks_vectors v ON v.source_id = p.source_id AND v.number = p.number \
    AND v.vector_key = ?2 \
    WHERE p.source_id = ?1 AND v.number IS NULL ORDER BY p.number LIMIT ?3";

const UPSERT_VECTOR: &str = "INSERT INTO notebooks_vectors (source_id, number, vector_key, vector) \
    VALUES (?1, ?2, ?3, ?4) ON CONFLICT (source_id, number) \
    DO UPDATE SET vector_key = excluded.vector_key, vector = excluded.vector";

const SET_EMBEDDED: &str =
    "UPDATE notebooks_sources SET embedded = ?2, failure = NULL WHERE id = ?1";

const SET_READY: &str = "UPDATE notebooks_sources SET state = 'ready', failure = NULL, \
    embedded = ?2, vector_key = ?3, indexed_at = ?4 WHERE id = ?1";

const COUNT_EMBEDDED: &str =
    "SELECT COUNT(*) FROM notebooks_vectors WHERE source_id = ?1 AND vector_key = ?2";

const SELECT_SECTIONS: &str = "SELECT position, place, text FROM notebooks_sections \
    WHERE source_id = ?1 AND position >= ?2 ORDER BY position LIMIT ?3";

const COUNT_SECTIONS: &str = "SELECT COUNT(*) FROM notebooks_sections WHERE source_id = ?1";

const SELECT_SECTION_TEXT: &str =
    "SELECT text FROM notebooks_sections WHERE source_id = ?1 AND position = ?2";

const SELECT_PASSAGE: &str = "SELECT section, start_offset, end_offset FROM notebooks_passages \
    WHERE source_id = ?1 AND number = ?2";

// The places of a source's sections without their text, which a read by page
// looks through.
const SELECT_SECTION_PLACES: &str = "SELECT position, place FROM notebooks_sections \
    WHERE source_id = ?1 ORDER BY position";

// The passages of a range of a source's sections by where they start, which
// give the cite links of a read.
const SELECT_SECTION_PASSAGES: &str = "SELECT number, section, start_offset \
    FROM notebooks_passages WHERE source_id = ?1 AND section BETWEEN ?2 AND ?3 \
    ORDER BY section, start_offset";

// The passages of a notebook's sources that are on and searched, with the
// vector of each that the current model made, if it made one.
const SELECT_CORPUS: &str = "SELECT p.source_id, src.name, p.number, p.section, \
    p.start_offset, p.end_offset, p.text, s.place, v.vector \
    FROM notebooks_passages p \
    JOIN notebooks_sources src ON src.id = p.source_id \
    JOIN notebooks_sections s ON s.source_id = p.source_id AND s.position = p.section \
    LEFT JOIN notebooks_vectors v ON v.source_id = p.source_id AND v.number = p.number \
    AND v.vector_key = ?2 \
    WHERE src.notebook_id = ?1 AND src.included = 1 AND src.state IN ('ready', 'indexing') \
    ORDER BY p.source_id, p.number";

const SELECT_PREFERENCES: &str = "SELECT document FROM notebooks_preferences WHERE id = 1";

const UPSERT_PREFERENCES: &str = "INSERT INTO notebooks_preferences (id, document) VALUES (1, ?1) \
    ON CONFLICT (id) DO UPDATE SET document = excluded.document";

// A source that was being read when the plugin stopped has lost its bytes
// (plugins/notebooks/docs/README.md, indexing).
const FAIL_INTERRUPTED: &str = "UPDATE notebooks_sources SET state = 'failed', \
    failure = 'interrupted' WHERE state = 'reading'";

// With a model whose vectors differ from a source's, the source is indexed
// again; its words stay searched meanwhile.
const REINDEX: &str = "UPDATE notebooks_sources SET state = 'indexing', failure = NULL \
    WHERE state IN ('ready', 'waiting') AND (vector_key IS NULL OR vector_key <> ?1)";

// Without an embedding model, what would be indexed waits for one.
const WAIT_FOR_MODEL: &str = "UPDATE notebooks_sources SET state = 'waiting', failure = NULL \
    WHERE state = 'indexing'";

/// The plugin's module of the store.
pub const MODULE: StoreModule = StoreModule::new(
    PREFIX,
    MIGRATIONS,
    &[
        SELECT_NOTEBOOKS,
        SELECT_SOURCES,
        INSERT_NOTEBOOK,
        RENAME_NOTEBOOK,
        TOUCH_NOTEBOOK,
        DELETE_NOTEBOOK,
        INSERT_SOURCE,
        DELETE_SOURCE,
        SET_INCLUDED,
        SET_STATE,
        SET_PAUSED,
        INSERT_SECTION,
        INSERT_PASSAGE,
        SET_TEXT_READ,
        SELECT_OUTDATED_PASSAGES,
        DELETE_PASSAGES,
        SET_SPLIT_AGAIN,
        SELECT_PASSAGES_TO_EMBED,
        UPSERT_VECTOR,
        SET_EMBEDDED,
        SET_READY,
        COUNT_EMBEDDED,
        SELECT_SECTIONS,
        COUNT_SECTIONS,
        SELECT_SECTION_TEXT,
        SELECT_PASSAGE,
        SELECT_SECTION_PLACES,
        SELECT_SECTION_PASSAGES,
        SELECT_CORPUS,
        SELECT_PREFERENCES,
        UPSERT_PREFERENCES,
        FAIL_INTERRUPTED,
        REINDEX,
        WAIT_FOR_MODEL,
    ],
);

/// The code of a name that another notebook has.
pub(crate) const NAME_TAKEN: &str = "name_taken";

/// The code of a file that the notebook holds already.
pub(crate) const DUPLICATE: &str = "duplicate";

/// A notebook as its row keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotebookRecord {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

/// A source as its row keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceRecord {
    pub(crate) id: String,
    pub(crate) notebook_id: String,
    pub(crate) kind: DocumentKind,
    pub(crate) name: String,
    pub(crate) bytes: i64,
    pub(crate) sha256: String,
    pub(crate) pages: Option<u32>,
    pub(crate) state: SourceState,
    pub(crate) failure: Option<String>,
    pub(crate) included: bool,
    pub(crate) passages: u32,
    pub(crate) embedded: u32,
    /// The key of the model whose vectors the source holds whole.
    pub(crate) vector_key: Option<String>,
    pub(crate) added_at: i64,
    pub(crate) indexed_at: Option<i64>,
}

/// A source to add, before its text is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewSource {
    pub(crate) id: String,
    pub(crate) notebook_id: String,
    pub(crate) kind: DocumentKind,
    pub(crate) name: String,
    pub(crate) bytes: i64,
    pub(crate) sha256: String,
}

/// The preferences as the store keeps them. A document of the first version
/// also holds `thresholdPercent`, which no longer applies (ADR-0049), so a
/// field that the plugin no longer knows is left aside here, while the
/// command `save_preferences` refuses one.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredPreferences {
    embedding_profile_id: Option<String>,
    rerank_profile_id: Option<String>,
    #[serde(default = "default_passages")]
    max_passages: u32,
}

const fn default_passages() -> u32 {
    DEFAULT_PASSAGES
}

impl From<StoredPreferences> for Preferences {
    fn from(stored: StoredPreferences) -> Self {
        let (fewest, most) = PASSAGE_RANGE;
        Self {
            embedding_profile_id: stored.embedding_profile_id,
            rerank_profile_id: stored.rerank_profile_id,
            max_passages: stored.max_passages.clamp(fewest, most),
        }
    }
}

/// A section of a source's text, to store or as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredSection {
    pub(crate) position: u32,
    pub(crate) place: SectionPlace,
    pub(crate) text: String,
}

/// A passage of a source, to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewPassage {
    pub(crate) number: u32,
    pub(crate) section: u32,
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) text: String,
}

/// Where a passage starts in its source: its section and the byte offset in
/// the section's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SectionPassage {
    pub(crate) number: u32,
    pub(crate) section: u32,
    pub(crate) start: u32,
}

/// A passage that waits for its vector, with the place of its section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingPassage {
    pub(crate) number: u32,
    pub(crate) text: String,
    pub(crate) place: SectionPlace,
}

/// A passage as a search reads it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CorpusRow {
    pub(crate) source_id: String,
    pub(crate) source_name: String,
    pub(crate) number: u32,
    pub(crate) section: u32,
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) text: String,
    pub(crate) place: SectionPlace,
    /// The vector that the current model made, scaled to a length of 1.
    pub(crate) vector: Option<Vec<f32>>,
}

/// Why the store refused a write that the plugin maps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WriteFailure {
    /// A constraint that the batch mapped failed, with the plugin's code.
    Constraint(&'static str),
    /// The row that the write changes is gone.
    Gone,
    /// The store failed.
    Store(StoreError),
}

impl From<StoreError> for WriteFailure {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Constraint { code } => Self::Constraint(code),
            StoreError::Expectation { .. } => Self::Gone,
            other => Self::Store(other),
        }
    }
}

fn state_name(state: SourceState) -> &'static str {
    match state {
        SourceState::Reading => "reading",
        SourceState::Waiting => "waiting",
        SourceState::Indexing => "indexing",
        SourceState::Ready => "ready",
        SourceState::Failed => "failed",
    }
}

fn parse_state(name: &str) -> Option<SourceState> {
    match name {
        "reading" => Some(SourceState::Reading),
        "waiting" => Some(SourceState::Waiting),
        "indexing" => Some(SourceState::Indexing),
        "ready" => Some(SourceState::Ready),
        "failed" => Some(SourceState::Failed),
        _other => None,
    }
}

/// A vector as its BLOB keeps it: little-endian 32-bit floats.
pub(crate) fn vector_bytes(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// The vector of a BLOB, or `None` for one whose length is no whole number of
/// floats.
pub(crate) fn vector_of(bytes: &[u8]) -> Option<Vec<f32>> {
    let (floats, rest) = bytes.as_chunks::<4>();
    if !rest.is_empty() {
        return None;
    }
    Some(
        floats
            .iter()
            .map(|float| f32::from_le_bytes(*float))
            .collect(),
    )
}

/// The batch with an insert of each passage of a source.
fn push_passages(mut batch: WriteBatch, id: &str, passages: &[NewPassage]) -> WriteBatch {
    for passage in passages {
        let insert = Statement::new(INSERT_PASSAGE)
            .bind(id)
            .bind(passage.number)
            .bind(passage.section)
            .bind(passage.start)
            .bind(passage.end)
            .bind(passage.text.as_str());
        batch = batch.push(insert, Expect::Changes(1));
    }
    batch
}

fn place_json(place: &SectionPlace) -> Result<String, StoreError> {
    serde_json::to_string(place).map_err(|_unencodable| StoreError::Failed)
}

fn place_of(row: &Row, column: usize) -> Result<SectionPlace, StoreError> {
    let text: String = row.get(column)?;
    serde_json::from_str(&text).map_err(|_invalid| StoreError::InvalidValue { column })
}

fn count(rows: &[Row]) -> Result<u32, StoreError> {
    rows.first().map_or(Ok(0), |row| row.get::<u32>(0))
}

fn notebook_of(row: &Row) -> Result<NotebookRecord, StoreError> {
    Ok(NotebookRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        created_at: row.get(2)?,
        updated_at: row.get(3)?,
    })
}

fn source_of(row: &Row) -> Result<SourceRecord, StoreError> {
    let kind: String = row.get(2)?;
    let state: String = row.get(7)?;
    Ok(SourceRecord {
        id: row.get(0)?,
        notebook_id: row.get(1)?,
        kind: DocumentKind::parse(&kind).ok_or(StoreError::InvalidValue { column: 2 })?,
        name: row.get(3)?,
        bytes: row.get(4)?,
        sha256: row.get(5)?,
        pages: row.get(6)?,
        state: parse_state(&state).ok_or(StoreError::InvalidValue { column: 7 })?,
        failure: row.get(8)?,
        included: row.get(9)?,
        passages: row.get(10)?,
        embedded: row.get(11)?,
        vector_key: row.get(12)?,
        added_at: row.get(13)?,
        indexed_at: row.get(14)?,
    })
}

/// The plugin's tables, through its module of the store.
#[derive(Debug, Clone)]
pub(crate) struct NotebookStore {
    api: Arc<dyn ModuleStoreApi>,
}

impl NotebookStore {
    pub(crate) fn new(api: Arc<dyn ModuleStoreApi>) -> Self {
        Self { api }
    }

    async fn write(&self, batch: WriteBatch) -> Result<Vec<u64>, WriteFailure> {
        Ok(self.api.write(batch, Durability::Full).await?)
    }

    /// Runs a batch that deletes the owner's text, which leaves every file of
    /// the store when it returns (T-67).
    async fn erase(&self, batch: WriteBatch) -> Result<Vec<u64>, WriteFailure> {
        Ok(self.api.write(batch, Durability::Erase).await?)
    }

    /// Every notebook, the newest first.
    pub(crate) async fn notebooks(&self) -> Result<Vec<NotebookRecord>, StoreError> {
        let rows = self.api.query(Statement::new(SELECT_NOTEBOOKS)).await?;
        rows.iter().map(notebook_of).collect()
    }

    /// Every source, the oldest first.
    pub(crate) async fn sources(&self) -> Result<Vec<SourceRecord>, StoreError> {
        let rows = self.api.query(Statement::new(SELECT_SOURCES)).await?;
        rows.iter().map(source_of).collect()
    }

    pub(crate) async fn insert_notebook(
        &self,
        notebook: &NotebookRecord,
    ) -> Result<(), WriteFailure> {
        let insert = Statement::new(INSERT_NOTEBOOK)
            .bind(notebook.id.as_str())
            .bind(notebook.name.as_str())
            .bind(notebook.created_at);
        let batch = WriteBatch::new()
            .push(insert, Expect::Changes(1))
            .on_constraint(
                Constraint::Key {
                    table: "notebooks_notebooks",
                    columns: &["name"],
                },
                NAME_TAKEN,
            );
        self.write(batch).await.map(drop)
    }

    pub(crate) async fn rename_notebook(
        &self,
        id: &str,
        name: &str,
        now: i64,
    ) -> Result<(), WriteFailure> {
        let rename = Statement::new(RENAME_NOTEBOOK)
            .bind(id)
            .bind(name)
            .bind(now);
        let batch = WriteBatch::new()
            .push(rename, Expect::Changes(1))
            .on_constraint(
                Constraint::Key {
                    table: "notebooks_notebooks",
                    columns: &["name"],
                },
                NAME_TAKEN,
            );
        self.write(batch).await.map(drop)
    }

    /// Deletes a notebook with its sources. Returns whether it existed.
    pub(crate) async fn delete_notebook(&self, id: &str) -> Result<bool, WriteFailure> {
        let batch = WriteBatch::new().push(Statement::new(DELETE_NOTEBOOK).bind(id), Expect::Any);
        let changed = self.erase(batch).await?;
        Ok(changed.first().copied().unwrap_or(0) > 0)
    }

    /// Adds a source that waits for its text to be read, and marks its
    /// notebook as changed.
    pub(crate) async fn insert_source(
        &self,
        source: &NewSource,
        now: i64,
    ) -> Result<(), WriteFailure> {
        let insert = Statement::new(INSERT_SOURCE)
            .bind(source.id.as_str())
            .bind(source.notebook_id.as_str())
            .bind(source.kind.as_str())
            .bind(source.name.as_str())
            .bind(source.bytes)
            .bind(source.sha256.as_str())
            .bind(now);
        let touch = Statement::new(TOUCH_NOTEBOOK)
            .bind(source.notebook_id.as_str())
            .bind(now);
        let batch = WriteBatch::new()
            .push(insert, Expect::Changes(1))
            .push(touch, Expect::Changes(1))
            .on_constraint(
                Constraint::Key {
                    table: "notebooks_sources",
                    columns: &["notebook_id", "sha256"],
                },
                DUPLICATE,
            );
        self.write(batch).await.map(drop)
    }

    /// Deletes a source with its text, passages and vectors. Returns whether
    /// it existed.
    pub(crate) async fn delete_source(&self, id: &str) -> Result<bool, WriteFailure> {
        let batch = WriteBatch::new().push(Statement::new(DELETE_SOURCE).bind(id), Expect::Any);
        let changed = self.erase(batch).await?;
        Ok(changed.first().copied().unwrap_or(0) > 0)
    }

    pub(crate) async fn set_included(&self, id: &str, included: bool) -> Result<(), WriteFailure> {
        let set = Statement::new(SET_INCLUDED).bind(id).bind(included);
        self.write(WriteBatch::new().push(set, Expect::Changes(1)))
            .await
            .map(drop)
    }

    pub(crate) async fn set_state(
        &self,
        id: &str,
        state: SourceState,
        failure: Option<&str>,
    ) -> Result<(), WriteFailure> {
        let set = Statement::new(SET_STATE)
            .bind(id)
            .bind(state_name(state))
            .bind(failure);
        self.write(WriteBatch::new().push(set, Expect::Changes(1)))
            .await
            .map(drop)
    }

    /// Records why the indexing of a source waits, while it stays indexed. A
    /// source that left `indexing` meanwhile changes nothing.
    pub(crate) async fn set_paused(&self, id: &str, code: &str) -> Result<(), WriteFailure> {
        let set = Statement::new(SET_PAUSED).bind(id).bind(code);
        self.write(WriteBatch::new().push(set, Expect::Any))
            .await
            .map(drop)
    }

    /// Stores the text that was read of a source and its passages, in one
    /// transaction, and moves it on to `state`. A source that was removed
    /// meanwhile is [`WriteFailure::Gone`].
    pub(crate) async fn store_text(
        &self,
        id: &str,
        sections: &[StoredSection],
        passages: &[NewPassage],
        pages: Option<u32>,
        state: SourceState,
    ) -> Result<(), WriteFailure> {
        // The source's row is checked first, so a removed source changes
        // nothing.
        let count = u32::try_from(passages.len()).unwrap_or(u32::MAX);
        let mut batch = WriteBatch::new().push(
            Statement::new(SET_TEXT_READ)
                .bind(id)
                .bind(state_name(state))
                .bind(pages)
                .bind(count)
                .bind(CHUNKER_VERSION),
            Expect::Changes(1),
        );
        for section in sections {
            let insert = Statement::new(INSERT_SECTION)
                .bind(id)
                .bind(section.position)
                .bind(place_json(&section.place)?)
                .bind(section.text.as_str());
            batch = batch.push(insert, Expect::Changes(1));
        }
        batch = push_passages(batch, id, passages);
        self.write(batch).await.map(drop)
    }

    /// The sources whose passages an earlier version of the splitter made,
    /// the oldest first.
    pub(crate) async fn outdated_passages(&self) -> Result<Vec<String>, StoreError> {
        let rows = self
            .api
            .query(Statement::new(SELECT_OUTDATED_PASSAGES).bind(CHUNKER_VERSION))
            .await?;
        rows.iter().map(|row| row.get(0)).collect()
    }

    /// Replaces a source's passages with those that the current splitter
    /// made of its stored text, in one transaction, and moves it on to
    /// `state`. Its vectors go with its old passages.
    pub(crate) async fn split_again(
        &self,
        id: &str,
        passages: &[NewPassage],
        state: SourceState,
    ) -> Result<(), WriteFailure> {
        let count = u32::try_from(passages.len()).unwrap_or(u32::MAX);
        let mut batch = WriteBatch::new()
            .push(
                Statement::new(SET_SPLIT_AGAIN)
                    .bind(id)
                    .bind(state_name(state))
                    .bind(count)
                    .bind(CHUNKER_VERSION),
                Expect::Changes(1),
            )
            .push(Statement::new(DELETE_PASSAGES).bind(id), Expect::Any);
        batch = push_passages(batch, id, passages);
        self.write(batch).await.map(drop)
    }

    /// The passages of a source without a vector of the model `key`, in
    /// order, at most `limit`.
    pub(crate) async fn passages_to_embed(
        &self,
        id: &str,
        key: &str,
        limit: u32,
    ) -> Result<Vec<PendingPassage>, StoreError> {
        let rows = self
            .api
            .query(
                Statement::new(SELECT_PASSAGES_TO_EMBED)
                    .bind(id)
                    .bind(key)
                    .bind(limit),
            )
            .await?;
        rows.iter()
            .map(|row| {
                Ok(PendingPassage {
                    number: row.get(0)?,
                    text: row.get(1)?,
                    place: place_of(row, 2)?,
                })
            })
            .collect()
    }

    /// Stores vectors of a source's passages, made by the model `key`, and the
    /// count of its passages that have one.
    pub(crate) async fn store_vectors(
        &self,
        id: &str,
        key: &str,
        vectors: &[(u32, Vec<f32>)],
        embedded: u32,
    ) -> Result<(), WriteFailure> {
        let mut batch = WriteBatch::new();
        for (number, vector) in vectors {
            let upsert = Statement::new(UPSERT_VECTOR)
                .bind(id)
                .bind(*number)
                .bind(key)
                .bind(SqlValue::Blob(vector_bytes(vector)));
            batch = batch.push(upsert, Expect::Changes(1));
        }
        batch = batch.push(
            Statement::new(SET_EMBEDDED).bind(id).bind(embedded),
            Expect::Changes(1),
        );
        self.write(batch).await.map(drop)
    }

    /// How many passages of a source have a vector of the model `key`.
    pub(crate) async fn embedded_count(&self, id: &str, key: &str) -> Result<u32, StoreError> {
        let rows = self
            .api
            .query(Statement::new(COUNT_EMBEDDED).bind(id).bind(key))
            .await?;
        count(&rows)
    }

    /// Marks a source as searched with the model `key`.
    pub(crate) async fn set_ready(
        &self,
        id: &str,
        key: &str,
        embedded: u32,
        now: i64,
    ) -> Result<(), WriteFailure> {
        let set = Statement::new(SET_READY)
            .bind(id)
            .bind(embedded)
            .bind(key)
            .bind(now);
        self.write(WriteBatch::new().push(set, Expect::Changes(1)))
            .await
            .map(drop)
    }

    /// At most `limit` sections of a source from `first`, and how many it
    /// has.
    pub(crate) async fn sections(
        &self,
        id: &str,
        first: u32,
        limit: u32,
    ) -> Result<(Vec<StoredSection>, u32), StoreError> {
        let rows = self
            .api
            .query(
                Statement::new(SELECT_SECTIONS)
                    .bind(id)
                    .bind(first)
                    .bind(limit),
            )
            .await?;
        let sections = rows
            .iter()
            .map(|row| {
                Ok(StoredSection {
                    position: row.get(0)?,
                    place: place_of(row, 1)?,
                    text: row.get(2)?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let total = count(
            &self
                .api
                .query(Statement::new(COUNT_SECTIONS).bind(id))
                .await?,
        )?;
        Ok((sections, total))
    }

    /// The text of a section of a source.
    pub(crate) async fn section_text(
        &self,
        id: &str,
        position: u32,
    ) -> Result<Option<String>, StoreError> {
        let rows = self
            .api
            .query(Statement::new(SELECT_SECTION_TEXT).bind(id).bind(position))
            .await?;
        rows.first().map(|row| row.get(0)).transpose()
    }

    /// The section and the byte offsets of a passage in it.
    pub(crate) async fn passage(
        &self,
        id: &str,
        number: u32,
    ) -> Result<Option<(u32, u32, u32)>, StoreError> {
        let rows = self
            .api
            .query(Statement::new(SELECT_PASSAGE).bind(id).bind(number))
            .await?;
        rows.first()
            .map(|row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .transpose()
    }

    /// The position and place of each section of a source, in order.
    pub(crate) async fn section_places(
        &self,
        id: &str,
    ) -> Result<Vec<(u32, SectionPlace)>, StoreError> {
        let rows = self
            .api
            .query(Statement::new(SELECT_SECTION_PLACES).bind(id))
            .await?;
        rows.iter()
            .map(|row| Ok((row.get(0)?, place_of(row, 1)?)))
            .collect()
    }

    /// The number, section and start of each passage of the sections
    /// `first` to `last` of a source, in the order of the text.
    pub(crate) async fn section_passages(
        &self,
        id: &str,
        first: u32,
        last: u32,
    ) -> Result<Vec<SectionPassage>, StoreError> {
        let rows = self
            .api
            .query(
                Statement::new(SELECT_SECTION_PASSAGES)
                    .bind(id)
                    .bind(first)
                    .bind(last),
            )
            .await?;
        rows.iter()
            .map(|row| {
                Ok(SectionPassage {
                    number: row.get(0)?,
                    section: row.get(1)?,
                    start: row.get(2)?,
                })
            })
            .collect()
    }

    /// The passages of a notebook's searched sources, with their vectors of
    /// the model `key`, if any.
    pub(crate) async fn corpus(
        &self,
        notebook: &str,
        key: Option<&str>,
    ) -> Result<Vec<CorpusRow>, StoreError> {
        let rows = self
            .api
            .query(Statement::new(SELECT_CORPUS).bind(notebook).bind(key))
            .await?;
        rows.iter()
            .map(|row| {
                let vector: Option<Vec<u8>> = row.get(8)?;
                Ok(CorpusRow {
                    source_id: row.get(0)?,
                    source_name: row.get(1)?,
                    number: row.get(2)?,
                    section: row.get(3)?,
                    start: row.get(4)?,
                    end: row.get(5)?,
                    text: row.get(6)?,
                    place: place_of(row, 7)?,
                    vector: vector.as_deref().and_then(vector_of),
                })
            })
            .collect()
    }

    /// The preferences, or `None` before the owner saved any.
    pub(crate) async fn preferences(&self) -> Result<Option<Preferences>, StoreError> {
        let rows = self.api.query(Statement::new(SELECT_PREFERENCES)).await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let document: String = row.get(0)?;
        serde_json::from_str::<StoredPreferences>(&document)
            .map(|stored| Some(stored.into()))
            .map_err(|_invalid| StoreError::InvalidValue { column: 0 })
    }

    pub(crate) async fn save_preferences(
        &self,
        preferences: &Preferences,
    ) -> Result<(), WriteFailure> {
        let document =
            serde_json::to_string(preferences).map_err(|_unencodable| StoreError::Failed)?;
        let upsert = Statement::new(UPSERT_PREFERENCES).bind(document);
        self.write(WriteBatch::new().push(upsert, Expect::Changes(1)))
            .await
            .map(drop)
    }

    /// Fails the sources that were being read when the plugin stopped.
    pub(crate) async fn fail_interrupted(&self) -> Result<u64, WriteFailure> {
        let batch = WriteBatch::new().push(Statement::new(FAIL_INTERRUPTED), Expect::Any);
        Ok(self.write(batch).await?.first().copied().unwrap_or(0))
    }

    /// Moves the sources on to the embedding model whose vectors are `key`,
    /// or to waiting without one.
    pub(crate) async fn follow_model(&self, key: Option<&str>) -> Result<(), WriteFailure> {
        let statement = match key {
            Some(key) => Statement::new(REINDEX).bind(key),
            None => Statement::new(WAIT_FOR_MODEL),
        };
        self.write(WriteBatch::new().push(statement, Expect::Any))
            .await
            .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::{StoredPreferences, vector_bytes, vector_of};
    use crate::commands::Preferences;

    #[test]
    fn stored_preferences_of_the_first_version_read_without_their_threshold() {
        let first = r#"{"embeddingProfileId":"e1","rerankProfileId":null,"thresholdPercent":50,"maxPassages":6}"#;
        let out_of_range = r#"{"embeddingProfileId":null,"rerankProfileId":null,"maxPassages":40}"#;

        let read = |document: &str| -> Preferences {
            serde_json::from_str::<StoredPreferences>(document)
                .unwrap()
                .into()
        };

        assert_eq!(
            read(first),
            Preferences {
                embedding_profile_id: Some("e1".to_owned()),
                rerank_profile_id: None,
                max_passages: 6,
            }
        );
        assert_eq!(read(out_of_range).max_passages, 12);
        assert_eq!(read("{}").max_passages, 8);
    }

    #[test]
    fn a_vector_keeps_its_floats_through_its_bytes() {
        let vector = vec![0.5_f32, -1.25, 3.0e-7];

        let bytes = vector_bytes(&vector);

        assert_eq!(bytes.len(), 12);
        assert_eq!(vector_of(&bytes), Some(vector));
        assert_eq!(vector_of(&bytes[..11]), None);
    }
}
