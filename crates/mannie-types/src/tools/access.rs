//! What an operation touches, as its preparation found it
//! (docs/features/system-tools/operations-and-approvals.md, accesses).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::{DatabaseKey, ProjectId};

use super::database::{DatabaseHome, StatementClass};
use super::external::{ToolHint, ToolServerEndpoint};
use super::program::ProgramEffect;
use super::terminal::TerminalHost;

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

/// What a browser tool does to the session (ADR-0064).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAction {
    /// Reads a loaded page.
    Observe,
    /// Manages tabs.
    Tabs,
    /// Starts navigation or changes the current history entry.
    Navigate,
    /// Resumes an intercepted navigation to another origin.
    ContinueNavigation,
    /// Sends input to a page.
    Interact,
    /// Enters an owner-saved site credential into a page.
    FillCredential,
}

/// Non-secret identity shown when a saved credential needs approval.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BrowserCredentialAccess {
    /// Immutable opaque reference of the saved credential.
    pub reference: String,
    /// The owner's label for the credential.
    pub label: String,
}

/// An origin and session revision that the browser prepared for an action.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAccess {
    /// The canonical HTTP or HTTPS origin, without a path or credentials.
    pub origin: String,
    /// The browser service's session generation.
    pub session: String,
    /// The service's identity for the prepared action and its live target.
    pub revision: String,
    /// The kind of action the tool prepared.
    pub action: BrowserAction,
    /// The credential identified by a protected fill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<BrowserCredentialAccess>,
}

impl BrowserAccess {
    /// Builds a prepared browser access. The core checks its shape and the
    /// browser service checks that the revision still holds before dispatch.
    pub fn new(origin: String, session: String, revision: String, action: BrowserAction) -> Self {
        Self {
            origin,
            session,
            revision,
            action,
            credential: None,
        }
    }

    /// Adds the saved credential that this access is requesting permission to fill.
    #[must_use]
    pub fn with_credential(mut self, reference: String, label: String) -> Self {
        self.action = BrowserAction::FillCredential;
        self.credential = Some(BrowserCredentialAccess { reference, label });
        self
    }

    /// An access names an exact canonical origin and opaque, bounded IDs.
    pub fn is_canonical(&self) -> bool {
        self.origin.len() <= 512
            && url::Url::parse(&self.origin).is_ok_and(|url| {
                matches!(url.scheme(), "http" | "https")
                    && url.origin().ascii_serialization() == self.origin
            })
            && is_browser_id(&self.session)
            && is_browser_id(&self.revision)
            && match (&self.credential, self.action) {
                (Some(credential), BrowserAction::FillCredential) => {
                    is_browser_id(&credential.reference)
                        && !credential.label.trim().is_empty()
                        && credential.label.len() <= 80
                        && !credential.label.chars().any(char::is_control)
                }
                (None, BrowserAction::FillCredential) | (Some(_), _) => false,
                (None, _) => true,
            }
    }
}

/// An owner-to-agent handoff prepared for a particular live browser state.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserControlAccess {
    /// The browser service's session generation.
    pub session: String,
    /// The browser service's control generation.
    pub revision: String,
}

impl BrowserControlAccess {
    /// Records the session and control generation that the owner is using.
    pub fn new(session: String, revision: String) -> Self {
        Self { session, revision }
    }

    /// Whether the handoff identifies a bounded session and control revision.
    pub fn is_canonical(&self) -> bool {
        is_browser_id(&self.session) && is_browser_id(&self.revision)
    }
}

fn is_browser_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
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
    /// A call of a tool that a tool server offers, such as an MCP server
    /// that the owner registered with a plugin (ADR-0043, decision 4). The
    /// plugin names the server and the call, and the core adds the plugin,
    /// so no plugin speaks for another.
    ExternalTool {
        /// The plugin that reaches the server.
        plugin: String,
        /// The plugin's key for the server, which stays while the server
        /// exists.
        target: String,
        /// The server's name, as the owner gave it.
        label: String,
        /// Where the server runs.
        endpoint: ToolServerEndpoint,
        /// The tool, as the server names it, or `read_resource` for a read
        /// of one of the server's resources.
        tool: String,
        /// The tool's title, as the server gives it, for display only.
        title: Option<String>,
        /// The call's arguments as JSON with sorted keys, or the URI of the
        /// resource that a read reads.
        arguments: String,
        /// Whether a call's view cut the arguments to its limit. An access
        /// that the policy decides on is never cut.
        #[serde(default)]
        arguments_cut: bool,
        /// What the server says of the tool, sorted, for display only.
        hints: Vec<ToolHint>,
    },
    /// A command that a plugin's tool types into a terminal, as the owner
    /// would (ADR-0053, decision 5). The plugin names the terminal's host by
    /// its reference; the core takes the host from its registry and adds the
    /// plugin, so no plugin speaks for another or names another's endpoint.
    TerminalCommand {
        /// The plugin whose tool types the command.
        plugin: String,
        /// Where the terminal runs.
        host: TerminalHost,
        /// What identifies a remote host beyond its address, such as the
        /// fingerprint of its pinned key, and empty on this machine. A
        /// changed identity changes the digest, so an earlier approval no
        /// longer counts.
        identity: String,
        /// The terminal's number in its conversation, or `None` for a
        /// terminal that the call opens. An approval covers only this
        /// terminal, and a grant or a rule any terminal of the host.
        terminal: Option<u32>,
        /// The shell's name, such as `bash` or `Windows PowerShell`, for
        /// display only.
        shell: String,
        /// The user and the folder that the terminal reported last, such as
        /// `root in /etc`, which an approval covers; `None` before the
        /// terminal reported them.
        state: Option<String>,
        /// The command's text, as the plugin types it.
        command: String,
    },
    /// A prepared action on an isolated browser session (ADR-0064).
    BrowserAction {
        /// The plugin whose tool prepared the action.
        plugin: String,
        /// The canonical HTTP or HTTPS origin, without a path or credentials.
        origin: String,
        /// The browser service's session generation.
        session: String,
        /// The service's identity for the prepared action and its live target.
        revision: String,
        /// The kind of action the tool prepared.
        action: BrowserAction,
        /// The credential identified by a protected fill.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        credential: Option<BrowserCredentialAccess>,
    },
    /// A request to return an isolated browser session to the agent.
    BrowserControl {
        /// The plugin that owns the browser session.
        plugin: String,
        /// The session generation the owner is using.
        session: String,
        /// The control generation the owner is using.
        revision: String,
    },
    /// A statement on a SQLite file that the owner registered for a plugin
    /// (ADR-0056, decision 5). The plugin names the database by its key and
    /// the statement with its parameters; the core adds the plugin, the
    /// file's identity and the statement's class, which it reads from SQLite
    /// itself, so no plugin speaks for another or names its own class.
    Database {
        /// The plugin whose tool runs the statement.
        plugin: String,
        /// The plugin's key for the database.
        key: DatabaseKey,
        /// The owner's name for the database, for display only.
        name: String,
        /// Where the database lives.
        home: DatabaseHome,
        /// The file's identity. A file that a program replaced since the
        /// approval changes the digest, so the approval no longer counts.
        identity: String,
        /// The statement's text.
        statement: String,
        /// Whether a call's view cut the statement to its limit. An access
        /// that the policy decides on is never cut.
        #[serde(default)]
        statement_cut: bool,
        /// The parameters' values as SQL writes them, at most
        /// [`MAX_VIEW_PARAMETERS`](super::database::MAX_VIEW_PARAMETERS) of
        /// them, each cut to its limit, for display.
        parameters: Vec<String>,
        /// How many parameters the statement has, of which `parameters`
        /// shows the first.
        parameter_count: u32,
        /// The digest of the parameters' full values, which an approval
        /// covers and a grant does not.
        parameters_digest: String,
        /// What the statement does, as SQLite reports it.
        class: StatementClass,
    },
}

impl Access {
    /// Adds the plugin that prepared a browser access.
    pub fn from_browser(plugin: String, access: BrowserAccess) -> Self {
        Self::BrowserAction {
            plugin,
            origin: access.origin,
            session: access.session,
            revision: access.revision,
            action: access.action,
            credential: access.credential,
        }
    }

    /// Adds the plugin that prepared a browser handoff.
    pub fn from_browser_control(plugin: String, access: BrowserControlAccess) -> Self {
        Self::BrowserControl {
            plugin,
            session: access.session,
            revision: access.revision,
        }
    }

    /// Whether the access changes something: a change of the file system, a
    /// command on this machine, on a remote host or in a terminal, a run of
    /// an installed program, a write or a schema change of a database, or a
    /// move into a project.
    pub fn changes(&self) -> bool {
        match self {
            Self::Path { mode, .. } => mode.changes(),
            Self::Database { class, .. } => *class != StatementClass::Read,
            Self::BrowserAction { action, .. } => *action != BrowserAction::Observe,
            Self::Command { .. }
            | Self::Project { .. }
            | Self::RemoteCommand { .. }
            | Self::Program { .. }
            | Self::ExternalTool { .. }
            | Self::TerminalCommand { .. }
            | Self::BrowserControl { .. } => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Access, AccessMode, BrowserAccess, BrowserAction, EntryCount, ProgramEffect};

    #[test]
    fn browser_access_names_an_origin_and_opaque_revisions() {
        let mut access = BrowserAccess::new(
            "http://[::1]:8080".to_owned(),
            "session-1".to_owned(),
            "document-3".to_owned(),
            BrowserAction::Observe,
        );
        assert!(access.is_canonical());
        assert!(!Access::from_browser("browser".to_owned(), access.clone()).changes());
        for invalid in [
            "https://site.example/form",
            "https://user:password@site.example",
            "https://site.example?code=auth",
            "file:///local.txt",
        ] {
            access.origin = invalid.to_owned();
            assert!(!access.is_canonical(), "{invalid}");
        }
        access.origin = "https://site.example".to_owned();
        access.revision = "input value".to_owned();
        assert!(!access.is_canonical());
    }

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
