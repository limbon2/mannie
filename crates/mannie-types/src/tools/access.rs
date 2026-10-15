//! What an operation touches, as its preparation found it
//! (docs/features/system-tools/operations-and-approvals.md, accesses).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::ProjectId;

/// How an operation uses a path.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    /// Reads a file or lists a folder.
    Read,
    /// Replaces or changes an existing file.
    Write,
    /// Creates a file or folder that does not exist yet.
    Create,
    /// Deletes a file or folder.
    Delete,
}

impl AccessMode {
    /// Whether the access changes the file system.
    pub fn changes(self) -> bool {
        !matches!(self, Self::Read)
    }
}

/// The files and folders inside a folder that an operation deletes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EntryCount {
    /// The files, links included.
    pub files: u32,
    /// The folders, the folder itself not included.
    pub folders: u32,
}

/// One access of an operation. Authorization decides on accesses, never on
/// the arguments the model wrote (docs/architecture/tools.md, operations).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Access {
    /// A path, in its canonical form.
    Path {
        /// The canonical path.
        path: String,
        /// How the operation uses it.
        mode: AccessMode,
        /// Whether the path is a folder, or becomes one when the operation
        /// creates it. A grant covers a folder itself, and the folder of a
        /// file.
        folder: bool,
        /// Whether the path existed when the operation was prepared.
        exists: bool,
        /// The identity of the file or folder, or of its nearest existing
        /// folder when it does not exist yet: the volume serial number and
        /// file index on Windows, the device and inode on Unix. A file that
        /// was replaced keeps its path but changes its identity.
        identity: String,
        /// For a recursive delete, what it removes inside the folder.
        entries: Option<EntryCount>,
    },
    /// A command, as the shell will receive it.
    Command {
        /// The command's text.
        command: String,
        /// The path of the shell that runs it.
        shell: String,
        /// The canonical working directory.
        working_directory: String,
    },
    /// The conversation moves into a project, whose folder becomes its
    /// workspace (docs/features/chat/projects.md).
    Project {
        /// The project.
        id: ProjectId,
        /// Its name.
        name: String,
        /// Its folder.
        folder: String,
    },
}

impl Access {
    /// Whether the access changes something: a change of the file system, a
    /// command or a move into a project.
    pub fn changes(&self) -> bool {
        match self {
            Self::Path { mode, .. } => mode.changes(),
            Self::Command { .. } | Self::Project { .. } => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Access, AccessMode, EntryCount};

    #[test]
    fn only_reads_change_nothing() {
        let read = Access::Path {
            path: "/work/a.txt".to_owned(),
            mode: AccessMode::Read,
            folder: false,
            exists: true,
            identity: "2049:12".to_owned(),
            entries: None,
        };
        let command = Access::Command {
            command: "ls".to_owned(),
            shell: "/bin/sh".to_owned(),
            working_directory: "/work".to_owned(),
        };

        assert!(!read.changes());
        assert!(command.changes());
        assert!(AccessMode::Delete.changes());
    }

    #[test]
    fn accesses_serialize_with_their_kind() {
        let delete = Access::Path {
            path: "/work/build".to_owned(),
            mode: AccessMode::Delete,
            folder: true,
            exists: true,
            identity: "2049:40".to_owned(),
            entries: Some(EntryCount {
                files: 3,
                folders: 1,
            }),
        };

        assert_eq!(
            serde_json::to_value(&delete).unwrap(),
            serde_json::json!({
                "kind": "path",
                "path": "/work/build",
                "mode": "delete",
                "folder": true,
                "exists": true,
                "identity": "2049:40",
                "entries": { "files": 3, "folders": 1 },
            })
        );
    }
}
