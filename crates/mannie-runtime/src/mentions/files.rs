//! The core's `files` provider: the files and folders of a conversation's
//! workspace (ADR-0032, decision 9; docs/features/chat/mentions.md, files and
//! folders). It reads only what the policy allows the model to read without
//! asking, and gives the model a note for the rest.
//!
//! The list searches an index of the workspace, whose walk takes seconds in a
//! large project. Queries that arrive while it is built wait for that one
//! build, and an index older than [`INDEX_LIFETIME`] still answers at once
//! while a new one is built in the background, so only the first query in a
//! workspace waits for a walk.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};
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

/// How long the index of a workspace answers before a query builds it again
/// in the background, so that typing a query walks the folder once.
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

/// Walks a workspace into an index, or gives `None` for a folder that is not
/// there.
type Walk = fn(&Path) -> Option<MentionIndex>;

/// A walk that runs, which every query that waits for it shares.
type Building = Shared<BoxFuture<'static, Result<Arc<MentionIndex>, String>>>;

/// The core's provider of the files and folders of a workspace.
#[derive(Debug)]
pub struct FileMentions {
    /// Shared with the walks, which keep their index when they end.
    indexes: Arc<Mutex<Vec<Workspace>>>,
    walk: Walk,
    /// The number of the next walk.
    walks: AtomicU64,
}

/// The index of one workspace.
struct Workspace {
    root: PathBuf,
    /// The latest index, with the moment its walk started.
    built: Option<(Instant, Arc<MentionIndex>)>,
    /// The walk that runs, if one does, with its number, so that a walk
    /// that ends after its workspace was dropped and asked for again keeps
    /// nothing.
    building: Option<(u64, Building)>,
    /// When a query last asked for the index, which decides the workspace
    /// whose index is dropped first.
    used: Instant,
}

impl fmt::Debug for Workspace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Workspace")
            .field("root", &self.root)
            .field("built", &self.built.as_ref().map(|(at, _index)| at))
            .field("building", &self.building.is_some())
            .field("used", &self.used)
            .finish()
    }
}

impl Default for FileMentions {
    fn default() -> Self {
        Self::walking(|folder| folder.is_dir().then(|| MentionIndex::build(folder)))
    }
}

impl FileMentions {
    /// The provider, without indexes.
    pub fn new() -> Self {
        Self::default()
    }

    /// The provider, which indexes a workspace with `walk`.
    fn walking(walk: Walk) -> Self {
        Self {
            indexes: Arc::new(Mutex::new(Vec::new())),
            walk,
            walks: AtomicU64::new(0),
        }
    }

    /// The index of a workspace. A query waits only when there is none yet,
    /// and then for the walk that runs, if one does.
    async fn index(&self, root: &Path) -> Result<Arc<MentionIndex>, String> {
        let building = {
            let mut indexes = lock(&self.indexes);
            let workspace = workspace(&mut indexes, root);
            if let Some((at, index)) = &workspace.built {
                let index = Arc::clone(index);
                if at.elapsed() >= INDEX_LIFETIME && workspace.building.is_none() {
                    workspace.building = Some(self.start_walk(root));
                }
                return Ok(index);
            }
            let (_number, building) = workspace
                .building
                .get_or_insert_with(|| self.start_walk(root));
            building.clone()
        };
        building.await
    }

    /// Starts walking a workspace in a task of its own, which keeps the index
    /// when it ends, also after every query that waited for it has gone.
    fn start_walk(&self, root: &Path) -> (u64, Building) {
        let number = self.walks.fetch_add(1, Ordering::Relaxed);
        let indexes = Arc::clone(&self.indexes);
        let root = root.to_path_buf();
        let walk = self.walk;
        let task = tokio::spawn(async move {
            let started = Instant::now();
            let folder = root.clone();
            let walked = tokio::task::spawn_blocking(move || walk(&folder)).await;
            let outcome = match walked {
                Ok(Some(index)) => Ok(Arc::new(index)),
                Ok(None) | Err(_) => Err(FOLDER_UNREADABLE.to_owned()),
            };
            keep(&indexes, &root, number, started, &outcome);
            outcome
        });
        let building = async move {
            task.await
                .unwrap_or_else(|_failed| Err(FOLDER_UNREADABLE.to_owned()))
        }
        .boxed()
        .shared();
        (number, building)
    }

    /// Waits for the walk of a workspace that runs, if one does.
    #[cfg(test)]
    async fn walked(&self, root: &Path) {
        let building = lock(&self.indexes)
            .iter()
            .find(|workspace| workspace.root == root)
            .and_then(|workspace| workspace.building.clone());
        if let Some((_number, building)) = building {
            let _outcome = building.await;
        }
    }
}

fn lock(indexes: &Mutex<Vec<Workspace>>) -> MutexGuard<'_, Vec<Workspace>> {
    indexes.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The workspace of `root`, marked as used.
fn workspace<'a>(indexes: &'a mut Vec<Workspace>, root: &Path) -> &'a mut Workspace {
    let now = Instant::now();
    let found = indexes.iter().position(|workspace| workspace.root == root);
    let position = found.unwrap_or_else(|| add_workspace(indexes, root, now));
    let workspace = &mut indexes[position];
    workspace.used = now;
    workspace
}

/// Adds the workspace of `root` without an index, in place of the workspace
/// used longest ago once [`MAX_INDEXES`] are kept, and gives its position.
fn add_workspace(indexes: &mut Vec<Workspace>, root: &Path, now: Instant) -> usize {
    if indexes.len() >= MAX_INDEXES
        && let Some(oldest) = (0..indexes.len()).min_by_key(|&index| indexes[index].used)
    {
        indexes.swap_remove(oldest);
    }
    indexes.push(Workspace {
        root: root.to_path_buf(),
        built: None,
        building: None,
        used: now,
    });
    indexes.len() - 1
}

/// Keeps what the walk `number` of `root` gave. A folder that could not be
/// walked drops its index, so that the list says so instead of listing what
/// was.
fn keep(
    indexes: &Mutex<Vec<Workspace>>,
    root: &Path,
    number: u64,
    started: Instant,
    outcome: &Result<Arc<MentionIndex>, String>,
) {
    let mut indexes = lock(indexes);
    let found = indexes.iter_mut().find(|workspace| {
        let is_root = workspace.root == root;
        is_root && matches!(workspace.building, Some((running, _)) if running == number)
    });
    // A workspace whose index was dropped while the walk ran keeps nothing.
    let Some(workspace) = found else {
        return;
    };
    workspace.building = None;
    workspace.built = outcome
        .as_ref()
        .ok()
        .map(|index| (started, Arc::clone(index)));
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
