//! Applying changes to a conversation: one transaction per batch, one event
//! per change, each with the conversation's next sequence number
//! (ADR-0012, decisions 3 and 10).

use mannie_types::ids::{ConversationId, PartId, RunId, TurnId};
use mannie_types::run::{RunActivity, RunOutcome, RunState};
use mannie_types::transcript::{ContextUsage, ConversationEvent, PartKind, PartState};
use rusqlite::{OptionalExtension, Transaction, params};

use super::rows::{
    self, CheckpointDocument, EventBody, PartDocument, StoredRow, kinds, streaming_text, to_json,
};
use super::{
    Applied, Change, DOCUMENT_VERSION, NewCheckpoint, NewConversation, NewPart, NewRun,
    ProviderItem, RuntimeStoreError,
};
use crate::writer::TransactError;
use crate::{Durability, Store};

const SELECT_CONVERSATION_EXISTS: &str = "SELECT 1 FROM runtime_conversations WHERE id = ?1";
const INSERT_CONVERSATION: &str = "INSERT INTO runtime_conversations \
     (id, title, title_source, profile_id, created_at, updated_at) VALUES (?1, ?2, 'first_message', ?3, ?4, ?4)";
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
const START_RUN: &str = "UPDATE runtime_runs SET state = 'running', started_at = ?3 \
     WHERE id = ?1 AND conversation_id = ?2 AND state = 'queued'";
const SET_ACTIVITY: &str = "UPDATE runtime_runs SET activity = ?3 \
     WHERE id = ?1 AND conversation_id = ?2 AND state IN ('queued', 'running')";
const END_RUN: &str = "UPDATE runtime_runs SET state = ?3, outcome = ?4, activity = NULL, ended_at = ?5 \
     WHERE id = ?1 AND conversation_id = ?2 AND state IN ('queued', 'running')";
const SELECT_ACTIVE_RUN_TURN: &str = "SELECT turn_id FROM runtime_runs \
     WHERE id = ?1 AND conversation_id = ?2 AND state IN ('queued', 'running')";
const SELECT_NEXT_PART_POSITION: &str =
    "SELECT COALESCE(MAX(position) + 1, 0) FROM runtime_parts WHERE turn_id = ?1";
const SELECT_PART_STATE: &str =
    "SELECT state FROM runtime_parts WHERE id = ?1 AND conversation_id = ?2";
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

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    SELECT_CONVERSATION_EXISTS,
    INSERT_CONVERSATION,
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
    START_RUN,
    SET_ACTIVITY,
    END_RUN,
    SELECT_ACTIVE_RUN_TURN,
    SELECT_NEXT_PART_POSITION,
    SELECT_PART_STATE,
    FINISH_PART,
    INSERT_PROVIDER_ITEM,
    DELETE_DELTAS,
    SELECT_STREAMING_PARTS_OF_RUN,
    SELECT_ATTEMPT_PARTS,
    INSERT_CHECKPOINT,
    UPDATE_CONTEXT,
    RENAME,
    SET_PROFILE,
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
    let summary = if batch.summary_changed {
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

impl Batch<'_> {
    fn apply(&mut self, change: Change) -> Result<(), RuntimeStoreError> {
        match change {
            Change::CreateConversation { conversation } => self.create_conversation(&conversation),
            Change::StartTurn {
                turn_id,
                user_part_id,
                text,
                run,
            } => self.start_turn(turn_id, &user_part_id, &text, run),
            Change::RetryTurn { turn_id, run } => self.retry_turn(&turn_id, run),
            Change::StartRun { run_id } => self.start_run(run_id),
            Change::SetActivity { run_id, activity } => self.set_activity(run_id, &activity),
            Change::EndRun {
                run_id,
                state,
                outcome,
            } => self.end_run(run_id, state, &outcome),
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
        }
    }

    fn create_conversation(
        &mut self,
        conversation: &NewConversation,
    ) -> Result<(), RuntimeStoreError> {
        let profile = conversation
            .profile_id
            .as_ref()
            .map(|id| id.as_str().to_owned());
        self.transaction
            .prepare_cached(INSERT_CONVERSATION)?
            .execute(params![
                self.conversation,
                conversation.title,
                profile,
                self.now
            ])?;
        self.summary_changed = true;
        Ok(())
    }

    fn start_turn(
        &mut self,
        turn_id: TurnId,
        user_part_id: &PartId,
        text: &str,
        run: NewRun,
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
        self.append_run_updated(run_id)
    }

    fn set_activity(
        &mut self,
        run_id: RunId,
        activity: &RunActivity,
    ) -> Result<(), RuntimeStoreError> {
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
        self.append_run_updated(run_id)
    }

    fn end_run(
        &mut self,
        run_id: RunId,
        state: RunState,
        outcome: &RunOutcome,
    ) -> Result<(), RuntimeStoreError> {
        if state.is_active() {
            return Err(RuntimeStoreError::invalid(
                "a run cannot end in an active state",
            ));
        }
        // The runtime finishes its parts before it ends a run; parts that are
        // still streaming end with the run, so none stays streaming forever.
        self.finish_streaming_parts(&run_id, part_state_for(state))?;
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
        for part_id in streaming {
            self.finish_part(&part_id, state, "")?;
            self.append(kinds::PART_FINISHED, Some(&part_id), &EventBody::empty())?;
        }
        Ok(())
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
