//! The conversation commands (docs/features/chat/conversations.md,
//! contracts): what the desktop's chat commands call.

use std::sync::Arc;

use mannie_store::{
    Change, Durability, NewConversation, NewRun, ProfileStoreError, RuntimeStoreError,
    SettingsStoreError, StoredProfile,
};
use mannie_types::chat::{
    ChatError, ChatSettings, CheckpointRef, EarlierTurns, LoadEarlierTurns, MAX_EARLIER_TURNS,
    RenameConversation, SendMessage, SetConversationProfile, StartConversation,
    StartedConversation, StartedReply, StopOutcome, SubscribeConversation, UpdateChatSettings,
    check_message, check_title, title_from_message,
};
use mannie_types::ids::{ConversationId, ProfileId};
use mannie_types::images::check_images;
use mannie_types::mentions::check_mentions;
use mannie_types::provider::context_budget;
use mannie_types::transcript::{Checkpoint, ConversationSummary};

use crate::agents::CONVERSATION_AGENT;
use crate::calls::CallOptions;
use crate::compaction::Trigger;
use crate::hub::{ConversationSubscription, ListSubscription};
use crate::{Busy, Runtime, lock};

/// A conversation that a command has reserved: for a new run, or for the
/// owner's compaction. Dropping it releases the conversation unless a run
/// took it over.
pub(crate) struct Reservation<'a> {
    runtime: &'a Runtime,
    conversation: ConversationId,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        let released = {
            let mut busy = lock(&self.runtime.inner.busy);
            let reserved = matches!(
                busy.get(&self.conversation),
                Some(Busy::Starting | Busy::Compacting)
            );
            if reserved {
                busy.remove(&self.conversation);
            }
            reserved
        };
        // A command that let the conversation go may leave it to a wake-up
        // (ADR-0041).
        if released {
            self.runtime.conversation_free(&self.conversation);
        }
    }
}

pub(crate) fn store_failure(error: RuntimeStoreError) -> ChatError {
    match error {
        RuntimeStoreError::ConversationNotFound => ChatError::ConversationNotFound,
        RuntimeStoreError::RunActive => ChatError::RunActive,
        RuntimeStoreError::NothingToRetry => ChatError::NothingToRetry,
        RuntimeStoreError::ProjectNotFound => ChatError::ProjectNotFound,
        other => {
            tracing::error!(error = %other, error.code = other.code(), "a conversation could not be read or saved");
            ChatError::StorageFailed
        }
    }
}

/// Refuses the owner's commands on a task conversation, whose turns its
/// parent's agent writes (ADR-0024, decision 3). Stop works on any
/// conversation.
fn refuse_task_conversation(summary: &ConversationSummary) -> Result<(), ChatError> {
    if summary.task.is_some() {
        return Err(ChatError::TaskConversation);
    }
    Ok(())
}

fn profile_store_failure(error: &ProfileStoreError) -> ChatError {
    tracing::error!(%error, error.code = error.code(), "a provider profile could not be read");
    ChatError::StorageFailed
}

fn settings_failure(error: &SettingsStoreError) -> ChatError {
    if let SettingsStoreError::NewerVersion { .. } = error {
        return ChatError::WrittenByNewerVersion;
    }
    tracing::error!(%error, error.code = error.code(), "the chat settings could not be read or saved");
    ChatError::StorageFailed
}

impl Runtime {
    fn refuse_when_quitting(&self) -> Result<(), ChatError> {
        if self.is_shutting_down() {
            return Err(ChatError::ShuttingDown);
        }
        Ok(())
    }

    pub(crate) fn reserve(
        &self,
        conversation: &ConversationId,
        busy_with: Busy,
    ) -> Result<Reservation<'_>, ChatError> {
        let mut busy = lock(&self.inner.busy);
        match busy.get(conversation) {
            Some(Busy::Compacting) => Err(ChatError::Compacting),
            Some(Busy::Starting | Busy::Running(_)) => Err(ChatError::RunActive),
            None => {
                busy.insert(conversation.clone(), busy_with);
                Ok(Reservation {
                    runtime: self,
                    conversation: conversation.clone(),
                })
            }
        }
    }

    /// The profile a conversation uses: its own, or the default when it has
    /// none or its own was deleted.
    pub(crate) async fn profile_for(
        &self,
        chosen: Option<&ProfileId>,
    ) -> Result<StoredProfile, ChatError> {
        if let Some(id) = chosen {
            let profile = self
                .inner
                .store
                .profile(id)
                .await
                .map_err(|error| profile_store_failure(&error))?;
            if let Some(profile) = profile {
                return Ok(profile);
            }
        }
        self.inner
            .store
            .default_profile()
            .await
            .map_err(|error| profile_store_failure(&error))?
            .ok_or(ChatError::NoProfile)
    }

    pub(crate) fn new_run(&self, profile: &StoredProfile) -> NewRun {
        NewRun {
            id: self.inner.ids.run(),
            agent: CONVERSATION_AGENT.to_owned(),
            profile_id: profile.id.clone(),
            binding: profile.binding(),
        }
    }

    /// Hands a reserved conversation over to a new run.
    pub(crate) fn start_reserved(
        &self,
        reservation: Reservation<'_>,
        turn: &mannie_types::ids::TurnId,
        run: &NewRun,
    ) {
        let _mailbox = self.spawn_run(
            reservation.conversation.clone(),
            turn.clone(),
            run.id.clone(),
            run.profile_id.clone(),
            run.agent.clone(),
            None,
        );
        // The run holds the conversation now; the reservation's drop leaves
        // it alone.
        drop(reservation);
    }

    /// Creates a conversation with the owner's first message and starts the
    /// reply. A conversation in a project works in the project's folder.
    pub async fn start_conversation(
        &self,
        input: StartConversation,
    ) -> Result<StartedConversation, ChatError> {
        self.refuse_when_quitting()?;
        check_message(&input.text, !input.images.is_empty())?;
        let mentions = check_mentions(input.mentions)?;
        let image_refs = check_images(input.images)?;
        let profile = match &input.profile_id {
            Some(id) => self
                .inner
                .store
                .profile(id)
                .await
                .map_err(|error| profile_store_failure(&error))?
                .ok_or(ChatError::ProfileNotFound)?,
            None => self.profile_for(None).await?,
        };
        if input
            .project_id
            .as_ref()
            .is_some_and(|project| self.is_being_removed(project))
        {
            return Err(ChatError::ProjectNotFound);
        }
        let full_access = self.full_access_for_new_conversations().await;
        let conversation_id = self.inner.ids.conversation();
        let reservation = self.reserve(&conversation_id, Busy::Starting)?;
        let mentions = if mentions.is_empty() {
            Vec::new()
        } else {
            let folder = match &input.project_id {
                Some(project) => Some(self.mention_project_folder(project).await?),
                None => None,
            };
            let context = self
                .mention_context_for(
                    None,
                    input.project_id.as_ref(),
                    folder.as_deref(),
                    full_access,
                )
                .await?;
            let budget = context_budget(profile.context_window, profile.output_limit);
            self.resolve_mentions(&context, &mentions, budget).await?
        };
        let images = self.resolve_images(&image_refs).await?;
        // A first message with only images takes its title from the first
        // image's name (ADR-0033, decision 1).
        let mut title = title_from_message(&input.text);
        if title.is_empty()
            && let Some(first) = images.first()
        {
            title = title_from_message(&first.view.name);
        }
        let turn_id = self.inner.ids.turn();
        let run = self.new_run(&profile);
        let mut changes = vec![Change::CreateConversation {
            conversation: NewConversation {
                id: conversation_id.clone(),
                title,
                profile_id: input.profile_id,
                project_id: input.project_id,
                task: None,
            },
        }];
        // In the same transaction, so the reply's first call already follows
        // it (ADR-0029, decision 2).
        if full_access {
            changes.push(Change::SetFullAccess { enabled: true });
        }
        changes.push(Change::StartTurn {
            turn_id: turn_id.clone(),
            user_part_id: self.inner.ids.part(),
            text: input.text,
            run: run.clone(),
            task: None,
            mentions,
            images,
        });
        self.inner
            .hubs
            .apply(
                &self.inner.store,
                &conversation_id,
                changes,
                Durability::Full,
            )
            .await
            .map_err(store_failure)?;
        self.release_images(&image_refs);
        self.start_reserved(reservation, &turn_id, &run);
        Ok(StartedConversation {
            conversation_id,
            turn_id,
            run_id: run.id,
        })
    }

    /// Adds the owner's message to a conversation and starts the reply.
    pub async fn send_message(&self, input: SendMessage) -> Result<StartedReply, ChatError> {
        self.refuse_when_quitting()?;
        check_message(&input.text, !input.images.is_empty())?;
        let mentions = check_mentions(input.mentions)?;
        let image_refs = check_images(input.images)?;
        let reservation = self.reserve(&input.conversation_id, Busy::Starting)?;
        let summary = self
            .inner
            .store
            .conversation_summary(&input.conversation_id)
            .await
            .map_err(store_failure)?
            .ok_or(ChatError::ConversationNotFound)?;
        refuse_task_conversation(&summary)?;
        // The owner's message ends the pause that their Stop began.
        self.inner.wake_ups.resume(&input.conversation_id);
        let profile = self.profile_for(summary.profile_id.as_ref()).await?;
        let mentions = if mentions.is_empty() {
            Vec::new()
        } else {
            let context = self
                .mention_context_for(
                    Some(&input.conversation_id),
                    summary.project_id.as_ref(),
                    summary.workspace.as_deref(),
                    summary.full_access,
                )
                .await?;
            let budget = context_budget(profile.context_window, profile.output_limit);
            self.resolve_mentions(&context, &mentions, budget).await?
        };
        let images = self.resolve_images(&image_refs).await?;
        let opening = self
            .opening_note(&input.conversation_id)
            .await
            .map_err(store_failure)?;
        let turn_id = self.inner.ids.turn();
        let run = self.new_run(&profile);
        let mut changes = vec![Change::StartTurn {
            turn_id: turn_id.clone(),
            user_part_id: self.inner.ids.part(),
            text: input.text,
            run: run.clone(),
            task: None,
            mentions,
            images,
        }];
        // The conversation's background commands reach the model with the
        // message, and the ends that the note tells are told in the same
        // transaction (ADR-0041).
        if let Some(opening) = opening {
            changes.push(Change::AddRequestNote {
                turn_id: turn_id.clone(),
                part_id: self.inner.ids.part(),
                text: opening.text,
            });
            if !opening.told.is_empty() {
                changes.push(Change::TellBackgroundEnds {
                    numbers: opening.told,
                });
            }
        }
        self.inner
            .hubs
            .apply(
                &self.inner.store,
                &input.conversation_id,
                changes,
                Durability::Full,
            )
            .await
            .map_err(store_failure)?;
        // Only once the message is stored: until then, a failed send keeps
        // its images in the message box for another try.
        self.release_images(&image_refs);
        self.start_reserved(reservation, &turn_id, &run);
        Ok(StartedReply {
            turn_id,
            run_id: run.id,
        })
    }

    /// Starts a new run for the latest turn when its reply failed, was
    /// stopped or was interrupted.
    pub async fn retry_reply(
        &self,
        conversation: &ConversationId,
    ) -> Result<StartedReply, ChatError> {
        self.refuse_when_quitting()?;
        let reservation = self.reserve(conversation, Busy::Starting)?;
        let summary = self
            .inner
            .store
            .conversation_summary(conversation)
            .await
            .map_err(store_failure)?
            .ok_or(ChatError::ConversationNotFound)?;
        refuse_task_conversation(&summary)?;
        let latest = self
            .inner
            .store
            .latest_turn(conversation)
            .await
            .map_err(store_failure)?;
        let Some((turn_id, Some(state))) = latest else {
            return Err(ChatError::NothingToRetry);
        };
        if !state.is_retryable() {
            return Err(ChatError::NothingToRetry);
        }
        let profile = self.profile_for(summary.profile_id.as_ref()).await?;
        let run = self.new_run(&profile);
        let change = Change::RetryTurn {
            turn_id: turn_id.clone(),
            run: run.clone(),
        };
        self.inner
            .hubs
            .apply(
                &self.inner.store,
                conversation,
                vec![change],
                Durability::Full,
            )
            .await
            .map_err(store_failure)?;
        self.start_reserved(reservation, &turn_id, &run);
        Ok(StartedReply {
            turn_id,
            run_id: run.id,
        })
    }

    /// Stops the active reply of a conversation and waits until it has
    /// ended, up to the stop timeout. Stopping a reply that already ended
    /// changes nothing.
    pub async fn stop_reply(&self, conversation: &ConversationId) -> StopOutcome {
        let running = {
            let busy = lock(&self.inner.busy);
            match busy.get(conversation) {
                Some(Busy::Running(handle)) => Some((handle.mailbox.clone(), handle.ended.clone())),
                _ => None,
            }
        };
        let Some((mailbox, mut ended)) = running else {
            // The conversation of an executor's task runs on its target.
            if self.stop_remote_task(conversation).await {
                return StopOutcome::Stopped;
            }
            return StopOutcome::NotRunning;
        };
        // A full mailbox already holds a command that ends the run.
        let _delivered = mailbox.try_send(crate::actor::Command::Stop);
        // No background command's end wakes the conversation until the
        // owner's next message (ADR-0041).
        self.inner.wake_ups.pause(conversation);
        let waited = tokio::time::timeout(
            self.inner.config.stop_timeout,
            ended.wait_for(|ended| *ended),
        )
        .await;
        if waited.is_err() {
            tracing::warn!(conversation = %conversation, "a stopped run did not end in time");
        }
        StopOutcome::Stopped
    }

    /// Renames a conversation.
    pub async fn rename_conversation(&self, input: RenameConversation) -> Result<(), ChatError> {
        let title = check_title(&input.title)?;
        self.refuse_when_task_conversation(&input.conversation_id)
            .await?;
        self.inner
            .hubs
            .apply(
                &self.inner.store,
                &input.conversation_id,
                vec![Change::Rename { title }],
                Durability::Full,
            )
            .await
            .map_err(store_failure)?;
        Ok(())
    }

    /// Chooses the profile of a conversation's next replies, or the default.
    pub async fn set_conversation_profile(
        &self,
        input: SetConversationProfile,
    ) -> Result<(), ChatError> {
        self.refuse_when_task_conversation(&input.conversation_id)
            .await?;
        if let Some(id) = &input.profile_id {
            self.inner
                .store
                .profile(id)
                .await
                .map_err(|error| profile_store_failure(&error))?
                .ok_or(ChatError::ProfileNotFound)?;
        }
        self.inner
            .hubs
            .apply(
                &self.inner.store,
                &input.conversation_id,
                vec![Change::SetProfile {
                    profile_id: input.profile_id,
                }],
                Durability::Full,
            )
            .await
            .map_err(store_failure)?;
        Ok(())
    }

    /// Deletes a conversation with everything in it, its children's task
    /// conversations too. An active reply is stopped first, which stops its
    /// children, and the background commands of each end with it.
    pub async fn delete_conversation(
        &self,
        conversation: &ConversationId,
    ) -> Result<(), ChatError> {
        self.refuse_when_task_conversation(conversation).await?;
        let running = {
            let busy = lock(&self.inner.busy);
            match busy.get(conversation) {
                Some(Busy::Compacting) => return Err(ChatError::Compacting),
                Some(Busy::Starting) => return Err(ChatError::RunActive),
                Some(Busy::Running(_)) => true,
                None => false,
            }
        };
        if running {
            self.stop_reply(conversation).await;
        }
        let children = self
            .inner
            .store
            .task_conversations(conversation)
            .await
            .map_err(store_failure)?;
        let deleted = self
            .inner
            .store
            .delete_conversation(conversation)
            .await
            .map_err(store_failure)?;
        if !deleted {
            return Err(ChatError::ConversationNotFound);
        }
        let mut forgotten = vec![conversation.clone()];
        forgotten.extend(children.iter().cloned());
        self.forget_background_commands(&forgotten).await;
        self.inner.hubs.deleted(conversation).await;
        for child in &children {
            self.inner.hubs.ended_task_conversation(child).await;
        }
        // The deletions of the conversations of executors' tasks wait in
        // the outbox until each executor takes them.
        self.inner.executors.notify_outbox();
        tracing::info!(conversation = %conversation, task_conversations = children.len(), "deleted a conversation");
        Ok(())
    }

    /// Compacts a conversation now, at the owner's request.
    pub async fn compact_conversation(
        &self,
        conversation: &ConversationId,
    ) -> Result<(), ChatError> {
        self.refuse_when_quitting()?;
        let _reservation = self.reserve(conversation, Busy::Compacting)?;
        let summary = self
            .inner
            .store
            .conversation_summary(conversation)
            .await
            .map_err(store_failure)?
            .ok_or(ChatError::ConversationNotFound)?;
        refuse_task_conversation(&summary)?;
        let profile = self.profile_for(summary.profile_id.as_ref()).await?;
        let setup = self
            .setup_for(&profile, CONVERSATION_AGENT, false)
            .await
            .map_err(|failure| ChatError::CompactionFailed { failure })?;
        let prepared = self
            .prepare_call(conversation, conversation, &setup, CallOptions::default())
            .await
            .map_err(call_failure)?;
        if prepared.turns.len() < 2 {
            return Err(ChatError::NothingToCompact);
        }
        let slots = Arc::clone(&self.inner.slots);
        // The runtime never closes the semaphore.
        let _permit = slots
            .acquire_owned()
            .await
            .map_err(|_closed| ChatError::ShuttingDown)?;
        let written = self
            .write_checkpoint(
                conversation,
                &setup,
                &prepared,
                Trigger::Manual,
                prepared.size,
            )
            .await
            .map_err(call_failure)?;
        if written {
            Ok(())
        } else {
            Err(ChatError::NothingToCompact)
        }
    }

    /// Refuses a command of the owner on a task conversation. A conversation
    /// that does not exist is left to the command, which reports it.
    async fn refuse_when_task_conversation(
        &self,
        conversation: &ConversationId,
    ) -> Result<(), ChatError> {
        let summary = self
            .inner
            .store
            .conversation_summary(conversation)
            .await
            .map_err(store_failure)?;
        match summary {
            Some(summary) => refuse_task_conversation(&summary),
            None => Ok(()),
        }
    }

    /// The owner's turns before `before_position`, page by page. The turns of
    /// an executor's task come from its target.
    pub async fn earlier_turns(&self, input: LoadEarlierTurns) -> Result<EarlierTurns, ChatError> {
        if let Some(remote) = self
            .remote_conversation(&input.conversation_id)
            .await
            .map_err(store_failure)?
        {
            return remote
                .chat_executor()?
                .earlier_turns(&remote.target, input)
                .await;
        }
        let limit = input.limit.clamp(1, MAX_EARLIER_TURNS);
        let (turns, has_earlier_turns) = self
            .inner
            .store
            .earlier_turns(&input.conversation_id, input.before_position, limit)
            .await
            .map_err(store_failure)?;
        Ok(EarlierTurns {
            turns,
            has_earlier_turns,
        })
    }

    /// A checkpoint with its summary.
    pub async fn checkpoint(&self, input: CheckpointRef) -> Result<Checkpoint, ChatError> {
        if let Some(remote) = self
            .remote_conversation(&input.conversation_id)
            .await
            .map_err(store_failure)?
        {
            return remote
                .chat_executor()?
                .checkpoint(&remote.target, input)
                .await;
        }
        self.inner
            .store
            .checkpoint(&input.conversation_id, &input.checkpoint_id)
            .await
            .map_err(store_failure)?
            .ok_or(ChatError::CheckpointNotFound)
    }

    /// The owner's chat settings.
    pub async fn chat_settings(&self) -> Result<ChatSettings, ChatError> {
        self.inner
            .store
            .load_settings::<ChatSettings>()
            .await
            .map_err(|error| settings_failure(&error))
    }

    /// Saves the owner's chat settings. Replies use them from their next
    /// call.
    pub async fn update_chat_settings(
        &self,
        input: UpdateChatSettings,
    ) -> Result<ChatSettings, ChatError> {
        let settings = input.into_settings()?;
        self.inner
            .store
            .save_settings(&settings)
            .await
            .map_err(|error| settings_failure(&error))?;
        Ok(settings)
    }

    /// Follows a conversation from a cursor, or from a snapshot. The
    /// conversation of an executor's task is followed on its target.
    pub async fn subscribe(
        &self,
        input: SubscribeConversation,
    ) -> Result<ConversationSubscription, ChatError> {
        if let Some(remote) = self
            .remote_conversation(&input.conversation_id)
            .await
            .map_err(store_failure)?
        {
            let stream = remote
                .chat_executor()?
                .subscribe(&remote.target, &input.conversation_id, input.after_seq)
                .await?;
            return Ok(ConversationSubscription::remote(stream, remote.summary));
        }
        self.inner
            .hubs
            .subscribe(
                &self.inner.store,
                &input.conversation_id,
                input.after_seq,
                self.inner.config.subscriptions,
            )
            .await
            .map_err(store_failure)?
            .ok_or(ChatError::ConversationNotFound)
    }

    /// Follows the conversation list.
    pub async fn subscribe_list(&self) -> Result<ListSubscription, ChatError> {
        self.inner
            .hubs
            .subscribe_list(&self.inner.store, self.inner.config.subscriptions)
            .await
            .map_err(store_failure)
    }
}

fn call_failure(error: crate::calls::CallError) -> ChatError {
    match error {
        crate::calls::CallError::Failed(failure) => ChatError::CompactionFailed { failure },
        crate::calls::CallError::Store(error) => store_failure(error),
    }
}
