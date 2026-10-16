//! Reading conversations: snapshots and the events after a cursor for
//! subscribers, earlier turns, the conversation list, checkpoints, and the
//! transcript as the model reads it.

use mannie_types::clock::UtcMillis;
use mannie_types::ids::{CheckpointId, ConversationId, PartId, RunId, TurnId};
use mannie_types::run::RunState;
use mannie_types::tasks::TaskPartDocument;
use mannie_types::tools::ToolCallDocument;
use mannie_types::transcript::{
    Audience, Checkpoint, ContextUsage, ConversationEvent, ConversationSnapshot,
    ConversationSummary, PartKind, PartState, PartView, TurnView,
};
use rusqlite::{Connection, OptionalExtension, params};

use super::projects::{self, ConversationList};
use super::rows::{
    self, PartDocument, StoredRow, from_json, millis, parse_id, part_from_row, position, run_view,
    summary_columns, turn_from_row,
};
use super::{ProviderItem, RuntimeStoreError};
use crate::Store;

const SELECT_LAST_SEQ: &str = "SELECT last_seq FROM runtime_conversations WHERE id = ?1";
const SELECT_LATEST_TURNS: &str = "SELECT id, position, current_run_id, created_at FROM runtime_turns \
     WHERE conversation_id = ?1 ORDER BY position DESC LIMIT ?2";
const SELECT_TURNS_BEFORE: &str = "SELECT id, position, current_run_id, created_at FROM runtime_turns \
     WHERE conversation_id = ?1 AND position < ?2 ORDER BY position DESC LIMIT ?3";
// A turn shows the owner's message, the parts of its current run, and the parts
// of the earlier runs it keeps because they made tool calls.
const SELECT_TURN_PARTS: &str = "SELECT id, turn_id, run_id, position, kind, audience, state, text, document, created_at \
     FROM runtime_parts WHERE turn_id = ?1 AND (run_id IS NULL OR run_id = ?2 \
     OR run_id IN (SELECT id FROM runtime_runs WHERE turn_id = ?1 AND kept = 1)) AND state <> 'discarded' \
     ORDER BY position";
const SELECT_LATEST_CHECKPOINT: &str = "SELECT id FROM runtime_checkpoints WHERE conversation_id = ?1 \
     ORDER BY covers_through_turn DESC, created_at DESC LIMIT 1";
const SELECT_CONTEXT_USAGE: &str = "SELECT context_usage FROM runtime_conversations WHERE id = ?1";
const SELECT_CONTEXT_LEDGER: &str = "SELECT context FROM runtime_conversations WHERE id = ?1";
const SELECT_EVENTS_AFTER: &str = "SELECT seq, kind, part_id, body FROM runtime_events \
     WHERE conversation_id = ?1 AND seq > ?2 ORDER BY seq";
// The conversation list holds the owner's conversations; task conversations
// show only through their parents (docs/features/child-agents/tasks.md).
const SELECT_ALL_SUMMARIES: &str = concat!(
    "SELECT ",
    summary_columns!(),
    " FROM runtime_conversations c WHERE c.parent_conversation_id IS NULL \
     ORDER BY c.updated_at DESC, c.id DESC"
);
const SELECT_CHECKPOINTS: &str = "SELECT id, covers_through_turn, content_hash, summary, created_at \
     FROM runtime_checkpoints WHERE conversation_id = ?1 \
     ORDER BY covers_through_turn DESC, created_at DESC";
const SELECT_CHECKPOINT_SUMMARY: &str =
    "SELECT summary FROM runtime_checkpoints WHERE id = ?1 AND conversation_id = ?2";
const SELECT_MODEL_TURNS: &str = "SELECT t.id, t.position, t.current_run_id, r.state FROM runtime_turns t \
     LEFT JOIN runtime_runs r ON r.id = t.current_run_id \
     WHERE t.conversation_id = ?1 AND t.position > ?2 ORDER BY t.position";
// A task in the background keeps its part open until it ends, and the model
// reads the call with the result it was given meanwhile (ADR-0024, decision 2).
const SELECT_MODEL_PARTS: &str = "SELECT p.id, p.kind, p.audience, p.state, p.text, i.binding, i.item, \
     p.run_id, p.document, p.attempt \
     FROM runtime_parts p LEFT JOIN runtime_provider_items i ON i.part_id = p.id \
     WHERE p.turn_id = ?1 AND (p.run_id IS NULL OR p.run_id = ?2 \
     OR p.run_id IN (SELECT id FROM runtime_runs WHERE turn_id = ?1 AND kept = 1)) \
     AND (p.state NOT IN ('discarded', 'streaming') OR (p.state = 'streaming' AND p.kind = 'task')) \
     ORDER BY p.position";
const SELECT_KEPT_RUNS_OF_TURN: &str = "SELECT id, state FROM runtime_runs \
     WHERE turn_id = ?1 AND kept = 1 ORDER BY created_at, id";
const SELECT_LATEST_TURN: &str = "SELECT t.id, r.state FROM runtime_turns t \
     LEFT JOIN runtime_runs r ON r.id = t.current_run_id \
     WHERE t.conversation_id = ?1 ORDER BY t.position DESC LIMIT 1";
const DELETE_CONVERSATION: &str = "DELETE FROM runtime_conversations WHERE id = ?1";
const SELECT_TURN_REQUEST: &str = "SELECT text FROM runtime_parts      WHERE turn_id = ?1 AND run_id IS NULL ORDER BY position LIMIT 1";
const SELECT_TOOL_CALLS_BEFORE: &str = "SELECT COUNT(*) FROM runtime_parts p \
     JOIN runtime_parts c ON c.turn_id = p.turn_id \
     WHERE c.id = ?1 AND p.kind IN ('tool_call', 'task') AND p.position < c.position";
const SELECT_ACTIVE_RUN_COUNT: &str =
    "SELECT COUNT(*) FROM runtime_runs WHERE state IN ('queued', 'running')";

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    SELECT_LAST_SEQ,
    SELECT_LATEST_TURNS,
    SELECT_TURNS_BEFORE,
    SELECT_TURN_PARTS,
    SELECT_LATEST_CHECKPOINT,
    SELECT_CONTEXT_USAGE,
    SELECT_CONTEXT_LEDGER,
    SELECT_EVENTS_AFTER,
    SELECT_ALL_SUMMARIES,
    SELECT_CHECKPOINTS,
    SELECT_CHECKPOINT_SUMMARY,
    SELECT_MODEL_TURNS,
    SELECT_MODEL_PARTS,
    SELECT_KEPT_RUNS_OF_TURN,
    SELECT_LATEST_TURN,
    DELETE_CONVERSATION,
    SELECT_TURN_REQUEST,
    SELECT_TOOL_CALLS_BEFORE,
    SELECT_ACTIVE_RUN_COUNT,
];

/// The events after a subscriber's cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventsAfter {
    /// Every event after the cursor that subscribers receive, in order, and
    /// the sequence number of the conversation's latest event.
    Events {
        /// The events with their sequence numbers.
        events: Vec<(u32, ConversationEvent)>,
        /// The latest sequence number, including events that subscribers do
        /// not receive.
        last_seq: u32,
    },
    /// The cursor is too far behind, or beyond the latest event: the
    /// subscriber starts from a snapshot instead.
    UseSnapshot,
    /// The conversation does not exist.
    NotFound,
}

/// A checkpoint as the runtime reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCheckpoint {
    /// The checkpoint.
    pub id: CheckpointId,
    /// The position of the last turn it covers.
    pub covers_through_turn: u32,
    /// The hash of the covered text.
    pub content_hash: String,
    /// The summary.
    pub summary: String,
    /// When it was written.
    pub created_at: UtcMillis,
}

/// A turn as the model reads it: the owner's message, the parts of the
/// earlier runs that the turn keeps, and the parts of the current run that the
/// model sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelTurn {
    /// The turn.
    pub turn_id: TurnId,
    /// Its position.
    pub position: u32,
    /// Its current run, if it has one.
    pub run_id: Option<RunId>,
    /// The state of its current run, if it has one.
    pub run_state: Option<RunState>,
    /// The earlier runs that the turn keeps, oldest first.
    pub kept_runs: Vec<KeptRun>,
    /// Its parts for the model, in order.
    pub parts: Vec<ModelPart>,
}

/// An earlier run whose parts a turn keeps, because it made tool calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptRun {
    /// The run.
    pub run_id: RunId,
    /// How it ended.
    pub state: RunState,
}

/// A part as the model reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPart {
    /// The part.
    pub id: PartId,
    /// The run that wrote it, or `None` for the owner's message.
    pub run_id: Option<RunId>,
    /// What it is, or `None` for a kind this version does not know.
    pub kind: Option<PartKind>,
    /// The kind's name, also for unknown kinds.
    pub kind_name: String,
    /// Its state.
    pub state: PartState,
    /// Its text.
    pub text: String,
    /// Its provider-private data, if any.
    pub provider_item: Option<ProviderItem>,
    /// The call of a tool call part, or the call record of a task part.
    pub tool_call: Option<ToolCallDocument>,
    /// The task section of a task part.
    pub task: Option<TaskPartDocument>,
    /// The model call of its run that wrote it, counting from 1, which tells
    /// the steps of a reply apart; 0 for the owner's message.
    pub attempt: u32,
}

impl Store {
    /// A snapshot of a conversation: the latest `max_turns` turns, fewer when
    /// their parts would exceed `max_parts`, and the sequence number it
    /// reflects. `None` when the conversation does not exist.
    pub async fn snapshot(
        &self,
        conversation_id: &ConversationId,
        max_turns: u32,
        max_parts: u32,
    ) -> Result<Option<ConversationSnapshot>, RuntimeStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        read_consistently(self, move |connection| {
            snapshot_in(connection, &conversation, max_turns, max_parts)
        })
        .await
    }

    /// Up to `limit` turns before `before_position`, oldest first, and whether
    /// even older turns exist.
    pub async fn earlier_turns(
        &self,
        conversation_id: &ConversationId,
        before_position: u32,
        limit: u32,
    ) -> Result<(Vec<TurnView>, bool), RuntimeStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        read_consistently(self, move |connection| {
            let rows = turn_rows(
                connection,
                SELECT_TURNS_BEFORE,
                params![conversation, before_position, i64::from(limit) + 1],
            )?;
            let has_earlier = rows.len() > limit as usize;
            let mut turns = Vec::with_capacity(rows.len());
            for row in rows.into_iter().take(limit as usize) {
                turns.push(turn_view(connection, &row)?);
            }
            turns.reverse();
            Ok((turns, has_earlier))
        })
        .await
    }

    /// The events after `after_seq` that subscribers receive, or
    /// [`EventsAfter::UseSnapshot`] when more than `max_events` came after it
    /// or the cursor is beyond the latest event.
    pub async fn events_after(
        &self,
        conversation_id: &ConversationId,
        after_seq: u32,
        max_events: u32,
    ) -> Result<EventsAfter, RuntimeStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        read_consistently(self, move |connection| {
            let Some(last_seq) = last_seq(connection, &conversation)? else {
                return Ok(EventsAfter::NotFound);
            };
            let is_ahead = after_seq > last_seq;
            let is_far_behind = last_seq - after_seq.min(last_seq) > max_events;
            if is_ahead || is_far_behind {
                return Ok(EventsAfter::UseSnapshot);
            }
            let stored: Vec<StoredRow> = {
                let mut select = connection.prepare_cached(SELECT_EVENTS_AFTER)?;
                let rows = select.query_map(params![conversation, after_seq], |row| {
                    Ok(StoredRow {
                        seq: row.get(0)?,
                        kind: row.get(1)?,
                        part_id: row.get(2)?,
                        body: row.get(3)?,
                    })
                })?;
                rows.collect::<Result<_, _>>()?
            };
            let mut events = Vec::with_capacity(stored.len());
            for row in &stored {
                if let Some(event) = rows::hydrate(connection, &conversation, row)? {
                    events.push((position(row.seq, "a sequence number")?, event));
                }
            }
            Ok(EventsAfter::Events { events, last_seq })
        })
        .await
    }

    /// Every conversation, the most recent first.
    pub async fn list_conversations(&self) -> Result<Vec<ConversationSummary>, RuntimeStoreError> {
        read_consistently(self, |connection| {
            let raw: Vec<rows::RawSummary> = {
                let mut select = connection.prepare_cached(SELECT_ALL_SUMMARIES)?;
                let rows = select.query_map([], rows::summary_from_row)?;
                rows.collect::<Result<_, _>>()?
            };
            raw.into_iter()
                .map(rows::RawSummary::into_summary)
                .collect()
        })
        .await
    }

    /// Every conversation and every project, as one read sees them, for a
    /// new subscriber of the conversation list.
    pub async fn conversation_list(&self) -> Result<ConversationList, RuntimeStoreError> {
        read_consistently(self, |connection| {
            let raw: Vec<rows::RawSummary> = {
                let mut select = connection.prepare_cached(SELECT_ALL_SUMMARIES)?;
                let rows = select.query_map([], rows::summary_from_row)?;
                rows.collect::<Result<_, _>>()?
            };
            let conversations = raw
                .into_iter()
                .map(rows::RawSummary::into_summary)
                .collect::<Result<_, _>>()?;
            let projects = projects::projects_in(connection)?;
            Ok(ConversationList {
                conversations,
                projects,
            })
        })
        .await
    }

    /// One conversation as the list shows it.
    pub async fn conversation_summary(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Option<ConversationSummary>, RuntimeStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        read_consistently(self, move |connection| {
            rows::conversation_summary(connection, &conversation)
        })
        .await
    }

    /// A conversation's checkpoints, the newest first.
    pub async fn checkpoints(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<StoredCheckpoint>, RuntimeStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        read_consistently(self, move |connection| {
            let raw: Vec<(String, i64, String, String, i64)> = {
                let mut select = connection.prepare_cached(SELECT_CHECKPOINTS)?;
                let rows = select.query_map([conversation], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                })?;
                rows.collect::<Result<_, _>>()?
            };
            raw.into_iter()
                .map(|(id, covers, content_hash, summary, created_at)| {
                    Ok(StoredCheckpoint {
                        id: parse_id(&id, CheckpointId::parse)?,
                        covers_through_turn: position(covers, "a checkpoint's turn")?,
                        content_hash,
                        summary,
                        created_at: millis(created_at),
                    })
                })
                .collect()
        })
        .await
    }

    /// A checkpoint with its summary, for the owner to read.
    pub async fn checkpoint(
        &self,
        conversation_id: &ConversationId,
        checkpoint_id: &CheckpointId,
    ) -> Result<Option<Checkpoint>, RuntimeStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        let checkpoint = checkpoint_id.as_str().to_owned();
        read_consistently(self, move |connection| {
            let summary: Option<String> = connection
                .prepare_cached(SELECT_CHECKPOINT_SUMMARY)?
                .query_row([&checkpoint, &conversation], |row| row.get(0))
                .optional()?;
            let Some(summary) = summary else {
                return Ok(None);
            };
            let view = rows::checkpoint_view(connection, &checkpoint)?;
            Ok(view.map(|view| Checkpoint { view, summary }))
        })
        .await
    }

    /// The turns after `after_position`, or every turn for `None`, as the
    /// model reads them.
    pub async fn model_turns(
        &self,
        conversation_id: &ConversationId,
        after_position: Option<u32>,
    ) -> Result<Vec<ModelTurn>, RuntimeStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        let after = after_position.map_or(-1, i64::from);
        read_consistently(self, move |connection| {
            let turns: Vec<(String, i64, Option<String>, Option<String>)> = {
                let mut select = connection.prepare_cached(SELECT_MODEL_TURNS)?;
                let rows = select.query_map(params![conversation, after], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?;
                rows.collect::<Result<_, _>>()?
            };
            let mut model_turns = Vec::with_capacity(turns.len());
            for (turn_id, position, current_run, run_state) in turns {
                let parts = model_parts(connection, &turn_id, current_run.as_deref())?;
                let kept_runs = kept_runs_of_turn(connection, &turn_id, current_run.as_deref())?;
                model_turns.push(ModelTurn {
                    turn_id: parse_id(&turn_id, TurnId::parse)?,
                    position: rows::position(position, "a turn's position")?,
                    run_id: current_run
                        .as_deref()
                        .map(|run| parse_id(run, RunId::parse))
                        .transpose()?,
                    run_state: run_state.as_deref().and_then(RunState::parse),
                    kept_runs,
                    parts,
                });
            }
            Ok(model_turns)
        })
        .await
    }

    /// The latest turn of a conversation and the state of its current run.
    pub async fn latest_turn(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Option<(TurnId, Option<RunState>)>, RuntimeStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        read_consistently(self, move |connection| {
            let latest: Option<(String, Option<String>)> = connection
                .prepare_cached(SELECT_LATEST_TURN)?
                .query_row([conversation], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()?;
            latest
                .map(|(turn, state)| {
                    let state = state.as_deref().and_then(RunState::parse);
                    Ok((parse_id(&turn, TurnId::parse)?, state))
                })
                .transpose()
        })
        .await
    }

    /// The runtime's context ledger of a conversation.
    pub async fn context_ledger(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Option<String>, RuntimeStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        read_consistently(self, move |connection| {
            Ok(connection
                .prepare_cached(SELECT_CONTEXT_LEDGER)?
                .query_row([conversation], |row| row.get(0))
                .optional()?)
        })
        .await
    }

    /// How many runs are active in every conversation together.
    pub async fn active_run_count(&self) -> Result<u32, RuntimeStoreError> {
        read_consistently(self, |connection| {
            let count: i64 = connection
                .prepare_cached(SELECT_ACTIVE_RUN_COUNT)?
                .query_row([], |row| row.get(0))?;
            position(count, "the number of active runs")
        })
        .await
    }

    /// The owner's message of a turn, as the owner wrote it. The model never
    /// writes it, unlike a checkpoint's summary.
    pub async fn turn_request(
        &self,
        turn_id: &TurnId,
    ) -> Result<Option<String>, RuntimeStoreError> {
        let turn = turn_id.as_str().to_owned();
        let text = self
            .read(move |connection| {
                connection
                    .prepare_cached(SELECT_TURN_REQUEST)?
                    .query_row([turn], |row| row.get(0))
                    .optional()
            })
            .await?;
        Ok(text)
    }

    /// How many tool calls of its turn come before a part, in every run of
    /// the turn.
    pub async fn tool_calls_before(&self, part_id: &PartId) -> Result<u32, RuntimeStoreError> {
        let part = part_id.as_str().to_owned();
        let count: i64 = self
            .read(move |connection| {
                connection
                    .prepare_cached(SELECT_TOOL_CALLS_BEFORE)?
                    .query_row([part], |row| row.get(0))
            })
            .await?;
        position(count, "the number of tool calls")
    }

    /// Deletes a conversation with everything that belongs to it. Returns
    /// whether it existed. The writer's `secure_delete` overwrites the freed
    /// pages, and emptying the write-ahead log removes the old pages from it
    /// too (security architecture, local data). The deletions of the task
    /// conversations that executors serve wait for their executors.
    pub async fn delete_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<bool, RuntimeStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        let now = self.clock().now().as_millis();
        let deleted = self
            .write(crate::Durability::Full, move |transaction| {
                super::tasks::queue_outbox_of_children(transaction, &conversation, now)?;
                transaction.execute(DELETE_CONVERSATION, [conversation])
            })
            .await?;
        if deleted > 0 {
            self.flush().await?;
        }
        Ok(deleted > 0)
    }
}

/// Runs `work` in one read transaction, so that everything it reads belongs
/// to the same state of the database.
async fn read_consistently<T, F>(store: &Store, work: F) -> Result<T, RuntimeStoreError>
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> Result<T, RuntimeStoreError> + Send + 'static,
{
    store
        .read(move |connection| {
            let transaction = connection.unchecked_transaction()?;
            let result = work(&transaction);
            // A read-only transaction has nothing to commit; ending it either
            // way releases its snapshot.
            transaction.finish()?;
            Ok(result)
        })
        .await?
}

fn last_seq(connection: &Connection, conversation: &str) -> Result<Option<u32>, RuntimeStoreError> {
    let last: Option<i64> = connection
        .prepare_cached(SELECT_LAST_SEQ)?
        .query_row([conversation], |row| row.get(0))
        .optional()?;
    last.map(|seq| position(seq, "the last sequence number"))
        .transpose()
}

fn snapshot_in(
    connection: &Connection,
    conversation: &str,
    max_turns: u32,
    max_parts: u32,
) -> Result<Option<ConversationSnapshot>, RuntimeStoreError> {
    let Some(summary) = rows::conversation_summary(connection, conversation)? else {
        return Ok(None);
    };
    let through_seq = last_seq(connection, conversation)?.unwrap_or_default();
    let rows = turn_rows(
        connection,
        SELECT_LATEST_TURNS,
        params![conversation, i64::from(max_turns) + 1],
    )?;
    let mut has_earlier_turns = rows.len() > max_turns as usize;
    let mut turns = Vec::new();
    let mut parts_so_far = 0_usize;
    for row in rows.into_iter().take(max_turns as usize) {
        let turn = turn_view(connection, &row)?;
        let exceeds = parts_so_far + turn.parts.len() > max_parts as usize;
        // The newest turn is always included, however many parts it has.
        if exceeds && !turns.is_empty() {
            has_earlier_turns = true;
            break;
        }
        parts_so_far += turn.parts.len();
        turns.push(turn);
    }
    turns.reverse();
    let checkpoint_id: Option<String> = connection
        .prepare_cached(SELECT_LATEST_CHECKPOINT)?
        .query_row([conversation], |row| row.get(0))
        .optional()?;
    let checkpoint = match checkpoint_id {
        Some(id) => rows::checkpoint_view(connection, &id)?,
        None => None,
    };
    let usage: Option<String> = connection
        .prepare_cached(SELECT_CONTEXT_USAGE)?
        .query_row([conversation], |row| row.get(0))?;
    let context = usage
        .as_deref()
        .map(|text| from_json::<ContextUsage>(text, "context usage"))
        .transpose()?;
    Ok(Some(ConversationSnapshot {
        conversation: summary,
        through_seq,
        turns,
        has_earlier_turns,
        checkpoint,
        context,
    }))
}

fn turn_rows(
    connection: &Connection,
    sql: &str,
    parameters: impl rusqlite::Params,
) -> Result<Vec<rows::TurnRow>, RuntimeStoreError> {
    let mut select = connection.prepare_cached(sql)?;
    let rows = select.query_map(parameters, turn_from_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// A turn with its current run and the parts the owner sees.
fn turn_view(connection: &Connection, row: &rows::TurnRow) -> Result<TurnView, RuntimeStoreError> {
    let run = match row.current_run_id.as_deref() {
        Some(run_id) => run_view(connection, run_id)?,
        None => None,
    };
    let part_rows: Vec<rows::PartRow> = {
        let mut select = connection.prepare_cached(SELECT_TURN_PARTS)?;
        let rows = select.query_map(params![row.id, row.current_run_id], part_from_row)?;
        rows.collect::<Result<_, _>>()?
    };
    let mut parts: Vec<PartView> = Vec::with_capacity(part_rows.len());
    for part in &part_rows {
        if let Some(view) = part.view(connection)? {
            parts.push(view);
        }
    }
    row.view(run, parts)
}

/// A part's row as the model's view reads it.
struct ModelPartRow {
    id: String,
    kind: String,
    audience: String,
    state: String,
    text: String,
    binding: Option<String>,
    item: Option<String>,
    run_id: Option<String>,
    document: String,
    /// `NULL` for the owner's message, which no model call wrote.
    attempt: Option<i64>,
}

fn model_parts(
    connection: &Connection,
    turn_id: &str,
    current_run: Option<&str>,
) -> Result<Vec<ModelPart>, RuntimeStoreError> {
    let raw: Vec<ModelPartRow> = {
        let mut select = connection.prepare_cached(SELECT_MODEL_PARTS)?;
        let rows = select.query_map(params![turn_id, current_run], |row| {
            Ok(ModelPartRow {
                id: row.get(0)?,
                kind: row.get(1)?,
                audience: row.get(2)?,
                state: row.get(3)?,
                text: row.get(4)?,
                binding: row.get(5)?,
                item: row.get(6)?,
                run_id: row.get(7)?,
                document: row.get(8)?,
                attempt: row.get(9)?,
            })
        })?;
        rows.collect::<Result<_, _>>()?
    };
    let mut parts = Vec::with_capacity(raw.len());
    for row in raw {
        let for_model = Audience::parse(&row.audience).is_some_and(Audience::includes_model);
        if !for_model {
            continue;
        }
        let state = PartState::parse(&row.state).ok_or_else(|| {
            RuntimeStoreError::invalid(format!("unknown part state {:?}", row.state))
        })?;
        let provider_item = row
            .binding
            .zip(row.item)
            .map(|(binding, item)| ProviderItem { binding, item });
        let kind = PartKind::parse(&row.kind);
        let (tool_call, task) = if kind.is_some_and(PartKind::is_call) {
            let document: PartDocument = from_json(&row.document, "part document")?;
            let task = if kind == Some(PartKind::Task) {
                Some(document.task.unwrap_or_default())
            } else {
                None
            };
            (document.tool_call, task)
        } else {
            (None, None)
        };
        parts.push(ModelPart {
            id: parse_id(&row.id, PartId::parse)?,
            run_id: row
                .run_id
                .as_deref()
                .map(|run| parse_id(run, RunId::parse))
                .transpose()?,
            kind,
            kind_name: row.kind,
            state,
            text: row.text,
            provider_item,
            tool_call,
            task,
            attempt: position(row.attempt.unwrap_or(0), "a part's attempt")?,
        });
    }
    Ok(parts)
}

/// The earlier runs of a turn that it keeps, oldest first, without its
/// current run.
fn kept_runs_of_turn(
    connection: &Connection,
    turn_id: &str,
    current_run: Option<&str>,
) -> Result<Vec<KeptRun>, RuntimeStoreError> {
    let raw: Vec<(String, String)> = {
        let mut select = connection.prepare_cached(SELECT_KEPT_RUNS_OF_TURN)?;
        let rows = select.query_map([turn_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut kept = Vec::with_capacity(raw.len());
    for (run, state) in raw {
        if Some(run.as_str()) == current_run {
            continue;
        }
        let state = RunState::parse(&state)
            .ok_or_else(|| RuntimeStoreError::invalid(format!("unknown run state {state:?}")))?;
        kept.push(KeptRun {
            run_id: parse_id(&run, RunId::parse)?,
            state,
        });
    }
    Ok(kept)
}
