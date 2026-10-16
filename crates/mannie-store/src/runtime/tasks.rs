//! Tasks (docs/features/child-agents/tasks.md, data; docs/features/workers/
//! delegation.md, data): each start or follow-up of a child's work or of a
//! worker's, the task conversation it runs in, and the numbers that name a
//! conversation's children.
//!
//! A child's task is stored with the turn that works on it, starts running
//! with that turn's run, and ends with it, in the transactions that change the
//! child's conversation. The parent's part that assigned a task also finishes
//! from the task's row when the parent's run ends first (`apply.rs`, ending a
//! call).
//!
//! On the master, a task delegated to a worker has a row without a turn: the
//! states that the worker publishes change it, and it outlives its part when
//! the master lets it go. On a worker, a task that the master assigned has a
//! row without a part, and every change of its state takes the worker's next
//! state version, which the master follows.

use mannie_types::ids::{ConversationId, PartId, ProfileId, TaskId, TurnId, WorkerId};
use mannie_types::tasks::{TaskState, TaskWorker};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::rows::{from_json, millis, parse_id, position, to_json};
use super::{
    DOCUMENT_VERSION, NewConversation, NewTask, RuntimeStoreError, StoredChild, StoredTask,
    TaskEnding, apply,
};
use crate::writer::TransactError;
use crate::{Durability, Store};

macro_rules! task_columns {
    () => {
        "id, conversation_id, part_id, turn_id, follow_up, background, state, ending, created_at, \
         ended_at, worker_id, remote_version, submission, cancel_requested_at, assignment, \
         state_version, profile_id"
    };
}

// The next state version of a worker's assigned tasks (docs/features/workers/
// delegation.md, following tasks). Every change of an assigned task's state
// takes one, so the master can follow the changes from the version it stored
// last; the versions are stored, so they keep growing when the worker
// restarts. Other tasks keep version 0.
macro_rules! next_state_version {
    () => {
        "(SELECT COALESCE(MAX(v.state_version), 0) + 1 FROM runtime_tasks v \
         WHERE v.assignment IS NOT NULL)"
    };
}

const INSERT_TASK: &str = concat!(
    "INSERT INTO runtime_tasks \
     (id, conversation_id, part_id, turn_id, follow_up, background, state, version, created_at, \
     assignment, state_version) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'accepted', ?7, ?8, ?9, CASE WHEN ?9 IS NULL THEN 0 ELSE ",
    next_state_version!(),
    " END) ON CONFLICT (id) DO NOTHING"
);
const START_TASK: &str = concat!(
    "UPDATE runtime_tasks SET state = 'running', \
     state_version = CASE WHEN assignment IS NULL THEN state_version ELSE ",
    next_state_version!(),
    " END WHERE conversation_id = ?1 AND state = 'accepted'"
);
const SELECT_ACTIVE_TASK: &str = "SELECT id FROM runtime_tasks \
     WHERE conversation_id = ?1 AND state IN ('accepted', 'running')";
const END_TASK: &str = concat!(
    "UPDATE runtime_tasks SET state = ?2, version = ?3, ending = ?4, ended_at = ?5, \
     state_version = CASE WHEN assignment IS NULL THEN state_version ELSE ",
    next_state_version!(),
    " END WHERE id = ?1 AND state IN ('accepted', 'running')"
);
const SELECT_TASK: &str = concat!(
    "SELECT ",
    task_columns!(),
    " FROM runtime_tasks WHERE id = ?1"
);
const SELECT_TASK_OF_PART: &str = concat!(
    "SELECT ",
    task_columns!(),
    " FROM runtime_tasks WHERE part_id = ?1"
);
const SELECT_LATEST_TASK: &str = concat!(
    "SELECT ",
    task_columns!(),
    " FROM runtime_tasks WHERE conversation_id = ?1 ORDER BY created_at DESC, id DESC LIMIT 1"
);
const SELECT_CHILD: &str = "SELECT id, task_number, task_agent, title, task_worker_id, \
     task_worker_name FROM runtime_conversations \
     WHERE parent_conversation_id = ?1 AND task_number = ?2";
const SELECT_NEXT_TASK_NUMBER: &str = "SELECT COALESCE(MAX(task_number), 0) + 1 \
     FROM runtime_conversations WHERE parent_conversation_id = ?1";
const SELECT_TASK_CONVERSATIONS: &str = "SELECT id FROM runtime_conversations \
     WHERE parent_conversation_id = ?1 ORDER BY task_number";
const SELECT_IS_TASK_CONVERSATION: &str =
    "SELECT parent_conversation_id IS NOT NULL FROM runtime_conversations WHERE id = ?1";
const INSERT_DELEGATED_TASK: &str = "INSERT INTO runtime_tasks \
     (id, conversation_id, part_id, follow_up, background, state, version, created_at, worker_id, \
     profile_id, submission) VALUES (?1, ?2, ?3, ?4, ?5, 'accepted', ?6, ?7, ?8, ?9, 'pending')";
const SET_SUBMISSION: &str =
    "UPDATE runtime_tasks SET submission = ?2 WHERE id = ?1 AND worker_id IS NOT NULL";
const UPDATE_REMOTE_STATE: &str = "UPDATE runtime_tasks SET state = ?2, remote_version = ?3 \
     WHERE id = ?1 AND worker_id IS NOT NULL AND state IN ('accepted', 'running') \
     AND (remote_version IS NULL OR remote_version < ?3)";
const END_REMOTE_TASK: &str = "UPDATE runtime_tasks SET state = ?2, version = ?3, ending = ?4, \
     ended_at = ?5, remote_version = MAX(COALESCE(remote_version, 0), ?6) \
     WHERE id = ?1 AND worker_id IS NOT NULL AND state IN ('accepted', 'running')";
const REQUEST_CANCEL: &str = "UPDATE runtime_tasks \
     SET cancel_requested_at = COALESCE(cancel_requested_at, ?2) \
     WHERE id = ?1 AND worker_id IS NOT NULL AND state IN ('accepted', 'running')";
const SELECT_ACTIVE_DELEGATED_TASKS: &str = concat!(
    "SELECT ",
    task_columns!(),
    " FROM runtime_tasks WHERE worker_id IS NOT NULL AND state IN ('accepted', 'running') \
     ORDER BY created_at, id"
);

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
    INSERT_DELEGATED_TASK,
    SET_SUBMISSION,
    UPDATE_REMOTE_STATE,
    END_REMOTE_TASK,
    REQUEST_CANCEL,
    SELECT_ACTIVE_DELEGATED_TASKS,
];

/// Whether the worker stored a delegated task (docs/features/workers/
/// delegation.md, submitting a task).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submission {
    /// The master stored the task and has not learned that the worker stored
    /// it. The master asks the worker before it submits the task again.
    Pending,
    /// The worker stored the task.
    Accepted,
}

impl Submission {
    /// The stored name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "pending" => Some(Self::Pending),
            "accepted" => Some(Self::Accepted),
            _ => None,
        }
    }
}

/// A task that the master delegates to a worker, as the parent's run stores
/// it after its part and before it submits the task (docs/features/workers/
/// delegation.md, submitting a task).
#[derive(Debug, Clone)]
pub struct NewDelegatedTask {
    /// The task, as the parent's run chose it.
    pub id: TaskId,
    /// The task's conversation.
    pub conversation_id: ConversationId,
    /// The conversation to create for a new task, with the parent, the
    /// number and the worker in its origin, or `None` for a follow-up, whose
    /// conversation exists.
    pub conversation: Option<NewConversation>,
    /// The parent's part that assigned the task.
    pub part_id: PartId,
    /// The worker that runs it.
    pub worker_id: WorkerId,
    /// The provider profile that the task uses, whose credential the master
    /// leases to the worker.
    pub profile_id: ProfileId,
    /// Whether the task continues an earlier task of the same conversation.
    pub follow_up: bool,
    /// Whether the parent continues while the worker works.
    pub background: bool,
}

/// A task's row.
struct TaskRow {
    id: String,
    conversation_id: String,
    part_id: Option<String>,
    turn_id: Option<String>,
    follow_up: bool,
    background: bool,
    state: String,
    ending: Option<String>,
    created_at: i64,
    ended_at: Option<i64>,
    worker_id: Option<String>,
    remote_version: Option<i64>,
    submission: Option<String>,
    cancel_requested_at: Option<i64>,
    assignment: Option<String>,
    state_version: i64,
    profile_id: Option<String>,
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
        worker_id: row.get(10)?,
        remote_version: row.get(11)?,
        submission: row.get(12)?,
        cancel_requested_at: row.get(13)?,
        assignment: row.get(14)?,
        state_version: row.get(15)?,
        profile_id: row.get(16)?,
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
        let submission = self
            .submission
            .as_deref()
            .map(|text| {
                Submission::parse(text).ok_or_else(|| {
                    RuntimeStoreError::invalid(format!("unknown submission {text:?}"))
                })
            })
            .transpose()?;
        Ok(StoredTask {
            id: parse_id(&self.id, TaskId::parse)?,
            conversation_id: parse_id(&self.conversation_id, ConversationId::parse)?,
            part_id: self
                .part_id
                .as_deref()
                .map(|id| parse_id(id, PartId::parse))
                .transpose()?,
            turn_id: self
                .turn_id
                .as_deref()
                .map(|id| parse_id(id, TurnId::parse))
                .transpose()?,
            follow_up: self.follow_up,
            background: self.background,
            state,
            ending,
            created_at: millis(self.created_at),
            ended_at: self.ended_at.map(millis),
            worker_id: self
                .worker_id
                .as_deref()
                .map(|id| parse_id(id, WorkerId::parse))
                .transpose()?,
            remote_version: self
                .remote_version
                .map(|version| version_from(version, "a remote version"))
                .transpose()?,
            submission,
            cancel_requested_at: self.cancel_requested_at.map(millis),
            assignment: self.assignment,
            state_version: version_from(self.state_version, "a state version")?,
            profile_id: self
                .profile_id
                .as_deref()
                .map(|id| parse_id(id, ProfileId::parse))
                .transpose()?,
        })
    }
}

fn version_from(value: i64, what: &str) -> Result<u64, RuntimeStoreError> {
    u64::try_from(value)
        .map_err(|error| RuntimeStoreError::invalid(format!("{what} is negative: {error}")))
}

fn version_to(value: u64, what: &str) -> Result<i64, RuntimeStoreError> {
    i64::try_from(value)
        .map_err(|error| RuntimeStoreError::invalid(format!("{what} is too large: {error}")))
}

fn select_task(
    connection: &Connection,
    task: &str,
) -> Result<Option<StoredTask>, RuntimeStoreError> {
    connection
        .prepare_cached(SELECT_TASK)?
        .query_row([task], task_from_row)
        .optional()?
        .map(TaskRow::into_task)
        .transpose()
}

fn select_tasks(
    connection: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<StoredTask>, RuntimeStoreError> {
    let rows: Vec<TaskRow> = {
        let mut select = connection.prepare_cached(sql)?;
        let rows = select.query_map(params, task_from_row)?;
        rows.collect::<Result<_, _>>()?
    };
    rows.into_iter().map(TaskRow::into_task).collect()
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
    if task.part_id.is_none() && task.assignment.is_none() {
        return Err(RuntimeStoreError::invalid(
            "a task needs the part or the assignment that started it",
        ));
    }
    connection.prepare_cached(INSERT_TASK)?.execute(params![
        task.id.as_str(),
        conversation,
        task.part_id.as_ref().map(PartId::as_str),
        turn_id,
        task.follow_up,
        task.background,
        DOCUMENT_VERSION,
        now,
        task.assignment.as_deref()
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

/// Stores a delegated task and, for a new task, its conversation. A task
/// with the same ID already stored stays as it is and is returned.
fn record_delegated(
    connection: &Connection,
    task: &NewDelegatedTask,
    now: i64,
) -> Result<StoredTask, RuntimeStoreError> {
    if let Some(stored) = select_task(connection, task.id.as_str())? {
        return Ok(stored);
    }
    let conversation = task.conversation_id.as_str();
    if let Some(new_conversation) = &task.conversation {
        let is_worker_conversation = new_conversation.id == task.conversation_id
            && new_conversation.project_id.is_none()
            && new_conversation
                .task
                .as_ref()
                .and_then(|origin| origin.worker.as_ref())
                .is_some_and(|worker| worker.id == task.worker_id);
        if !is_worker_conversation {
            return Err(RuntimeStoreError::invalid(
                "a delegated task's conversation must name its worker",
            ));
        }
        apply::insert_conversation(connection, conversation, new_conversation, now)?;
    }
    let active: Option<String> = connection
        .prepare_cached(SELECT_ACTIVE_TASK)?
        .query_row([conversation], |row| row.get(0))
        .optional()?;
    if active.is_some() {
        return Err(RuntimeStoreError::TaskActive);
    }
    connection
        .prepare_cached(INSERT_DELEGATED_TASK)?
        .execute(params![
            task.id.as_str(),
            conversation,
            task.part_id.as_str(),
            task.follow_up,
            task.background,
            DOCUMENT_VERSION,
            now,
            task.worker_id.as_str(),
            task.profile_id.as_str()
        ])?;
    select_task(connection, task.id.as_str())?
        .ok_or_else(|| RuntimeStoreError::invalid("a delegated task was not stored"))
}

fn rejected(error: TransactError<RuntimeStoreError>) -> RuntimeStoreError {
    match error {
        TransactError::Rejected(error) => error,
        TransactError::Store(error) => RuntimeStoreError::Store(error),
    }
}

impl Store {
    /// A task.
    pub async fn task(&self, task_id: &TaskId) -> Result<Option<StoredTask>, RuntimeStoreError> {
        let task = task_id.as_str().to_owned();
        self.read(move |connection| Ok(select_task(connection, &task)))
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

    /// The latest task of a conversation.
    pub async fn latest_task(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Option<StoredTask>, RuntimeStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        self.read(move |connection| {
            Ok(connection
                .prepare_cached(SELECT_LATEST_TASK)?
                .query_row([conversation], task_from_row)
                .optional()?
                .map(TaskRow::into_task)
                .transpose())
        })
        .await?
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

    /// Stores a task that the master delegates to a worker, with its
    /// conversation for a new task, before the master submits it, and
    /// returns it. The submission starts as pending. Storing a task again
    /// returns the stored task and changes nothing.
    pub async fn record_delegated_task(
        &self,
        task: NewDelegatedTask,
    ) -> Result<StoredTask, RuntimeStoreError> {
        let now = self.clock().now().as_millis();
        self.transact(Durability::Full, move |transaction| {
            record_delegated(transaction, &task, now).map_err(TransactError::Rejected)
        })
        .await
        .map_err(rejected)
    }

    /// Records whether the worker stored a delegated task. Returns whether
    /// the task exists.
    pub async fn set_task_submission(
        &self,
        task_id: &TaskId,
        submission: Submission,
    ) -> Result<bool, RuntimeStoreError> {
        let task = task_id.as_str().to_owned();
        let changed = self
            .write(Durability::Full, move |transaction| {
                transaction
                    .prepare_cached(SET_SUBMISSION)?
                    .execute(params![task, submission.as_str()])
            })
            .await?;
        Ok(changed > 0)
    }

    /// Stores a state of a delegated task that its worker published. An
    /// active state changes the row when its version is newer than the one
    /// stored; an ending ends the task once, whichever version brings it.
    /// Returns whether the row changed.
    pub async fn apply_remote_task_state(
        &self,
        task_id: &TaskId,
        version: u64,
        state: TaskState,
        ending: Option<TaskEnding>,
    ) -> Result<bool, RuntimeStoreError> {
        let task = task_id.as_str().to_owned();
        let version = version_to(version, "a remote version")?;
        let now = self.clock().now().as_millis();
        let changed = self
            .transact(Durability::Full, move |transaction| {
                let changed = if state.is_active() {
                    transaction
                        .prepare_cached(UPDATE_REMOTE_STATE)?
                        .execute(params![task, state.as_str(), version])?
                } else {
                    let ending = ending.unwrap_or(TaskEnding {
                        state,
                        report: None,
                        failure: None,
                    });
                    if ending.state != state {
                        return Err(TransactError::Rejected(RuntimeStoreError::invalid(
                            "a task's ending does not match its state",
                        )));
                    }
                    let ending = to_json(&ending).map_err(TransactError::Rejected)?;
                    transaction
                        .prepare_cached(END_REMOTE_TASK)?
                        .execute(params![
                            task,
                            state.as_str(),
                            DOCUMENT_VERSION,
                            ending,
                            now,
                            version
                        ])?
                };
                Ok(changed > 0)
            })
            .await
            .map_err(rejected)?;
        Ok(changed)
    }

    /// Records that the owner or the parent's run asked to cancel a delegated
    /// task, so the cancel is sent again until the worker ends the task.
    /// Returns whether the task is active.
    pub async fn request_task_cancel(&self, task_id: &TaskId) -> Result<bool, RuntimeStoreError> {
        let task = task_id.as_str().to_owned();
        let now = self.clock().now().as_millis();
        let changed = self
            .write(Durability::Full, move |transaction| {
                transaction
                    .prepare_cached(REQUEST_CANCEL)?
                    .execute(params![task, now])
            })
            .await?;
        Ok(changed > 0)
    }

    /// The delegated tasks that have not ended, the oldest first.
    pub async fn active_delegated_tasks(&self) -> Result<Vec<StoredTask>, RuntimeStoreError> {
        self.read(|connection| {
            Ok(select_tasks(
                connection,
                SELECT_ACTIVE_DELEGATED_TASKS,
                params![],
            ))
        })
        .await?
    }
}

/// The child of a parent with a number, with its latest task.
fn child_in(
    connection: &Connection,
    parent: &str,
    number: u32,
) -> Result<Option<StoredChild>, RuntimeStoreError> {
    let child: Option<ChildRow> = connection
        .prepare_cached(SELECT_CHILD)?
        .query_row(params![parent, number], |row| {
            Ok(ChildRow {
                id: row.get(0)?,
                number: row.get(1)?,
                agent: row.get(2)?,
                title: row.get(3)?,
                worker_id: row.get(4)?,
                worker_name: row.get(5)?,
            })
        })
        .optional()?;
    let Some(child) = child else {
        return Ok(None);
    };
    let worker = match child.worker_id.as_deref() {
        Some(worker_id) => Some(TaskWorker {
            id: parse_id(worker_id, WorkerId::parse)?,
            name: child.worker_name.unwrap_or_default(),
        }),
        None => None,
    };
    let id = child.id;
    let latest = connection
        .prepare_cached(SELECT_LATEST_TASK)?
        .query_row([&id], task_from_row)
        .optional()?
        .map(TaskRow::into_task)
        .transpose()?;
    Ok(Some(StoredChild {
        conversation_id: parse_id(&id, ConversationId::parse)?,
        number: position(child.number, "a task number")?,
        agent: child.agent.unwrap_or_default(),
        title: child.title,
        worker,
        latest,
    }))
}

/// A child's conversation row.
struct ChildRow {
    id: String,
    number: i64,
    agent: Option<String>,
    title: String,
    worker_id: Option<String>,
    worker_name: Option<String>,
}
