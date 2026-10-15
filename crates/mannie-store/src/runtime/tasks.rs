//! Tasks of child agents (docs/features/child-agents/tasks.md, data): each
//! start or follow-up of a child's work, the task conversation it runs in, and
//! the numbers that name a conversation's children.
//!
//! A task is stored with the turn that works on it, starts running with that
//! turn's run, and ends with it, in the transactions that change the child's
//! conversation. The parent's part that assigned a task also finishes from the
//! task's row when the parent's run ends first (`apply.rs`, ending a call).

use mannie_types::ids::{ConversationId, PartId, TaskId, TurnId};
use mannie_types::tasks::TaskState;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::rows::{from_json, millis, parse_id, position, to_json};
use super::{DOCUMENT_VERSION, NewTask, RuntimeStoreError, StoredChild, StoredTask, TaskEnding};
use crate::Store;

const INSERT_TASK: &str = "INSERT INTO runtime_tasks \
     (id, conversation_id, part_id, turn_id, follow_up, background, state, version, created_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'accepted', ?7, ?8) ON CONFLICT (id) DO NOTHING";
const START_TASK: &str = "UPDATE runtime_tasks SET state = 'running' \
     WHERE conversation_id = ?1 AND state = 'accepted'";
const SELECT_ACTIVE_TASK: &str = "SELECT id FROM runtime_tasks \
     WHERE conversation_id = ?1 AND state IN ('accepted', 'running')";
const END_TASK: &str = "UPDATE runtime_tasks SET state = ?2, version = ?3, ending = ?4, ended_at = ?5 \
     WHERE id = ?1 AND state IN ('accepted', 'running')";
const SELECT_TASK: &str = "SELECT id, conversation_id, part_id, turn_id, follow_up, background, state, \
     ending, created_at, ended_at FROM runtime_tasks WHERE id = ?1";
const SELECT_TASK_OF_PART: &str = "SELECT id, conversation_id, part_id, turn_id, follow_up, background, \
     state, ending, created_at, ended_at FROM runtime_tasks WHERE part_id = ?1";
const SELECT_LATEST_TASK: &str = "SELECT id, conversation_id, part_id, turn_id, follow_up, background, \
     state, ending, created_at, ended_at FROM runtime_tasks WHERE conversation_id = ?1 \
     ORDER BY created_at DESC, id DESC LIMIT 1";
const SELECT_CHILD: &str = "SELECT id, task_number, task_agent, title FROM runtime_conversations \
     WHERE parent_conversation_id = ?1 AND task_number = ?2";
const SELECT_NEXT_TASK_NUMBER: &str = "SELECT COALESCE(MAX(task_number), 0) + 1 \
     FROM runtime_conversations WHERE parent_conversation_id = ?1";
const SELECT_TASK_CONVERSATIONS: &str = "SELECT id FROM runtime_conversations \
     WHERE parent_conversation_id = ?1 ORDER BY task_number";
const SELECT_IS_TASK_CONVERSATION: &str =
    "SELECT parent_conversation_id IS NOT NULL FROM runtime_conversations WHERE id = ?1";

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    INSERT_TASK,
    START_TASK,
    SELECT_ACTIVE_TASK,
    END_TASK,
    SELECT_TASK,
    SELECT_TASK_OF_PART,
    SELECT_LATEST_TASK,
    SELECT_CHILD,
    SELECT_NEXT_TASK_NUMBER,
    SELECT_TASK_CONVERSATIONS,
    SELECT_IS_TASK_CONVERSATION,
];

/// A task's row.
struct TaskRow {
    id: String,
    conversation_id: String,
    part_id: String,
    turn_id: String,
    follow_up: bool,
    background: bool,
    state: String,
    ending: Option<String>,
    created_at: i64,
    ended_at: Option<i64>,
}

fn task_from_row(row: &Row<'_>) -> rusqlite::Result<TaskRow> {
    Ok(TaskRow {
        id: row.get(0)?,
        conversation_id: row.get(1)?,
        part_id: row.get(2)?,
        turn_id: row.get(3)?,
        follow_up: row.get(4)?,
        background: row.get(5)?,
        state: row.get(6)?,
        ending: row.get(7)?,
        created_at: row.get(8)?,
        ended_at: row.get(9)?,
    })
}

impl TaskRow {
    fn into_task(self) -> Result<StoredTask, RuntimeStoreError> {
        let state = TaskState::parse(&self.state).ok_or_else(|| {
            RuntimeStoreError::invalid(format!("unknown task state {:?}", self.state))
        })?;
        let ending = self
            .ending
            .as_deref()
            .map(|text| from_json::<TaskEnding>(text, "task ending"))
            .transpose()?;
        Ok(StoredTask {
            id: parse_id(&self.id, TaskId::parse)?,
            conversation_id: parse_id(&self.conversation_id, ConversationId::parse)?,
            part_id: parse_id(&self.part_id, PartId::parse)?,
            turn_id: parse_id(&self.turn_id, TurnId::parse)?,
            follow_up: self.follow_up,
            background: self.background,
            state,
            ending,
            created_at: millis(self.created_at),
            ended_at: self.ended_at.map(millis),
        })
    }
}

/// Stores a task with its turn. A task with the same ID already stored stays
/// as it is.
pub(super) fn insert(
    connection: &Connection,
    conversation: &str,
    turn_id: &str,
    task: &NewTask,
    now: i64,
) -> Result<(), RuntimeStoreError> {
    connection.prepare_cached(INSERT_TASK)?.execute(params![
        task.id.as_str(),
        conversation,
        task.part_id.as_str(),
        turn_id,
        task.follow_up,
        task.background,
        DOCUMENT_VERSION,
        now
    ])?;
    Ok(())
}

/// The conversation's accepted task starts running, with its turn's run.
pub(super) fn start(connection: &Connection, conversation: &str) -> Result<(), RuntimeStoreError> {
    connection
        .prepare_cached(START_TASK)?
        .execute([conversation])?;
    Ok(())
}

/// Ends the conversation's active task, if it has one.
pub(super) fn end_active(
    connection: &Connection,
    conversation: &str,
    ending: &TaskEnding,
    now: i64,
) -> Result<(), RuntimeStoreError> {
    let active: Option<String> = connection
        .prepare_cached(SELECT_ACTIVE_TASK)?
        .query_row([conversation], |row| row.get(0))
        .optional()?;
    if let Some(task) = active {
        end(connection, &task, ending, now)?;
    }
    Ok(())
}

/// Ends a task that has not ended yet. A task that ended keeps its first
/// ending, whichever side stored it first.
pub(super) fn end(
    connection: &Connection,
    task: &str,
    ending: &TaskEnding,
    now: i64,
) -> Result<(), RuntimeStoreError> {
    if ending.state.is_active() {
        return Err(RuntimeStoreError::invalid(
            "a task cannot end in an active state",
        ));
    }
    connection.prepare_cached(END_TASK)?.execute(params![
        task,
        ending.state.as_str(),
        DOCUMENT_VERSION,
        to_json(ending)?,
        now
    ])?;
    Ok(())
}

/// The task that a part of a parent conversation assigned.
pub(super) fn of_part(
    connection: &Connection,
    part_id: &str,
) -> Result<Option<StoredTask>, RuntimeStoreError> {
    connection
        .prepare_cached(SELECT_TASK_OF_PART)?
        .query_row([part_id], task_from_row)
        .optional()?
        .map(TaskRow::into_task)
        .transpose()
}

/// Whether a conversation is a task conversation.
pub(super) fn is_task_conversation(
    connection: &Connection,
    conversation: &str,
) -> Result<bool, RuntimeStoreError> {
    let is_task: Option<bool> = connection
        .prepare_cached(SELECT_IS_TASK_CONVERSATION)?
        .query_row([conversation], |row| row.get(0))
        .optional()?;
    Ok(is_task.unwrap_or(false))
}

/// The task conversations of a parent, by their numbers.
pub(super) fn task_conversations_in(
    connection: &Connection,
    parent: &str,
) -> Result<Vec<ConversationId>, RuntimeStoreError> {
    let ids: Vec<String> = {
        let mut select = connection.prepare_cached(SELECT_TASK_CONVERSATIONS)?;
        let rows = select.query_map([parent], |row| row.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    ids.iter()
        .map(|id| parse_id(id, ConversationId::parse))
        .collect()
}

impl Store {
    /// A task.
    pub async fn task(&self, task_id: &TaskId) -> Result<Option<StoredTask>, RuntimeStoreError> {
        let task = task_id.as_str().to_owned();
        self.read(move |connection| {
            Ok(connection
                .prepare_cached(SELECT_TASK)?
                .query_row([task], task_from_row)
                .optional()?
                .map(TaskRow::into_task)
                .transpose())
        })
        .await?
    }

    /// The task that a part of a parent conversation assigned.
    pub async fn task_of_part(
        &self,
        part_id: &PartId,
    ) -> Result<Option<StoredTask>, RuntimeStoreError> {
        let part = part_id.as_str().to_owned();
        self.read(move |connection| Ok(of_part(connection, &part)))
            .await?
    }

    /// The child of a conversation with a number, with its latest task.
    pub async fn child_by_number(
        &self,
        parent_id: &ConversationId,
        number: u32,
    ) -> Result<Option<StoredChild>, RuntimeStoreError> {
        let parent = parent_id.as_str().to_owned();
        self.read(move |connection| Ok(child_in(connection, &parent, number)))
            .await?
    }

    /// The number of a parent conversation's next child.
    pub async fn next_task_number(
        &self,
        parent_id: &ConversationId,
    ) -> Result<u32, RuntimeStoreError> {
        let parent = parent_id.as_str().to_owned();
        let next: i64 = self
            .read(move |connection| {
                connection
                    .prepare_cached(SELECT_NEXT_TASK_NUMBER)?
                    .query_row([parent], |row| row.get(0))
            })
            .await?;
        position(next, "a task number")
    }

    /// The task conversations of a parent conversation, by their numbers.
    pub async fn task_conversations(
        &self,
        parent_id: &ConversationId,
    ) -> Result<Vec<ConversationId>, RuntimeStoreError> {
        let parent = parent_id.as_str().to_owned();
        self.read(move |connection| Ok(task_conversations_in(connection, &parent)))
            .await?
    }
}

/// The child of a parent with a number, with its latest task.
fn child_in(
    connection: &Connection,
    parent: &str,
    number: u32,
) -> Result<Option<StoredChild>, RuntimeStoreError> {
    let child: Option<(String, i64, Option<String>, String)> = connection
        .prepare_cached(SELECT_CHILD)?
        .query_row(params![parent, number], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .optional()?;
    let Some((id, number, agent, title)) = child else {
        return Ok(None);
    };
    let latest = connection
        .prepare_cached(SELECT_LATEST_TASK)?
        .query_row([&id], task_from_row)
        .optional()?
        .map(TaskRow::into_task)
        .transpose()?;
    Ok(Some(StoredChild {
        conversation_id: parse_id(&id, ConversationId::parse)?,
        number: position(number, "a task number")?,
        agent: agent.unwrap_or_default(),
        title,
        latest,
    }))
}
