//! Applying changes to a conversation: one transaction per batch, one event
//! per change, each with the conversation's next sequence number
//! (ADR-0012, decisions 3 and 10).

use mannie_types::ids::{ConversationId, PartId, ProfileId, ProjectId, RunId, TurnId};
use mannie_types::run::{RunActivity, RunOutcome, RunState};
use mannie_types::tasks::{TaskPartDocument, TaskRefusal, TaskState};
use mannie_types::tools::{ToolCallDocument, ToolCallStatus};
use mannie_types::transcript::{ContextUsage, ConversationEvent, PartKind, PartState};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::rows::ImageDocument;
use super::rows::{
    self, CheckpointDocument, EventBody, PartDocument, StoredRow, kinds, streaming_text, to_json,
};
use super::{
    Applied, Change, DOCUMENT_VERSION, NewArchive, NewCheckpoint, NewConversation, NewImage,
    NewMention, NewPart, NewRun, NewTask, ProviderItem, RuntimeStoreError, TaskEnding, archives,
    images, tasks,
};
use crate::writer::TransactError;
use crate::{Durability, Store};

const SELECT_CONVERSATION_EXISTS: &str = "SELECT 1 FROM runtime_conversations WHERE id = ?1";
const INSERT_CONVERSATION: &str = "INSERT INTO runtime_conversations \
     (id, title, title_source, profile_id, project_id, workspace, parent_conversation_id, \
     task_number, task_agent, task_executor_kind, task_executor_target, task_executor_name, \
     task_executor_noun, created_at, updated_at) \
     VALUES (?1, ?2, 'first_message', ?3, ?5, \
     (SELECT folder FROM runtime_projects WHERE id = ?5), ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?4, ?4)";
const SELECT_PROJECT_EXISTS: &str = "SELECT 1 FROM runtime_projects WHERE id = ?1";
const SELECT_CONVERSATION_PROJECT: &str =
    "SELECT project_id FROM runtime_conversations WHERE id = ?1";
const NEXT_SEQ: &str =
    "UPDATE runtime_conversations SET last_seq = last_seq + 1 WHERE id = ?1 RETURNING last_seq";
const SELECT_LAST_SEQ: &str = "SELECT last_seq FROM runtime_conversations WHERE id = ?1";
const INSERT_EVENT: &str = "INSERT INTO runtime_events \
     (conversation_id, seq, kind, part_id, version, body, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";
const SELECT_ACTIVE_RUN: &str = "SELECT id FROM runtime_runs \
     WHERE conversation_id = ?1 AND state IN ('queued', 'running') LIMIT 1";
const SELECT_NEXT_TURN_POSITION: &str =
    "SELECT COALESCE(MAX(position) + 1, 0) FROM runtime_turns WHERE conversation_id = ?1";
const INSERT_TURN: &str = "INSERT INTO runtime_turns (id, conversation_id, position, current_run_id, created_at) \
     VALUES (?1, ?2, ?3, ?4, ?5)";
const INSERT_RUN: &str = "INSERT INTO runtime_runs \
     (id, conversation_id, turn_id, agent, profile_id, binding, state, version, created_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'queued', ?7, ?8)";
const INSERT_PART: &str = "INSERT INTO runtime_parts \
     (id, conversation_id, turn_id, run_id, attempt, position, kind, audience, state, text, version, document, \
     created_at, finished_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)";
const TOUCH_CONVERSATION: &str = "UPDATE runtime_conversations SET updated_at = ?2 WHERE id = ?1";
const SELECT_LATEST_TURN: &str = "SELECT t.id, r.state FROM runtime_turns t \
     LEFT JOIN runtime_runs r ON r.id = t.current_run_id \
     WHERE t.conversation_id = ?1 ORDER BY t.position DESC LIMIT 1";
const SET_CURRENT_RUN: &str = "UPDATE runtime_turns SET current_run_id = ?2 WHERE id = ?1";
// A run that made calls, of tools or of task tools, stays in its turn when a
// retry replaces it (docs/features/system-tools/tool-calls.md, retry), so its
// operations and its tasks are never assigned again.
const KEEP_CURRENT_RUN: &str = "UPDATE runtime_runs SET kept = 1 \
     WHERE id = (SELECT current_run_id FROM runtime_turns WHERE id = ?1) \
     AND EXISTS (SELECT 1 FROM runtime_parts p WHERE p.run_id = runtime_runs.id \
     AND p.kind IN ('tool_call', 'task') AND p.state <> 'discarded')";
const START_RUN: &str = "UPDATE runtime_runs SET state = 'running', started_at = ?3 \
     WHERE id = ?1 AND conversation_id = ?2 AND state = 'queued'";
const SET_ACTIVITY: &str = "UPDATE runtime_runs SET activity = ?3 \
     WHERE id = ?1 AND conversation_id = ?2 AND state IN ('queued', 'running')";
const SELECT_AWAITING_APPROVAL: &str = "SELECT json_extract(activity, '$.kind') = 'awaiting_approval' \
     FROM runtime_runs WHERE id = ?1 AND conversation_id = ?2";
const END_RUN: &str = "UPDATE runtime_runs SET state = ?3, outcome = ?4, activity = NULL, ended_at = ?5 \
     WHERE id = ?1 AND conversation_id = ?2 AND state IN ('queued', 'running')";
const SELECT_ACTIVE_RUN_TURN: &str = "SELECT turn_id FROM runtime_runs \
     WHERE id = ?1 AND conversation_id = ?2 AND state IN ('queued', 'running')";
const SELECT_NEXT_PART_POSITION: &str =
    "SELECT COALESCE(MAX(position) + 1, 0) FROM runtime_parts WHERE turn_id = ?1";
const SELECT_PART_STATE: &str =
    "SELECT state FROM runtime_parts WHERE id = ?1 AND conversation_id = ?2";
const SELECT_PART_KIND: &str =
    "SELECT kind FROM runtime_parts WHERE id = ?1 AND conversation_id = ?2";
const UPDATE_PART_DOCUMENT: &str =
    "UPDATE runtime_parts SET version = ?2, document = ?3 WHERE id = ?1";
const FINISH_PART: &str =
    "UPDATE runtime_parts SET text = ?2, state = ?3, finished_at = ?4 WHERE id = ?1";
const INSERT_PROVIDER_ITEM: &str = "INSERT INTO runtime_provider_items (part_id, binding, version, item) \
     VALUES (?1, ?2, ?3, ?4)";
const DELETE_DELTAS: &str = "DELETE FROM runtime_events WHERE part_id = ?1 AND kind = 'part_delta'";
const SELECT_STREAMING_PARTS_OF_RUN: &str =
    "SELECT id FROM runtime_parts WHERE run_id = ?1 AND state = 'streaming' ORDER BY position";
const SELECT_ATTEMPT_PARTS: &str = "SELECT id FROM runtime_parts \
     WHERE run_id = ?1 AND attempt = ?2 AND state <> 'discarded' ORDER BY position";
const INSERT_CHECKPOINT: &str = "INSERT INTO runtime_checkpoints \
     (id, conversation_id, covers_through_turn, content_hash, summary, version, document, created_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)";
const UPDATE_CONTEXT: &str = "UPDATE runtime_conversations \
     SET context = ?2, context_version = ?3, context_usage = ?4 WHERE id = ?1";
const RENAME: &str =
    "UPDATE runtime_conversations SET title = ?2, title_source = 'owner' WHERE id = ?1";
const SET_PROFILE: &str = "UPDATE runtime_conversations SET profile_id = ?2 WHERE id = ?1";
const SET_FULL_ACCESS: &str = "UPDATE runtime_conversations SET full_access = ?2 WHERE id = ?1";
const SET_PROJECT: &str = "UPDATE runtime_conversations \
     SET project_id = ?2, workspace = (SELECT folder FROM runtime_projects WHERE id = ?2) \
     WHERE id = ?1";

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    SELECT_CONVERSATION_EXISTS,
    INSERT_CONVERSATION,
    SELECT_PROJECT_EXISTS,
    SELECT_CONVERSATION_PROJECT,
    NEXT_SEQ,
    SELECT_LAST_SEQ,
    INSERT_EVENT,
    SELECT_ACTIVE_RUN,
    SELECT_NEXT_TURN_POSITION,
    INSERT_TURN,
    INSERT_RUN,
    INSERT_PART,
    TOUCH_CONVERSATION,
    SELECT_LATEST_TURN,
    SET_CURRENT_RUN,
    KEEP_CURRENT_RUN,
    START_RUN,
    SET_ACTIVITY,
    SELECT_AWAITING_APPROVAL,
    END_RUN,
    SELECT_ACTIVE_RUN_TURN,
    SELECT_NEXT_PART_POSITION,
    SELECT_PART_STATE,
    SELECT_PART_KIND,
    UPDATE_PART_DOCUMENT,
    FINISH_PART,
    INSERT_PROVIDER_ITEM,
    DELETE_DELTAS,
    SELECT_STREAMING_PARTS_OF_RUN,
    SELECT_ATTEMPT_PARTS,
    INSERT_CHECKPOINT,
    UPDATE_CONTEXT,
    RENAME,
    SET_PROFILE,
    SET_FULL_ACCESS,
    SET_PROJECT,
];

impl Store {
    /// Applies `changes` to a conversation in one transaction with the given
    /// durability, appending one event per change. Either every change is
    /// stored or none is.
    ///
    /// The caller publishes the returned events in order, after this returns
    /// and before it applies the conversation's next batch, so that subscribers
    /// receive events in the order of their sequence numbers (ADR-0012).
    pub async fn apply(
        &self,
        conversation_id: &ConversationId,
        changes: Vec<Change>,
        durability: Durability,
    ) -> Result<Applied, RuntimeStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        let now = self.clock().now().as_millis();
        self.transact(durability, move |transaction| {
            apply_in(transaction, &conversation, changes, now).map_err(TransactError::Rejected)
        })
        .await
        .map_err(|error| match error {
            TransactError::Rejected(error) => error,
            TransactError::Store(error) => RuntimeStoreError::Store(error),
        })
    }
}

/// Stores a conversation's row. The caller checks that its project exists.
pub(super) fn insert_conversation(
    connection: &Connection,
    id: &str,
    conversation: &NewConversation,
    now: i64,
) -> Result<(), RuntimeStoreError> {
    let profile = conversation.profile_id.as_ref().map(ProfileId::as_str);
    let project = conversation.project_id.as_ref().map(ProjectId::as_str);
    let task = conversation.task.as_ref();
    let executor = task.and_then(|origin| origin.executor.as_ref());
    connection
        .prepare_cached(INSERT_CONVERSATION)?
        .execute(params![
            id,
            conversation.title,
            profile,
            now,
            project,
            task.map(|origin| origin.parent_id.as_str()),
            task.map(|origin| origin.number),
            task.map(|origin| origin.agent.as_str()),
            executor.map(|executor| executor.kind.as_str()),
            executor.map(|executor| executor.target_id.as_str()),
            executor.map(|executor| executor.target_name.as_str()),
            executor.map(|executor| executor.noun.as_str())
        ])?;
    Ok(())
}

/// The state of one batch while it is applied.
struct Batch<'a> {
    transaction: &'a Transaction<'a>,
    conversation: &'a str,
    now: i64,
    events: Vec<(u32, ConversationEvent)>,
    summary_changed: bool,
}

fn apply_in(
    transaction: &Transaction<'_>,
    conversation: &str,
    changes: Vec<Change>,
    now: i64,
) -> Result<Applied, RuntimeStoreError> {
    let creates = matches!(changes.first(), Some(Change::CreateConversation { .. }));
    let exists = transaction
        .prepare_cached(SELECT_CONVERSATION_EXISTS)?
        .query_row([conversation], |_row| Ok(()))
        .optional()?
        .is_some();
    if creates && exists {
        return Err(RuntimeStoreError::ConversationExists);
    }
    if !creates && !exists {
        return Err(RuntimeStoreError::ConversationNotFound);
    }
    let mut batch = Batch {
        transaction,
        conversation,
        now,
        events: Vec::new(),
        summary_changed: false,
    };
    for (index, change) in changes.into_iter().enumerate() {
        let is_misplaced_create = index > 0 && matches!(change, Change::CreateConversation { .. });
        if is_misplaced_create {
            return Err(RuntimeStoreError::ConversationExists);
        }
        batch.apply(change)?;
    }
    let last_seq: i64 = transaction
        .prepare_cached(SELECT_LAST_SEQ)?
        .query_row([conversation], |row| row.get(0))?;
    // The conversation list never shows a task conversation
    // (docs/features/child-agents/tasks.md, task conversations).
    let listed = batch.summary_changed && !tasks::is_task_conversation(transaction, conversation)?;
    let summary = if listed {
        rows::conversation_summary(transaction, conversation)?
    } else {
        None
    };
    Ok(Applied {
        events: batch.events,
        last_seq: rows::position(last_seq, "the last sequence number")?,
        summary,
    })
}

/// What follows the owner's text in a new turn: its mentions, then its
/// images.
struct Attached<'a> {
    mentions: &'a [NewMention],
    images: &'a [NewImage],
}

impl Batch<'_> {
    fn apply(&mut self, change: Change) -> Result<(), RuntimeStoreError> {
        match change {
            Change::CreateConversation { conversation } => self.create_conversation(&conversation),
            Change::StartTurn {
                turn_id,
                user_part_id,
                text,
                run,
                task,
                mentions,
                images,
            } => self.start_turn(
                turn_id,
                &user_part_id,
                &text,
                run,
                task.as_ref(),
                &Attached {
                    mentions: &mentions,
                    images: &images,
                },
            ),
            Change::RetryTurn { turn_id, run } => self.retry_turn(&turn_id, run),
            Change::StartRun { run_id } => self.start_run(run_id),
            Change::SetActivity { run_id, activity } => self.set_activity(run_id, &activity),
            Change::EndRun {
                run_id,
                state,
                outcome,
                task,
            } => self.end_run(run_id, state, &outcome, task.as_ref()),
            Change::StartPart { part } => {
                self.insert_part(&part, PartState::Streaming, "")?;
                self.append(
                    kinds::PART_STARTED,
                    Some(part.id.as_str()),
                    &EventBody::empty(),
                )?;
                Ok(())
            }
            Change::Delta { part_id, text } => self.delta(&part_id, text),
            Change::FinishPart {
                part_id,
                state,
                tail,
                provider_item,
            } => self.finish_streaming_part(&part_id, state, &tail, provider_item),
            Change::AddPart { part, text } => {
                self.insert_part(&part, PartState::Complete, &text)?;
                self.append(
                    kinds::PART_FINISHED,
                    Some(part.id.as_str()),
                    &EventBody::empty(),
                )?;
                Ok(())
            }
            Change::UpdatePart {
                part_id,
                tool_call,
                archives,
            } => self.update_part(&part_id, &tool_call, &archives),
            Change::UpdateTaskPart {
                part_id,
                tool_call,
                task,
                archives,
            } => self.update_task_part(&part_id, tool_call, *task, &archives),
            Change::DiscardAttempt { run_id, attempt } => self.discard_attempt(run_id, attempt),
            Change::AddCheckpoint { checkpoint } => self.add_checkpoint(checkpoint),
            Change::UpdateContext { ledger, usage } => self.update_context(&ledger, usage),
            Change::Rename { title } => {
                self.transaction
                    .prepare_cached(RENAME)?
                    .execute(params![self.conversation, title])?;
                self.append(kinds::CONVERSATION_CHANGED, None, &EventBody::empty())?;
                self.summary_changed = true;
                Ok(())
            }
            Change::SetProfile { profile_id } => {
                let profile = profile_id.as_ref().map(|id| id.as_str().to_owned());
                self.transaction
                    .prepare_cached(SET_PROFILE)?
                    .execute(params![self.conversation, profile])?;
                self.append(kinds::CONVERSATION_CHANGED, None, &EventBody::empty())?;
                self.summary_changed = true;
                Ok(())
            }
            Change::SetFullAccess { enabled } => {
                self.transaction
                    .prepare_cached(SET_FULL_ACCESS)?
                    .execute(params![self.conversation, enabled])?;
                self.append(kinds::CONVERSATION_CHANGED, None, &EventBody::empty())?;
                self.summary_changed = true;
                Ok(())
            }
            Change::SetProject { project_id } => self.set_project(&project_id),
        }
    }

    fn create_conversation(
        &mut self,
        conversation: &NewConversation,
    ) -> Result<(), RuntimeStoreError> {
        if let Some(project) = &conversation.project_id {
            self.require_project(project.as_str())?;
        }
        insert_conversation(self.transaction, self.conversation, conversation, self.now)?;
        self.summary_changed = true;
        Ok(())
    }

    /// Moves a general conversation into a project. The store copies the
    /// project's folder, so the workspace always is the project's folder.
    fn set_project(&mut self, project_id: &ProjectId) -> Result<(), RuntimeStoreError> {
        let current: Option<String> = self
            .transaction
            .prepare_cached(SELECT_CONVERSATION_PROJECT)?
            .query_row([self.conversation], |row| row.get(0))?;
        if current.is_some() {
            return Err(RuntimeStoreError::AlreadyInProject);
        }
        self.require_project(project_id.as_str())?;
        self.transaction
            .prepare_cached(SET_PROJECT)?
            .execute(params![self.conversation, project_id.as_str()])?;
        self.append(kinds::CONVERSATION_CHANGED, None, &EventBody::empty())?;
        self.summary_changed = true;
        Ok(())
    }

    fn require_project(&self, project: &str) -> Result<(), RuntimeStoreError> {
        let exists = self
            .transaction
            .prepare_cached(SELECT_PROJECT_EXISTS)?
            .query_row([project], |_row| Ok(()))
            .optional()?
            .is_some();
        if exists {
            Ok(())
        } else {
            Err(RuntimeStoreError::ProjectNotFound)
        }
    }

    fn start_turn(
        &mut self,
        turn_id: TurnId,
        user_part_id: &PartId,
        text: &str,
        run: NewRun,
        task: Option<&NewTask>,
        attached: &Attached<'_>,
    ) -> Result<(), RuntimeStoreError> {
        self.refuse_active_run()?;
        let position: i64 = self
            .transaction
            .prepare_cached(SELECT_NEXT_TURN_POSITION)?
            .query_row([self.conversation], |row| row.get(0))?;
        self.transaction
            .prepare_cached(INSERT_TURN)?
            .execute(params![
                turn_id.as_str(),
                self.conversation,
                position,
                run.id.as_str(),
                self.now
            ])?;
        self.insert_run(turn_id.as_str(), &run)?;
        self.transaction
            .prepare_cached(INSERT_PART)?
            .execute(params![
                user_part_id.as_str(),
                self.conversation,
                turn_id.as_str(),
                Option::<&str>::None,
                Option::<u32>::None,
                0,
                PartKind::UserText.as_str(),
                PartKind::UserText.audience().as_str(),
                PartState::Complete.as_str(),
                text,
                DOCUMENT_VERSION,
                "{}",
                self.now,
                self.now
            ])?;
        // The mentions follow the owner's text, which stays at position 0
        // (ADR-0032, decision 7).
        for (index, mention) in attached.mentions.iter().enumerate() {
            let document = PartDocument {
                mention: Some(mention.view.clone()),
                ..PartDocument::default()
            };
            self.transaction
                .prepare_cached(INSERT_PART)?
                .execute(params![
                    mention.part_id.as_str(),
                    self.conversation,
                    turn_id.as_str(),
                    Option::<&str>::None,
                    Option::<u32>::None,
                    u32::try_from(index + 1).unwrap_or(u32::MAX),
                    PartKind::Mention.as_str(),
                    PartKind::Mention.audience().as_str(),
                    PartState::Complete.as_str(),
                    mention.text,
                    DOCUMENT_VERSION,
                    to_json(&document)?,
                    self.now,
                    self.now
                ])?;
        }
        // The images follow the mentions, and their bytes are stored with
        // them, so that `turn_started` carries them (ADR-0033, decision 4).
        for (index, image) in attached.images.iter().enumerate() {
            let document = PartDocument {
                image: Some(ImageDocument::of(image)),
                ..PartDocument::default()
            };
            self.transaction
                .prepare_cached(INSERT_PART)?
                .execute(params![
                    image.part_id.as_str(),
                    self.conversation,
                    turn_id.as_str(),
                    Option::<&str>::None,
                    Option::<u32>::None,
                    u32::try_from(1 + attached.mentions.len() + index).unwrap_or(u32::MAX),
                    PartKind::Image.as_str(),
                    PartKind::Image.audience().as_str(),
                    PartState::Complete.as_str(),
                    "",
                    DOCUMENT_VERSION,
                    to_json(&document)?,
                    self.now,
                    self.now
                ])?;
            images::insert(self.transaction, self.conversation, image, self.now)?;
        }
        // A task is stored with the turn that works on it, in the same
        // transaction (ADR-0024, decision 4).
        if let Some(task) = task {
            tasks::insert(
                self.transaction,
                self.conversation,
                turn_id.as_str(),
                task,
                self.now,
            )?;
        }
        self.touch()?;
        let body = EventBody {
            turn_id: Some(turn_id),
            run_id: Some(run.id),
            ..EventBody::empty()
        };
        self.append(kinds::TURN_STARTED, None, &body)?;
        self.summary_changed = true;
        Ok(())
    }

    fn retry_turn(&mut self, turn_id: &TurnId, run: NewRun) -> Result<(), RuntimeStoreError> {
        self.refuse_active_run()?;
        let latest: Option<(String, Option<String>)> = self
            .transaction
            .prepare_cached(SELECT_LATEST_TURN)?
            .query_row([self.conversation], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?;
        let retryable = latest.as_ref().is_some_and(|(latest_turn, state)| {
            let is_latest = latest_turn == turn_id.as_str();
            let state_allows = state
                .as_deref()
                .and_then(RunState::parse)
                .is_some_and(RunState::is_retryable);
            is_latest && state_allows
        });
        if !retryable {
            return Err(RuntimeStoreError::NothingToRetry);
        }
        self.transaction
            .prepare_cached(KEEP_CURRENT_RUN)?
            .execute([turn_id.as_str()])?;
        self.insert_run(turn_id.as_str(), &run)?;
        self.transaction
            .prepare_cached(SET_CURRENT_RUN)?
            .execute([turn_id.as_str(), run.id.as_str()])?;
        self.touch()?;
        let body = EventBody {
            run_id: Some(run.id),
            ..EventBody::empty()
        };
        self.append(kinds::RUN_STARTED, None, &body)?;
        self.summary_changed = true;
        Ok(())
    }

    fn start_run(&mut self, run_id: RunId) -> Result<(), RuntimeStoreError> {
        let updated = self
            .transaction
            .prepare_cached(START_RUN)?
            .execute(params![run_id.as_str(), self.conversation, self.now])?;
        if updated == 0 {
            return Err(RuntimeStoreError::RunNotActive { run: run_id });
        }
        // The task of a task conversation runs once its turn's run does.
        tasks::start(self.transaction, self.conversation)?;
        self.append_run_updated(run_id)
    }

    fn set_activity(
        &mut self,
        run_id: RunId,
        activity: &RunActivity,
    ) -> Result<(), RuntimeStoreError> {
        // The conversation list shows whether a reply waits for the owner
        // (docs/features/system-tools/tool-activity.md, status), so the
        // summary changes when a run starts or stops waiting.
        let was_waiting: Option<bool> = self
            .transaction
            .prepare_cached(SELECT_AWAITING_APPROVAL)?
            .query_row(params![run_id.as_str(), self.conversation], |row| {
                row.get(0)
            })
            .optional()?
            .flatten();
        let updated = self
            .transaction
            .prepare_cached(SET_ACTIVITY)?
            .execute(params![
                run_id.as_str(),
                self.conversation,
                to_json(activity)?
            ])?;
        if updated == 0 {
            return Err(RuntimeStoreError::RunNotActive { run: run_id });
        }
        let waits = matches!(activity, RunActivity::AwaitingApproval { .. });
        if was_waiting.unwrap_or(false) != waits {
            self.summary_changed = true;
        }
        self.append_run_updated(run_id)
    }

    fn end_run(
        &mut self,
        run_id: RunId,
        state: RunState,
        outcome: &RunOutcome,
        task: Option<&TaskEnding>,
    ) -> Result<(), RuntimeStoreError> {
        if state.is_active() {
            return Err(RuntimeStoreError::invalid(
                "a run cannot end in an active state",
            ));
        }
        // The runtime finishes its parts before it ends a run; parts that are
        // still streaming end with the run, so none stays streaming forever.
        self.finish_streaming_parts(&run_id, part_state_for(state))?;
        // In a task conversation, the task ends with its run: with the
        // child's report when the runtime gives one, or with the state that
        // the run's end implies, as when recovery ends it.
        let ending = match task {
            Some(ending) => ending.clone(),
            None => TaskEnding {
                state: task_state_for(state),
                report: None,
                failure: outcome.failure.clone(),
            },
        };
        tasks::end_active(self.transaction, self.conversation, &ending, self.now)?;
        let updated = self.transaction.prepare_cached(END_RUN)?.execute(params![
            run_id.as_str(),
            self.conversation,
            state.as_str(),
            to_json(outcome)?,
            self.now
        ])?;
        if updated == 0 {
            return Err(RuntimeStoreError::RunNotActive { run: run_id });
        }
        self.append_run_updated(run_id)?;
        self.summary_changed = true;
        Ok(())
    }

    fn delta(&mut self, part_id: &PartId, text: String) -> Result<(), RuntimeStoreError> {
        if text.is_empty() {
            return Ok(());
        }
        self.require_streaming(part_id)?;
        let body = EventBody {
            text: Some(text),
            ..EventBody::empty()
        };
        self.append(kinds::PART_DELTA, Some(part_id.as_str()), &body)?;
        Ok(())
    }

    fn finish_streaming_part(
        &mut self,
        part_id: &PartId,
        state: PartState,
        tail: &str,
        provider_item: Option<ProviderItem>,
    ) -> Result<(), RuntimeStoreError> {
        if state == PartState::Streaming {
            return Err(RuntimeStoreError::invalid(
                "a part cannot finish as streaming",
            ));
        }
        self.require_streaming(part_id)?;
        self.finish_part(part_id.as_str(), state, tail)?;
        if let Some(item) = provider_item {
            self.transaction
                .prepare_cached(INSERT_PROVIDER_ITEM)?
                .execute(params![
                    part_id.as_str(),
                    item.binding,
                    DOCUMENT_VERSION,
                    item.item
                ])?;
        }
        self.append(
            kinds::PART_FINISHED,
            Some(part_id.as_str()),
            &EventBody::empty(),
        )?;
        Ok(())
    }

    fn update_part(
        &mut self,
        part_id: &PartId,
        tool_call: &ToolCallDocument,
        archives: &[NewArchive],
    ) -> Result<(), RuntimeStoreError> {
        self.require_streaming(part_id)?;
        let kind: Option<String> = self
            .transaction
            .prepare_cached(SELECT_PART_KIND)?
            .query_row([part_id.as_str(), self.conversation], |row| row.get(0))
            .optional()?;
        if kind.as_deref() != Some(PartKind::ToolCall.as_str()) {
            return Err(RuntimeStoreError::NotAToolCall {
                part: part_id.clone(),
            });
        }
        let document = PartDocument {
            failure: None,
            tool_call: Some(tool_call.clone()),
            task: None,
            mention: None,
            image: None,
        };
        self.transaction
            .prepare_cached(UPDATE_PART_DOCUMENT)?
            .execute(params![
                part_id.as_str(),
                DOCUMENT_VERSION,
                to_json(&document)?
            ])?;
        for archive in archives {
            archives::insert(
                self.transaction,
                self.conversation,
                part_id.as_str(),
                archive,
                self.now,
            )?;
        }
        self.append(
            kinds::PART_UPDATED,
            Some(part_id.as_str()),
            &EventBody::empty(),
        )?;
        Ok(())
    }

    /// Stores a task part's call record and task section, with the outputs
    /// it archives, such as a long report.
    fn update_task_part(
        &mut self,
        part_id: &PartId,
        tool_call: ToolCallDocument,
        task: TaskPartDocument,
        archives: &[NewArchive],
    ) -> Result<(), RuntimeStoreError> {
        self.require_streaming(part_id)?;
        let kind: Option<String> = self
            .transaction
            .prepare_cached(SELECT_PART_KIND)?
            .query_row([part_id.as_str(), self.conversation], |row| row.get(0))
            .optional()?;
        if kind.as_deref() != Some(PartKind::Task.as_str()) {
            return Err(RuntimeStoreError::NotATaskPart {
                part: part_id.clone(),
            });
        }
        let document = PartDocument {
            failure: None,
            tool_call: Some(tool_call),
            task: Some(task),
            mention: None,
            image: None,
        };
        self.transaction
            .prepare_cached(UPDATE_PART_DOCUMENT)?
            .execute(params![
                part_id.as_str(),
                DOCUMENT_VERSION,
                to_json(&document)?
            ])?;
        for archive in archives {
            archives::insert(
                self.transaction,
                self.conversation,
                part_id.as_str(),
                archive,
                self.now,
            )?;
        }
        self.append(
            kinds::PART_UPDATED,
            Some(part_id.as_str()),
            &EventBody::empty(),
        )?;
        Ok(())
    }

    fn discard_attempt(&mut self, run_id: RunId, attempt: u32) -> Result<(), RuntimeStoreError> {
        let parts: Vec<String> = {
            let mut select = self.transaction.prepare_cached(SELECT_ATTEMPT_PARTS)?;
            let rows = select.query_map(params![run_id.as_str(), attempt], |row| row.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        let mut part_ids = Vec::with_capacity(parts.len());
        for part_id in parts {
            self.finish_part(&part_id, PartState::Discarded, "")?;
            part_ids.push(PartId::parse(&part_id)?);
        }
        let body = EventBody {
            run_id: Some(run_id),
            part_ids,
            ..EventBody::empty()
        };
        self.append(kinds::ATTEMPT_DISCARDED, None, &body)?;
        Ok(())
    }

    fn add_checkpoint(&mut self, checkpoint: NewCheckpoint) -> Result<(), RuntimeStoreError> {
        let document = CheckpointDocument {
            tokens_before: checkpoint.tokens_before,
            tokens_after: checkpoint.tokens_after,
            trigger: checkpoint.trigger,
        };
        self.transaction
            .prepare_cached(INSERT_CHECKPOINT)?
            .execute(params![
                checkpoint.id.as_str(),
                self.conversation,
                checkpoint.covers_through_turn,
                checkpoint.content_hash,
                checkpoint.summary,
                DOCUMENT_VERSION,
                to_json(&document)?,
                self.now
            ])?;
        let body = EventBody {
            checkpoint_id: Some(checkpoint.id),
            ..EventBody::empty()
        };
        self.append(kinds::CHECKPOINT_CREATED, None, &body)?;
        Ok(())
    }

    fn update_context(
        &mut self,
        ledger: &str,
        usage: ContextUsage,
    ) -> Result<(), RuntimeStoreError> {
        self.transaction
            .prepare_cached(UPDATE_CONTEXT)?
            .execute(params![
                self.conversation,
                ledger,
                DOCUMENT_VERSION,
                to_json(&usage)?
            ])?;
        let body = EventBody {
            usage: Some(usage),
            ..EventBody::empty()
        };
        self.append(kinds::CONTEXT_USAGE_CHANGED, None, &body)?;
        Ok(())
    }

    /// Appends an event and records it for publishing when subscribers
    /// receive it.
    fn append(
        &mut self,
        kind: &str,
        part_id: Option<&str>,
        body: &EventBody,
    ) -> Result<u32, RuntimeStoreError> {
        let seq: i64 = self
            .transaction
            .prepare_cached(NEXT_SEQ)?
            .query_row([self.conversation], |row| row.get(0))
            .map_err(|error| {
                // The last_seq column's CHECK refuses numbers beyond u32.
                tracing::error!(%error, "the conversation's sequence numbers ran out");
                RuntimeStoreError::SequenceExhausted
            })?;
        let body = to_json(body)?;
        self.transaction
            .prepare_cached(INSERT_EVENT)?
            .execute(params![
                self.conversation,
                seq,
                kind,
                part_id,
                DOCUMENT_VERSION,
                body,
                self.now
            ])?;
        let stored = StoredRow {
            seq,
            kind: kind.to_owned(),
            part_id: part_id.map(str::to_owned),
            body,
        };
        let seq = rows::position(seq, "a sequence number")?;
        if let Some(event) = rows::hydrate(self.transaction, self.conversation, &stored)? {
            self.events.push((seq, event));
        }
        Ok(seq)
    }

    fn append_run_updated(&mut self, run_id: RunId) -> Result<(), RuntimeStoreError> {
        let body = EventBody {
            run_id: Some(run_id),
            ..EventBody::empty()
        };
        self.append(kinds::RUN_UPDATED, None, &body)?;
        Ok(())
    }

    fn refuse_active_run(&self) -> Result<(), RuntimeStoreError> {
        let active: Option<String> = self
            .transaction
            .prepare_cached(SELECT_ACTIVE_RUN)?
            .query_row([self.conversation], |row| row.get(0))
            .optional()?;
        if active.is_some() {
            return Err(RuntimeStoreError::RunActive);
        }
        Ok(())
    }

    fn touch(&self) -> Result<(), RuntimeStoreError> {
        self.transaction
            .prepare_cached(TOUCH_CONVERSATION)?
            .execute(params![self.conversation, self.now])?;
        Ok(())
    }

    fn insert_run(&self, turn_id: &str, run: &NewRun) -> Result<(), RuntimeStoreError> {
        self.transaction
            .prepare_cached(INSERT_RUN)?
            .execute(params![
                run.id.as_str(),
                self.conversation,
                turn_id,
                run.agent,
                run.profile_id.as_str(),
                run.binding,
                DOCUMENT_VERSION,
                self.now
            ])?;
        Ok(())
    }

    fn insert_part(
        &self,
        part: &NewPart,
        state: PartState,
        text: &str,
    ) -> Result<(), RuntimeStoreError> {
        let turn: Option<String> = self
            .transaction
            .prepare_cached(SELECT_ACTIVE_RUN_TURN)?
            .query_row(params![part.run_id.as_str(), self.conversation], |row| {
                row.get(0)
            })
            .optional()?;
        let belongs = turn.as_deref() == Some(part.turn_id.as_str());
        if !belongs {
            return Err(RuntimeStoreError::RunNotActive {
                run: part.run_id.clone(),
            });
        }
        let position: i64 = self
            .transaction
            .prepare_cached(SELECT_NEXT_PART_POSITION)?
            .query_row([part.turn_id.as_str()], |row| row.get(0))?;
        let document = PartDocument {
            failure: part.failure.clone(),
            tool_call: part.tool_call.clone(),
            task: None,
            mention: None,
            image: None,
        };
        let finished_at = (state != PartState::Streaming).then_some(self.now);
        self.transaction
            .prepare_cached(INSERT_PART)?
            .execute(params![
                part.id.as_str(),
                self.conversation,
                part.turn_id.as_str(),
                part.run_id.as_str(),
                part.attempt,
                position,
                part.kind.as_str(),
                part.kind.audience().as_str(),
                state.as_str(),
                text,
                DOCUMENT_VERSION,
                to_json(&document)?,
                self.now,
                finished_at
            ])?;
        Ok(())
    }

    fn require_streaming(&self, part_id: &PartId) -> Result<(), RuntimeStoreError> {
        let state: Option<String> = self
            .transaction
            .prepare_cached(SELECT_PART_STATE)?
            .query_row([part_id.as_str(), self.conversation], |row| row.get(0))
            .optional()?;
        if state.as_deref() == Some(PartState::Streaming.as_str()) {
            Ok(())
        } else {
            Err(RuntimeStoreError::PartNotStreaming {
                part: part_id.clone(),
            })
        }
    }

    /// Stores a part's final text and state, and drops its delta events. The
    /// final text of a streaming part is its deltas plus `tail`; a part that
    /// already finished keeps its text.
    fn finish_part(
        &self,
        part_id: &str,
        state: PartState,
        tail: &str,
    ) -> Result<(), RuntimeStoreError> {
        let Some(row) = rows::part_row(self.transaction, part_id)? else {
            return Err(RuntimeStoreError::invalid(format!(
                "the part {part_id} is missing"
            )));
        };
        let text = if row.state()? == PartState::Streaming {
            let mut text = streaming_text(self.transaction, part_id)?;
            text.push_str(tail);
            text
        } else {
            row.text
        };
        self.transaction
            .prepare_cached(FINISH_PART)?
            .execute(params![part_id, text, state.as_str(), self.now])?;
        self.transaction
            .prepare_cached(DELETE_DELTAS)?
            .execute([part_id])?;
        Ok(())
    }

    /// Finishes the parts of a run that still stream, with an event for each.
    fn finish_streaming_parts(
        &mut self,
        run_id: &RunId,
        state: PartState,
    ) -> Result<(), RuntimeStoreError> {
        let streaming: Vec<String> = {
            let mut select = self
                .transaction
                .prepare_cached(SELECT_STREAMING_PARTS_OF_RUN)?;
            let rows = select.query_map([run_id.as_str()], |row| row.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        // A task that has not ended when its parent's run ends ends with it:
        // interrupted when Mannie quits, canceled otherwise.
        let unfinished = if state == PartState::Interrupted {
            TaskState::Interrupted
        } else {
            TaskState::Canceled
        };
        for part_id in streaming {
            self.end_call(&part_id, unfinished)?;
            self.finish_part(&part_id, state, "")?;
            self.append(kinds::PART_FINISHED, Some(&part_id), &EventBody::empty())?;
        }
        Ok(())
    }

    /// Ends a call that ends with its run, unfinished: a tool call or a task
    /// part.
    fn end_call(&self, part_id: &str, unfinished: TaskState) -> Result<(), RuntimeStoreError> {
        let Some(row) = rows::part_row(self.transaction, part_id)? else {
            return Ok(());
        };
        match PartKind::parse(&row.kind) {
            Some(PartKind::ToolCall) => self.end_tool_call(&row),
            Some(PartKind::Task) => self.end_task_part(&row, unfinished),
            _ => Ok(()),
        }
    }

    /// Gives a tool call that ends with its run, unfinished, the status that
    /// says what may have happened: an operation that was executing becomes
    /// uncertain, and every other call did not run
    /// (docs/features/system-tools/operations-and-approvals.md, recovery).
    /// The status is stored in the same transaction as the end of the part
    /// and of the run.
    fn end_tool_call(&self, row: &rows::PartRow) -> Result<(), RuntimeStoreError> {
        let part_id = row.id.as_str();
        let mut document: PartDocument = rows::from_json(&row.document, "part document")?;
        let Some(tool_call) = document.tool_call.as_mut() else {
            return Ok(());
        };
        if tool_call.status.is_final() {
            return Ok(());
        }
        tool_call.status = tool_call.status.after_interruption();
        tool_call.ended_at = Some(rows::millis(self.now));
        self.transaction
            .prepare_cached(UPDATE_PART_DOCUMENT)?
            .execute(params![part_id, DOCUMENT_VERSION, to_json(&document)?])?;
        Ok(())
    }

    /// Ends a task part whose parent's run ends before the part did
    /// (docs/features/child-agents/tasks.md, recovery). A task that ended
    /// already, such as a report stored just before Mannie ended, gives the
    /// part its ending; a task that still runs ends as `unfinished`; a task
    /// that was never stored did not start. A task that ended keeps the first
    /// ending stored, whichever run stored it.
    ///
    /// A task that an executor still runs keeps running there (ADR-0027,
    /// ADR-0031): the part lets it go, and its row stays active, with any
    /// cancel that waits in the executor's document, until the executor
    /// publishes how it ended.
    fn end_task_part(
        &self,
        row: &rows::PartRow,
        unfinished: TaskState,
    ) -> Result<(), RuntimeStoreError> {
        let mut document: PartDocument = rows::from_json(&row.document, "part document")?;
        let Some(call) = document.tool_call.as_mut() else {
            return Ok(());
        };
        let now = rows::millis(self.now);
        let mut section = document.task.take().unwrap_or_default();
        let mut call_completed = false;
        if let Some(task) = section.task.as_mut().filter(|task| task.state.is_active()) {
            match tasks::of_part(self.transaction, &row.id)? {
                Some(stored) if stored.state.is_active() && stored.executor.is_some() => {
                    task.detached = true;
                    call_completed = true;
                }
                Some(stored) if !stored.state.is_active() => {
                    let ending = stored.ending.unwrap_or(TaskEnding {
                        state: stored.state,
                        report: None,
                        failure: None,
                    });
                    task.state = ending.state;
                    task.report = ending.report;
                    task.failure = ending.failure;
                    task.ended_at = stored.ended_at.or(Some(now));
                    call_completed = true;
                }
                Some(stored) => {
                    let ending = TaskEnding {
                        state: unfinished,
                        report: None,
                        failure: None,
                    };
                    tasks::end(self.transaction, stored.id.as_str(), &ending, self.now)?;
                    task.state = unfinished;
                    task.ended_at = Some(now);
                }
                None => {
                    task.state = TaskState::Interrupted;
                    task.ended_at = Some(now);
                    section.refusal = Some(TaskRefusal::NotStarted);
                }
            }
            task.activity = None;
            task.approval = None;
        }
        if !call.status.is_final() {
            call.status = if call_completed {
                ToolCallStatus::Completed
            } else {
                call.status.after_interruption()
            };
            call.ended_at = Some(now);
        }
        document.task = Some(section);
        self.transaction
            .prepare_cached(UPDATE_PART_DOCUMENT)?
            .execute(params![row.id, DOCUMENT_VERSION, to_json(&document)?])?;
        Ok(())
    }
}

/// The state of a task whose run ended without saying how the task ended, as
/// when recovery ends it.
fn task_state_for(run: RunState) -> TaskState {
    match run {
        RunState::Completed => TaskState::Reported,
        RunState::Failed => TaskState::Failed,
        RunState::Stopped => TaskState::Canceled,
        RunState::Interrupted | RunState::Queued | RunState::Running => TaskState::Interrupted,
    }
}

/// The state that a run's streaming parts take when the run ends.
pub(crate) fn part_state_for(run: RunState) -> PartState {
    match run {
        RunState::Completed => PartState::Complete,
        RunState::Stopped => PartState::Stopped,
        RunState::Interrupted | RunState::Queued | RunState::Running => PartState::Interrupted,
        RunState::Failed => PartState::Incomplete,
    }
}
