//! Owner-initiated navigation to local paths in replies (ADR-0069).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::ConversationId;

/// A reference from the conversation that contains the reply.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileLinkRef {
    /// The containing conversation, absent outside a transcript.
    pub conversation_id: Option<ConversationId>,
    /// A native path or local file URL, possibly with a source location.
    pub path: String,
}

/// The only supported destination. Callers cannot supply a program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FileLinkTarget {
    /// The desktop's file manager.
    FileManager,
    /// An unrecognized target is refused without inspecting or opening a path.
    Unsupported,
}

// Keep unrecognized strings in the typed error path rather than rejecting
// the whole IPC payload before it can return a catalog-backed error.
impl<'de> Deserialize<'de> for FileLinkTarget {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let target = String::deserialize(deserializer)?;
        Ok(if target == "file_manager" {
            Self::FileManager
        } else {
            Self::Unsupported
        })
    }
}

/// The owner's request to open a checked reference, which is checked again.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenFileLink {
    /// The original reference, including its conversation context.
    pub link: FileLinkRef,
    /// The fixed opening destination.
    pub target: FileLinkTarget,
}

/// What a path identifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FileLinkKind {
    /// Reveal this file in its parent directory.
    File,
    /// Open this directory itself.
    Directory,
}

/// A catalog key for the available application, never display text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FileLinkApplication {
    /// Windows File Explorer.
    FileExplorer,
    /// The default Linux file manager.
    FileManager,
}

/// A normalized local path and the opening action available for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedFileLink {
    /// The absolute path with filesystem links resolved.
    pub path: String,
    /// Whether to reveal a file or open a directory.
    pub kind: FileLinkKind,
    /// The application label the UI obtains from its catalog.
    pub application: FileLinkApplication,
    /// The supported opening destination.
    pub target: FileLinkTarget,
}

/// Why local navigation failed. No operating-system error text crosses IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum FileLinkError {
    /// Empty, malformed, oversized or unsafe input.
    #[error("the path is invalid")]
    InvalidPath,
    /// A remote or foreign-platform path.
    #[error("the path is not local to this desktop")]
    NotLocal,
    /// A relative path has no project to resolve against.
    #[error("the reference has no project")]
    NoProject,
    /// The containing conversation is no longer present.
    #[error("the conversation no longer exists")]
    ConversationMissing,
    /// No file or directory exists at the path.
    #[error("the path does not exist")]
    NotFound,
    /// Devices, sockets and other special files cannot be opened.
    #[error("the path is not a regular file or directory")]
    UnsupportedKind,
    /// The store or filesystem could not resolve the reference.
    #[error("the path could not be checked")]
    Unavailable,
    /// Only the built-in file-manager destination can open a path.
    #[error("the opening target is unsupported")]
    UnsupportedTarget,
    /// The desktop could not start its file manager.
    #[error("the file manager could not be opened")]
    OpenFailed,
}

impl FileLinkError {
    /// Stable codes for structured tracing.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidPath => "file_link.invalid_path",
            Self::NotLocal => "file_link.not_local",
            Self::NoProject => "file_link.no_project",
            Self::ConversationMissing => "file_link.conversation_missing",
            Self::NotFound => "file_link.not_found",
            Self::UnsupportedKind => "file_link.unsupported_kind",
            Self::Unavailable => "file_link.unavailable",
            Self::UnsupportedTarget => "file_link.unsupported_target",
            Self::OpenFailed => "file_link.open_failed",
        }
    }
}
