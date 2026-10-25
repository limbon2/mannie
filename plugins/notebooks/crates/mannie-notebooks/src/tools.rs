//! The plugin's tools (ADR-0052; plugins/notebooks/docs/README.md, tools):
//! the model lists, searches and reads the notebooks that are attached to its
//! conversation, or to the conversation that gave a child its task. The core
//! offers them only there and passes the attached notebooks with each call,
//! and a tool reaches only those notebooks and their sources that answers
//! use. They read only the plugin's own data, so the owner is never asked.
//!
//! What a tool gives cites by the links that the core's excerpts carry
//! (ADR-0049, decision 4), so the model copies a link from either.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::points::context::cite_link;
use mannie_plugin_sdk::points::tools::{
    AgentSelector, PluginTool, Tool, ToolAccess, ToolCall, ToolExecution, ToolOffering,
    ToolRefusal, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;

use crate::commands::{SectionPlace, SourceState};
use crate::indexer::place_words;
use crate::library::Library;
use crate::plugin::NOTEBOOKS;
use crate::provider::{
    NO_EMBEDDING_PROFILE, NOTHING_INDEXED, STORAGE_FAILED, Search, cite_label, find, key_of,
    kind_words, passage_cite, passage_text,
};
use crate::store::{SectionPassage, SourceRecord, StoredSection};

/// The tool that lists the attached notebooks' sources.
pub(crate) const LIST_SOURCES: &str = "list_notebook_sources";

/// The tool that searches the attached notebooks.
pub(crate) const SEARCH: &str = "search_notebooks";

/// The tool that reads a source.
pub(crate) const READ_SOURCE: &str = "read_notebook_source";

/// The agents that the tools are offered to: the owner's conversations and
/// the child agents that answer from documents, as the earlier harness's
/// subagents did (ADR-0052, decision 4).
const AGENTS: [&str; 3] = ["conversation", "general", "explore"];

/// The passages that a search gives unless the model asks for another
/// number, and the most that it may ask for.
const DEFAULT_HITS: u32 = 6;
const MOST_HITS: u32 = 12;

/// The characters that a read gives unless the model asks for another
/// number, and the fewest and most that it may ask for.
const DEFAULT_READ_CHARACTERS: u32 = 6_000;
const FEWEST_READ_CHARACTERS: u32 = 500;
const MOST_READ_CHARACTERS: u32 = 20_000;

/// The share of a read by passage that comes before the passage, so the
/// model reads around it.
const BEFORE_PASSAGE_SHARE: u32 = 5;

/// The sections that a read takes from the store at a time.
const SECTIONS_PER_QUERY: u32 = 16;

/// The most sections that the outline at the start of a Markdown source
/// names.
const OUTLINED_SECTIONS: usize = 60;

/// The bytes of a result that its lines around the text take, at most, so
/// the text fits the core's limit with them.
const RESULT_MARGIN_BYTES: usize = 2 * 1024;

/// The failure code of a page that the source does not have.
const PAGE_NOT_FOUND: &str = "page_not_found";

/// The plugin's tools, offered where notebooks are attached.
pub(crate) fn tools(library: &Arc<Library>) -> Vec<PluginTool> {
    let offered = |tool: Arc<dyn Tool>| {
        PluginTool::new(
            tool,
            AgentSelector::Agents(AGENTS.map(str::to_owned).to_vec()),
        )
        .offered_where(ToolOffering::WhereAttached(NOTEBOOKS.to_owned()))
    };
    vec![
        offered(Arc::new(ListSources(Arc::clone(library)))),
        offered(Arc::new(SearchNotebooks(Arc::clone(library)))),
        offered(Arc::new(ReadSource(Arc::clone(library)))),
    ]
}

/// Reads a call's arguments. A model that calls a tool without arguments may
/// send nothing.
fn read_arguments<A: DeserializeOwned>(text: &str) -> Result<A, ToolRefusal> {
    let text = if text.trim().is_empty() || text.trim() == "null" {
        "{}"
    } else {
        text
    };
    serde_json::from_str(text)
        .map_err(|error| ToolRefusal::new(format!("The arguments are not valid: {error}.")))
}

/// The attached notebooks that exist, or a refusal when none does.
fn attached_notebooks(
    library: &Library,
    attachments: &[String],
) -> Result<Vec<String>, ToolRefusal> {
    let notebooks = library.existing_notebooks(attachments);
    if notebooks.is_empty() {
        return Err(ToolRefusal::new(
            "No notebook is attached to this conversation any more.",
        ));
    }
    Ok(notebooks)
}

/// The notebooks that a call names, each attached, or every attached one.
fn chosen_notebooks(
    library: &Library,
    attachments: &[String],
    named: Option<&[String]>,
) -> Result<Vec<String>, ToolRefusal> {
    let attached = attached_notebooks(library, attachments)?;
    let Some(named) = named.filter(|named| !named.is_empty()) else {
        return Ok(attached);
    };
    let mut chosen = Vec::new();
    for notebook in named {
        if !attached.contains(notebook) {
            return Err(ToolRefusal::new(format!(
                "The notebook {notebook} is not attached to this conversation. The attached notebooks are {}.",
                notebook_list(library, &attached)
            )));
        }
        if !chosen.contains(notebook) {
            chosen.push(notebook.clone());
        }
    }
    Ok(chosen)
}

/// The notebooks as a sentence names them: each name with its ID.
fn notebook_list(library: &Library, notebooks: &[String]) -> String {
    notebooks
        .iter()
        .map(|id| {
            let name = library.notebook_name(id).unwrap_or_default();
            format!("\"{name}\" ({id})")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A source that a call names, which must be in an attached notebook and used
/// in answers.
fn chosen_source(
    library: &Library,
    attachments: &[String],
    source: &str,
) -> Result<SourceRecord, ToolRefusal> {
    let attached = attached_notebooks(library, attachments)?;
    let record = library
        .source(source)
        .filter(|record| attached.contains(&record.notebook_id))
        .ok_or_else(|| {
            ToolRefusal::new(format!(
                "The source {source} is not in the attached notebooks. list_notebook_sources lists their sources."
            ))
        })?;
    if !record.included {
        return Err(ToolRefusal::new(format!(
            "The owner switched the source {source} off for answers, so it is not read."
        )));
    }
    Ok(record)
}

/// How a source's state reads after its kind, or nothing for a source that
/// is searched in full.
fn state_words(source: &SourceRecord) -> String {
    match source.state {
        SourceState::Ready => String::new(),
        SourceState::Reading => ", being read".to_owned(),
        SourceState::Waiting => ", waiting for an embedding model".to_owned(),
        SourceState::Indexing => ", being indexed: searched by its words so far".to_owned(),
        SourceState::Failed => format!(
            ", failed ({})",
            source.failure.as_deref().unwrap_or("unknown")
        ),
    }
}

/// A source as a line names it.
fn source_line(source: &SourceRecord) -> String {
    let pages = source
        .pages
        .map(|pages| format!(", {pages} pages"))
        .unwrap_or_default();
    format!(
        "- \"{}\" (source {}, {}{pages}, {} passages{})",
        source.name,
        source.id,
        kind_words(source.kind),
        source.passages,
        state_words(source)
    )
}

/// The arguments of `list_notebook_sources`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArguments {
    #[serde(default)]
    notebook: Option<String>,
}

/// `list_notebook_sources`.
struct ListSources(Arc<Library>);

impl fmt::Debug for ListSources {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ListSources")
            .finish_non_exhaustive()
    }
}

impl ListSources {
    fn chosen(&self, arguments: &str, attachments: &[String]) -> Result<Vec<String>, ToolRefusal> {
        let arguments: ListArguments = read_arguments(arguments)?;
        let named = arguments.notebook.map(|notebook| vec![notebook]);
        chosen_notebooks(&self.0, attachments, named.as_deref())
    }
}

impl Tool for ListSources {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            LIST_SOURCES,
            "Lists the sources of the notebooks attached to this conversation: each source's ID, \
             kind, pages, passages and whether it is searched yet. Sources that the owner switched \
             off for answers are left out.",
            json!({
                "type": "object",
                "properties": {
                    "notebook": {
                        "type": ["string", "null"],
                        "description": "The ID of one attached notebook, or null for every attached notebook."
                    }
                },
                "required": ["notebook"],
                "additionalProperties": false
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let checked = self
            .chosen(&call.arguments, &call.attachments)
            .map(|_chosen| Vec::new());
        Box::pin(std::future::ready(checked))
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let result = match self.chosen(&execution.arguments, &execution.attachments) {
            Ok(chosen) => list(&self.0, &chosen),
            Err(refusal) => ToolResult::new(refusal.detail),
        };
        Box::pin(std::future::ready(result))
    }
}

/// The sources of the notebooks, each notebook with its name and ID.
fn list(library: &Library, notebooks: &[String]) -> ToolResult {
    let sources: Vec<SourceRecord> = library
        .sources()
        .into_iter()
        .filter(|source| source.included)
        .collect();
    let mut lines = Vec::new();
    let mut listed = 0_u32;
    for notebook in notebooks {
        let name = library.notebook_name(notebook).unwrap_or_default();
        let held: Vec<&SourceRecord> = sources
            .iter()
            .filter(|source| source.notebook_id == *notebook)
            .collect();
        if held.is_empty() {
            lines.push(format!(
                "Notebook \"{name}\" (notebook {notebook}) holds no source that answers use."
            ));
            continue;
        }
        lines.push(format!(
            "Notebook \"{name}\" (notebook {notebook}), {} sources:",
            held.len()
        ));
        for source in held {
            lines.push(source_line(source));
            listed += 1;
        }
    }
    ToolResult::new(lines.join("\n")).with_data(json!({
        "notebooks": notebooks.len(),
        "sources": listed,
    }))
}

/// The arguments of `search_notebooks`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArguments {
    query: String,
    #[serde(default)]
    notebooks: Option<Vec<String>>,
    #[serde(default)]
    sources: Option<Vec<String>>,
    #[serde(default)]
    limit: Option<u32>,
}

/// A search that a call asks for, checked against the attachments.
#[derive(Debug)]
struct ChosenSearch {
    query: String,
    notebooks: Vec<String>,
    sources: Option<HashSet<String>>,
    hits: usize,
}

/// `search_notebooks`.
struct SearchNotebooks(Arc<Library>);

impl fmt::Debug for SearchNotebooks {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchNotebooks")
            .finish_non_exhaustive()
    }
}

impl SearchNotebooks {
    fn chosen(&self, arguments: &str, attachments: &[String]) -> Result<ChosenSearch, ToolRefusal> {
        let arguments: SearchArguments = read_arguments(arguments)?;
        let query = arguments.query.trim().to_owned();
        if query.is_empty() {
            return Err(ToolRefusal::new("The query is empty."));
        }
        let notebooks = chosen_notebooks(&self.0, attachments, arguments.notebooks.as_deref())?;
        let sources = match arguments.sources.filter(|sources| !sources.is_empty()) {
            None => None,
            Some(named) => {
                let mut sources = HashSet::new();
                for source in named {
                    let record = chosen_source(&self.0, attachments, &source)?;
                    if !notebooks.contains(&record.notebook_id) {
                        return Err(ToolRefusal::new(format!(
                            "The source {source} is not in the notebooks that the search names."
                        )));
                    }
                    sources.insert(source);
                }
                Some(sources)
            }
        };
        let hits = arguments.limit.unwrap_or(DEFAULT_HITS).clamp(1, MOST_HITS);
        Ok(ChosenSearch {
            query,
            notebooks,
            sources,
            hits: usize::try_from(hits).unwrap_or(1),
        })
    }
}

impl Tool for SearchNotebooks {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            SEARCH,
            "Searches the documents of the notebooks attached to this conversation by meaning and by \
             words, and returns the passages that best match the query, the best first, each with its \
             source, its place, its passage number and a cite link. Search with a question or with the \
             terms that the documents would use; search again with other words when the passages do \
             not answer, and read around a passage with read_notebook_source.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "What to find, in words that the documents would use."
                    },
                    "notebooks": {
                        "type": ["array", "null"],
                        "items": { "type": "string" },
                        "description": "The IDs of attached notebooks to search, or null for every attached notebook."
                    },
                    "sources": {
                        "type": ["array", "null"],
                        "items": { "type": "string" },
                        "description": "The IDs of sources to search, or null for every source of the notebooks."
                    },
                    "limit": {
                        "type": ["integer", "null"],
                        "description": "How many passages to return, from 1 to 12; null for 6."
                    }
                },
                "required": ["query", "notebooks", "sources", "limit"],
                "additionalProperties": false
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let checked = self
            .chosen(&call.arguments, &call.attachments)
            .map(|_chosen| Vec::new());
        Box::pin(std::future::ready(checked))
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let library = Arc::clone(&self.0);
        let chosen = self.chosen(&execution.arguments, &execution.attachments);
        let room = room_of(&execution);
        Box::pin(async move {
            match chosen {
                Ok(chosen) => search(&library, &chosen, room).await,
                Err(refusal) => ToolResult::new(refusal.detail),
            }
        })
    }
}

/// The bytes of text that a result has room for within the core's limit.
fn room_of(execution: &ToolExecution) -> usize {
    usize::try_from(execution.result_limit)
        .unwrap_or(usize::MAX)
        .saturating_sub(RESULT_MARGIN_BYTES)
}

/// What the model reads of a failed search, by its code.
fn search_failure(code: &str) -> String {
    match code {
        NO_EMBEDDING_PROFILE => "Notebooks cannot search: no embedding model is chosen. The owner chooses one in Settings, Notebooks.".to_owned(),
        NOTHING_INDEXED => "The notebooks hold no searched passage yet: their sources are being read or indexed, or they have none.".to_owned(),
        STORAGE_FAILED => "Notebooks could not read its passages.".to_owned(),
        other => format!("The search failed ({other})."),
    }
}

/// Searches, and gives the best passages that fit the room.
async fn search(library: &Library, chosen: &ChosenSearch, room: usize) -> ToolResult {
    let search = Search {
        notebooks: &chosen.notebooks,
        sources: chosen.sources.as_ref(),
        query: &chosen.query,
        earlier: &[],
    };
    let found = match find(library, &search).await {
        Ok(found) => found,
        Err(code) => return ToolResult::new(search_failure(code)).with_failure(code),
    };
    let mut lines = Vec::new();
    if let Some(code) = found.by_words_only {
        lines.push(format!(
            "The embedding server failed ({code}), so these passages were found by their words alone."
        ));
    }
    let names = notebook_list(library, &chosen.notebooks);
    let mut used = 0;
    let mut given = 0;
    let mut left_out = 0;
    for row in found.rows.iter().take(chosen.hits) {
        let place = place_words(&row.place)
            .map(|place| format!(", {place}"))
            .unwrap_or_default();
        let hit = format!(
            "[{}] {}{place} · source {}, passage {} · cite: {}\n{}",
            given + 1,
            row.source_name,
            row.source_id,
            row.number,
            passage_cite(row),
            passage_text(&row.text)
        );
        if used + hit.len() > room {
            left_out += 1;
            continue;
        }
        used += hit.len() + 2;
        given += 1;
        lines.push(hit);
    }
    let heading = if given == 0 {
        format!("Nothing in {names} matches \"{}\".", chosen.query)
    } else {
        format!(
            "The {given} passages of {names} that best match \"{}\":",
            chosen.query
        )
    };
    lines.insert(0, heading);
    if left_out > 0 {
        lines.push(format!(
            "{left_out} more passages did not fit. Search with fewer passages or read a source."
        ));
    }
    ToolResult::new(lines.join("\n\n")).with_data(json!({
        "query": chosen.query,
        "found": given,
    }))
}

/// The arguments of `read_notebook_source`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArguments {
    source: String,
    #[serde(default)]
    passage: Option<u32>,
    #[serde(default)]
    page: Option<u32>,
    #[serde(default)]
    page_end: Option<u32>,
    #[serde(default)]
    section: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
    #[serde(default)]
    limit: Option<u32>,
}

/// Where a read starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadStart {
    /// Around a passage, by its number.
    Passage(u32),
    /// At a page, through another page.
    Pages { first: u32, last: u32 },
    /// At a section, from a character of it.
    Section { section: u32, offset: u32 },
}

/// A read that a call asks for, checked against the attachments.
#[derive(Debug)]
struct ChosenRead {
    source: SourceRecord,
    start: ReadStart,
    characters: u32,
}

/// `read_notebook_source`.
struct ReadSource(Arc<Library>);

impl fmt::Debug for ReadSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("ReadSource").finish_non_exhaustive()
    }
}

impl ReadSource {
    fn chosen(&self, arguments: &str, attachments: &[String]) -> Result<ChosenRead, ToolRefusal> {
        let arguments: ReadArguments = read_arguments(arguments)?;
        let source = chosen_source(&self.0, attachments, &arguments.source)?;
        let ways = [
            arguments.passage.is_some(),
            arguments.page.is_some(),
            arguments.section.is_some(),
        ];
        if ways.into_iter().filter(|given| *given).count() > 1 {
            return Err(ToolRefusal::new(
                "Give one of passage, page and section, or none to read from the start.",
            ));
        }
        if arguments.page_end.is_some() && arguments.page.is_none() {
            return Err(ToolRefusal::new("page_end needs page."));
        }
        if arguments.offset.is_some() && arguments.section.is_none() {
            return Err(ToolRefusal::new("offset needs section."));
        }
        let start = match (arguments.passage, arguments.page, arguments.section) {
            (Some(passage), _, _) => ReadStart::Passage(passage),
            (None, Some(first), _) => {
                let last = arguments.page_end.unwrap_or(first);
                if first == 0 || last < first {
                    return Err(ToolRefusal::new(
                        "Pages count from 1, and page_end is not before page.",
                    ));
                }
                ReadStart::Pages { first, last }
            }
            (None, None, section) => ReadStart::Section {
                section: section.unwrap_or(0),
                offset: arguments.offset.unwrap_or(0),
            },
        };
        let characters = arguments
            .limit
            .unwrap_or(DEFAULT_READ_CHARACTERS)
            .clamp(FEWEST_READ_CHARACTERS, MOST_READ_CHARACTERS);
        Ok(ChosenRead {
            source,
            start,
            characters,
        })
    }
}

impl Tool for ReadSource {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            READ_SOURCE,
            "Reads the text of a source of the notebooks attached to this conversation: around a \
             passage that a search found, from a page through another page of a PDF, or from a \
             section and a character of it, as a previous read says to read on. Each page or section \
             starts with a line that holds its cite link. A read from the start of a Markdown source \
             begins with its sections.",
            json!({
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "The source's ID."
                    },
                    "passage": {
                        "type": ["integer", "null"],
                        "description": "A passage number from a search, to read around it; or null."
                    },
                    "page": {
                        "type": ["integer", "null"],
                        "description": "The first page of a PDF to read, from 1; or null."
                    },
                    "page_end": {
                        "type": ["integer", "null"],
                        "description": "The last page to read, with page; null for page alone."
                    },
                    "section": {
                        "type": ["integer", "null"],
                        "description": "The section to read from, as a previous read or the outline names it; or null."
                    },
                    "offset": {
                        "type": ["integer", "null"],
                        "description": "The character of the section to start at, with section; null for its start."
                    },
                    "limit": {
                        "type": ["integer", "null"],
                        "description": "How many characters to read, from 500 to 20000; null for 6000."
                    }
                },
                "required": ["source", "passage", "page", "page_end", "section", "offset", "limit"],
                "additionalProperties": false
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let checked = self
            .chosen(&call.arguments, &call.attachments)
            .map(|_chosen| Vec::new());
        Box::pin(std::future::ready(checked))
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let library = Arc::clone(&self.0);
        let chosen = self.chosen(&execution.arguments, &execution.attachments);
        let room = room_of(&execution);
        Box::pin(async move {
            match chosen {
                Ok(chosen) => match read(&library, &chosen, room).await {
                    Ok(result) => result,
                    Err(code) => ToolResult::new(read_failure(&chosen, code)).with_failure(code),
                },
                Err(refusal) => ToolResult::new(refusal.detail),
            }
        })
    }
}

/// What the model reads of a failed read, by its code.
fn read_failure(chosen: &ChosenRead, code: &str) -> String {
    match (code, chosen.start) {
        (PAGE_NOT_FOUND, ReadStart::Pages { first, .. }) => {
            let pages = chosen
                .source
                .pages
                .map(|pages| format!(" It has {pages} pages."))
                .unwrap_or_default();
            format!("The source has no text from page {first} on.{pages}")
        }
        (PASSAGE_NOT_FOUND, ReadStart::Passage(passage)) => format!(
            "The source has no passage {passage}. A search gives the numbers of its passages."
        ),
        (SECTION_NOT_FOUND, ReadStart::Section { section, .. }) => format!(
            "The source has no section {section}. A read from its start lists the sections of a Markdown source."
        ),
        (NO_TEXT, _) => "The source holds no text yet: it is being read, or it failed.".to_owned(),
        _other => "Notebooks could not read the source.".to_owned(),
    }
}

/// The failure code of a passage that the source does not have.
const PASSAGE_NOT_FOUND: &str = "passage_not_found";

/// The failure code of a section past the source's end.
const SECTION_NOT_FOUND: &str = "section_not_found";

/// The failure code of a source without text.
const NO_TEXT: &str = "no_text";

/// The section and the character where a read starts, and the page where it
/// stops, if it stops at one.
async fn start_of(
    library: &Library,
    chosen: &ChosenRead,
) -> Result<(u32, usize, Option<u32>), &'static str> {
    let store = &library.store;
    let source = &chosen.source.id;
    match chosen.start {
        ReadStart::Section { section, offset } => {
            Ok((section, usize::try_from(offset).unwrap_or(usize::MAX), None))
        }
        ReadStart::Passage(number) => {
            let (section, start, _end) = store
                .passage(source, number)
                .await
                .map_err(|_failed| STORAGE_FAILED)?
                .ok_or(PASSAGE_NOT_FOUND)?;
            let text = store
                .section_text(source, section)
                .await
                .map_err(|_failed| STORAGE_FAILED)?
                .ok_or(PASSAGE_NOT_FOUND)?;
            let start = usize::try_from(start).unwrap_or(0);
            let before = text.get(..start).map_or(0, |prefix| prefix.chars().count());
            let lead = usize::try_from(chosen.characters / BEFORE_PASSAGE_SHARE).unwrap_or(0);
            Ok((section, before.saturating_sub(lead), None))
        }
        ReadStart::Pages { first, last } => {
            let places = store
                .section_places(source)
                .await
                .map_err(|_failed| STORAGE_FAILED)?;
            let section = places
                .iter()
                .find(|(_position, place)| {
                    matches!(place, SectionPlace::Page { number } if *number >= first)
                })
                .map(|(position, _place)| *position)
                .ok_or(PAGE_NOT_FOUND)?;
            Ok((section, 0, Some(last)))
        }
    }
}

/// The page of a section, if it is one.
fn page_of(place: &SectionPlace) -> Option<u32> {
    match place {
        SectionPlace::Page { number } => Some(*number),
        SectionPlace::Headings { .. } | SectionPlace::Whole => None,
    }
}

/// The text of a section from a character, at most `characters` of it: the
/// byte where it starts, the text, and the character where it stopped, or
/// `None` when it reached the end.
fn piece(text: &str, from: usize, characters: usize) -> (usize, &str, Option<usize>) {
    let start = text
        .char_indices()
        .nth(from)
        .map_or(text.len(), |(index, _character)| index);
    let rest = &text[start..];
    match rest.char_indices().nth(characters) {
        Some((end, _character)) => (start, &rest[..end], Some(from + characters)),
        None => (start, rest, None),
    }
}

/// The key of the passage that holds a byte of a section: the last one that
/// starts at or before it, or the section's first.
fn passage_at(passages: &[SectionPassage], section: u32, byte: u32) -> Option<u32> {
    let mut found = None;
    for passage in passages.iter().filter(|passage| passage.section == section) {
        if passage.start <= byte || found.is_none() {
            found = Some(passage.number);
        }
        if passage.start > byte {
            break;
        }
    }
    found
}

/// The line that starts a section of a read: its place and its cite link.
fn section_line(source: &SourceRecord, section: &StoredSection, passage: Option<u32>) -> String {
    let place = place_words(&section.place).unwrap_or_else(|| "text".to_owned());
    match passage {
        Some(number) => format!(
            "--- section {}, {place} · cite: {} ---",
            section.position,
            cite_link(
                NOTEBOOKS,
                &key_of(&source.id, number),
                &cite_label(&source.name, &section.place)
            )
        ),
        None => format!("--- section {}, {place} ---", section.position),
    }
}

/// The sections of a Markdown source, for a read from its start.
async fn outline(library: &Library, source: &SourceRecord) -> Result<Option<String>, &'static str> {
    let places = library
        .store
        .section_places(&source.id)
        .await
        .map_err(|_failed| STORAGE_FAILED)?;
    let named: Vec<String> = places
        .iter()
        .filter_map(|(position, place)| match place {
            SectionPlace::Headings { path } if !path.is_empty() => {
                Some(format!("[{position}] {}", path.join(" › ")))
            }
            SectionPlace::Headings { .. } | SectionPlace::Page { .. } | SectionPlace::Whole => None,
        })
        .collect();
    if named.is_empty() {
        return Ok(None);
    }
    let more = named.len().saturating_sub(OUTLINED_SECTIONS);
    let mut lines: Vec<String> = named.into_iter().take(OUTLINED_SECTIONS).collect();
    if more > 0 {
        lines.push(format!("and {more} more sections"));
    }
    Ok(Some(format!("Sections: {}", lines.join("; "))))
}

/// What a read gathered so far: its lines, its room, and what its card
/// shows.
#[derive(Debug)]
struct Reading {
    lines: Vec<String>,
    /// The sections that it gave text of.
    sections: usize,
    /// The characters that it may still give.
    left: usize,
    /// The bytes that its lines take, and the most that they may take.
    used: usize,
    room: usize,
    first_page: Option<u32>,
    last_page: Option<u32>,
    first_heading: Option<String>,
}

impl Reading {
    /// A read of a source that starts with the line that names the source.
    fn new(source: &SourceRecord, characters: u32, room: usize) -> Self {
        let pages = source
            .pages
            .map(|pages| format!(", {pages} pages"))
            .unwrap_or_default();
        let first = format!(
            "\"{}\" (source {}, {}{pages}):",
            source.name,
            source.id,
            kind_words(source.kind)
        );
        Self {
            used: first.len(),
            lines: vec![first],
            sections: 0,
            left: usize::try_from(characters).unwrap_or(usize::MAX),
            room,
            first_page: None,
            last_page: None,
            first_heading: None,
        }
    }

    /// Adds a line, such as the sections of a Markdown source.
    fn push_line(&mut self, line: String) {
        self.used += line.len() + 2;
        self.lines.push(line);
    }

    /// Whether the read has no room for more text.
    fn is_full(&self) -> bool {
        self.left == 0 || self.used >= self.room
    }

    /// Adds a section's text from a character, and returns the character
    /// where it stopped, or `None` when it gave the rest of the section.
    fn add(
        &mut self,
        source: &SourceRecord,
        section: &StoredSection,
        passages: &[SectionPassage],
        from: usize,
    ) -> Option<usize> {
        // A character takes at most four bytes, so the text fits the
        // result's room whatever its script.
        let room = self
            .left
            .min(self.room.saturating_sub(self.used) / 4)
            .max(1);
        let (start_byte, text, stopped) = piece(&section.text, from, room);
        let byte = u32::try_from(start_byte).unwrap_or(u32::MAX);
        let line = section_line(
            source,
            section,
            passage_at(passages, section.position, byte),
        );
        self.push_line(format!("{line}\n{}", text.trim_end()));
        self.sections += 1;
        self.left = self.left.saturating_sub(text.chars().count());
        let page = page_of(&section.place);
        if self.first_page.is_none() {
            self.first_page = page;
        }
        if page.is_some() {
            self.last_page = page;
        }
        if self.first_heading.is_none()
            && let SectionPlace::Headings { path } = &section.place
        {
            self.first_heading = path.last().cloned();
        }
        stopped
    }

    /// The result, which ends with where the source goes on, if it does.
    fn finish(mut self, source: &SourceRecord, continues: Option<(u32, usize)>) -> ToolResult {
        self.lines.push(match continues {
            Some((section, offset)) => format!(
                "The source goes on. To read on, call {READ_SOURCE} with source \"{}\", section {section} and offset {offset}.",
                source.id
            ),
            None => "The source ends here.".to_owned(),
        });
        ToolResult::new(self.lines.join("\n\n")).with_data(json!({
            "source": source.name,
            "firstPage": self.first_page,
            "lastPage": self.last_page,
            "heading": self.first_heading,
        }))
    }
}

/// Reads a source from where the call asks, within its characters and the
/// result's room.
async fn read(
    library: &Library,
    chosen: &ChosenRead,
    room: usize,
) -> Result<ToolResult, &'static str> {
    let source = &chosen.source;
    let (first_section, first_character, last_page) = start_of(library, chosen).await?;
    let mut reading = Reading::new(source, chosen.characters, room);
    let from_start = first_section == 0 && first_character == 0;
    if from_start && let Some(outline) = outline(library, source).await? {
        reading.push_line(outline);
    }
    let mut next = first_section;
    let mut from = first_character;
    // Where the source goes on, if the read stopped before its end.
    let mut continues: Option<(u32, usize)> = None;
    'reading: loop {
        let (sections, total) = library
            .store
            .sections(&source.id, next, SECTIONS_PER_QUERY)
            .await
            .map_err(|_failed| STORAGE_FAILED)?;
        if sections.is_empty() {
            if reading.sections == 0 {
                return Err(if total == 0 {
                    NO_TEXT
                } else {
                    SECTION_NOT_FOUND
                });
            }
            break;
        }
        let last_position = sections.last().map_or(next, |section| section.position);
        let passages = library
            .store
            .section_passages(&source.id, next, last_position)
            .await
            .map_err(|_failed| STORAGE_FAILED)?;
        for section in &sections {
            let past_pages = last_page
                .zip(page_of(&section.place))
                .is_some_and(|(last, page)| page > last);
            if past_pages {
                // The pages asked for hold no text, such as pages that could
                // not be read, when the first page found is past them.
                if reading.sections == 0 {
                    return Err(PAGE_NOT_FOUND);
                }
                continues = Some((section.position, 0));
                break 'reading;
            }
            if reading.is_full() {
                continues = Some((section.position, from));
                break 'reading;
            }
            if let Some(stopped) = reading.add(source, section, &passages, from) {
                continues = Some((section.position, stopped));
                break 'reading;
            }
            from = 0;
        }
        next = last_position + 1;
        if next >= total {
            break;
        }
    }
    Ok(reading.finish(source, continues))
}

#[cfg(test)]
mod tests {
    use super::{SectionPassage, passage_at, piece};

    #[test]
    fn a_piece_counts_characters_and_says_where_it_stopped() {
        let text = "Grüße aus Köln.";

        assert_eq!(piece(text, 0, 5), (0, "Grüße", Some(5)));
        assert_eq!(piece(text, 6, 100), (8, "aus Köln.", None));
        assert_eq!(piece(text, 100, 5), (text.len(), "", None));
    }

    #[test]
    fn the_passage_of_a_byte_is_the_last_that_starts_before_it() {
        let passages = [
            SectionPassage {
                number: 4,
                section: 1,
                start: 0,
            },
            SectionPassage {
                number: 5,
                section: 1,
                start: 1_300,
            },
            SectionPassage {
                number: 6,
                section: 2,
                start: 0,
            },
        ];

        assert_eq!(passage_at(&passages, 1, 0), Some(4));
        assert_eq!(passage_at(&passages, 1, 1_299), Some(4));
        assert_eq!(passage_at(&passages, 1, 1_300), Some(5));
        assert_eq!(passage_at(&passages, 2, 50), Some(6));
        assert_eq!(passage_at(&passages, 3, 0), None);
    }
}
