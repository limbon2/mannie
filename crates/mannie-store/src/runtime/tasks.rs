//! Tasks (docs/features/child-agents/tasks.md, data; ADR-0031): each start
//! or follow-up of a child's work or of a task that a plugin's executor runs,
//! the task conversation it runs in, and the numbers that name a
//! conversation's children.
//!
//! A child's task is stored with the turn that works on it, starts running
//! with that turn's run, and ends with it, in the transactions that change the
//! child's conversation. The parent's part that assigned a task also finishes
//! from the task's row when the parent's run ends first (`apply.rs`, ending a
//! call).
//!
//! A task that a plugin's executor runs has a row without a turn: the states
//! that the executor publishes change it, and it outlives its part when the
//! run lets it go. Its row also keeps the executor's own document, which the
//! core never reads. A task that another machine assigned has a row without a
//! part, and every change of its state takes the machine's next state
//! version, which the assigning side follows.

use mannie_types::ids::{ConversationId, PartId, ProfileId, TaskId, TurnId};
use mannie_types::tasks::{TaskExecutorRef, TaskState};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::rows::{from_json, millis, parse_id, position, to_json};
use super::{
    DOCUMENT_VERSION, NewConversation, NewTask, RuntimeStoreError, StoredChild, StoredTask,
    TaskEnding, TaskExecution, apply,
};
use crate::writer::TransactError;
use crate::{Durability, Store};

macro_rules! task_columns {
    () => {
        "id, conversation_id, part_id, turn_id, follow_up, background, state, ending, created_at, \
         ended_at, executor_kind, executor_target, executor_version, executor_document, \
         assignment, state_version"
    };
}

// The next state version of the assigned tasks (ADR-0028, decision 1). Every
// change of an assigned task's state takes one, so the assigning side can
// follow the changes from the version it stored last; the versions are
// stored, so they keep growing when the machine restarts. Other tasks keep
// version 0.
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
const SELECT_CHILD: &str = "SELECT id, task_number, task_agent, title, task_executor_kind, \
     task_executor_target, task_executor_name, task_executor_noun FROM runtime_conversations \
     WHERE parent_conversation_id = ?1 AND task_number = ?2";
const SELECT_NEXT_TASK_NUMBER: &str = "SELECT COALESCE(MAX(task_number), 0) + 1 \
     FROM runtime_conversations WHERE parent_conversation_id = ?1";
const SELECT_TASK_CONVERSATIONS: &str = "SELECT id FROM runtime_conversations \
     WHERE parent_conversation_id = ?1 ORDER BY task_number";
const SELECT_IS_TASK_CONVERSATION: &str =
    "SELECT parent_conversation_id IS NOT NULL FROM runtime_conversations WHERE id = ?1";
// The table's check asks a task without a turn for a worker until a later
// release builds the table again, so an executor's target also goes into
// worker_id (migration 0023).
const INSERT_EXECUTOR_TASK: &str = "INSERT INTO runtime_tasks \
     (id, conversation_id, part_id, follow_up, background, state, version, created_at, \
     executor_kind, executor_target, executor_document, worker_id) \
     VALUES (?1, ?2, ?3, ?4, ?5, 'accepted', ?6, ?7, ?8, ?9, ?10, ?9)";
const SET_EXECUTOR_DOCUMENT: &str =
    "UPDATE runtime_tasks SET executor_document = ?3 WHERE id = ?1 AND executor_kind = ?2";
const UPDATE_EXECUTOR_STATE: &str = "UPDATE runtime_tasks SET state = ?3, executor_version = ?4 \
     WHERE id = ?1 AND executor_kind = ?2 AND state IN ('accepted', 'running') \
     AND executor_version < ?4";
const END_EXECUTOR_TASK: &str = "UPDATE runtime_tasks SET state = ?3, version = ?4, ending = ?5, \
     ended_at = ?6, executor_version = MAX(executor_version, ?7) \
     WHERE id = ?1 AND executor_kind = ?2 AND state IN ('accepted', 'running')";
const SELECT_ACTIVE_EXECUTOR_TASKS: &str = concat!(
    "SELECT ",
    task_columns!(),
    " FROM runtime_tasks WHERE executor_kind = ?1 AND state IN ('accepted', 'running') \
     ORDER BY created_at, id"
);
// The run that gave an executor's task wrote the part that assigned it, so
// the run's profile is the profile that the task runs with. The joins are
// outer, so a part that names no run still finds the task.
const SELECT_EXECUTOR_TASK_PROFILE: &str = "SELECT t.state, r.profile_id FROM runtime_tasks t \
     LEFT JOIN runtime_parts p ON p.id = t.part_id LEFT JOIN runtime_runs r ON r.id = p.run_id \
     WHERE t.id = ?1 AND t.executor_kind = ?2";
// The task conversations of a parent that an executor serves. The WHERE
// clause also keeps SQLite from reading the upsert's ON as a join.
const QUEUE_OUTBOX_OF_CHILDREN: &str = "INSERT INTO runtime_executor_outbox \
     (executor_kind, executor_target, conversation_id, created_at) \
     SELECT c.task_executor_kind, c.task_executor_target, c.id, ?2 \
     FROM runtime_conversations c \
     WHERE c.parent_conversation_id = ?1 AND c.task_executor_kind IS NOT NULL \
     AND c.task_executor_target IS NOT NULL \
     ON CONFLICT DO NOTHING";
const SELECT_OUTBOX: &str = "SELECT executor_target, conversation_id FROM runtime_executor_outbox \
     WHERE executor_kind = ?1 ORDER BY created_at, executor_target, conversation_id";
const DELETE_OUTBOX_ENTRY: &str = "DELETE FROM runtime_executor_outbox \
     WHERE executor_kind = ?1 AND executor_target = ?2 AND conversation_id = ?3";
const DELETE_OUTBOX_OF_TARGET: &str = "DELETE FROM runtime_executor_outbox \
     WHERE executor_kind = ?1 AND executor_target = ?2";

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
    INSERT_EXECUTOR_TASK,
    SET_EXECUTOR_DOCUMENT,
    UPDATE_EXECUTOR_STATE,
    END_EXECUTOR_TASK,
    SELECT_ACTIVE_EXECUTOR_TASKS,
    SELECT_EXECUTOR_TASK_PROFILE,
    QUEUE_OUTBOX_OF_CHILDREN,
    SELECT_OUTBOX,
    DELETE_OUTBOX_ENTRY,
    DELETE_OUTBOX_OF_TARGET,
];

/// A task that a plugin's executor runs, as the parent's run stores it after
/// its part and before the executor submits it (ADR-0031, decision 4).
#[derive(Debug, Clone)]
pub struct NewExecutorTask {
    /// The task, as the parent's run chose it.
    pub id: TaskId,
    /// The task's conversation.
    pub conversation_id: ConversationId,
    /// The conversation to create for a new task, whose origin names the same
    /// executor and target, or `None` for a follow-up, whose conversation
    /// exists.
    pub conversation: Option<NewConversation>,
    /// The parent's part that assigned the task.
    pub part_id: PartId,
    /// The executor's kind: its plugin's ID.
    pub kind: String,
    /// The target that runs the task, as the executor names it.
    pub target: String,
    /// The executor's own document, which the core keeps as it is.
    pub document: Option<String>,
    /// Whether the task continues an earlier task of the same conversation.
    pub follow_up: bool,
    /// Whether the parent continues while the target works.
    pub background: bool,
}

/// Whether a task of an executor is active, and the profile of the run that
/// gave it, which bind the credentials that the executor may borrow for the
/// task (ADR-0034, decision 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorTaskProfile {
    /// Whether the task has not ended.
    pub active: bool,
    /// The profile of the run that wrote the part that assigned the task, or
    /// `None` when that part names no run.
    pub profile_id: Option<ProfileId>,
}

/// A deletion of a task conversation that waits for its executor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxEntry {
    /// The target that keeps the conversation.
    pub target: String,
    /// The conversation.
    pub conversation_id: ConversationId,
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
    executor_kind: Option<String>,
    executor_target: Option<String>,
    executor_version: i64,
    executor_document: Option<String>,
    assignment: Option<String>,
    state_version: i64,
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
        executor_kind: row.get(10)?,
        executor_target: row.get(11)?,
        executor_version: row.get(12)?,
        executor_document: row.get(13)?,
        assignment: row.get(14)?,
        state_version: row.get(15)?,
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
        let executor = match (self.executor_kind, self.executor_target) {
            (Some(kind), Some(target)) => Some(TaskExecution {
                kind,
                target,
                version: version_from(self.executor_version, "an executor's version")?,
                document: self.executor_document,
            }),
            (None, None) => None,
            _ => {
                return Err(RuntimeStoreError::invalid(format!(
                    "task {} names an executor without its target",
                    self.id
                )));
            }
        };
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
            executor,
            assignment: self.assignment,
            state_version: version_from(self.state_version, "a state version")?,
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

/// Keeps the deletions of a parent's task conversations that executors serve
/// until each executor takes them (ADR-0031, decision 9), in the transaction
/// that deletes the parent, so a deletion is never lost. This machine's
/// copies go with the parent through the foreign key.
pub(super) fn queue_outbox_of_children(
    connection: &Connection,
    parent: &str,
    now: i64,
) -> rusqlite::Result<()> {
    connection
        .prepare_cached(QUEUE_OUTBOX_OF_CHILDREN)?
        .execute(params![parent, now])?;
    Ok(())
}

/// Stores a task that an executor runs and, for a new task, its
/// conversation. A task with the same ID already stored stays as it is and
/// is returned.
fn record_executor(
    connection: &Connection,
    task: &NewExecutorTask,
    now: i64,
) -> Result<StoredTask, RuntimeStoreError> {
    if let Some(stored) = select_task(connection, task.id.as_str())? {
        return Ok(stored);
    }
    let conversation = task.conversation_id.as_str();
    if let Some(new_conversation) = &task.conversation {
        let names_the_executor = new_conversation.id == task.conversation_id
            && new_conversation.project_id.is_none()
            && new_conversation
                .task
                .as_ref()
                .and_then(|origin| origin.executor.as_ref())
                .is_some_and(|executor| {
                    executor.kind == task.kind && executor.target_id == task.target
                });
        if !names_the_executor {
            return Err(RuntimeStoreError::invalid(
                "the conversation of an executor's task must name its executor and target",
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
        .prepare_cached(INSERT_EXECUTOR_TASK)?
        .execute(params![
            task.id.as_str(),
            conversation,
            task.part_id.as_str(),
            task.follow_up,
            task.background,
            DOCUMENT_VERSION,
            now,
            task.kind,
            task.target,
            task.document
        ])?;
    select_task(connection, task.id.as_str())?
        .ok_or_else(|| RuntimeStoreError::invalid("an executor's task was not stored"))
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

    /// Stores a task that a plugin's executor runs, with its conversation for
    /// a new task, before the executor submits it, and returns it. Storing a
    /// task again returns the stored task and changes nothing.
    pub async fn record_executor_task(
        &self,
        task: NewExecutorTask,
    ) -> Result<StoredTask, RuntimeStoreError> {
        let now = self.clock().now().as_millis();
        self.transact(Durability::Full, move |transaction| {
            record_executor(transaction, &task, now).map_err(TransactError::Rejected)
        })
        .await
        .map_err(rejected)
    }

    /// Replaces the document of an executor's task. Returns whether the task
    /// exists with that executor.
    pub async fn set_executor_document(
        &self,
        task_id: &TaskId,
        kind: &str,
        document: Option<String>,
    ) -> Result<bool, RuntimeStoreError> {
        let task = task_id.as_str().to_owned();
        let kind = kind.to_owned();
        let changed = self
            .write(Durability::Full, move |transaction| {
                transaction
                    .prepare_cached(SET_EXECUTOR_DOCUMENT)?
                    .execute(params![task, kind, document])
            })
            .await?;
        Ok(changed > 0)
    }

    /// Stores a state of an executor's task that its executor published. An
    /// active state changes the row when its version is newer than the one
    /// stored; an ending ends the task once, whichever version brings it.
    /// Returns whether the row changed.
    pub async fn apply_executor_task_state(
        &self,
        task_id: &TaskId,
        kind: &str,
        version: u64,
        state: TaskState,
        ending: Option<TaskEnding>,
    ) -> Result<bool, RuntimeStoreError> {
        let task = task_id.as_str().to_owned();
        let kind = kind.to_owned();
        let version = version_to(version, "an executor's version")?;
        let now = self.clock().now().as_millis();
        let changed = self
            .transact(Durability::Full, move |transaction| {
                let changed = if state.is_active() {
                    transaction
                        .prepare_cached(UPDATE_EXECUTOR_STATE)?
                        .execute(params![task, kind, state.as_str(), version])?
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
                        .prepare_cached(END_EXECUTOR_TASK)?
                        .execute(params![
                            task,
                            kind,
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

    /// The tasks of an executor that have not ended, the oldest first.
    pub async fn active_executor_tasks(
        &self,
        kind: &str,
    ) -> Result<Vec<StoredTask>, RuntimeStoreError> {
        let kind = kind.to_owned();
        self.read(move |connection| {
            Ok(select_tasks(
                connection,
                SELECT_ACTIVE_EXECUTOR_TASKS,
                params![kind],
            ))
        })
        .await?
    }

    /// Whether a task of the executor of `kind` is active, and the profile
    /// of the run that gave it. A follow-up has a row and a part of its own,
    /// so it names the profile of the run that gave the follow-up, whichever
    /// profile the earlier task ran with. `None` for an unknown task, and
    /// for a task of another executor or of none.
    pub async fn executor_task_profile(
        &self,
        task_id: &TaskId,
        kind: &str,
    ) -> Result<Option<ExecutorTaskProfile>, RuntimeStoreError> {
        let task = task_id.as_str().to_owned();
        let kind = kind.to_owned();
        let row: Option<(String, Option<String>)> = self
            .read(move |connection| {
                connection
                    .prepare_cached(SELECT_EXECUTOR_TASK_PROFILE)?
                    .query_row(params![task, kind], |row| Ok((row.get(0)?, row.get(1)?)))
                    .optional()
            })
            .await?;
        let Some((state, profile_id)) = row else {
            return Ok(None);
        };
        let state = TaskState::parse(&state)
            .ok_or_else(|| RuntimeStoreError::invalid(format!("unknown task state {state:?}")))?;
        let profile_id = profile_id
            .as_deref()
            .map(|id| parse_id(id, ProfileId::parse))
            .transpose()?;
        Ok(Some(ExecutorTaskProfile {
            active: state.is_active(),
            profile_id,
        }))
    }

    /// The deletions that wait for an executor, the oldest first.
    pub async fn executor_outbox(&self, kind: &str) -> Result<Vec<OutboxEntry>, RuntimeStoreError> {
        let kind = kind.to_owned();
        let raw: Vec<(String, String)> = self
            .read(move |connection| {
                let mut select = connection.prepare_cached(SELECT_OUTBOX)?;
                let rows = select.query_map([kind], |row| Ok((row.get(0)?, row.get(1)?)))?;
                rows.collect()
            })
            .await?;
        raw.into_iter()
            .map(|(target, conversation)| {
                Ok(OutboxEntry {
                    target,
                    conversation_id: parse_id(&conversation, ConversationId::parse)?,
                })
            })
            .collect()
    }

    /// Forgets deletions that an executor took.
    pub async fn drop_executor_outbox(
        &self,
        kind: &str,
        entries: &[OutboxEntry],
    ) -> Result<(), RuntimeStoreError> {
        let kind = kind.to_owned();
        let entries: Vec<(String, String)> = entries
            .iter()
            .map(|entry| {
                (
                    entry.target.clone(),
                    entry.conversation_id.as_str().to_owned(),
                )
            })
            .collect();
        self.write(Durability::Full, move |transaction| {
            for (target, conversation) in &entries {
                transaction
                    .prepare_cached(DELETE_OUTBOX_ENTRY)?
                    .execute(params![kind, target, conversation])?;
            }
            Ok(())
        })
        .await?;
        Ok(())
    }

    /// Forgets the deletions that wait for one target of an executor, such
    /// as a worker that the owner removed. Returns how many there were.
    pub async fn drop_executor_target_outbox(
        &self,
        kind: &str,
        target: &str,
    ) -> Result<usize, RuntimeStoreError> {
        let kind = kind.to_owned();
        let target = target.to_owned();
        let dropped = self
            .write(Durability::Full, move |transaction| {
                transaction
                    .prepare_cached(DELETE_OUTBOX_OF_TARGET)?
                    .execute(params![kind, target])
            })
            .await?;
        Ok(dropped)
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
                executor_kind: row.get(4)?,
                executor_target: row.get(5)?,
                executor_name: row.get(6)?,
                executor_noun: row.get(7)?,
            })
        })
        .optional()?;
    let Some(child) = child else {
        return Ok(None);
    };
    let executor = executor_ref(
        child.executor_kind,
        child.executor_target,
        child.executor_name,
        child.executor_noun,
    );
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
        executor,
        latest,
    }))
}

/// The executor of a task conversation, from its row's columns.
pub(super) fn executor_ref(
    kind: Option<String>,
    target: Option<String>,
    name: Option<String>,
    noun: Option<String>,
) -> Option<TaskExecutorRef> {
    match (kind, target) {
        (Some(kind), Some(target_id)) => Some(TaskExecutorRef {
            kind,
            target_id,
            target_name: name.unwrap_or_default(),
            noun: noun.unwrap_or_default(),
        }),
        _ => None,
    }
}

/// A child's conversation row.
struct ChildRow {
    id: String,
    number: i64,
    agent: Option<String>,
    title: String,
    executor_kind: Option<String>,
    executor_target: Option<String>,
    executor_name: Option<String>,
    executor_noun: Option<String>,
}
