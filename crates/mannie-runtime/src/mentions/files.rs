//! The core's `files` provider: the files and folders of a conversation's
//! workspace (ADR-0032, decision 9; docs/features/chat/mentions.md, files and
//! folders). It reads only what the policy allows the model to read without
//! asking, and gives the model a note for the rest.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_plugin_sdk::points::mentions::{MentionContent, ProviderSuggestions, RenderedMention};
use mannie_tools::{
    FileMention, FolderMention, IndexMatch, MentionIndex, MentionPathError, MentionReadFailure,
    PageShows, Verdict, may_hold_secrets, resolve_mention,
};
use mannie_types::mentions::{MentionEntry, MentionSummary};
use tokio::time::Instant;

use super::{MentionContext, MentionProvider};
use crate::texts;

/// The ID of the core's provider of files and folders.
pub const FILES: &str = "files";

/// How long the index of a workspace serves queries before it is built again,
/// so that typing a query walks the folder once.
const INDEX_LIFETIME: Duration = Duration::from_secs(15);

/// The most workspaces whose indexes are kept.
const MAX_INDEXES: usize = 4;

/// The variant of a file's entry.
const FILE: &str = "file";
/// The variant of a folder's entry.
const FOLDER: &str = "folder";
/// The warning on a file that may hold secrets.
const SECRET: &str = "secret";

/// The refusal of a key that is not a path below the workspace.
const OUTSIDE_WORKSPACE: &str = "outside_workspace";
/// The refusal of a key where there is no workspace.
const NO_WORKSPACE: &str = "no_workspace";
/// The failure of a list whose workspace cannot be read.
const FOLDER_UNREADABLE: &str = "folder_unreadable";

/// The core's provider of the files and folders of a workspace.
#[derive(Debug, Default)]
pub struct FileMentions {
    indexes: Mutex<Vec<CachedIndex>>,
}

#[derive(Debug)]
struct CachedIndex {
    root: PathBuf,
    built: Instant,
    index: Arc<MentionIndex>,
}

impl FileMentions {
    /// The provider, without indexes.
    pub fn new() -> Self {
        Self::default()
    }

    /// The index of a workspace: the one built in the last 15 seconds, or a
    /// new one.
    async fn index(&self, root: &Path) -> Result<Arc<MentionIndex>, String> {
        if let Some(index) = self.cached(root) {
            return Ok(index);
        }
        let folder = root.to_path_buf();
        let built = tokio::task::spawn_blocking(move || {
            folder.is_dir().then(|| MentionIndex::build(&folder))
        })
        .await
        .map_err(|_failed| FOLDER_UNREADABLE.to_owned())?
        .ok_or_else(|| FOLDER_UNREADABLE.to_owned())?;
        let index = Arc::new(built);
        self.keep(root, Arc::clone(&index));
        Ok(index)
    }

    fn cached(&self, root: &Path) -> Option<Arc<MentionIndex>> {
        let indexes = self.indexes.lock().unwrap_or_else(PoisonError::into_inner);
        indexes
            .iter()
            .find(|cached| cached.root == root && cached.built.elapsed() < INDEX_LIFETIME)
            .map(|cached| Arc::clone(&cached.index))
    }

    fn keep(&self, root: &Path, index: Arc<MentionIndex>) {
        let mut indexes = self.indexes.lock().unwrap_or_else(PoisonError::into_inner);
        indexes.retain(|cached| cached.root != root && cached.built.elapsed() < INDEX_LIFETIME);
        indexes.push(CachedIndex {
            root: root.to_path_buf(),
            built: Instant::now(),
            index,
        });
        let surplus = indexes.len().saturating_sub(MAX_INDEXES);
        indexes.drain(..surplus);
    }
}

impl MentionProvider for FileMentions {
    fn id(&self) -> &str {
        FILES
    }

    fn offered(&self, context: &MentionContext) -> bool {
        context.workspace().is_some()
    }

    fn suggest<'a>(
        &'a self,
        context: &'a MentionContext,
        query: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, Result<ProviderSuggestions, String>> {
        async move {
            let Some(root) = context.workspace() else {
                return Ok(ProviderSuggestions::default());
            };
            let index = self.index(root).await?;
            let found = index.suggest(query, limit);
            Ok(ProviderSuggestions::new(
                found.matches.iter().map(entry).collect(),
                found.incomplete,
            ))
        }
        .boxed()
    }

    fn resolve<'a>(
        &'a self,
        context: &'a MentionContext,
        key: &'a str,
        max_bytes: usize,
    ) -> BoxFuture<'a, Result<Box<dyn MentionContent>, String>> {
        async move {
            let Some(root) = context.workspace().map(Path::to_path_buf) else {
                return Err(NO_WORKSPACE.to_owned());
            };
            let context = context.clone();
            let key = key.to_owned();
            let read = tokio::task::spawn_blocking(move || read(&context, &root, &key, max_bytes))
                .await
                .map_err(|_failed| FOLDER_UNREADABLE.to_owned())??;
            let mention: Box<dyn MentionContent> = Box::new(read);
            Ok(mention)
        }
        .boxed()
    }
}

/// The entry of an indexed path.
fn entry(found: &IndexMatch) -> MentionEntry {
    let label = if found.folder {
        format!("{}/", found.path)
    } else {
        found.path.clone()
    };
    let token = if label.chars().any(char::is_whitespace) {
        format!("@\"{label}\"")
    } else {
        format!("@{label}")
    };
    let warning = (!found.folder && may_hold_secrets(&found.path)).then(|| SECRET.to_owned());
    MentionEntry {
        key: found.path.clone(),
        label,
        token,
        variant: variant(found.folder).to_owned(),
        warning,
        expands: found.folder,
    }
}

fn variant(folder: bool) -> &'static str {
    if folder { FOLDER } else { FILE }
}

/// What a mention of a file or folder gives.
#[derive(Debug)]
struct FileContent {
    /// The path below the workspace.
    shown: String,
    folder: bool,
    body: Body,
}

#[derive(Debug)]
enum Body {
    File(FileMention),
    Folder(FolderMention),
    Note(Note),
}

/// Why the model gets a note instead of the content.
#[derive(Debug, Clone, Copy)]
enum Note {
    NotText,
    TooLarge { limit_bytes: u32 },
    Unreadable,
    NeedsApproval,
    NotFound,
}

/// Checks the key, asks the policy and reads the file or folder.
fn read(
    context: &MentionContext,
    root: &Path,
    key: &str,
    max_bytes: usize,
) -> Result<FileContent, String> {
    let target = match resolve_mention(root, key) {
        Ok(target) => target,
        Err(MentionPathError::OutsideWorkspace) => return Err(OUTSIDE_WORKSPACE.to_owned()),
        Err(MentionPathError::NotFound) => return Ok(note(key, false, Note::NotFound)),
        Err(MentionPathError::Unreadable { kind }) => {
            tracing::debug!(%kind, "a mentioned path could not be resolved");
            return Ok(note(key, false, Note::Unreadable));
        }
    };
    let allowed = matches!(
        context.decide(std::slice::from_ref(&target.access)),
        Verdict::Unrestricted | Verdict::Allowed(_)
    );
    if !allowed {
        return Ok(note(&target.shown, target.folder, Note::NeedsApproval));
    }
    let body = if target.folder {
        FolderMention::list(&target.path, &target.shown, max_bytes).map(Body::Folder)
    } else {
        FileMention::read(&target.path, max_bytes).map(Body::File)
    };
    let body = body.unwrap_or_else(|failure| {
        Body::Note(match failure {
            MentionReadFailure::NotText => Note::NotText,
            MentionReadFailure::TooLarge { limit_bytes } => Note::TooLarge { limit_bytes },
            MentionReadFailure::Unreadable { kind } => {
                tracing::debug!(%kind, "a mentioned path could not be read");
                Note::Unreadable
            }
        })
    });
    Ok(FileContent {
        shown: target.shown,
        folder: target.folder,
        body,
    })
}

/// A mention that gives a note: the key as the owner reads it, with `/`
/// between names.
fn note(key: &str, folder: bool, note: Note) -> FileContent {
    let separated = if cfg!(windows) {
        key.replace('\\', "/")
    } else {
        key.to_owned()
    };
    let shown = separated.trim_end_matches('/');
    FileContent {
        shown: if shown.is_empty() {
            separated.clone()
        } else {
            shown.to_owned()
        },
        folder,
        body: Body::Note(note),
    }
}

impl MentionContent for FileContent {
    fn render(&self, limit: usize) -> RenderedMention {
        let label = if self.folder && self.shown != "." {
            format!("{}/", self.shown)
        } else {
            self.shown.clone()
        };
        let (text, summary) = match &self.body {
            Body::File(file) => {
                let mut text = texts::mention_file(&self.shown);
                let page = file.render(limit.saturating_sub(text.len()));
                text.push_str(&page.text);
                (text, shows(page.shows))
            }
            Body::Folder(folder) => {
                let mut text = texts::mention_folder(&self.shown);
                let page = folder.render(limit.saturating_sub(text.len()));
                text.push_str(&page.text);
                (text, shows(page.shows))
            }
            Body::Note(note) => (
                note_text(&self.shown, self.folder, *note),
                note_summary(*note),
            ),
        };
        RenderedMention::new(text, label, variant(self.folder).to_owned(), summary)
    }
}

/// The summary of what a page shows.
fn shows(shows: PageShows) -> MentionSummary {
    match shows {
        PageShows::Lines { first, last, total } => summary(
            "lines",
            &[("first", first), ("last", last), ("total", total)],
        ),
        PageShows::Entries { shown, complete } => {
            let code = if complete { "entries" } else { "entries_cut" };
            summary(code, &[("shown", shown)])
        }
    }
}

fn note_summary(note: Note) -> MentionSummary {
    match note {
        Note::NotText => summary("not_text", &[]),
        Note::TooLarge { limit_bytes } => summary(
            "too_large",
            &[("limitMebibytes", limit_bytes / (1024 * 1024))],
        ),
        Note::Unreadable => summary("unreadable", &[]),
        Note::NeedsApproval => summary("needs_approval", &[]),
        Note::NotFound => summary("not_found", &[]),
    }
}

fn note_text(shown: &str, folder: bool, note: Note) -> String {
    match note {
        Note::NotText => texts::mention_not_text(shown),
        Note::TooLarge { limit_bytes } => {
            texts::mention_too_large(shown, limit_bytes / (1024 * 1024))
        }
        Note::Unreadable => texts::mention_unreadable(shown),
        Note::NeedsApproval => texts::mention_needs_approval(shown, folder),
        Note::NotFound => texts::mention_not_found(shown),
    }
}

fn summary(code: &str, values: &[(&str, u32)]) -> MentionSummary {
    MentionSummary {
        code: code.to_owned(),
        values: values
            .iter()
            .map(|(name, value)| ((*name).to_owned(), *value))
            .collect::<BTreeMap<String, u32>>(),
    }
}

#[cfg(test)]
mod tests;
