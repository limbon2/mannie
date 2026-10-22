//! What an operation touches, as its preparation found it
//! (docs/features/system-tools/operations-and-approvals.md, accesses).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::ProjectId;

use super::program::ProgramEffect;

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
        /// What a plugin's tool read of the place when it prepared, such as
        /// a repository's HEAD and the stamp of its index (ADR-0038,
        /// decision 4). The digest covers it, so a place that changed while
        /// the owner decided goes back to the policy. The core's tools give
        /// none.
        #[serde(default)]
        state: Option<String>,
    },
    /// A command, as the shell will receive it.
    Command {
        /// The command's text.
        command: String,
        /// The path of the shell that runs it.
        shell: String,
        /// The canonical working directory.
        working_directory: String,
        /// Whether the command runs in the background, past its call, until
        /// it ends or is stopped (ADR-0041). An access of a version before it
        /// reads as the foreground.
        #[serde(default)]
        background: bool,
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
    /// A command on a remote host that a plugin reaches itself, such as a
    /// host that the owner registered with the SSH plugin (ADR-0036). The
    /// plugin names the host and the command, and the core adds the plugin,
    /// so no plugin speaks for another.
    RemoteCommand {
        /// The plugin whose tool runs the command.
        plugin: String,
        /// The plugin's key for the host, which stays while the host exists.
        target: String,
        /// The host's name, as the owner gave it.
        label: String,
        /// Where the command runs, such as `deploy@build-box.lan:22`.
        endpoint: String,
        /// What identifies the host beyond its address, such as the
        /// fingerprint of its pinned key. A changed identity changes the
        /// digest, so an earlier approval no longer counts.
        identity: String,
        /// The command's text, as the host receives it.
        command: String,
        /// Whether the command runs in the background, past its call, until
        /// it ends or is stopped (ADR-0041). An access of a version before it
        /// reads as the foreground.
        #[serde(default)]
        background: bool,
    },
    /// A run of an installed program that a plugin's manifest declares, such
    /// as git, with an effect beyond the workspace that the owner decides on
    /// (ADR-0038, decision 5). The plugin names the program, its arguments,
    /// its working directory and its effects; the core adds the plugin, the
    /// program's path and the arguments that the manifest fixes, so no
    /// plugin speaks for another.
    Program {
        /// The plugin whose tool runs the program.
        plugin: String,
        /// The program's name in the plugin's manifest, such as `git`.
        name: String,
        /// The canonical path of the program that the core found.
        program: String,
        /// The arguments that the manifest fixes, which every run passes
        /// first.
        fixed: Vec<String>,
        /// The plugin's arguments, exactly as the program receives them.
        arguments: Vec<String>,
        /// The canonical working directory.
        working_directory: String,
        /// What the run does beyond the workspace, each shown to the owner.
        effects: Vec<ProgramEffect>,
        /// What the plugin read of the place when it prepared, such as a
        /// repository's HEAD. The digest covers it, so a place that changed
        /// while the owner decided goes back to the policy.
        state: Option<String>,
    },
}

impl Access {
    /// Whether the access changes something: a change of the file system, a
    /// command on this machine or a remote host, a run of an installed
    /// program, or a move into a project.
    pub fn changes(&self) -> bool {
        match self {
            Self::Path { mode, .. } => mode.changes(),
            Self::Command { .. }
            | Self::Project { .. }
            | Self::RemoteCommand { .. }
            | Self::Program { .. } => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Access, AccessMode, EntryCount, ProgramEffect};

    #[test]
    fn only_reads_change_nothing() {
        let read = Access::Path {
            path: "/work/a.txt".to_owned(),
            mode: AccessMode::Read,
            folder: false,
            exists: true,
            identity: "2049:12".to_owned(),
            entries: None,
            state: None,
        };
        let command = Access::Command {
            command: "ls".to_owned(),
            shell: "/bin/sh".to_owned(),
            working_directory: "/work".to_owned(),
            background: false,
        };

        assert!(!read.changes());
        assert!(command.changes());
        assert!(remote().changes());
        assert!(AccessMode::Delete.changes());
    }

    fn remote() -> Access {
        Access::RemoteCommand {
            plugin: "ssh".to_owned(),
            target: "0199e0f2-0000-7000-8000-0000000000aa".to_owned(),
            label: "build-box".to_owned(),
            endpoint: "deploy@build-box.lan:22".to_owned(),
            identity: "SHA256:vh3fjoyg7pBnxLhORIqZjyJQqYYN/gzBTuj+mPdeJlo".to_owned(),
            command: "uname -a".to_owned(),
            background: false,
        }
    }

    #[test]
    fn a_command_names_its_mode_and_an_older_access_reads_as_the_foreground() {
        let foreground = Access::Command {
            command: "npm run dev".to_owned(),
            shell: "/bin/sh".to_owned(),
            working_directory: "/work".to_owned(),
            background: false,
        };
        let background = Access::Command {
            command: "npm run dev".to_owned(),
            shell: "/bin/sh".to_owned(),
            working_directory: "/work".to_owned(),
            background: true,
        };

        assert_eq!(
            serde_json::to_value(&foreground).unwrap(),
            serde_json::json!({
                "kind": "command",
                "command": "npm run dev",
                "shell": "/bin/sh",
                "workingDirectory": "/work",
                "background": false,
            })
        );
        assert_eq!(
            serde_json::to_value(&background).unwrap()["background"],
            true
        );
        let older: Access = serde_json::from_value(serde_json::json!({
            "kind": "command",
            "command": "ls",
            "shell": "/bin/sh",
            "workingDirectory": "/work",
        }))
        .unwrap();
        assert!(matches!(
            older,
            Access::Command {
                background: false,
                ..
            }
        ));
    }

    #[test]
    fn a_program_run_changes_something_and_serializes_with_its_kind_and_every_field() {
        let run = Access::Program {
            plugin: "git".to_owned(),
            name: "git".to_owned(),
            program: "/usr/bin/git".to_owned(),
            fixed: vec!["-c".to_owned(), "gc.auto=0".to_owned()],
            arguments: vec!["push".to_owned(), "origin".to_owned()],
            working_directory: "/work".to_owned(),
            effects: vec![ProgramEffect::Network {
                endpoint: "https://example.com/repo.git".to_owned(),
            }],
            state: Some("HEAD 1a2b3c".to_owned()),
        };

        assert!(run.changes());
        assert_eq!(
            serde_json::to_value(&run).unwrap(),
            serde_json::json!({
                "kind": "program",
                "plugin": "git",
                "name": "git",
                "program": "/usr/bin/git",
                "fixed": ["-c", "gc.auto=0"],
                "arguments": ["push", "origin"],
                "workingDirectory": "/work",
                "effects": [{ "effect": "network", "endpoint": "https://example.com/repo.git" }],
                "state": "HEAD 1a2b3c",
            })
        );
    }

    #[test]
    fn a_remote_command_serializes_with_its_kind_and_every_field() {
        assert_eq!(
            serde_json::to_value(remote()).unwrap(),
            serde_json::json!({
                "kind": "remote_command",
                "plugin": "ssh",
                "target": "0199e0f2-0000-7000-8000-0000000000aa",
                "label": "build-box",
                "endpoint": "deploy@build-box.lan:22",
                "identity": "SHA256:vh3fjoyg7pBnxLhORIqZjyJQqYYN/gzBTuj+mPdeJlo",
                "command": "uname -a",
                "background": false,
            })
        );
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
            state: None,
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
                "state": null,
            })
        );
    }

    #[test]
    fn a_plugin_s_state_of_a_path_serializes_and_an_access_without_one_reads() {
        let write = Access::Path {
            path: "/work/repository".to_owned(),
            mode: AccessMode::Write,
            folder: true,
            exists: true,
            identity: "2049:41".to_owned(),
            entries: None,
            state: Some("HEAD 1a2b3c".to_owned()),
        };
        let older = serde_json::json!({
            "kind": "path",
            "path": "/work/repository",
            "mode": "write",
            "folder": true,
            "exists": true,
            "identity": "2049:41",
            "entries": null,
        });

        assert_eq!(
            serde_json::to_value(&write).unwrap()["state"],
            serde_json::json!("HEAD 1a2b3c")
        );
        let read: Access = serde_json::from_value(older).unwrap();
        assert!(matches!(read, Access::Path { state: None, .. }), "{read:?}");
    }
}
