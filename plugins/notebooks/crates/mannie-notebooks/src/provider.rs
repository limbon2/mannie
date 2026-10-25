//! The context provider `notebooks` (ADR-0046, decision 2; ADR-0049) and the
//! search that it and the plugin's tools share (plugins/notebooks/docs/
//! README.md, search).
//!
//! A search ranks a notebook's passages by meaning and by words, for the
//! message alone and, when the owner wrote earlier messages, for those
//! messages and the message together, so that a follow-up keeps its subject.
//! The rankings are fused by reciprocal rank, and the reranker orders the
//! best [`RERANKED`] against the fuller query. Nothing drops a passage: the
//! best ones go, within the room. When the embedding server fails, the
//! search goes on by words, and when the reranker fails, it keeps the fused
//! order, as the earlier harness did.

use std::collections::HashSet;
use std::sync::Arc;

use mannie_notebooks_documents::DocumentKind;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::ContextLocation;
use mannie_plugin_sdk::points::context::{ContextItem, ContextProvider, ContextRequest, cite_link};
use mannie_plugin_sdk::services::models::{InputKind, MAX_INPUT_BYTES, ModelsError};

use crate::commands::{SectionPlace, SourceState};
use crate::indexer::cut_to;
use crate::library::{Library, models_failure};
use crate::plugin::NOTEBOOKS;
use crate::search::{Corpus, MAX_QUERY_CHARACTERS, RERANKED, choose_rows, fuse, normalized};
use crate::store::CorpusRow;

/// The code of keys that name no notebook.
pub(crate) const NOTEBOOK_NOT_FOUND: &str = "notebook_not_found";

/// The code of a search without an embedding model.
pub(crate) const NO_EMBEDDING_PROFILE: &str = "no_embedding_profile";

/// The code of notebooks that hold no searched passage yet.
pub(crate) const NOTHING_INDEXED: &str = "nothing_indexed";

/// The code of a search whose passages could not be read.
pub(crate) const STORAGE_FAILED: &str = "storage_failed";

/// The most characters of a passage that an item or a tool's hit carries;
/// the tool that reads a source gives the rest.
pub(crate) const PASSAGE_CHARACTERS: usize = 2_000;

/// The most sources that an outline names; the tool that lists sources
/// names them all.
const OUTLINED_SOURCES: usize = 80;

/// A passage's text as an item or a hit carries it: at most
/// [`PASSAGE_CHARACTERS`], and "…" where it was cut.
pub(crate) fn passage_text(text: &str) -> String {
    let mut characters = text.char_indices();
    match characters.nth(PASSAGE_CHARACTERS) {
        Some((end, _character)) => format!("{}…", text[..end].trim_end()),
        None => text.to_owned(),
    }
}

/// Where a passage stands, as the core keeps it.
fn location_of(place: &SectionPlace) -> Option<ContextLocation> {
    match place {
        SectionPlace::Page { number } => Some(ContextLocation::Page { number: *number }),
        SectionPlace::Headings { path } if !path.is_empty() => {
            Some(ContextLocation::Headings { path: path.clone() })
        }
        SectionPlace::Headings { .. } | SectionPlace::Whole => None,
    }
}

/// The key of a passage, `<source ID>:<number>`, which its citations name.
pub(crate) fn passage_key(row: &CorpusRow) -> String {
    key_of(&row.source_id, row.number)
}

/// The key of a source's passage by its number.
pub(crate) fn key_of(source_id: &str, number: u32) -> String {
    format!("{source_id}:{number}")
}

/// The label of a cite link: the source's name with its page or last
/// heading, as the core labels those of the items.
pub(crate) fn cite_label(source_name: &str, place: &SectionPlace) -> String {
    match place {
        SectionPlace::Page { number } => format!("{source_name} p. {number}"),
        SectionPlace::Headings { path } => match path.last() {
            Some(heading) => format!("{source_name} › {heading}"),
            None => source_name.to_owned(),
        },
        SectionPlace::Whole => source_name.to_owned(),
    }
}

/// The cite link of a passage, which names its source and its page or last
/// heading.
pub(crate) fn passage_cite(row: &CorpusRow) -> String {
    cite_link(
        NOTEBOOKS,
        &passage_key(row),
        &cite_label(&row.source_name, &row.place),
    )
}

/// The item of a chosen passage.
fn item_of(row: CorpusRow) -> ContextItem {
    let key = passage_key(&row);
    let location = location_of(&row.place);
    ContextItem::new(key, row.source_name, location, row.text)
}

/// A search of notebooks' passages.
#[derive(Debug)]
pub(crate) struct Search<'a> {
    /// The notebooks, which exist.
    pub(crate) notebooks: &'a [String],
    /// Only these sources, or every source of the notebooks.
    pub(crate) sources: Option<&'a HashSet<String>>,
    /// What to find, such as the owner's message or a tool's query.
    pub(crate) query: &'a str,
    /// The owner's earlier messages, newest first, which make a second query
    /// with `query`.
    pub(crate) earlier: &'a [String],
}

/// What a search found: the passages, best first, before the choice that
/// fits them in a room.
#[derive(Debug)]
pub(crate) struct Found {
    pub(crate) rows: Vec<CorpusRow>,
    /// Why the search went on by words alone, as the code of the embedding
    /// server's failure, or `None` when it searched by meaning too.
    pub(crate) by_words_only: Option<&'static str>,
    /// Whether the reranker ordered the passages.
    pub(crate) reranked: bool,
}

/// The query of the earlier messages, oldest first, and the message, at
/// most [`MAX_QUERY_CHARACTERS`] of their end, so that the message stays
/// whole.
fn with_earlier(query: &str, earlier: &[String]) -> Option<String> {
    if earlier.is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = earlier.iter().rev().map(String::as_str).collect();
    parts.push(query);
    let joined = parts.join("\n");
    let characters = joined.chars().count();
    let skipped = characters.saturating_sub(MAX_QUERY_CHARACTERS);
    Some(joined.chars().skip(skipped).collect())
}

/// Searches the passages of the notebooks that are searched and used in
/// answers.
pub(crate) async fn find(library: &Library, search: &Search<'_>) -> Result<Found, &'static str> {
    let preferences = library.preferences();
    let (profile, key) = library.embedding().ok_or(NO_EMBEDDING_PROFILE)?;
    let mut corpora: Vec<Arc<Corpus>> = Vec::with_capacity(search.notebooks.len());
    for notebook in search.notebooks {
        let corpus = library
            .corpus(notebook, &key)
            .await
            .map_err(|_failed| STORAGE_FAILED)?;
        corpora.push(corpus);
    }
    if corpora.iter().all(|corpus| corpus.is_empty()) {
        return Err(NOTHING_INDEXED);
    }
    let query: String = search.query.chars().take(MAX_QUERY_CHARACTERS).collect();
    let fuller = with_earlier(&query, search.earlier);
    let texts: Vec<String> = std::iter::once(query.clone())
        .chain(fuller.clone())
        .map(|text| cut_to(&text, MAX_INPUT_BYTES).to_owned())
        .collect();
    let (vectors, by_words_only) = match library
        .models
        .embed(&profile, InputKind::Query, texts.clone())
        .await
    {
        Ok(vectors) => (vectors.into_iter().filter_map(normalized).collect(), None),
        // The profile is gone: the owner has a model to choose again.
        Err(error @ (ModelsError::ProfileNotFound | ModelsError::WrongKind)) => {
            return Err(models_failure(&error));
        }
        Err(error) => {
            let code = models_failure(&error);
            tracing::info!(
                error.code = error.code(),
                "the query could not be embedded, so the search goes on by words"
            );
            (Vec::new(), Some(code))
        }
    };
    let keep = |row: &CorpusRow| {
        search
            .sources
            .is_none_or(|sources| sources.contains(&row.source_id))
    };
    let mut rankings: Vec<Vec<(usize, usize)>> = Vec::new();
    for (index, corpus) in corpora.iter().enumerate() {
        let tagged = |ranking: Vec<usize>| -> Vec<(usize, usize)> {
            ranking
                .into_iter()
                .map(|passage| (index, passage))
                .collect()
        };
        for vector in &vectors {
            rankings.push(tagged(corpus.by_meaning(vector, &keep)));
        }
        for text in &texts {
            rankings.push(tagged(corpus.by_words(text, &keep)));
        }
    }
    let fused = fuse(&rankings);
    let rerank_query = fuller.as_deref().unwrap_or(&query);
    let (ranked, reranked) = rerank(
        library,
        preferences.rerank_profile_id.as_deref(),
        rerank_query,
        &corpora,
        fused,
    )
    .await;
    let rows = ranked
        .iter()
        .filter_map(|(corpus, passage)| corpora.get(*corpus)?.passage(*passage))
        .cloned()
        .collect();
    Ok(Found {
        rows,
        by_words_only,
        reranked,
    })
}

/// The fused passages in the reranker's order, its best [`RERANKED`] first
/// and the rest after them, or the fused order without a reranker or when it
/// fails, with whether it ordered them.
async fn rerank(
    library: &Library,
    profile: Option<&str>,
    query: &str,
    corpora: &[Arc<Corpus>],
    mut fused: Vec<(usize, usize)>,
) -> (Vec<(usize, usize)>, bool) {
    let Some(profile) = profile else {
        return (fused, false);
    };
    let rest = fused.split_off(fused.len().min(RERANKED));
    let candidates = fused;
    if candidates.is_empty() {
        return (candidates, false);
    }
    let documents: Vec<String> = candidates
        .iter()
        .filter_map(|(corpus, passage)| corpora.get(*corpus)?.passage(*passage))
        .map(|row| cut_to(&row.text, MAX_INPUT_BYTES).to_owned())
        .collect();
    let scores = library
        .models
        .rerank(
            profile,
            cut_to(query, MAX_INPUT_BYTES).to_owned(),
            documents,
        )
        .await;
    match scores {
        Ok(scores) if scores.len() == candidates.len() => {
            // The reranker orders the passages and drops none (ADR-0049):
            // the best ones go, whatever its scale of scores.
            let mut by_score: Vec<((usize, usize), f32)> =
                candidates.into_iter().zip(scores).collect();
            // A stable sort keeps the fused order among equal scores.
            by_score.sort_by(|(_first, first), (_second, second)| second.total_cmp(first));
            let ordered = by_score
                .into_iter()
                .map(|(candidate, _score)| candidate)
                .chain(rest)
                .collect();
            (ordered, true)
        }
        Ok(_scores) => {
            tracing::warn!(
                "the reranker gave another number of scores, so the search keeps the fused order"
            );
            (candidates.into_iter().chain(rest).collect(), false)
        }
        Err(error) => {
            tracing::info!(
                error.code = error.code(),
                "the reranker failed, so the search keeps the fused order"
            );
            (candidates.into_iter().chain(rest).collect(), false)
        }
    }
}

/// The provider, which reads the plugin's library.
#[derive(Debug)]
pub(crate) struct NotebookContext {
    library: Arc<Library>,
}

impl NotebookContext {
    pub(crate) fn new(library: Arc<Library>) -> Self {
        Self { library }
    }
}

impl ContextProvider for NotebookContext {
    fn describe(&self, key: String) -> BoxFuture<'static, Result<String, String>> {
        let described = self
            .library
            .notebook_name(&key)
            .ok_or_else(|| NOTEBOOK_NOT_FOUND.to_owned());
        Box::pin(std::future::ready(described))
    }

    fn gather(
        &self,
        request: ContextRequest,
    ) -> BoxFuture<'static, Result<Vec<ContextItem>, String>> {
        let library = Arc::clone(&self.library);
        Box::pin(async move { gather(&library, request).await.map_err(str::to_owned) })
    }

    fn outline(&self, keys: Vec<String>) -> BoxFuture<'static, Result<String, String>> {
        let notebooks = self.library.existing_notebooks(&keys);
        let outline = if notebooks.is_empty() {
            Err(NOTEBOOK_NOT_FOUND.to_owned())
        } else {
            Ok(outline(&self.library, &notebooks))
        };
        Box::pin(std::future::ready(outline))
    }
}

/// The passages of the request's notebooks that apply to its message, best
/// first, within the core's room.
async fn gather(
    library: &Library,
    request: ContextRequest,
) -> Result<Vec<ContextItem>, &'static str> {
    let notebooks = library.existing_notebooks(&request.keys);
    if notebooks.is_empty() {
        return Err(NOTEBOOK_NOT_FOUND);
    }
    let search = Search {
        notebooks: &notebooks,
        sources: None,
        query: &request.message,
        earlier: &request.earlier_messages,
    };
    let found = find(library, &search).await?;
    let preferences = library.preferences();
    let max_items = usize::try_from(preferences.max_passages.min(request.max_items)).unwrap_or(0);
    let max_bytes = usize::try_from(request.max_bytes).unwrap_or(usize::MAX);
    let cut: Vec<CorpusRow> = found
        .rows
        .into_iter()
        .map(|row| CorpusRow {
            text: passage_text(&row.text),
            ..row
        })
        .collect();
    let rows: Vec<&CorpusRow> = cut.iter().collect();
    let chosen = choose_rows(&rows, max_items, max_bytes);
    tracing::info!(
        notebooks = notebooks.len(),
        earlier = request.earlier_messages.len(),
        candidates = cut.len(),
        passages = chosen.len(),
        reranked = found.reranked,
        by_words_only = found.by_words_only,
        "a search of notebooks ended"
    );
    Ok(chosen.into_iter().map(item_of).collect())
}

/// How a source's kind reads in an outline.
pub(crate) fn kind_words(kind: DocumentKind) -> &'static str {
    match kind {
        DocumentKind::Pdf => "PDF",
        DocumentKind::Markdown => "Markdown",
        DocumentKind::Text => "text",
    }
}

/// What the notebooks hold, for the model's instructions: each notebook with
/// the sources that answers use, their IDs, kinds and pages, at most
/// [`OUTLINED_SOURCES`] of them.
pub(crate) fn outline(library: &Library, notebooks: &[String]) -> String {
    let sources = library.sources();
    let mut lines = Vec::new();
    let mut listed = 0;
    let mut left_out = 0;
    for notebook in notebooks {
        let Some(name) = library.notebook_name(notebook) else {
            continue;
        };
        let used: Vec<_> = sources
            .iter()
            .filter(|source| source.notebook_id == *notebook && source.included)
            .collect();
        let empty = if used.is_empty() {
            ": no sources yet"
        } else {
            ""
        };
        lines.push(format!(
            "- Notebook \"{name}\" (notebook {notebook}){empty}"
        ));
        for source in used {
            if listed == OUTLINED_SOURCES {
                left_out += 1;
                continue;
            }
            listed += 1;
            let pages = source
                .pages
                .map(|pages| format!(", {pages} pages"))
                .unwrap_or_default();
            let state = match source.state {
                SourceState::Ready => "",
                SourceState::Failed => ", failed",
                SourceState::Reading | SourceState::Waiting | SourceState::Indexing => {
                    ", not ready yet"
                }
            };
            lines.push(format!(
                "  - \"{}\" (source {}, {}{pages}{state})",
                source.name,
                source.id,
                kind_words(source.kind)
            ));
        }
    }
    if left_out > 0 {
        lines.push(format!(
            "- {left_out} more sources are not listed; list_notebook_sources lists them all."
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::ContextLocation;

    use super::{location_of, passage_cite, passage_text, with_earlier};
    use crate::commands::SectionPlace;
    use crate::store::CorpusRow;

    fn row(place: SectionPlace) -> CorpusRow {
        CorpusRow {
            source_id: "s1".to_owned(),
            source_name: "Guide.pdf".to_owned(),
            number: 4,
            section: 0,
            start: 0,
            end: 5,
            text: "Light".to_owned(),
            place,
            vector: None,
        }
    }

    #[test]
    fn a_passage_s_place_becomes_its_location() {
        assert_eq!(
            location_of(&SectionPlace::Page { number: 2 }),
            Some(ContextLocation::Page { number: 2 })
        );
        assert_eq!(
            location_of(&SectionPlace::Headings {
                path: vec!["Waves".to_owned()]
            }),
            Some(ContextLocation::Headings {
                path: vec!["Waves".to_owned()]
            })
        );
        assert_eq!(
            location_of(&SectionPlace::Headings { path: Vec::new() }),
            None
        );
        assert_eq!(location_of(&SectionPlace::Whole), None);
    }

    #[test]
    fn a_passage_s_cite_link_names_its_source_and_place() {
        assert_eq!(
            passage_cite(&row(SectionPlace::Page { number: 12 })),
            "[Guide.pdf p. 12](cite:notebooks/s1:4)"
        );
        assert_eq!(
            passage_cite(&row(SectionPlace::Headings {
                path: vec!["Optics".to_owned(), "Lenses".to_owned()]
            })),
            "[Guide.pdf › Lenses](cite:notebooks/s1:4)"
        );
        assert_eq!(
            passage_cite(&row(SectionPlace::Whole)),
            "[Guide.pdf](cite:notebooks/s1:4)"
        );
    }

    #[test]
    fn a_long_passage_is_cut_with_a_mark() {
        let long = "word ".repeat(1_000);

        let cut = passage_text(&long);

        assert!(cut.ends_with('…'));
        assert!(cut.chars().count() <= 2_001);
        assert_eq!(passage_text("Short."), "Short.");
    }

    #[test]
    fn earlier_messages_come_first_and_the_message_stays_whole() {
        assert_eq!(with_earlier("And the proxy?", &[]), None);
        assert_eq!(
            with_earlier(
                "And the proxy?",
                &[
                    "How does SSH work?".to_owned(),
                    "What is WALLIX?".to_owned()
                ]
            )
            .as_deref(),
            Some("What is WALLIX?\nHow does SSH work?\nAnd the proxy?")
        );
        let long = "x".repeat(5_000);
        let fuller = with_earlier("And the proxy?", &[long]).unwrap();
        assert_eq!(fuller.chars().count(), 4_000);
        assert!(fuller.ends_with("\nAnd the proxy?"));
    }
}
