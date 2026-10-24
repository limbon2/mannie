//! Full conversation records and the owner-initiated export contract.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::clock::UtcMillis;
use crate::ids::{ArchiveId, ConversationId, PartId, RunId};
use crate::images::ImageFormat;
use crate::run::RunView;
use crate::tools::BackgroundCommandView;
use crate::transcript::{Checkpoint, ConversationSummary, TurnView};

/// The complete-record encoding understood by this version.
pub const RECORD_VERSION: u32 = 1;

/// Maximum payload of a complete-record transport chunk (ADR-0040).
pub const MAX_EXPORT_CHUNK_BYTES: usize = 256 * 1024;

/// One conversation captured consistently from persisted data. This is not a
/// shortened UI view and never contains credentials or provider-private items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationRecord {
    /// The encoding version.
    pub version: u32,
    /// The conversation, including its parent when it is a task.
    pub conversation: ConversationSummary,
    /// The clock of the machine that captured the record.
    pub captured_at: UtcMillis,
    /// The last committed event included.
    pub through_seq: u32,
    /// Every current turn, oldest first.
    pub turns: Vec<RecordTurn>,
    /// Every saved checkpoint, oldest first.
    pub checkpoints: Vec<Checkpoint>,
    /// Full stored images of the included parts.
    pub images: Vec<RecordImage>,
    /// The images of the results of the included tool calls (ADR-0044,
    /// decision 7). A record of a version before them has none.
    #[serde(default)]
    pub tool_images: Vec<RecordToolImage>,
    /// Complete archived outputs of the included parts.
    pub archives: Vec<RecordArchive>,
    /// The children to collect separately, in task-number order.
    pub children: Vec<ConversationSummary>,
    /// The conversation's background commands, oldest first, each with the
    /// part of the call that started it (ADR-0041). A record of a version
    /// before them has none.
    #[serde(default)]
    pub background_commands: Vec<BackgroundCommandView>,
    /// Saved references whose content is unavailable.
    pub warnings: Vec<ExportWarning>,
}

/// A turn's UI metadata alongside its unshortened parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordTurn {
    /// Run, turn and timestamp metadata. Its parts are empty; `parts` below
    /// contains the complete record instead of the shortened views.
    pub view: TurnView,
    /// Earlier replies retained for their tool activity, with their full status.
    pub retained_runs: Vec<RunView>,
    /// Owner-facing parts in their persisted order.
    pub parts: Vec<RecordPart>,
}

/// A part's unshortened persisted text and versioned document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordPart {
    /// The part.
    pub id: PartId,
    /// Its kind, including kinds this version does not know.
    pub kind: String,
    /// Its lifecycle state as stored.
    pub state: String,
    /// The run that wrote the part, or none for the owner.
    pub run_id: Option<RunId>,
    /// The persisted position in its turn.
    pub position: u32,
    /// The complete persisted text.
    pub text: String,
    /// The saved document, excluding provider-private items kept elsewhere.
    pub document: String,
    /// When the part was created.
    pub created_at: UtcMillis,
}

/// Image bytes in the full-record transport. This deliberately does not cross
/// desktop IPC; only the Rust exporter and authenticated link consume it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordImage {
    /// The image part.
    pub part_id: PartId,
    /// The encoding of the stored bytes.
    pub format: ImageFormat,
    /// The stored bytes, rather than a thumbnail or a source-file reference.
    pub data: Vec<u8>,
}

/// An image of an included tool call's result, in the full-record
/// transport, which never crosses desktop IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordToolImage {
    /// The tool call's part.
    pub part_id: PartId,
    /// The image's position among the result's images.
    pub position: u32,
    /// The encoding of the stored bytes.
    pub format: ImageFormat,
    /// The stored bytes.
    pub data: Vec<u8>,
}

/// An archive attached to an included tool or task part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordArchive {
    /// The archive.
    pub id: ArchiveId,
    /// The part whose result holds it.
    pub part_id: PartId,
    /// Its output name, such as stdout.
    pub name: String,
    /// Its complete saved text, including original line breaks.
    pub content: String,
}

/// Something the export could not include. Codes are translated at the
/// desktop and Markdown boundaries, never supplied as display text by Rust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportWarning {
    /// The affected conversation.
    pub conversation_id: ConversationId,
    /// The missing part or archive ID, or the conversation ID.
    pub reference: String,
    /// The reason.
    pub reason: ExportError,
}

/// The result of the native export action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ExportOutcome {
    /// The owner closed the native folder picker.
    Canceled,
    /// The new export folder was flushed and completed.
    Saved {
        /// The new folder, chosen and created by Rust.
        destination: String,
        /// Every omission, also recorded in the main document.
        warnings: Vec<ExportWarning>,
    },
}

/// Stable refusals at export boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ExportError {
    /// The requested conversation is no longer present.
    #[error("the conversation does not exist")]
    ConversationNotFound,
    /// The store could not provide or journal the record.
    #[error("the conversation record could not be read or journaled")]
    StorageFailed,
    /// The core has not opened its store.
    #[error("the store is unavailable")]
    StoreUnavailable,
    /// The native picker failed.
    #[error("the folder picker failed")]
    DialogFailed,
    /// Creating or flushing the export failed.
    #[error("the export folder could not be written")]
    WriteFailed,
    /// One export at a time keeps record collection within its memory budget.
    #[error("an export is already running")]
    ExportActive,
    /// The executor or peer does not provide complete records.
    #[error("complete-record reads are unsupported")]
    Unsupported,
    /// The executor's host could not be reached or disconnected.
    #[error("the task host is unavailable")]
    HostUnavailable,
    /// The plugin that serves this conversation is off.
    #[error("the task plugin is off")]
    PluginOff,
    /// An attachment's saved bytes or archive no longer exist.
    #[error("saved attachment content is missing")]
    AttachmentMissing,
    /// The record is malformed or uses an unsupported version.
    #[error("the conversation record is not supported")]
    InvalidRecord,
}

impl ExportError {
    /// A stable code for logs and tests.
    pub fn code(&self) -> &'static str {
        match self {
            Self::ConversationNotFound => "export.conversation_not_found",
            Self::StorageFailed => "export.storage_failed",
            Self::StoreUnavailable => "export.store_unavailable",
            Self::DialogFailed => "export.dialog_failed",
            Self::WriteFailed => "export.write_failed",
            Self::ExportActive => "export.export_active",
            Self::Unsupported => "export.unsupported",
            Self::HostUnavailable => "export.host_unavailable",
            Self::PluginOff => "export.plugin_off",
            Self::AttachmentMissing => "export.attachment_missing",
            Self::InvalidRecord => "export.invalid_record",
        }
    }
}
