//! Commands that run in the background (ADR-0041,
//! docs/features/system-tools/background-commands.md): how one ended, where
//! it runs, its view for the UI and the conversation's events, and the record
//! that the store keeps of it beside its start call's part.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::clock::UtcMillis;
use crate::ids::{ArchiveId, PartId};

use super::call::ProcessRecord;

/// The most characters of a command's text that its view carries. The
/// record keeps the whole text.
pub const MAX_VIEW_COMMAND_CHARS: usize = 2_000;

/// The background commands that run at once in one conversation, by default.
pub const MAX_COMMANDS_PER_CONVERSATION: u32 = 8;

/// The background commands that run at once on one machine, by default.
pub const MAX_COMMANDS_PER_MACHINE: u32 = 32;

/// How a background command ended, and who ended it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundEnding {
    /// The command ended on its own, with its exit code.
    Exited,
    /// A remote command's host reported its end, with an exit status or a
    /// signal, or closed its channel.
    EndedByHost,
    /// The agent stopped it with `stop_command`.
    StoppedByAgent,
    /// The owner stopped it.
    StoppedByOwner,
    /// The run of its task conversation ended.
    TaskEnded,
    /// Its plugin went off.
    PluginOff,
    /// Mannie closed the session that it ran in, as when the owner edits or
    /// removes a host.
    SessionClosed,
    /// Mannie or the worker quit, or a crash ended it.
    Interrupted,
    /// Its plugin let it go without saying how it ended.
    LetGo,
}

impl BackgroundEnding {
    /// Whether the command ended on its own, which wakes an idle agent
    /// (docs/features/system-tools/wake-ups.md).
    pub fn is_its_own(self) -> bool {
        matches!(self, Self::Exited | Self::EndedByHost)
    }

    /// Whether the end must be told to the model: every end but the one that
    /// the agent itself asked for, whose result told it.
    pub fn is_news(self) -> bool {
        self != Self::StoppedByAgent
    }

    /// The ending's name as logs and the store keep it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exited => "exited",
            Self::EndedByHost => "ended_by_host",
            Self::StoppedByAgent => "stopped_by_agent",
            Self::StoppedByOwner => "stopped_by_owner",
            Self::TaskEnded => "task_ended",
            Self::PluginOff => "plugin_off",
            Self::SessionClosed => "session_closed",
            Self::Interrupted => "interrupted",
            Self::LetGo => "let_go",
        }
    }
}

/// Where a background command runs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum BackgroundSource {
    /// In a shell of the machine whose runtime keeps it.
    Local {
        /// The shell's path.
        shell: String,
        /// The canonical working directory.
        working_directory: String,
    },
    /// On a remote host that a plugin reaches (ADR-0041, decision 11).
    Remote {
        /// The plugin that reaches the host.
        plugin: String,
        /// The host's name.
        label: String,
        /// Where the command runs, such as `deploy@build-box.lan:22`.
        endpoint: String,
    },
}

/// A command that runs, or ran, in the background, as the UI and the
/// conversation's events show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundCommandView {
    /// Its number in its conversation, from 1.
    pub number: u32,
    /// The part of the call that started it.
    pub part_id: PartId,
    /// Its text, cut at [`MAX_VIEW_COMMAND_CHARS`] characters.
    pub command: String,
    /// Where it runs.
    pub source: BackgroundSource,
    /// When it started.
    pub started_at: UtcMillis,
    /// How it ended, or `None` while it runs.
    pub ending: Option<BackgroundEnding>,
    /// Its exit code, when it ended with one.
    pub exit_code: Option<i32>,
    /// The signal that ended it on its host, when the host reported one.
    pub signal: Option<String>,
    /// When it ended.
    pub ended_at: Option<UtcMillis>,
    /// The lines that it wrote, known once it ended; 0 while it runs, whose
    /// output the UI follows on its own.
    pub lines: u32,
    /// The bytes that it wrote, known once it ended.
    pub bytes: u32,
    /// The archive of its output, once it ended.
    pub archive_id: Option<ArchiveId>,
    /// Whether it runs on a worker, whose task the owner stops to stop it.
    /// The master sets it on the commands of a worker's task conversation.
    pub on_worker: bool,
}

impl BackgroundCommandView {
    /// Whether the command still runs.
    pub fn is_running(&self) -> bool {
        self.ending.is_none()
    }
}

/// The request of a turn that the end of background commands started, which
/// the owner sees as a row in place of a message (docs/features/
/// system-tools/wake-ups.md, the part `command_note`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CommandNoteView {
    /// The ends that the note tells, by the commands' numbers.
    pub ends: Vec<CommandEndView>,
}

/// A background command's end, as a wake-up's row names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CommandEndView {
    /// The command's number in its conversation.
    pub number: u32,
    /// Its text, cut at [`MAX_VIEW_COMMAND_CHARS`] characters.
    pub command: String,
    /// How it ended.
    pub ending: BackgroundEnding,
    /// Its exit code, when it ended with one.
    pub exit_code: Option<i32>,
    /// The signal that its host reported, when it reported one.
    pub signal: Option<String>,
    /// How many of its lines the model had not read.
    pub unread: u32,
}

/// The record of a background command that the store keeps beside its
/// start call's part (ADR-0041, decision 6). The fields grow by addition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundCommandDocument {
    /// The command's whole text.
    pub command: String,
    /// Where it runs.
    pub source: BackgroundSource,
    /// The shell's process, for the recovery after a crash; `None` for a
    /// remote command.
    #[serde(default)]
    pub process: Option<ProcessRecord>,
    /// When it started.
    pub started_at: UtcMillis,
    /// How it ended, or `None` while it runs.
    #[serde(default)]
    pub ending: Option<BackgroundEnding>,
    /// Its exit code, when it ended with one.
    #[serde(default)]
    pub exit_code: Option<i32>,
    /// The signal that ended it on its host.
    #[serde(default)]
    pub signal: Option<String>,
    /// When it ended.
    #[serde(default)]
    pub ended_at: Option<UtcMillis>,
    /// The lines that it wrote.
    #[serde(default)]
    pub lines: u64,
    /// The bytes that it wrote.
    #[serde(default)]
    pub bytes: u64,
    /// Whether bytes that are not valid UTF-8 were replaced.
    #[serde(default)]
    pub replaced: bool,
    /// How many of its first lines the archive holds before the lines that
    /// the log let go.
    #[serde(default)]
    pub head_lines: u64,
    /// The lines that the log let go, when it let any go.
    #[serde(default)]
    pub left_out: Option<(u64, u64)>,
    /// The archive of its output, once it ended.
    #[serde(default)]
    pub archive_id: Option<ArchiveId>,
    /// The last line that the model read without naming a line.
    #[serde(default)]
    pub read_up_to: u64,
    /// Whether the model was told of its end, by a note or by a result.
    #[serde(default)]
    pub told: bool,
}

impl BackgroundCommandDocument {
    /// The view of the command with this number, started by this part.
    pub fn view(&self, number: u32, part_id: PartId) -> BackgroundCommandView {
        BackgroundCommandView {
            number,
            part_id,
            command: self.command.chars().take(MAX_VIEW_COMMAND_CHARS).collect(),
            source: self.source.clone(),
            started_at: self.started_at,
            ending: self.ending,
            exit_code: self.exit_code,
            signal: self.signal.clone(),
            ended_at: self.ended_at,
            lines: u32::try_from(self.lines).unwrap_or(u32::MAX),
            bytes: u32::try_from(self.bytes).unwrap_or(u32::MAX),
            archive_id: self.archive_id.clone(),
            on_worker: false,
        }
    }
}

/// Which limit of background commands a start reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundLimit {
    /// The commands that run at once in one conversation.
    Conversation,
    /// The commands that run at once on the machine.
    Machine,
}

#[cfg(test)]
mod tests {
    use super::{BackgroundCommandDocument, BackgroundEnding, BackgroundSource};
    use crate::clock::UtcMillis;
    use crate::ids::PartId;

    #[test]
    fn only_a_command_s_own_end_wakes_and_every_end_but_the_agent_s_is_news() {
        assert!(BackgroundEnding::Exited.is_its_own());
        assert!(BackgroundEnding::EndedByHost.is_its_own());
        assert!(!BackgroundEnding::StoppedByOwner.is_its_own());
        assert!(!BackgroundEnding::Interrupted.is_its_own());
        assert!(!BackgroundEnding::StoppedByAgent.is_news());
        assert!(BackgroundEnding::StoppedByOwner.is_news());
    }

    #[test]
    fn a_record_reads_with_the_fields_of_its_first_version_only() {
        let document: BackgroundCommandDocument = serde_json::from_value(serde_json::json!({
            "command": "npm run dev",
            "source": { "kind": "local", "shell": "/bin/sh", "workingDirectory": "/work" },
            "startedAt": 1_000,
        }))
        .unwrap();

        assert_eq!(document.ending, None);
        assert_eq!(document.read_up_to, 0);
        assert!(!document.told);
        let view = document.view(
            2,
            PartId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap(),
        );
        assert!(view.is_running());
        assert_eq!(view.started_at, UtcMillis::from_millis(1_000));
        assert_eq!(
            view.source,
            BackgroundSource::Local {
                shell: "/bin/sh".to_owned(),
                working_directory: "/work".to_owned(),
            }
        );
    }

    #[test]
    fn a_view_cuts_a_long_command() {
        let document = BackgroundCommandDocument {
            command: "x".repeat(super::MAX_VIEW_COMMAND_CHARS + 10),
            source: BackgroundSource::Remote {
                plugin: "ssh".to_owned(),
                label: "build-box".to_owned(),
                endpoint: "deploy@build-box.lan:22".to_owned(),
            },
            process: None,
            started_at: UtcMillis::from_millis(0),
            ending: Some(BackgroundEnding::EndedByHost),
            exit_code: Some(0),
            signal: None,
            ended_at: Some(UtcMillis::from_millis(5)),
            lines: 3,
            bytes: 12,
            replaced: false,
            head_lines: 3,
            left_out: None,
            archive_id: None,
            read_up_to: 3,
            told: true,
        };

        let view = document.view(
            1,
            PartId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap(),
        );

        assert_eq!(view.command.chars().count(), super::MAX_VIEW_COMMAND_CHARS);
        assert!(!view.is_running());
        assert_eq!(view.lines, 3);
    }
}
