//! Saved text changes of successful file operations, for the transcript.

use serde::{Deserialize, Serialize};
use specta::Type;

/// A comparison of the text before and after one successful file operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    /// Added lines in the complete comparison, including omitted hunks.
    pub added: u32,
    /// Removed lines in the complete comparison, including omitted hunks.
    pub removed: u32,
    /// A bounded preview with three lines of surrounding context.
    pub hunks: Vec<FileChangeHunk>,
    /// Some lines or line text did not fit in the saved preview.
    pub truncated: bool,
}

/// One group of nearby changes and their context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeHunk {
    /// The first old line, or zero for an empty range at the file's start.
    pub old_start: u32,
    /// The old lines in the full hunk, including any omitted from the preview.
    pub old_lines: u32,
    /// The first new line, or zero for an empty range at the file's start.
    pub new_start: u32,
    /// The new lines in the full hunk, including any omitted from the preview.
    pub new_lines: u32,
    /// The saved lines, in unified diff order.
    pub lines: Vec<FileChangeLine>,
}

/// One line of a saved change. Its text is content, never markup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeLine {
    /// How the line participates in the change.
    pub kind: FileChangeLineKind,
    /// Its one-based old line number, absent for additions.
    pub old: Option<u32>,
    /// Its one-based new line number, absent for removals.
    pub new: Option<u32>,
    /// Text without its final LF. A CR in mixed line endings stays visible.
    pub text: String,
    /// The source line ended without a newline.
    pub no_newline: bool,
    /// The end of this line's text did not fit in the preview.
    pub text_truncated: bool,
}

/// The role of a line in a unified diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeLineKind {
    /// A line present only in the new text.
    Added,
    /// An unchanged line near a change.
    Context,
    /// A line present only in the old text.
    Removed,
}
