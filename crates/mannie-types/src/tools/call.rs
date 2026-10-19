//! A tool call: the document of a `tool_call` part, which is the journal record
//! of its operation, and the view the UI shows
//! (docs/features/system-tools/tool-calls.md, the transcript part).

use serde::{Deserialize, Deserializer, Serialize};
use specta::Type;

use crate::clock::UtcMillis;

use super::access::Access;
use super::approval::{ApprovalRequest, Decision};
use super::outcome::{ArchiveRef, ToolOutcome, ToolResult};

/// The largest arguments of one tool call, in bytes: the size of one event of
/// a provider's stream.
pub const MAX_ARGUMENTS_BYTES: usize = 1024 * 1024;

/// The most bytes of a call's arguments that the UI receives. Longer arguments
/// arrive cut, which the view says.
pub const MAX_VIEW_ARGUMENTS_BYTES: usize = 16 * 1024;

/// Where a tool call is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    /// The model is writing the call.
    Receiving,
    /// The call is complete and waits for its turn, its preparation or the
    /// policy.
    Pending,
    /// The owner decides.
    AwaitingApproval,
    /// An operation with side effects runs. This status was stored before the
    /// operation started.
    Executing,
    /// A read-only tool runs.
    Running,
    /// The tool finished, successfully or with a failure.
    Completed,
    /// The owner denied it.
    Denied,
    /// It did not run, because the reply stopped or Mannie ended first.
    Canceled,
    /// Mannie ended while the operation was executing, so it may have taken
    /// effect. Nothing repeats it automatically.
    Uncertain,
    /// It was never started; the document says why.
    NotRun,
}

impl ToolCallStatus {
    /// Every status.
    pub const ALL: [Self; 10] = [
        Self::Receiving,
        Self::Pending,
        Self::AwaitingApproval,
        Self::Executing,
        Self::Running,
        Self::Completed,
        Self::Denied,
        Self::Canceled,
        Self::Uncertain,
        Self::NotRun,
    ];

    /// Whether the call has ended.
    pub fn is_final(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Denied | Self::Canceled | Self::Uncertain | Self::NotRun
        )
    }

    /// The status of a call whose run ended before the call did: an operation
    /// that was executing may have taken effect, and every other call did not
    /// run (docs/features/system-tools/operations-and-approvals.md, recovery).
    #[must_use]
    pub fn after_interruption(self) -> Self {
        match self {
            Self::Executing => Self::Uncertain,
            status if status.is_final() => status,
            _ => Self::Canceled,
        }
    }
}

/// Why a tool call was never started.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(
    tag = "reason",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum NotRunReason {
    /// The model called a tool that does not exist, or that its agent may not
    /// use.
    UnknownTool,
    /// The arguments are not valid for the tool.
    InvalidArguments {
        /// What is wrong, as the parser said it.
        detail: String,
    },
    /// The output limit cut the call off before its arguments were complete.
    CutOff,
    /// The owner denied an earlier call of the same step.
    EarlierCallDenied,
    /// The reply used up its tool calls.
    OverBudget,
    /// The provider profile does not use tools.
    ToolsOff,
    /// A worker's policy ceiling forbids the operation (docs/features/
    /// workers/host-and-ceiling.md, the ceiling).
    Refused {
        /// What the ceiling forbids.
        refusal: Refusal,
    },
    /// The owner turned off the plugin of the tool, or of the agent that the
    /// call gives a task to, after the reply started (ADR-0030, decision 8).
    PluginOff {
        /// The plugin's ID.
        plugin: String,
    },
}

/// What a worker's policy ceiling forbids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    /// The ceiling, or the task, lets the worker only read: every change and
    /// every command is refused.
    ReadOnlyHost,
    /// A worker has no projects to move a conversation into.
    NoProjectsOnHost,
    /// A worker runs no commands on remote hosts. Plugins do not run on a
    /// worker, so its tools never ask; the refusal keeps the ceiling's
    /// answer for every access (ADR-0036).
    NoRemoteCommands,
}

/// Why an operation that was executing ended without its result while the
/// reply went on or ended, which makes it uncertain. A call without one was
/// interrupted because Mannie ended.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(
    tag = "reason",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Interruption {
    /// The owner stopped the reply while a plugin's operation ran, and the
    /// core let it go (ADR-0034, decision 3).
    Stopped,
    /// The owner turned off the operation's plugin while it ran, and it did
    /// not end in time (ADR-0034, decision 8).
    PluginOff {
        /// The plugin's ID.
        plugin: String,
    },
}

/// The process of a running command, so that the recovery after a crash can
/// find what is left of it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessRecord {
    /// The process ID of the command's shell.
    pub pid: u32,
    /// The shell's start time as the operating system records it, which tells
    /// the shell apart from a later process with the same ID.
    pub start_time: String,
}

/// The document of a `tool_call` part: the journal record of the call.
///
/// The call's arguments are the part's text. Fields that later versions add
/// are optional with a default, so older documents keep reading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallDocument {
    /// The call's ID as the provider named it, or as Mannie generated it.
    pub call_id: String,
    /// The tool's name as the model wrote it.
    pub tool: String,
    /// The few words in which the model said what the call does, cleaned for
    /// display (docs/features/system-tools/tool-calls.md, the activity of a
    /// call). It is the model's claim: nothing that authorizes the call reads
    /// it.
    #[serde(default)]
    pub activity: Option<String>,
    /// Where the call is in its life.
    pub status: ToolCallStatus,
    /// What the operation touches, once it was prepared.
    #[serde(default)]
    pub accesses: Vec<Access>,
    /// The question to the owner, while the call waits for it, and afterwards
    /// as a record of what the owner saw.
    #[serde(default)]
    pub request: Option<ApprovalRequest>,
    /// What allowed or denied the call.
    #[serde(default)]
    pub decision: Option<Decision>,
    /// When the decision was made.
    #[serde(default)]
    pub decided_at: Option<UtcMillis>,
    /// When the tool started.
    #[serde(default)]
    pub started_at: Option<UtcMillis>,
    /// When the call ended.
    #[serde(default)]
    pub ended_at: Option<UtcMillis>,
    /// The result of a call that ran.
    #[serde(default, deserialize_with = "stored_result")]
    pub result: Option<ToolResult>,
    /// Why a call never started.
    #[serde(default)]
    pub not_run: Option<NotRunReason>,
    /// The process of a command, while it runs and after.
    #[serde(default)]
    pub process: Option<ProcessRecord>,
    /// Why an operation that was executing ended without its result, when
    /// the owner stopped it or turned off its plugin.
    #[serde(default)]
    pub interruption: Option<Interruption>,
}

/// Reads the result of a stored call. An outcome that this version cannot
/// read, such as one of a tool that a later version added, reads as
/// [`ToolOutcome::Unknown`], and the rest of the result stays: the outcome
/// only feeds the UI's summary, and the model reads the output.
///
/// The fallback lives here, in a document that the UI never receives. A
/// `#[serde(other)]` on the outcome would give every type that contains one a
/// second shape in the generated bindings, one for each direction.
fn stored_result<'de, D>(deserializer: D) -> Result<Option<ToolResult>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct StoredResult {
        outcome: StoredOutcome,
        output: String,
        #[serde(default)]
        archives: Vec<ArchiveRef>,
    }

    // Serde tries the variants in order, so anything that is not an outcome
    // of this version lands in the second.
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StoredOutcome {
        Known(ToolOutcome),
        Unreadable(serde::de::IgnoredAny),
    }

    let Some(stored) = Option::<StoredResult>::deserialize(deserializer)? else {
        return Ok(None);
    };
    let outcome = match stored.outcome {
        StoredOutcome::Known(outcome) => outcome,
        StoredOutcome::Unreadable(_) => ToolOutcome::Unknown,
    };
    Ok(Some(ToolResult {
        outcome,
        output: stored.output,
        archives: stored.archives,
    }))
}

impl ToolCallDocument {
    /// A call that the model started to write.
    pub fn receiving(call_id: String, tool: String) -> Self {
        Self {
            call_id,
            tool,
            activity: None,
            status: ToolCallStatus::Receiving,
            accesses: Vec::new(),
            request: None,
            decision: None,
            decided_at: None,
            started_at: None,
            ended_at: None,
            result: None,
            not_run: None,
            process: None,
            interruption: None,
        }
    }

    /// The view of the call for the UI, with its arguments cut to
    /// [`MAX_VIEW_ARGUMENTS_BYTES`].
    pub fn view(&self, arguments: &str) -> ToolCallView {
        let cut = arguments.len() > MAX_VIEW_ARGUMENTS_BYTES;
        let shown = if cut {
            let mut end = MAX_VIEW_ARGUMENTS_BYTES;
            while !arguments.is_char_boundary(end) {
                end -= 1;
            }
            &arguments[..end]
        } else {
            arguments
        };
        ToolCallView {
            tool: self.tool.clone(),
            activity: self.activity.clone(),
            arguments: shown.to_owned(),
            arguments_cut: cut,
            status: self.status,
            accesses: self.accesses.clone(),
            request: self.request.clone(),
            decision: self.decision,
            started_at: self.started_at,
            ended_at: self.ended_at,
            result: self.result.clone(),
            not_run: self.not_run.clone(),
            interruption: self.interruption.clone(),
        }
    }
}

/// A tool call as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallView {
    /// The tool's name as the model wrote it.
    pub tool: String,
    /// What the model said the call does, in a few words. The model wrote
    /// it, so it is content and never what will happen.
    pub activity: Option<String>,
    /// The arguments as the model wrote them, cut to
    /// [`MAX_VIEW_ARGUMENTS_BYTES`].
    pub arguments: String,
    /// Whether the arguments are cut.
    pub arguments_cut: bool,
    /// Where the call is in its life.
    pub status: ToolCallStatus,
    /// What the operation touches, once it was prepared.
    pub accesses: Vec<Access>,
    /// The question to the owner.
    pub request: Option<ApprovalRequest>,
    /// What allowed or denied the call.
    pub decision: Option<Decision>,
    /// When the tool started.
    pub started_at: Option<UtcMillis>,
    /// When the call ended.
    pub ended_at: Option<UtcMillis>,
    /// The result of a call that ran.
    pub result: Option<ToolResult>,
    /// Why a call never started.
    pub not_run: Option<NotRunReason>,
    /// Why an operation that was executing ended without its result, when
    /// the owner stopped it or turned off its plugin.
    #[serde(default)]
    pub interruption: Option<Interruption>,
}

#[cfg(test)]
mod tests {
    use super::{Interruption, MAX_VIEW_ARGUMENTS_BYTES, ToolCallDocument, ToolCallStatus};
    use crate::tools::outcome::ToolOutcome;

    #[test]
    fn an_interruption_serializes_with_its_reason_and_older_documents_have_none() {
        assert_eq!(
            serde_json::to_value(Interruption::PluginOff {
                plugin: "ssh".to_owned()
            })
            .unwrap(),
            serde_json::json!({ "reason": "plugin_off", "plugin": "ssh" })
        );
        assert_eq!(
            serde_json::to_value(Interruption::Stopped).unwrap(),
            serde_json::json!({ "reason": "stopped" })
        );
        let older: ToolCallDocument = serde_json::from_value(serde_json::json!({
            "callId": "call_1",
            "tool": "run_command",
            "status": "uncertain",
        }))
        .unwrap();
        assert_eq!(older.interruption, None);
    }

    #[test]
    fn an_interrupted_call_is_uncertain_only_when_it_was_executing() {
        let after: Vec<_> = ToolCallStatus::ALL
            .into_iter()
            .map(|status| (status, status.after_interruption()))
            .collect();

        for (before, now) in after {
            let expected = match before {
                ToolCallStatus::Executing => ToolCallStatus::Uncertain,
                status if status.is_final() => status,
                _ => ToolCallStatus::Canceled,
            };
            assert_eq!(now, expected, "{before:?}");
            assert!(now.is_final(), "{before:?}");
        }
    }

    #[test]
    fn the_view_cuts_long_arguments_at_a_character() {
        let document = ToolCallDocument::receiving("call_1".to_owned(), "write_file".to_owned());
        let long = "é".repeat(MAX_VIEW_ARGUMENTS_BYTES);

        let view = document.view(&long);
        let short = document.view("{}");

        assert!(view.arguments_cut);
        assert!(view.arguments.len() <= MAX_VIEW_ARGUMENTS_BYTES);
        assert!(!short.arguments_cut);
        assert_eq!(short.arguments, "{}");
    }

    #[test]
    fn a_document_without_later_fields_still_reads() {
        let document: ToolCallDocument = serde_json::from_value(serde_json::json!({
            "callId": "call_1",
            "tool": "read_file",
            "status": "pending",
        }))
        .unwrap();

        assert_eq!(document.status, ToolCallStatus::Pending);
        assert_eq!(document.activity, None);
        assert_eq!(document.accesses, Vec::new());
        assert_eq!(document.result, None);
    }

    #[test]
    fn the_view_carries_the_activity() {
        let mut document = ToolCallDocument::receiving("call_1".to_owned(), "read_file".to_owned());
        document.activity = Some("Reading package.json".to_owned());

        let view = document.view(r#"{"activity":"Reading package.json","path":"package.json"}"#);

        assert_eq!(view.activity.as_deref(), Some("Reading package.json"));
    }

    #[test]
    fn a_result_with_an_outcome_of_an_unknown_tool_keeps_its_output() {
        let document: ToolCallDocument = serde_json::from_value(serde_json::json!({
            "callId": "call_1",
            "tool": "translate",
            "status": "completed",
            "result": {
                "outcome": { "kind": "translated", "language": "de" },
                "output": "Hallo",
            },
        }))
        .unwrap();

        let result = document.result.unwrap();
        assert_eq!(result.outcome, ToolOutcome::Unknown);
        assert_eq!(result.output, "Hallo");
        assert_eq!(result.archives, Vec::new());
    }

    #[test]
    fn a_plugin_tool_s_result_reads_back_and_one_without_its_plugin_reads_as_unknown() {
        let stored = |outcome: serde_json::Value| -> ToolCallDocument {
            serde_json::from_value(serde_json::json!({
                "callId": "call_1",
                "tool": "notes_find",
                "status": "completed",
                "result": { "outcome": outcome, "output": "2 notes." },
            }))
            .unwrap()
        };

        let complete = stored(serde_json::json!({ "kind": "plugin", "plugin": "notes" }));
        let without_plugin = stored(serde_json::json!({ "kind": "plugin" }));

        assert_eq!(
            complete.result.unwrap().outcome,
            ToolOutcome::Plugin {
                plugin: "notes".to_owned()
            }
        );
        let result = without_plugin.result.unwrap();
        assert_eq!(result.outcome, ToolOutcome::Unknown);
        assert_eq!(result.output, "2 notes.");
    }

    #[test]
    fn a_result_round_trips_through_the_document() {
        let mut document = ToolCallDocument::receiving("call_1".to_owned(), "edit_file".to_owned());
        document.activity = Some("Fixing the typo".to_owned());
        document.status = ToolCallStatus::Completed;
        document.result = Some(super::ToolResult {
            outcome: ToolOutcome::Edited {
                path: "notes.md".to_owned(),
                replacements: 1,
            },
            output: "Edited notes.md.".to_owned(),
            archives: Vec::new(),
        });

        let text = serde_json::to_string(&document).unwrap();
        let read: ToolCallDocument = serde_json::from_str(&text).unwrap();

        assert_eq!(read, document);
    }
}
