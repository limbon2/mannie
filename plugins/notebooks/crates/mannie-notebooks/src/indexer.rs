//! The indexer (plugins/notebooks/docs/README.md, indexing): one task that
//! reads and indexes one source at a time, the oldest first. It extracts a
//! file's text on a blocking thread, stores the text by page or section and
//! its passages, then embeds the passages, 32 to a call, with a header of the
//! source's name and place. A server that fails pauses the source, which is
//! tried again after each of the timing's waits and then fails with the
//! reason until the owner tries again. Searches run beside it.
//!
//! When it starts, it first splits again the stored text of the sources whose
//! passages an earlier version of the splitter made, which then embed again.

use std::collections::HashMap;
use std::sync::Arc;

use mannie_notebooks_documents::{
    ChunkOptions, Document, DocumentKind, ExtractError, Location, Section, extract, passages,
};
use mannie_plugin_sdk::services::models::{InputKind, MAX_INPUT_BYTES, ModelsError};
use tokio::time::Instant;

use crate::commands::{SectionPlace, SourceState};
use crate::library::{Library, models_failure};
use crate::search::normalized;
use crate::store::{NewPassage, PendingPassage, StoredSection, WriteFailure};

/// How many passages one call embeds.
const BATCH: u32 = 32;

/// The place of a section as the plugin keeps it.
pub(crate) fn place_of(location: &Location) -> SectionPlace {
    match location {
        Location::Page(number) => SectionPlace::Page { number: *number },
        Location::Headings(path) => SectionPlace::Headings { path: path.clone() },
        Location::Whole => SectionPlace::Whole,
    }
}

/// The location of a stored section, as the splitter reads it.
fn location_of(place: &SectionPlace) -> Location {
    match place {
        SectionPlace::Page { number } => Location::Page(*number),
        SectionPlace::Headings { path } => Location::Headings(path.clone()),
        SectionPlace::Whole => Location::Whole,
    }
}

/// Where a section stands, in the words of the text that the model embeds
/// and reads, or `None` for the whole of a text.
pub(crate) fn place_words(place: &SectionPlace) -> Option<String> {
    match place {
        SectionPlace::Page { number } => Some(format!("page {number}")),
        SectionPlace::Headings { path } if !path.is_empty() => Some(path.join(" > ")),
        SectionPlace::Headings { .. } | SectionPlace::Whole => None,
    }
}

/// A text cut to at most `max_bytes`, at a character's start.
pub(crate) fn cut_to(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// What the model embeds of a passage: the source's name and the passage's
/// place, then its text.
fn document_text(source: &str, passage: &PendingPassage) -> String {
    let header = match place_words(&passage.place) {
        Some(place) => format!("{source}, {place}"),
        None => source.to_owned(),
    };
    let text = format!("{header}\n\n{}", passage.text);
    cut_to(&text, MAX_INPUT_BYTES).to_owned()
}

/// The sections and passages of a document, as the store keeps them.
fn split(document: &Document) -> (Vec<StoredSection>, Vec<NewPassage>) {
    let sections: Vec<StoredSection> = document
        .sections
        .iter()
        .enumerate()
        .map(|(position, section)| StoredSection {
            position: u32::try_from(position).unwrap_or(u32::MAX),
            place: place_of(&section.location),
            text: section.text.clone(),
        })
        .collect();
    let found = passages(document, ChunkOptions::default());
    let new_passages = found
        .iter()
        .enumerate()
        .map(|(number, passage)| NewPassage {
            number: u32::try_from(number).unwrap_or(u32::MAX),
            section: u32::try_from(passage.section).unwrap_or(u32::MAX),
            start: u32::try_from(passage.start).unwrap_or(u32::MAX),
            end: u32::try_from(passage.end).unwrap_or(u32::MAX),
            text: document.sections[passage.section].text[passage.start..passage.end].to_owned(),
        })
        .collect();
    (sections, new_passages)
}

/// The work that comes next.
enum Job {
    /// Reads a source's text.
    Read(String),
    /// Embeds a source's passages.
    Embed(String),
}

/// The indexer's own record of the sources whose server failed: how often,
/// and when each is tried again.
#[derive(Debug, Default)]
struct Pauses {
    sources: HashMap<String, (usize, Instant)>,
}

impl Pauses {
    fn due(&self, source: &str, now: Instant) -> bool {
        self.sources
            .get(source)
            .is_none_or(|(_failures, at)| *at <= now)
    }

    fn earliest(&self) -> Option<Instant> {
        self.sources.values().map(|(_failures, at)| *at).min()
    }
}

/// The passages of stored sections, in their order.
fn split_stored(sections: &[StoredSection]) -> Vec<NewPassage> {
    let document = Document {
        sections: sections
            .iter()
            .map(|section| Section {
                location: location_of(&section.place),
                text: section.text.clone(),
            })
            .collect(),
        pages: None,
    };
    split(&document).1
}

/// Splits again the stored text of the sources whose passages an earlier
/// version of the splitter made, and moves them on to their indexing.
async fn split_outdated(library: &Library) {
    let outdated = match library.store.outdated_passages().await {
        Ok(outdated) => outdated,
        Err(error) => {
            tracing::warn!(
                error.code = error.code(),
                "the sources to split again could not be read"
            );
            return;
        }
    };
    if outdated.is_empty() {
        return;
    }
    let state = if library.embedding().is_some() {
        SourceState::Indexing
    } else {
        SourceState::Waiting
    };
    for id in &outdated {
        let sections = match library.store.sections(id, 0, u32::MAX).await {
            Ok((sections, _total)) => sections,
            Err(error) => {
                tracing::warn!(source = %id, error.code = error.code(), "the text of a source could not be read to split it again");
                continue;
            }
        };
        let new_passages = split_stored(&sections);
        match library.store.split_again(id, &new_passages, state).await {
            Ok(()) => {
                tracing::info!(source = %id, passages = new_passages.len(), "a source's text was split again");
            }
            // The owner removed the source meanwhile.
            Err(WriteFailure::Gone) => {}
            Err(failure) => log_write(id, &failure),
        }
        if let Some(source) = library.source(id) {
            library.forget_corpus(&source.notebook_id);
        }
    }
    if let Err(error) = library.refresh().await {
        tracing::warn!(error.code = %error, "the notebooks could not be read after sources were split again");
    }
}

/// Runs the indexer until the plugin stops, which aborts it.
pub(crate) async fn run(library: Arc<Library>) {
    split_outdated(&library).await;
    let mut pauses = Pauses::default();
    loop {
        match next_job(&library, &pauses) {
            Some(Job::Read(source)) => read(&library, &source).await,
            Some(Job::Embed(source)) => embed(&library, &source, &mut pauses).await,
            None => {
                // A wake that came while the indexer worked left its permit,
                // so this returns at once and finds the new work.
                match pauses.earliest() {
                    Some(at) => {
                        tokio::select! {
                            () = library.wake.notified() => {}
                            () = tokio::time::sleep_until(at) => {}
                        }
                    }
                    None => library.wake.notified().await,
                }
            }
        }
        pauses
            .sources
            .retain(|source, _pause| library.source(source).is_some());
    }
}

/// The oldest source that waits to be read, or else the oldest whose
/// passages wait for their vectors and whose pause ended.
fn next_job(library: &Library, pauses: &Pauses) -> Option<Job> {
    let sources = library.sources();
    let reading = sources
        .iter()
        .find(|source| source.state == SourceState::Reading && library.has_pending(&source.id));
    if let Some(source) = reading {
        return Some(Job::Read(source.id.clone()));
    }
    library.embedding()?;
    let now = Instant::now();
    sources
        .iter()
        .find(|source| source.state == SourceState::Indexing && pauses.due(&source.id, now))
        .map(|source| Job::Embed(source.id.clone()))
}

/// Reads a source's text, stores it with its passages, and moves the source
/// on to its indexing, or to waiting without an embedding model.
async fn read(library: &Library, id: &str) {
    let Some(source) = library.source(id) else {
        return;
    };
    let Some(bytes) = library.take_pending(id) else {
        return;
    };
    let kind = source.kind;
    let extracted = tokio::task::spawn_blocking(move || extract(kind, bytes)).await;
    let outcome = match extracted {
        Ok(Ok(document)) => store_document(library, id, kind, &document).await,
        Ok(Err(error)) => Err(error.code()),
        Err(_panic_or_cancel) => Err(ExtractError::Unreadable.code()),
    };
    if let Err(code) = outcome {
        tracing::info!(source = %id, code, "a source's text could not be read");
        if let Err(failure) = library
            .store
            .set_state(id, SourceState::Failed, Some(code))
            .await
        {
            log_write(id, &failure);
        }
    }
    if let Err(error) = library.refresh().await {
        tracing::warn!(error.code = %error, "the notebooks could not be read after a source's text");
    }
}

async fn store_document(
    library: &Library,
    id: &str,
    kind: DocumentKind,
    document: &Document,
) -> Result<(), &'static str> {
    let (sections, new_passages) = split(document);
    let state = if library.embedding().is_some() {
        SourceState::Indexing
    } else {
        SourceState::Waiting
    };
    match library
        .store
        .store_text(id, &sections, &new_passages, document.pages, state)
        .await
    {
        Ok(()) => {
            tracing::info!(
                source = %id,
                kind = kind.as_str(),
                sections = sections.len(),
                passages = new_passages.len(),
                "a source's text was read"
            );
            Ok(())
        }
        // The owner removed the source while its text was read.
        Err(WriteFailure::Gone) => Ok(()),
        Err(failure) => {
            log_write(id, &failure);
            Err("storage_failed")
        }
    }
}

fn log_write(id: &str, failure: &WriteFailure) {
    tracing::warn!(source = %id, ?failure, "a source could not be saved");
}

/// Embeds a source's passages that wait for a vector of the current model,
/// batch by batch, until the source is ready, or until a failure pauses it.
async fn embed(library: &Library, id: &str, pauses: &mut Pauses) {
    loop {
        let Some(source) = library.source(id) else {
            return;
        };
        if source.state != SourceState::Indexing {
            return;
        }
        let Some((profile, key)) = library.embedding() else {
            return;
        };
        let batch = match library.store.passages_to_embed(id, &key, BATCH).await {
            Ok(batch) => batch,
            Err(error) => {
                tracing::warn!(source = %id, error.code = error.code(), "the passages of a source could not be read");
                pause(library, id, pauses, "storage_failed").await;
                return;
            }
        };
        if batch.is_empty() {
            if finish(library, id, &key).await {
                pauses.sources.remove(id);
            } else {
                pause(library, id, pauses, "storage_failed").await;
            }
            return;
        }
        let inputs: Vec<String> = batch
            .iter()
            .map(|passage| document_text(&source.name, passage))
            .collect();
        match library
            .models
            .embed(&profile, InputKind::Document, inputs)
            .await
        {
            Ok(vectors) if vectors.len() == batch.len() => {
                // A batch that the store did not keep would come back at
                // once, so the source waits instead.
                if !store_batch(library, id, &key, &batch, vectors).await {
                    pause(library, id, pauses, "storage_failed").await;
                    return;
                }
                pauses.sources.remove(id);
            }
            Ok(_vectors) => {
                tracing::warn!(source = %id, "the embedding server gave another number of vectors");
                pause(library, id, pauses, "server_error").await;
                return;
            }
            Err(error) => {
                handle_failure(library, id, pauses, &error).await;
                return;
            }
        }
    }
}

/// Stores a batch's vectors, scaled to a length of 1, with the source's
/// progress. Returns whether the store kept them; a source that the owner
/// removed meanwhile counts as kept.
async fn store_batch(
    library: &Library,
    id: &str,
    key: &str,
    batch: &[PendingPassage],
    vectors: Vec<Vec<f32>>,
) -> bool {
    let vectors: Vec<(u32, Vec<f32>)> = batch
        .iter()
        .zip(vectors)
        // A vector without length is kept as it is: no query is near it.
        .map(|(passage, vector)| (passage.number, normalized(vector.clone()).unwrap_or(vector)))
        .collect();
    let embedded = match library.store.embedded_count(id, key).await {
        Ok(count) => count.saturating_add(u32::try_from(vectors.len()).unwrap_or(u32::MAX)),
        Err(error) => {
            tracing::warn!(source = %id, error.code = error.code(), "the progress of a source could not be read");
            return false;
        }
    };
    let stored = match library
        .store
        .store_vectors(id, key, &vectors, embedded)
        .await
    {
        Ok(()) => true,
        Err(failure) => {
            log_write(id, &failure);
            library.source(id).is_none()
        }
    };
    if let Some(source) = library.source(id) {
        library.forget_corpus(&source.notebook_id);
    }
    if let Err(error) = library.refresh().await {
        tracing::warn!(error.code = %error, "the notebooks could not be read after a batch of vectors");
    }
    stored
}

/// Marks a source whose passages all have vectors as ready. Returns whether
/// the store kept it.
async fn finish(library: &Library, id: &str, key: &str) -> bool {
    let embedded = library.store.embedded_count(id, key).await.unwrap_or(0);
    // A source that the owner removed meanwhile needs nothing more.
    let stored = match library
        .store
        .set_ready(id, key, embedded, library.now())
        .await
    {
        Ok(()) | Err(WriteFailure::Gone) => true,
        Err(failure) => {
            log_write(id, &failure);
            false
        }
    };
    if let Some(source) = library.source(id) {
        library.forget_corpus(&source.notebook_id);
    }
    tracing::info!(source = %id, passages = embedded, "a source is indexed");
    if let Err(error) = library.refresh().await {
        tracing::warn!(error.code = %error, "the notebooks could not be read after a source was indexed");
    }
    stored
}

async fn handle_failure(library: &Library, id: &str, pauses: &mut Pauses, error: &ModelsError) {
    tracing::info!(source = %id, error.code = error.code(), "the embedding of a source failed");
    match error {
        // The profile is gone: the source waits for a model.
        ModelsError::ProfileNotFound | ModelsError::WrongKind => {
            if let Err(failure) = library
                .store
                .set_state(id, SourceState::Waiting, None)
                .await
            {
                log_write(id, &failure);
            }
            if let Err(error) = library.follow_models().await {
                tracing::warn!(error.code = %error, "the notebooks could not follow the models");
            }
        }
        // A passage that the model cannot take never fits; trying again
        // changes nothing.
        ModelsError::InputTooLong | ModelsError::InvalidInputs => {
            fail(library, id, models_failure(error)).await;
        }
        _other => pause(library, id, pauses, models_failure(error)).await,
    }
}

/// Pauses a source after a failure, or fails it with `code` once the waits
/// are used up.
async fn pause(library: &Library, id: &str, pauses: &mut Pauses, code: &'static str) {
    let failures = pauses
        .sources
        .get(id)
        .map_or(0, |(failures, _at)| *failures);
    let Some(wait) = library.timing.retry_delays.get(failures) else {
        pauses.sources.remove(id);
        fail(library, id, code).await;
        return;
    };
    pauses
        .sources
        .insert(id.to_owned(), (failures + 1, Instant::now() + *wait));
    tracing::info!(source = %id, code, failures = failures + 1, wait_ms = wait.as_millis(), "a source waits before it is indexed again");
    // The source shows why it waits until a batch is stored.
    if let Err(failure) = library.store.set_paused(id, code).await {
        log_write(id, &failure);
    }
    if let Err(error) = library.refresh().await {
        tracing::warn!(error.code = %error, "the notebooks could not be read after a source paused");
    }
}

async fn fail(library: &Library, id: &str, code: &'static str) {
    if let Err(failure) = library
        .store
        .set_state(id, SourceState::Failed, Some(code))
        .await
    {
        log_write(id, &failure);
    }
    if let Err(error) = library.refresh().await {
        tracing::warn!(error.code = %error, "the notebooks could not be read after a source failed");
    }
}

#[cfg(test)]
mod tests {
    use mannie_notebooks_documents::{Document, Location, Section};

    use super::{cut_to, place_words, split, split_stored};
    use crate::commands::SectionPlace;
    use crate::store::StoredSection;

    #[test]
    fn stored_sections_split_as_their_document_does() {
        let long = "A sentence of the page. ".repeat(200);
        let sections = vec![
            StoredSection {
                position: 0,
                place: SectionPlace::Page { number: 1 },
                text: long.trim().to_owned(),
            },
            StoredSection {
                position: 1,
                place: SectionPlace::Headings {
                    path: vec!["Waves".to_owned()],
                },
                text: "Waves carry energy.".to_owned(),
            },
        ];

        let passages = split_stored(&sections);

        // 4,800 characters make four passages of about 1,500 that overlap.
        assert!(passages.len() >= 3, "{}", passages.len());
        assert!(passages.iter().all(|passage| passage.text.len() <= 1_800));
        let last = passages.last().unwrap();
        assert_eq!(
            (last.section, last.text.as_str()),
            (1, "Waves carry energy.")
        );
    }

    #[test]
    fn a_document_splits_into_its_sections_and_passages_with_their_text() {
        let document = Document {
            sections: vec![
                Section {
                    location: Location::Page(1),
                    text: "Light bends.".to_owned(),
                },
                Section {
                    location: Location::Headings(vec!["Waves".to_owned()]),
                    text: "Waves carry energy.".to_owned(),
                },
            ],
            pages: Some(2),
        };

        let (sections, passages) = split(&document);

        assert_eq!(sections[0].place, SectionPlace::Page { number: 1 });
        assert_eq!(sections[1].position, 1);
        assert_eq!(passages.len(), 2);
        assert_eq!((passages[1].number, passages[1].section), (1, 1));
        assert_eq!(passages[1].text, "Waves carry energy.");
    }

    #[test]
    fn a_place_reads_as_a_page_or_its_headings() {
        assert_eq!(
            place_words(&SectionPlace::Page { number: 3 }).as_deref(),
            Some("page 3")
        );
        assert_eq!(
            place_words(&SectionPlace::Headings {
                path: vec!["A".to_owned(), "B".to_owned()]
            })
            .as_deref(),
            Some("A > B")
        );
        assert_eq!(
            place_words(&SectionPlace::Headings { path: Vec::new() }),
            None
        );
        assert_eq!(place_words(&SectionPlace::Whole), None);
    }

    #[test]
    fn a_cut_ends_at_a_character_s_start() {
        assert_eq!(cut_to("Grüße", 3), "Gr");
        assert_eq!(cut_to("Grüße", 100), "Grüße");
    }
}
