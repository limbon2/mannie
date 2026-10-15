//! What a tool call produced: a typed outcome for the UI, the rendering that
//! the model reads, and the archived outputs
//! (docs/features/system-tools/tools-and-results.md).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::ArchiveId;

/// The kind of a file system entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// A file.
    File,
    /// A folder.
    Folder,
    /// A symbolic link or a junction.
    Link,
    /// Something else, such as a device or a socket.
    Other,
}

/// How a command ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CommandEnding {
    /// The command ended on its own, with an exit code.
    Exited,
    /// The command ran past its timeout, and Mannie ended it.
    TimedOut,
    /// The owner stopped the reply, and Mannie ended the command.
    Stopped,
    /// Mannie quit while the command ran, and ended it.
    Interrupted,
}

/// Why Mannie refused a path before it touched the file system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PathRefusal {
    /// The path is empty or contains a control character.
    Empty,
    /// A UNC path, or another path that starts with two separators.
    NetworkPath,
    /// A device or verbatim path, such as `\\.\` or `\\?\`.
    DevicePath,
    /// An alternate data stream, such as `file.txt:stream`.
    AlternateStream,
    /// A reserved device name, such as `CON` or `COM1`.
    ReservedName,
    /// A component that ends in a dot or a space.
    TrailingDotOrSpace,
    /// A path relative to the current folder of a drive, such as `C:file`.
    DriveRelative,
    /// The path is longer than Mannie accepts.
    TooLong,
    /// The path is not absolute where it must be, such as a workspace.
    NotAbsolute,
}

/// Why a tool call failed. The model reads the rendering of the failure, and
/// the UI shows the message for the code (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ToolFailure {
    /// A relative path or a command without a working directory, in a
    /// conversation without a workspace.
    #[error("the conversation has no workspace")]
    NoWorkspace,
    /// The path was refused before any access to the file system.
    #[error("the path is not allowed: {reason:?}")]
    PathRefused {
        /// Why.
        reason: PathRefusal,
    },
    /// A change through a path that contains a link.
    #[error("the path goes through a link; the resolved path is {resolved:?}")]
    PathThroughLink {
        /// The path with its links resolved.
        resolved: String,
    },
    /// Nothing exists at the path.
    #[error("{path:?} does not exist")]
    NotFound {
        /// The path.
        path: String,
    },
    /// The path is not a file.
    #[error("{path:?} is not a file")]
    NotAFile {
        /// The path.
        path: String,
    },
    /// The path is not a folder.
    #[error("{path:?} is not a folder")]
    NotAFolder {
        /// The path.
        path: String,
    },
    /// Something already exists at the path.
    #[error("{path:?} already exists")]
    AlreadyExists {
        /// The path.
        path: String,
    },
    /// The operating system refused access.
    #[error("access to {path:?} was refused")]
    PermissionDenied {
        /// The path.
        path: String,
    },
    /// Another program holds the file.
    #[error("{path:?} is in use by another program")]
    InUse {
        /// The path.
        path: String,
    },
    /// The file changed since the revision the call named.
    #[error("the file changed; its revision is now {current}")]
    RevisionMismatch {
        /// The file's revision now.
        current: String,
    },
    /// An edit's old text does not appear in the file.
    #[error("the old text of edit {edit} does not appear in the file")]
    EditNotFound {
        /// The edit, counting from 1.
        edit: u32,
    },
    /// An edit's old text appears more than once.
    #[error("the old text of edit {edit} appears {matches} times")]
    EditAmbiguous {
        /// The edit, counting from 1.
        edit: u32,
        /// How often it appears.
        matches: u32,
    },
    /// Two edits change overlapping text.
    #[error("edits {first} and {second} overlap")]
    EditsOverlap {
        /// The first edit, counting from 1.
        first: u32,
        /// The second edit.
        second: u32,
    },
    /// The file or the content is larger than Mannie handles.
    #[error("larger than {limit_bytes} bytes")]
    TooLarge {
        /// The limit.
        limit_bytes: u32,
    },
    /// The file is binary.
    #[error("{path:?} is a binary file")]
    BinaryFile {
        /// The path.
        path: String,
    },
    /// The file's text is in an encoding that Mannie does not handle.
    #[error("{path:?} is not UTF-8 or UTF-16 with a byte order mark")]
    UnsupportedEncoding {
        /// The path.
        path: String,
    },
    /// A folder to copy contains a link.
    #[error("{path:?} is a link, which Mannie does not copy")]
    LinkInSource {
        /// The link.
        path: String,
    },
    /// A move between two volumes.
    #[error("the source and the destination are on different volumes")]
    DifferentVolume,
    /// A folder that is not empty, deleted without `recursive`.
    #[error("{path:?} is not empty")]
    FolderNotEmpty {
        /// The folder.
        path: String,
    },
    /// More entries than Mannie copies or deletes at once.
    #[error("more than {limit} entries")]
    TooManyEntries {
        /// The limit.
        limit: u32,
    },
    /// A glob pattern or regular expression that does not parse.
    #[error("the pattern is not valid: {reason}")]
    InvalidPattern {
        /// What the parser said.
        reason: String,
    },
    /// The archived output does not exist in this conversation.
    #[error("the archived output does not exist")]
    OutputNotFound,
    /// `move_to_project` named no project, or a name that several projects
    /// share.
    #[error("there is no project {project:?}")]
    ProjectNotFound {
        /// The project as the call named it, cut to 200 characters.
        project: String,
    },
    /// The conversation already belongs to a project, which it never leaves.
    #[error("the conversation already belongs to the project {name:?}")]
    AlreadyInProject {
        /// The project's name.
        name: String,
    },
    /// What a path names changed between the authorization and the start of
    /// the change, such as a file replaced by another program, so nothing was
    /// changed.
    #[error("the path changed after the operation was authorized")]
    PathChanged,
    /// The shell that runs commands is missing.
    #[error("the shell {path:?} is missing")]
    ShellMissing {
        /// Where Mannie looked for it.
        path: String,
    },
    /// The command could not be started.
    #[error("the command could not be started: {reason}")]
    SpawnFailed {
        /// What the operating system said.
        reason: String,
    },
    /// Another error of the operating system.
    #[error("the operating system reported an error: {kind}")]
    IoError {
        /// What the operating system reported, in its words.
        kind: String,
    },
}

impl ToolFailure {
    /// The stable code of the failure, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoWorkspace => "tool.no_workspace",
            Self::PathRefused { .. } => "tool.path_refused",
            Self::PathThroughLink { .. } => "tool.path_through_link",
            Self::NotFound { .. } => "tool.not_found",
            Self::NotAFile { .. } => "tool.not_a_file",
            Self::NotAFolder { .. } => "tool.not_a_folder",
            Self::AlreadyExists { .. } => "tool.already_exists",
            Self::PermissionDenied { .. } => "tool.permission_denied",
            Self::InUse { .. } => "tool.in_use",
            Self::RevisionMismatch { .. } => "tool.revision_mismatch",
            Self::EditNotFound { .. } => "tool.edit_not_found",
            Self::EditAmbiguous { .. } => "tool.edit_ambiguous",
            Self::EditsOverlap { .. } => "tool.edits_overlap",
            Self::TooLarge { .. } => "tool.too_large",
            Self::BinaryFile { .. } => "tool.binary_file",
            Self::UnsupportedEncoding { .. } => "tool.unsupported_encoding",
            Self::LinkInSource { .. } => "tool.link_in_source",
            Self::DifferentVolume => "tool.different_volume",
            Self::FolderNotEmpty { .. } => "tool.folder_not_empty",
            Self::TooManyEntries { .. } => "tool.too_many_entries",
            Self::InvalidPattern { .. } => "tool.invalid_pattern",
            Self::OutputNotFound => "tool.output_not_found",
            Self::ProjectNotFound { .. } => "tool.project_not_found",
            Self::AlreadyInProject { .. } => "tool.already_in_project",
            Self::PathChanged => "tool.path_changed",
            Self::ShellMissing { .. } => "tool.shell_missing",
            Self::SpawnFailed { .. } => "tool.spawn_failed",
            Self::IoError { .. } => "tool.io_error",
        }
    }
}

/// What a finished tool call did, for the UI's summary of the call. Every
/// tool has its variant; a call that failed has [`ToolOutcome::Failed`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ToolOutcome {
    /// `system_info` described the system.
    SystemInfo {
        /// The operating system and its version.
        system: String,
        /// The shell that runs commands.
        shell: String,
    },
    /// `list_directory` listed a folder.
    Listed {
        /// The folder.
        path: String,
        /// The entries shown.
        shown: u32,
        /// The entries in the folder.
        total: u32,
    },
    /// `file_info` described a path.
    Described {
        /// The path.
        path: String,
        /// What it is.
        entry: EntryKind,
    },
    /// `find_files` found files.
    Found {
        /// The folder searched.
        path: String,
        /// The files shown.
        shown: u32,
        /// Whether more matches follow.
        more: bool,
    },
    /// `search_text` searched files.
    Searched {
        /// The file or folder searched.
        path: String,
        /// The matching lines shown.
        shown: u32,
        /// Whether more matches follow.
        more: bool,
    },
    /// `read_file` read lines of a file.
    Read {
        /// The file.
        path: String,
        /// The first line shown, from 1.
        first_line: u32,
        /// The last line shown.
        last_line: u32,
        /// The lines in the file.
        total_lines: u32,
    },
    /// `write_file` created or replaced a file.
    Written {
        /// The file.
        path: String,
        /// Whether it created the file.
        created: bool,
        /// The bytes written.
        bytes: u32,
    },
    /// `edit_file` changed a file.
    Edited {
        /// The file.
        path: String,
        /// The replacements made.
        replacements: u32,
    },
    /// `create_directory` created folders.
    FolderCreated {
        /// The folder.
        path: String,
        /// The folders created, including missing parents.
        created: u32,
    },
    /// `copy_path` copied a file or folder.
    Copied {
        /// The source.
        source: String,
        /// The destination.
        destination: String,
        /// The files copied.
        files: u32,
    },
    /// `move_path` moved a file or folder.
    Moved {
        /// The source.
        source: String,
        /// The destination.
        destination: String,
    },
    /// `delete_path` deleted a file or folder.
    Deleted {
        /// The path.
        path: String,
        /// The files deleted.
        files: u32,
        /// The folders deleted, the path itself included when it was one.
        folders: u32,
    },
    /// `run_command` ran a command.
    CommandRan {
        /// The command's text.
        command: String,
        /// How it ended.
        ending: CommandEnding,
        /// Its exit code, when it ended on its own.
        exit_code: Option<i32>,
        /// How long it ran, in milliseconds.
        duration_ms: u32,
    },
    /// `read_tool_output` read lines of an archived output.
    OutputRead {
        /// The first line shown, from 1.
        first_line: u32,
        /// The last line shown.
        last_line: u32,
        /// The lines of the output.
        total_lines: u32,
    },
    /// `move_to_project` moved the conversation into a project.
    MovedToProject {
        /// The project's name.
        name: String,
        /// Its folder, the conversation's workspace from now on.
        folder: String,
    },
    /// The tool failed.
    Failed {
        /// Why.
        failure: ToolFailure,
    },
    /// An outcome that this version cannot read, such as one of a tool that a
    /// later version added. A stored result reads it as this
    /// ([`ToolCallDocument`](super::ToolCallDocument)).
    Unknown,
}

/// An output that was archived outside the transcript, which the model and
/// the owner read back in pages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveRef {
    /// The archive.
    pub id: ArchiveId,
    /// What it holds, such as `stdout` or `stderr`.
    pub name: String,
    /// Its size in bytes.
    pub bytes: u32,
    /// Its lines.
    pub lines: u32,
}

/// The result of a finished tool call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    /// What the call did, for the UI.
    pub outcome: ToolOutcome,
    /// The result as the model reads it, within the result limit.
    pub output: String,
    /// The outputs archived because they did not fit.
    #[serde(default)]
    pub archives: Vec<ArchiveRef>,
}

#[cfg(test)]
mod tests {
    use super::{PathRefusal, ToolFailure, ToolOutcome};

    #[test]
    fn failures_serialize_with_their_code_and_arguments() {
        let failure = ToolFailure::EditAmbiguous {
            edit: 2,
            matches: 3,
        };

        assert_eq!(
            serde_json::to_value(&failure).unwrap(),
            serde_json::json!({ "code": "edit_ambiguous", "edit": 2, "matches": 3 })
        );
        assert_eq!(failure.code(), "tool.edit_ambiguous");
        assert_eq!(
            serde_json::to_value(ToolFailure::PathRefused {
                reason: PathRefusal::NetworkPath
            })
            .unwrap(),
            serde_json::json!({ "code": "path_refused", "reason": "network_path" })
        );
    }

    #[test]
    fn outcomes_serialize_with_their_kind() {
        assert_eq!(
            serde_json::to_value(ToolOutcome::Unknown).unwrap(),
            serde_json::json!({ "kind": "unknown" })
        );
        assert_eq!(
            serde_json::to_value(ToolOutcome::Edited {
                path: "notes.md".to_owned(),
                replacements: 2,
            })
            .unwrap(),
            serde_json::json!({ "kind": "edited", "path": "notes.md", "replacements": 2 })
        );
    }
}
