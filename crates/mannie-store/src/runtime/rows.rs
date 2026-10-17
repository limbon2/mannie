//! Reading rows into the views the UI shows, and turning stored events into
//! the events subscribers receive. Live publishing and replay both build their
//! events here, so a subscriber sees the same event either way.

use mannie_types::clock::UtcMillis;
use mannie_types::ids::{
    CheckpointId, ConversationId, PartId, ProfileId, ProjectId, RunId, TurnId,
};
use mannie_types::images::ImageView;
use mannie_types::mentions::MentionView;
use mannie_types::run::{RunActivity, RunFailure, RunOutcome, RunState, RunView};
use mannie_types::tasks::{TaskConversation, TaskPartDocument};
use mannie_types::tools::ToolCallDocument;
use mannie_types::transcript::{
    Audience, CheckpointView, ContextUsage, ConversationEvent, ConversationSummary, PartContent,
    PartKind, PartState, PartView, TitleSource, TurnView,
};
use rusqlite::{Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::{NewImage, RuntimeStoreError};

pub(crate) const SELECT_RUN: &str = "SELECT id, turn_id, profile_id, state, activity, outcome, created_at, ended_at \
     FROM runtime_runs WHERE id = ?1";
pub(crate) const SELECT_PART: &str = "SELECT id, turn_id, run_id, position, kind, audience, state, text, document, created_at \
     FROM runtime_parts WHERE id = ?1";
pub(crate) const SELECT_DELTAS: &str = "SELECT body FROM runtime_events \
     WHERE part_id = ?1 AND kind = 'part_delta' ORDER BY seq";
/// The columns of a conversation summary, for every query that reads one.
macro_rules! summary_columns {
    () => {
        "c.id, c.title, c.title_source, c.profile_id, c.created_at, c.updated_at, \
         EXISTS (SELECT 1 FROM runtime_runs r WHERE r.conversation_id = c.id \
         AND r.state IN ('queued', 'running')), \
         EXISTS (SELECT 1 FROM runtime_runs r WHERE r.conversation_id = c.id \
         AND r.state IN ('queued', 'running') \
         AND json_extract(r.activity, '$.kind') = 'awaiting_approval'), \
         c.workspace, c.project_id, c.parent_conversation_id, c.task_number, c.task_agent, \
         c.task_executor_kind, c.task_executor_target, c.task_executor_name, \
         c.task_executor_noun, c.full_access"
    };
}
pub(crate) use summary_columns;
pub(crate) const SELECT_SUMMARY: &str = concat!(
    "SELECT ",
    summary_columns!(),
    " FROM runtime_conversations c WHERE c.id = ?1"
);
pub(crate) const SELECT_KEPT_RUNS: &str = "SELECT k.id FROM runtime_runs k \
     JOIN runtime_runs r ON r.turn_id = k.turn_id \
     WHERE r.id = ?1 AND k.kept = 1 AND k.id <> ?1 ORDER BY k.created_at, k.id";
pub(crate) const SELECT_TURN: &str =
    "SELECT id, position, current_run_id, created_at FROM runtime_turns WHERE id = ?1";
pub(crate) const SELECT_OWNER_PARTS: &str = "SELECT id FROM runtime_parts \
     WHERE turn_id = ?1 AND run_id IS NULL ORDER BY position";
pub(crate) const SELECT_CHECKPOINT_VIEW: &str = "SELECT id, covers_through_turn, document, created_at \
     FROM runtime_checkpoints WHERE id = ?1";

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    SELECT_RUN,
    SELECT_PART,
    SELECT_DELTAS,
    SELECT_SUMMARY,
    SELECT_KEPT_RUNS,
    SELECT_TURN,
    SELECT_OWNER_PARTS,
    SELECT_CHECKPOINT_VIEW,
];

/// The document of a part: what its kind needs beyond its text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PartDocument {
    /// The failure of an error part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) failure: Option<RunFailure>,
    /// The call of a tool call part, which is also its operation's journal
    /// record (ADR-0013), or the call record of a task part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tool_call: Option<ToolCallDocument>,
    /// The task section of a task part (ADR-0024, decision 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) task: Option<TaskPartDocument>,
    /// What a mention part names and what the model got of it (ADR-0032).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) mention: Option<MentionView>,
    /// What an image part's image is; its bytes are in `runtime_images`
    /// (ADR-0033).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) image: Option<ImageDocument>,
}

/// The document of an image part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImageDocument {
    /// The source that gave the image.
    pub(crate) source: String,
    /// What the owner sees.
    pub(crate) view: ImageView,
    /// The SHA-256 digest of the image's bytes, in hexadecimal.
    pub(crate) digest: String,
}

impl ImageDocument {
    pub(crate) fn of(image: &NewImage) -> Self {
        Self {
            source: image.source.clone(),
            view: image.view.clone(),
            digest: image.digest.clone(),
        }
    }
}

/// The document of a checkpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CheckpointDocument {
    pub(crate) tokens_before: u32,
    pub(crate) tokens_after: u32,
    pub(crate) trigger: String,
}

/// The body of a stored event. Most events name the row they concern, and the
/// row supplies the content when the event is read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EventBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) turn_id: Option<TurnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) run_id: Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) part_ids: Vec<PartId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) checkpoint_id: Option<CheckpointId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) usage: Option<ContextUsage>,
}

impl EventBody {
    pub(crate) fn empty() -> Self {
        Self {
            turn_id: None,
            run_id: None,
            text: None,
            part_ids: Vec::new(),
            checkpoint_id: None,
            usage: None,
        }
    }
}

/// The kinds of stored events.
pub(crate) mod kinds {
    pub(crate) const TURN_STARTED: &str = "turn_started";
    pub(crate) const RUN_STARTED: &str = "run_started";
    pub(crate) const RUN_UPDATED: &str = "run_updated";
    pub(crate) const PART_STARTED: &str = "part_started";
    pub(crate) const PART_DELTA: &str = "part_delta";
    pub(crate) const PART_FINISHED: &str = "part_finished";
    pub(crate) const PART_UPDATED: &str = "part_updated";
    pub(crate) const ATTEMPT_DISCARDED: &str = "attempt_discarded";
    pub(crate) const CHECKPOINT_CREATED: &str = "checkpoint_created";
    pub(crate) const CONTEXT_USAGE_CHANGED: &str = "context_usage_changed";
    pub(crate) const CONVERSATION_CHANGED: &str = "conversation_changed";
}

pub(crate) fn to_json<T: Serialize>(value: &T) -> Result<String, RuntimeStoreError> {
    serde_json::to_string(value).map_err(|error| {
        RuntimeStoreError::invalid(format!("could not encode a document: {error}"))
    })
}

pub(crate) fn from_json<T: for<'de> Deserialize<'de>>(
    text: &str,
    what: &str,
) -> Result<T, RuntimeStoreError> {
    serde_json::from_str(text)
        .map_err(|error| RuntimeStoreError::invalid(format!("{what}: {error}")))
}

pub(crate) fn millis(value: i64) -> UtcMillis {
    UtcMillis::from_millis(value)
}

pub(crate) fn position(value: i64, what: &str) -> Result<u32, RuntimeStoreError> {
    u32::try_from(value)
        .map_err(|_out_of_range| RuntimeStoreError::invalid(format!("{what} is out of range")))
}

pub(crate) fn parse_id<T>(
    text: &str,
    parse: fn(&str) -> Result<T, mannie_types::ids::InvalidId>,
) -> Result<T, RuntimeStoreError> {
    parse(text).map_err(|error| RuntimeStoreError::invalid(error.to_string()))
}

/// A run's row.
fn run_from_row(row: &Row<'_>) -> rusqlite::Result<RawRun> {
    Ok(RawRun {
        id: row.get(0)?,
        turn_id: row.get(1)?,
        profile_id: row.get(2)?,
        state: row.get(3)?,
        activity: row.get(4)?,
        outcome: row.get(5)?,
        created_at: row.get(6)?,
        ended_at: row.get(7)?,
    })
}

struct RawRun {
    id: String,
    turn_id: String,
    profile_id: String,
    state: String,
    activity: Option<String>,
    outcome: Option<String>,
    created_at: i64,
    ended_at: Option<i64>,
}

impl RawRun {
    fn into_view(self) -> Result<RunView, RuntimeStoreError> {
        let state = RunState::parse(&self.state).ok_or_else(|| {
            RuntimeStoreError::invalid(format!("unknown run state {:?}", self.state))
        })?;
        let activity = self
            .activity
            .as_deref()
            .map(|text| from_json::<RunActivity>(text, "run activity"))
            .transpose()?;
        let outcome = self
            .outcome
            .as_deref()
            .map(|text| from_json::<RunOutcome>(text, "run outcome"))
            .transpose()?;
        Ok(RunView {
            id: parse_id(&self.id, RunId::parse)?,
            turn_id: parse_id(&self.turn_id, TurnId::parse)?,
            profile_id: parse_id(&self.profile_id, ProfileId::parse)?,
            state,
            activity,
            outcome,
            created_at: millis(self.created_at),
            ended_at: self.ended_at.map(millis),
        })
    }
}

/// Reads a run.
pub(crate) fn run_view(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<RunView>, RuntimeStoreError> {
    let raw = connection
        .prepare_cached(SELECT_RUN)?
        .query_row([run_id], run_from_row)
        .optional()?;
    raw.map(RawRun::into_view).transpose()
}

/// A part's row.
pub(crate) struct PartRow {
    pub(crate) id: String,
    pub(crate) turn_id: String,
    pub(crate) run_id: Option<String>,
    pub(crate) position: i64,
    pub(crate) kind: String,
    pub(crate) audience: String,
    pub(crate) state: String,
    pub(crate) text: String,
    pub(crate) document: String,
    pub(crate) created_at: i64,
}

pub(crate) fn part_from_row(row: &Row<'_>) -> rusqlite::Result<PartRow> {
    Ok(PartRow {
        id: row.get(0)?,
        turn_id: row.get(1)?,
        run_id: row.get(2)?,
        position: row.get(3)?,
        kind: row.get(4)?,
        audience: row.get(5)?,
        state: row.get(6)?,
        text: row.get(7)?,
        document: row.get(8)?,
        created_at: row.get(9)?,
    })
}

impl PartRow {
    /// Who the part is for. An unknown audience counts as the model's only,
    /// so nothing unexpected reaches the window.
    pub(crate) fn audience(&self) -> Audience {
        Audience::parse(&self.audience).unwrap_or(Audience::Model)
    }

    pub(crate) fn state(&self) -> Result<PartState, RuntimeStoreError> {
        PartState::parse(&self.state).ok_or_else(|| {
            RuntimeStoreError::invalid(format!("unknown part state {:?}", self.state))
        })
    }

    /// The call of a tool call part.
    pub(crate) fn tool_call(&self) -> Result<ToolCallDocument, RuntimeStoreError> {
        let document: PartDocument = from_json(&self.document, "part document")?;
        document.tool_call.ok_or_else(|| {
            RuntimeStoreError::invalid(format!("the tool call part {} has no call", self.id))
        })
    }

    /// The call record and the task section of a task part. A part whose
    /// task section is not stored yet has an empty one.
    pub(crate) fn task_call(
        &self,
    ) -> Result<(ToolCallDocument, TaskPartDocument), RuntimeStoreError> {
        let document: PartDocument = from_json(&self.document, "part document")?;
        let call = document.tool_call.ok_or_else(|| {
            RuntimeStoreError::invalid(format!("the task part {} has no call", self.id))
        })?;
        Ok((call, document.task.unwrap_or_default()))
    }

    /// Whether the part is a call, whose arguments subscribers do not receive
    /// as deltas.
    pub(crate) fn is_call(&self) -> bool {
        PartKind::parse(&self.kind).is_some_and(PartKind::is_call)
    }

    /// The part as the UI shows it, or `None` when the owner does not see it.
    /// A streaming part's text is rebuilt from its delta events.
    pub(crate) fn view(
        &self,
        connection: &Connection,
    ) -> Result<Option<PartView>, RuntimeStoreError> {
        if !self.audience().includes_owner() {
            return Ok(None);
        }
        let state = self.state()?;
        let text = if state == PartState::Streaming {
            streaming_text(connection, &self.id)?
        } else {
            self.text.clone()
        };
        let content = match PartKind::parse(&self.kind) {
            Some(PartKind::UserText) => PartContent::UserText { text },
            Some(PartKind::Text) => PartContent::Text { text },
            Some(PartKind::Reasoning) => PartContent::Reasoning { text },
            Some(PartKind::Error) => {
                let document: PartDocument = from_json(&self.document, "part document")?;
                let failure = document.failure.unwrap_or(RunFailure::Internal);
                PartContent::Error { failure }
            }
            Some(PartKind::ContinuationNote) => return Ok(None),
            Some(PartKind::ToolCall) => {
                let call = self.tool_call()?;
                PartContent::ToolCall {
                    call: Box::new(call.view(&text)),
                }
            }
            Some(PartKind::Task) => {
                let (call, task) = self.task_call()?;
                PartContent::Task {
                    call: Box::new(task.view(&call)),
                }
            }
            Some(PartKind::Mention) => {
                let document: PartDocument = from_json(&self.document, "part document")?;
                match document.mention {
                    Some(mention) => PartContent::Mention { mention },
                    None => PartContent::Unknown {
                        name: self.kind.clone(),
                    },
                }
            }
            Some(PartKind::Image) => {
                let document: PartDocument = from_json(&self.document, "part document")?;
                match document.image {
                    Some(image) => PartContent::Image { image: image.view },
                    None => PartContent::Unknown {
                        name: self.kind.clone(),
                    },
                }
            }
            None => PartContent::Unknown {
                name: self.kind.clone(),
            },
        };
        Ok(Some(PartView {
            id: parse_id(&self.id, PartId::parse)?,
            turn_id: parse_id(&self.turn_id, TurnId::parse)?,
            run_id: self
                .run_id
                .as_deref()
                .map(|run| parse_id(run, RunId::parse))
                .transpose()?,
            position: position(self.position, "a part's position")?,
            state,
            content,
            created_at: millis(self.created_at),
        }))
    }
}

/// Reads a part's row.
pub(crate) fn part_row(
    connection: &Connection,
    part_id: &str,
) -> Result<Option<PartRow>, RuntimeStoreError> {
    Ok(connection
        .prepare_cached(SELECT_PART)?
        .query_row([part_id], part_from_row)
        .optional()?)
}

/// The text of a streaming part: its delta events, in order.
pub(crate) fn streaming_text(
    connection: &Connection,
    part_id: &str,
) -> Result<String, RuntimeStoreError> {
    let mut select = connection.prepare_cached(SELECT_DELTAS)?;
    let bodies = select.query_map([part_id], |row| row.get::<_, String>(0))?;
    let mut text = String::new();
    for body in bodies {
        let body: EventBody = from_json(&body?, "delta event")?;
        if let Some(delta) = body.text {
            text.push_str(&delta);
        }
    }
    Ok(text)
}

/// Reads a conversation as the list shows it.
pub(crate) fn conversation_summary(
    connection: &Connection,
    conversation_id: &str,
) -> Result<Option<ConversationSummary>, RuntimeStoreError> {
    let raw = connection
        .prepare_cached(SELECT_SUMMARY)?
        .query_row([conversation_id], summary_from_row)
        .optional()?;
    raw.map(RawSummary::into_summary).transpose()
}

pub(crate) struct RawSummary {
    id: String,
    title: String,
    title_source: String,
    profile_id: Option<String>,
    created_at: i64,
    updated_at: i64,
    has_active_run: bool,
    awaiting_approval: bool,
    workspace: Option<String>,
    project_id: Option<String>,
    parent_conversation_id: Option<String>,
    task_number: Option<i64>,
    task_agent: Option<String>,
    task_executor_kind: Option<String>,
    task_executor_target: Option<String>,
    task_executor_name: Option<String>,
    task_executor_noun: Option<String>,
    full_access: bool,
}

/// Reads the columns of [`summary_columns`].
pub(crate) fn summary_from_row(row: &Row<'_>) -> rusqlite::Result<RawSummary> {
    Ok(RawSummary {
        id: row.get(0)?,
        title: row.get(1)?,
        title_source: row.get(2)?,
        profile_id: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        has_active_run: row.get(6)?,
        awaiting_approval: row.get(7)?,
        workspace: row.get(8)?,
        project_id: row.get(9)?,
        parent_conversation_id: row.get(10)?,
        task_number: row.get(11)?,
        task_agent: row.get(12)?,
        task_executor_kind: row.get(13)?,
        task_executor_target: row.get(14)?,
        task_executor_name: row.get(15)?,
        task_executor_noun: row.get(16)?,
        full_access: row.get(17)?,
    })
}

impl RawSummary {
    pub(crate) fn into_summary(self) -> Result<ConversationSummary, RuntimeStoreError> {
        // A title source this version does not know shows as the owner's.
        let title_source = TitleSource::parse(&self.title_source).unwrap_or(TitleSource::Owner);
        let executor = super::tasks::executor_ref(
            self.task_executor_kind,
            self.task_executor_target,
            self.task_executor_name,
            self.task_executor_noun,
        );
        let task = match self.parent_conversation_id.as_deref() {
            Some(parent) => Some(TaskConversation {
                parent_id: parse_id(parent, ConversationId::parse)?,
                number: position(self.task_number.unwrap_or(0), "a task number")?,
                agent: self.task_agent.unwrap_or_default(),
                worker: None,
                executor,
            }),
            None => None,
        };
        Ok(ConversationSummary {
            id: parse_id(&self.id, ConversationId::parse)?,
            title: self.title,
            title_source,
            profile_id: self
                .profile_id
                .as_deref()
                .map(|profile| parse_id(profile, ProfileId::parse))
                .transpose()?,
            has_active_run: self.has_active_run,
            awaiting_approval: self.awaiting_approval,
            full_access: self.full_access,
            project_id: self
                .project_id
                .as_deref()
                .map(|project| parse_id(project, ProjectId::parse))
                .transpose()?,
            workspace: self.workspace,
            task,
            created_at: millis(self.created_at),
            updated_at: millis(self.updated_at),
        })
    }
}

/// A turn's row.
pub(crate) struct TurnRow {
    pub(crate) id: String,
    pub(crate) position: i64,
    pub(crate) current_run_id: Option<String>,
    pub(crate) created_at: i64,
}

pub(crate) fn turn_from_row(row: &Row<'_>) -> rusqlite::Result<TurnRow> {
    Ok(TurnRow {
        id: row.get(0)?,
        position: row.get(1)?,
        current_run_id: row.get(2)?,
        created_at: row.get(3)?,
    })
}

impl TurnRow {
    /// The turn with the given run and parts.
    pub(crate) fn view(
        &self,
        run: Option<RunView>,
        parts: Vec<PartView>,
    ) -> Result<TurnView, RuntimeStoreError> {
        Ok(TurnView {
            id: parse_id(&self.id, TurnId::parse)?,
            position: position(self.position, "a turn's position")?,
            run,
            parts,
            created_at: millis(self.created_at),
        })
    }
}

/// Reads a checkpoint as the conversation view marks it.
pub(crate) fn checkpoint_view(
    connection: &Connection,
    checkpoint_id: &str,
) -> Result<Option<CheckpointView>, RuntimeStoreError> {
    let raw = connection
        .prepare_cached(SELECT_CHECKPOINT_VIEW)?
        .query_row([checkpoint_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .optional()?;
    raw.map(|(id, covers, document, created_at)| {
        let document: CheckpointDocument = from_json(&document, "checkpoint document")?;
        Ok(CheckpointView {
            id: parse_id(&id, CheckpointId::parse)?,
            covers_through_turn: position(covers, "a checkpoint's turn")?,
            tokens_before: document.tokens_before,
            tokens_after: document.tokens_after,
            created_at: millis(created_at),
        })
    })
    .transpose()
}

/// A stored event, as the event table holds it.
pub(crate) struct StoredRow {
    pub(crate) seq: i64,
    pub(crate) kind: String,
    pub(crate) part_id: Option<String>,
    pub(crate) body: String,
}

/// Builds the event a subscriber receives from a stored event, or `None` when
/// subscribers do not receive it, as for the parts of the model only.
pub(crate) fn hydrate(
    connection: &Connection,
    conversation_id: &str,
    stored: &StoredRow,
) -> Result<Option<ConversationEvent>, RuntimeStoreError> {
    let body: EventBody = from_json(&stored.body, "event body")?;
    let event = match stored.kind.as_str() {
        kinds::TURN_STARTED => {
            let turn_id = required(body.turn_id.as_ref(), "turn_started without a turn")?;
            let run_id = required(body.run_id.as_ref(), "turn_started without a run")?;
            turn_started(connection, turn_id.as_str(), run_id.as_str())?
                .map(|turn| ConversationEvent::TurnStarted { turn })
        }
        kinds::RUN_STARTED => {
            let run_id = required(body.run_id.as_ref(), "run_started without a run")?;
            match run_view(connection, run_id.as_str())? {
                Some(run) => Some(ConversationEvent::RunStarted {
                    run,
                    kept_run_ids: kept_runs(connection, run_id.as_str())?,
                }),
                None => None,
            }
        }
        kinds::RUN_UPDATED => {
            let run_id = required(body.run_id.as_ref(), "run_updated without a run")?;
            run_view(connection, run_id.as_str())?.map(|run| ConversationEvent::RunUpdated { run })
        }
        kinds::PART_STARTED | kinds::PART_FINISHED | kinds::PART_UPDATED => {
            let part_id = required(stored.part_id.as_ref(), "a part event without a part")?;
            let Some(row) = part_row(connection, part_id)? else {
                return Ok(None);
            };
            row.view(connection)?
                .map(|part| match stored.kind.as_str() {
                    kinds::PART_STARTED => ConversationEvent::PartStarted { part },
                    kinds::PART_UPDATED => ConversationEvent::PartUpdated { part },
                    _ => ConversationEvent::PartFinished { part },
                })
        }
        kinds::PART_DELTA => {
            let part_id = required(stored.part_id.as_ref(), "a delta without a part")?;
            // A call's arguments arrive in one delta that can be large;
            // subscribers get the call with the part's next update instead,
            // cut to what the view shows, and a task part's view has none.
            let audience_includes_owner = part_row(connection, part_id)?
                .is_some_and(|row| row.audience().includes_owner() && !row.is_call());
            if audience_includes_owner {
                Some(ConversationEvent::PartDelta {
                    part_id: parse_id(part_id, PartId::parse)?,
                    text: body.text.unwrap_or_default(),
                })
            } else {
                None
            }
        }
        kinds::ATTEMPT_DISCARDED => {
            let run_id = required(body.run_id, "attempt_discarded without a run")?;
            Some(ConversationEvent::AttemptDiscarded {
                run_id,
                part_ids: body.part_ids,
            })
        }
        kinds::CHECKPOINT_CREATED => {
            let checkpoint_id = required(
                body.checkpoint_id.as_ref(),
                "checkpoint_created without a checkpoint",
            )?;
            checkpoint_view(connection, checkpoint_id.as_str())?
                .map(|checkpoint| ConversationEvent::CheckpointCreated { checkpoint })
        }
        kinds::CONTEXT_USAGE_CHANGED => {
            let usage = required(body.usage, "context_usage_changed without usage")?;
            Some(ConversationEvent::ContextUsageChanged { usage })
        }
        kinds::CONVERSATION_CHANGED => conversation_summary(connection, conversation_id)?
            .map(|conversation| ConversationEvent::ConversationChanged { conversation }),
        other => Some(ConversationEvent::Unknown {
            name: other.to_owned(),
        }),
    };
    Ok(event)
}

/// The earlier runs of a run's turn whose parts the turn keeps, oldest first.
pub(crate) fn kept_runs(
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<RunId>, RuntimeStoreError> {
    let mut select = connection.prepare_cached(SELECT_KEPT_RUNS)?;
    let rows = select.query_map([run_id], |row| row.get::<_, String>(0))?;
    let mut kept = Vec::new();
    for row in rows {
        kept.push(parse_id(&row?, RunId::parse)?);
    }
    Ok(kept)
}

/// The turn as `turn_started` shows it: the owner's parts, which are the
/// message and what it mentions, and the run that started with the turn.
fn turn_started(
    connection: &Connection,
    turn_id: &str,
    run_id: &str,
) -> Result<Option<TurnView>, RuntimeStoreError> {
    let Some(turn) = connection
        .prepare_cached(SELECT_TURN)?
        .query_row([turn_id], turn_from_row)
        .optional()?
    else {
        return Ok(None);
    };
    let run = run_view(connection, run_id)?;
    let owner_part_ids: Vec<String> = connection
        .prepare_cached(SELECT_OWNER_PARTS)?
        .query_map([turn_id], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    let mut parts = Vec::new();
    for part_id in owner_part_ids {
        if let Some(row) = part_row(connection, &part_id)?
            && let Some(view) = row.view(connection)?
        {
            parts.push(view);
        }
    }
    Ok(Some(turn.view(run, parts)?))
}

fn required<T>(value: Option<T>, what: &str) -> Result<T, RuntimeStoreError> {
    value.ok_or_else(|| RuntimeStoreError::invalid(what))
}
