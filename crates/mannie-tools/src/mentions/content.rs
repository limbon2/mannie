//! What the model reads of a mentioned file or folder (docs/features/chat/
//! mentions.md, what the model gets).
//!
//! A mention is read once, when its message is sent, and keeps at most the
//! bytes that the message's budget could give it. The runtime divides the
//! budget among a message's mentions afterwards and renders each one within
//! its share, without reading the disk again.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use crate::digest::revision;
use crate::paths::path_text;
use crate::results::{FRAME_BYTES, Page, count};
use crate::text::decode;
use crate::texts;
use crate::tools::read_file::{MAX_LIMIT, MAX_TEXT_FILE_BYTES, file_header, numbered_line};

/// The most entries of one folder that a folder's listing shows.
pub const MAX_FOLDER_ENTRIES: usize = 50;

/// The folder whose contents a listing never shows.
const GIT_FOLDER: &str = ".git";

/// The bytes that a folder's line may grow by, for the note on its entries.
const FOLDER_NOTE_BYTES: usize = 40;

/// Why a mention gives the model a note instead of its content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MentionReadFailure {
    /// The file is not text that the tools read.
    NotText,
    /// The file is larger than the tools read.
    TooLarge {
        /// The limit.
        limit_bytes: u32,
    },
    /// The file or folder could not be read.
    Unreadable {
        /// What failed, in English, for logs.
        kind: String,
    },
}

/// What a rendering of a mention shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageShows {
    /// Lines `first` to `last` of the file's `total`. `last` is below `first`
    /// when no line fits.
    Lines {
        /// The first line shown, from 1.
        first: u32,
        /// The last line shown.
        last: u32,
        /// The lines of the file.
        total: u32,
    },
    /// `shown` entries of the folder.
    Entries {
        /// The entries shown.
        shown: u32,
        /// Whether they are every entry, apart from what `.git` holds.
        complete: bool,
    },
}

/// A mention rendered within a limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionPage {
    /// What the model reads.
    pub text: String,
    /// What the text shows.
    pub shows: PageShows,
}

/// A mentioned text file: its header and its first numbered lines.
#[derive(Debug, Clone)]
pub struct FileMention {
    header: String,
    lines: Vec<String>,
    total: usize,
}

impl FileMention {
    /// Reads a text file whose page may take at most `max_bytes`. A file that
    /// is not text, or larger than the text tools read, gives a failure.
    pub fn read(path: &Path, max_bytes: usize) -> Result<Self, MentionReadFailure> {
        let text_path = path_text(path).map_err(|failure| MentionReadFailure::Unreadable {
            kind: failure.to_string(),
        })?;
        let metadata = fs::metadata(path).map_err(|error| unreadable(&error))?;
        if metadata.len() > MAX_TEXT_FILE_BYTES {
            return Err(too_large());
        }
        let bytes = fs::read(path).map_err(|error| unreadable(&error))?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_TEXT_FILE_BYTES {
            return Err(too_large());
        }
        let revision = revision(&bytes);
        let text = decode(&bytes).map_err(|_not_text| MentionReadFailure::NotText)?;
        let all = crate::text::lines(&text.text);
        let max_lines = usize::try_from(MAX_LIMIT).unwrap_or(usize::MAX);
        let mut lines = Vec::new();
        let mut kept_bytes = 0_usize;
        for (index, line) in all.iter().enumerate().take(max_lines) {
            let numbered = numbered_line(index, line);
            kept_bytes += numbered.len() + 1;
            if kept_bytes > max_bytes {
                break;
            }
            lines.push(numbered);
        }
        Ok(Self {
            header: file_header(&text_path, &revision, &text),
            lines,
            total: all.len(),
        })
    }

    /// The page as `read_file` shows it from line 1, within `limit` bytes,
    /// with a note on how to read on when lines are left out.
    pub fn render(&self, limit: usize) -> MentionPage {
        let mut page = Page::new(limit, &[&self.header]);
        let mut last = 0_usize;
        for line in &self.lines {
            if !page.push(line) {
                break;
            }
            last += 1;
        }
        let mut text = self.header.clone();
        if self.total == 0 {
            text.push_str(texts::EMPTY_FILE);
            text.push('\n');
        } else {
            text.push_str(&texts::lines_shown(1, last, self.total));
            text.push('\n');
            text.push_str(&page.into_body());
            if last < self.total {
                text.push_str(&texts::mention_read_on(last + 1));
                text.push('\n');
            }
        }
        MentionPage {
            text,
            shows: PageShows::Lines {
                first: 1,
                last: count(last),
                total: count(self.total),
            },
        }
    }
}

/// What an entry of a listing is.
#[derive(Debug, Clone)]
enum ListedKind {
    File { size: u64 },
    Folder,
    Link,
    Other,
}

/// An entry of a listing.
#[derive(Debug, Clone)]
struct Listed {
    /// The path below the workspace, with `/` between names.
    path: String,
    kind: ListedKind,
    /// The listed folder that holds it, by its index, or `None` for an entry
    /// of the mentioned folder itself.
    parent: Option<usize>,
}

/// An entry of a folder as the file system lists it.
struct FolderEntry {
    name: String,
    path: PathBuf,
    kind: ListedKind,
}

/// A mentioned folder: its entries, breadth first, at most
/// [`MAX_FOLDER_ENTRIES`] of each folder.
#[derive(Debug, Clone)]
pub struct FolderMention {
    entries: Vec<Listed>,
    /// How many entries each listed folder that was read holds, by its index.
    held: HashMap<usize, usize>,
    /// How many entries the mentioned folder holds.
    root_held: usize,
    /// Whether the listing stopped before it read every folder.
    cut: bool,
}

impl FolderMention {
    /// Lists a folder breadth first, keeping at most `max_bytes` of lines.
    /// `shown` is the folder's path below the workspace, which every entry's
    /// path starts with. Links are listed and not followed, and `.git` is
    /// listed without its entries.
    pub fn list(path: &Path, shown: &str, max_bytes: usize) -> Result<Self, MentionReadFailure> {
        let root = read_folder(path).map_err(|error| unreadable(&error))?;
        let mut listing = Self {
            entries: Vec::new(),
            held: HashMap::new(),
            root_held: root.len(),
            cut: false,
        };
        let mut queue: VecDeque<(usize, PathBuf)> = VecDeque::new();
        let mut bytes = 0_usize;
        let base = if shown == "." { "" } else { shown };
        let mut full = !listing.add(None, base, root, &mut queue, &mut bytes, max_bytes);
        while !full && let Some((index, folder)) = queue.pop_front() {
            // A folder that cannot be read shows as one whose entries are not
            // shown.
            let Ok(children) = read_folder(&folder) else {
                continue;
            };
            listing.held.insert(index, children.len());
            let parent_path = listing.entries[index].path.clone();
            full = !listing.add(
                Some(index),
                &parent_path,
                children,
                &mut queue,
                &mut bytes,
                max_bytes,
            );
        }
        listing.cut = full || !queue.is_empty();
        Ok(listing)
    }

    /// Adds the first entries of a folder, while they fit in `max_bytes`, and
    /// queues their folders. Returns `false` once the bytes are used up.
    fn add(
        &mut self,
        parent: Option<usize>,
        parent_path: &str,
        children: Vec<FolderEntry>,
        queue: &mut VecDeque<(usize, PathBuf)>,
        bytes: &mut usize,
        max_bytes: usize,
    ) -> bool {
        for child in children.into_iter().take(MAX_FOLDER_ENTRIES) {
            let path = if parent_path.is_empty() {
                child.name.clone()
            } else {
                format!("{parent_path}/{}", child.name)
            };
            let line_bytes = path.len() + FOLDER_NOTE_BYTES;
            if *bytes + line_bytes > max_bytes {
                return false;
            }
            *bytes += line_bytes;
            let expands = matches!(child.kind, ListedKind::Folder) && child.name != GIT_FOLDER;
            self.entries.push(Listed {
                path,
                kind: child.kind,
                parent,
            });
            if expands {
                queue.push_back((self.entries.len() - 1, child.path));
            }
        }
        true
    }

    /// The listing within `limit` bytes: the first entries in breadth-first
    /// order that fit, sorted by their paths, with a note on each folder whose
    /// entries are not all shown.
    pub fn render(&self, limit: usize) -> MentionPage {
        let mut page = Page::new(limit, &[]);
        let budget = limit.saturating_sub(FRAME_BYTES);
        let mut used = 0_usize;
        let mut taken = 0_usize;
        for entry in &self.entries {
            let line_bytes = entry.path.len() + FOLDER_NOTE_BYTES + 1;
            if used + line_bytes > budget {
                break;
            }
            used += line_bytes;
            taken += 1;
        }
        let included = &self.entries[..taken];
        let mut shown_in: HashMap<Option<usize>, usize> = HashMap::new();
        for entry in included {
            *shown_in.entry(entry.parent).or_default() += 1;
        }
        let mut order: Vec<usize> = (0..taken).collect();
        order.sort_by_cached_key(|index| sort_key(&self.entries[*index].path));
        let mut complete = taken == self.entries.len()
            && !self.cut
            && self.root_held == shown_in.get(&None).copied().unwrap_or(0);
        let mut shown = 0_usize;
        for index in order {
            let entry = &self.entries[index];
            let line = match &entry.kind {
                ListedKind::File { size } => format!("{} ({size} bytes)", entry.path),
                ListedKind::Link => format!("{} (link)", entry.path),
                ListedKind::Other => entry.path.clone(),
                ListedKind::Folder => {
                    let is_git = entry.path.rsplit('/').next() == Some(GIT_FOLDER);
                    let held = self.held.get(&index).copied();
                    let listed = shown_in.get(&Some(index)).copied().unwrap_or(0);
                    let note = folder_note(held, listed);
                    if note.is_some() && !is_git {
                        complete = false;
                    }
                    format!("{}/{}", entry.path, note.unwrap_or_default())
                }
            };
            if !page.push(&line) {
                complete = false;
                break;
            }
            shown += 1;
        }
        let mut text = page.into_body();
        if self.entries.is_empty() {
            text.push_str(texts::EMPTY_FOLDER);
            text.push('\n');
        } else if !complete {
            text.push_str(texts::LISTING_CUT);
            text.push('\n');
        }
        MentionPage {
            text,
            shows: PageShows::Entries {
                shown: count(shown),
                complete,
            },
        }
    }
}

/// The note on a folder's line: none when all its entries are shown.
fn folder_note(held: Option<usize>, listed: usize) -> Option<String> {
    match held {
        Some(held) if listed == held => None,
        Some(held) if listed > 0 => Some(format!(" ({listed} of {held} entries shown)")),
        _ => Some(" (entries not shown)".to_owned()),
    }
}

/// The order of a listing's lines: by name at each level, without regard to
/// case, the way people read listings.
fn sort_key(path: &str) -> Vec<String> {
    path.split('/').map(str::to_lowercase).collect()
}

/// The entries of a folder, sorted by name without regard to case. A name
/// that is not valid Unicode is left out, since the model reads paths as
/// text.
fn read_folder(folder: &Path) -> std::io::Result<Vec<FolderEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(folder)? {
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        // Entries that are neither links nor folders get their exact kind
        // from their metadata, which does not follow links either.
        let kind = match entry.file_type() {
            Ok(kind) if kind.is_symlink() => ListedKind::Link,
            Ok(kind) if kind.is_dir() => ListedKind::Folder,
            _ => match entry.metadata() {
                Ok(metadata) if metadata.is_file() => ListedKind::File {
                    size: metadata.len(),
                },
                _ => ListedKind::Other,
            },
        };
        entries.push(FolderEntry {
            name,
            path: entry.path(),
            kind,
        });
    }
    entries.sort_by_cached_key(|entry| (entry.name.to_lowercase(), entry.name.clone()));
    Ok(entries)
}

fn too_large() -> MentionReadFailure {
    MentionReadFailure::TooLarge {
        limit_bytes: u32::try_from(MAX_TEXT_FILE_BYTES).unwrap_or(u32::MAX),
    }
}

fn unreadable(error: &std::io::Error) -> MentionReadFailure {
    MentionReadFailure::Unreadable {
        kind: error.kind().to_string(),
    }
}
