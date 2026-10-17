//! The file system side of mentions (ADR-0032; docs/features/chat/
//! mentions.md): the index of a workspace that the "@" list searches, the
//! names of files that may hold secrets, the check of a mentioned path, and
//! what the model reads of a mentioned file or folder. The runtime's `files`
//! provider decides with the policy whether a mention may be read
//! ([`crate::decide`]), and words what the model reads around it.

mod content;
mod index;
mod secrets;

use std::path::{Component, Path, PathBuf};

pub use content::{
    FileMention, FolderMention, MAX_FOLDER_ENTRIES, MentionPage, MentionReadFailure, PageShows,
};
pub use index::{IndexMatch, IndexMatches, MAX_INDEX_ENTRIES, MentionIndex};
use mannie_types::tools::{Access, AccessMode};
pub use secrets::may_hold_secrets;

use crate::paths::{FinalLink, check_text, is_inside, locate, resolve};
use crate::tools::path_access;

/// A mentioned path that exists below the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionTarget {
    /// The path below the workspace as the owner reads it, with `/` between
    /// names, or `.` for the workspace itself.
    pub shown: String,
    /// The path in canonical form, with its links resolved.
    pub path: PathBuf,
    /// Whether it is a folder.
    pub folder: bool,
    /// The access that reading it is, for the policy.
    pub access: Access,
}

/// Why a key names no path that a mention reads.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MentionPathError {
    /// The key is no path below the workspace: it is absolute, it leads out of
    /// the workspace, or a check of paths refuses it.
    #[error("the path is not below the workspace")]
    OutsideWorkspace,
    /// Nothing exists at the path.
    #[error("nothing exists at the path")]
    NotFound,
    /// The path could not be resolved.
    #[error("the path could not be resolved: {kind}")]
    Unreadable {
        /// What failed, in English, for logs.
        kind: String,
    },
}

/// Checks a mentioned path, which names a file or folder relative to the
/// workspace, and resolves its links as a read does. The text is refused
/// before anything touches the file system when it is absolute or leads out
/// of the workspace, so a forged key reaches nothing (ADR-0004). A link may
/// still lead out of the workspace, which the policy then decides on.
pub fn resolve_mention(workspace: &Path, key: &str) -> Result<MentionTarget, MentionPathError> {
    check_text(key).map_err(|_refused| MentionPathError::OutsideWorkspace)?;
    if Path::new(key).is_absolute() {
        return Err(MentionPathError::OutsideWorkspace);
    }
    let located =
        locate(Some(workspace), key).map_err(|_refused| MentionPathError::OutsideWorkspace)?;
    if !is_inside(&located, workspace) {
        return Err(MentionPathError::OutsideWorkspace);
    }
    let shown = shown_path(workspace, &located).ok_or(MentionPathError::OutsideWorkspace)?;
    let resolved =
        resolve(&located, FinalLink::Follow).map_err(|failure| MentionPathError::Unreadable {
            kind: failure.to_string(),
        })?;
    if !resolved.exists() {
        return Err(MentionPathError::NotFound);
    }
    let folder = resolved.is_folder();
    let access = path_access(&resolved, AccessMode::Read, folder).map_err(|failure| {
        MentionPathError::Unreadable {
            kind: failure.to_string(),
        }
    })?;
    Ok(MentionTarget {
        shown,
        path: resolved.path,
        folder,
        access,
    })
}

/// A located path below the workspace as text with `/` between names.
fn shown_path(workspace: &Path, located: &Path) -> Option<String> {
    let mut names = Vec::new();
    for component in located.components().skip(workspace.components().count()) {
        match component {
            Component::Normal(name) => names.push(name.to_str()?),
            _ => return None,
        }
    }
    if names.is_empty() {
        Some(".".to_owned())
    } else {
        Some(names.join("/"))
    }
}

#[cfg(test)]
mod tests;
