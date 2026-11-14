//! The conversation commands (docs/features/chat/conversations.md,
//! contracts): what the desktop's chat commands call.

use std::sync::Arc;

use mannie_store::{
    Change, Durability, NewConversation, NewMention, NewRun, ProfileStoreError, RuntimeStoreError,
    SettingsStoreError, StoredProfile,
};
use mannie_types::chat::{
    ChatError, ChatSettings, CheckpointRef, EarlierTurns, LoadEarlierTurns, MAX_EARLIER_TURNS,
    OpenDraft, RenameConversation, SendMessage, SetConversationAgent, SetConversationProfile,
    StartConversation, StartedConversation, StartedReply, StopOutcome, SubscribeConversation,
    UpdateChatSettings, check_message, check_title, title_from_message,
};
use mannie_types::context::{Attachment, AttachmentRef, ChangeAttachment};
use mannie_types::ids::{ConversationId, ProfileId, ProjectId};
use mannie_types::images::check_images;
use mannie_types::mentions::{MentionRef, check_mentions};
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
        RuntimeStoreError::TooManyAttachments => ChatError::TooManyAttachments {
            max: u32::try_from(mannie_types::context::MAX_ATTACHMENTS).unwrap_or(u32::MAX),
        },
        other => {
            tracing::error!(error = %other, error.code = other.code(), "a conversation could not be read or saved");
            ChatError::StorageFailed
        }
    }
}

/// Refuses the owner's commands on a task conversation, whose turns its
/// parent's agent writes (ADR-0024, decision 3). Stop works on any
/// conversation.
pub(crate) fn refuse_task_conversation(summary: &ConversationSummary) -> Result<(), ChatError> {
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

    /// A new run of a conversation's replies, with the agent that it names
    /// (ADR-0059, decision 6).
    pub(crate) fn new_run(&self, profile: &StoredProfile, agent_key: Option<&str>) -> NewRun {
        NewRun {
            id: self.inner.ids.run(),
            agent: self.conversation_agent_name(agent_key),
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

    /// Refuses a new conversation in a project that is being removed, as if
    /// the project were gone.
    fn refuse_project_being_removed(&self, project: Option<&ProjectId>) -> Result<(), ChatError> {
        if project.is_some_and(|project| self.is_being_removed(project)) {
            return Err(ChatError::ProjectNotFound);
        }
        Ok(())
    }

    /// The attachments that a new conversation starts with: its home first
    /// and once, the others that it names, and those that its first message
    /// names with "@" (ADR-0050, ADR-0051), each with the label that its
    /// provider describes, and the mentions that the message stores for the
    /// named ones. The providers check the keys before anything is stored, so
    /// a key that a provider refuses leaves the message in the message box.
    async fn starting_attachments(
        &self,
        home: Option<&AttachmentRef>,
        attachments: Vec<AttachmentRef>,
        attaching: &[MentionRef],
    ) -> Result<(Vec<Attachment>, Vec<NewMention>), ChatError> {
        let starting: Vec<AttachmentRef> = home
            .into_iter()
            .cloned()
            .chain(
                attachments
                    .into_iter()
                    .filter(|attachment| Some(attachment) != home),
            )
            .collect();
        let mut described = self.describe_attachments(starting).await?;
        let (named, named_mentions) = self.attach_mentioned(attaching).await?;
        described.extend(named);
        Ok((described, named_mentions))
    }

    /// The profile that a new conversation's reply uses, and the one that the
    /// conversation records: the owner's choice, or else the profile of its
    /// agent, or else none, for the default.
    async fn starting_profile(
        &self,
        chosen: Option<ProfileId>,
        agent_key: Option<&str>,
    ) -> Result<(StoredProfile, Option<ProfileId>), ChatError> {
        let agent_profile = match agent_key {
            Some(key) => {
                self.conversation_agent(key)
                    .ok_or(ChatError::AgentNotFound)?
                    .profile
            }
            None => None,
        };
        if let Some(id) = chosen {
            let profile = self
                .inner
                .store
                .profile(&id)
                .await
                .map_err(|error| profile_store_failure(&error))?
                .ok_or(ChatError::ProfileNotFound)?;
            return Ok((profile, Some(id)));
        }
        let profile = self.profile_for(agent_profile.as_ref()).await?;
        // The agent's profile only when it still exists, which `profile_for`
        // found out.
        let recorded = agent_profile.filter(|id| *id == profile.id);
        Ok((profile, recorded))
    }

    /// Creates a conversation with the owner's first message and starts the
    /// reply. A conversation in a project works in the project's folder, and
    /// one that belongs to an attachment starts with it attached first and
    /// never has a project (ADR-0051).
    pub async fn start_conversation(
        &self,
        input: StartConversation,
    ) -> Result<StartedConversation, ChatError> {
        self.refuse_when_quitting()?;
        check_message(&input.text, !input.images.is_empty())?;
        if input.home.is_some() && input.project_id.is_some() {
            return Err(ChatError::HomeConversation);
        }
        let (attaching, mentions) = self.split_attaching(check_mentions(input.mentions)?);
        let image_refs = check_images(input.images)?;
        // The built-in agent is what no key means, so its name is stored as
        // none.
        let agent_key = input.agent_key.filter(|key| key != CONVERSATION_AGENT);
        let (profile, chosen_profile) = self
            .starting_profile(input.profile_id, agent_key.as_deref())
            .await?;
        self.refuse_project_being_removed(input.project_id.as_ref())?;
        let full_access = self.full_access_for_new_conversations().await;
        let home = input.home;
        let (attachments, named_mentions) = self
            .starting_attachments(home.as_ref(), input.attachments, &attaching)
            .await?;
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
        let mut mentions = mentions;
        mentions.extend(named_mentions);
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
        let run = self.new_run(&profile, agent_key.as_deref());
        let mut changes = vec![Change::CreateConversation {
            conversation: NewConversation {
                id: conversation_id.clone(),
                title,
                profile_id: chosen_profile,
                project_id: input.project_id,
                task: None,
                home,
                agent_key,
            },
        }];
        // In the same transaction, so the reply's first call already follows
        // it (ADR-0029, decision 2).
        if full_access {
            changes.push(Change::SetFullAccess { enabled: true });
        }
        // Before the turn, so the run that answers it gathers what they give
        // (ADR-0046, decision 3; ADR-0050).
        changes.extend(
            attachments
                .into_iter()
                .map(|attachment| Change::Attach { attachment }),
        );
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

    /// Opens the draft conversation of a place (ADR-0068): the one it has,
    /// or a new one with no turns, which the conversation list leaves out
    /// until its first message. The draft gets the attachments and the agent
    /// that the input names, through the checks of `attach_context` and
    /// `set_conversation_agent`.
    pub async fn open_draft(&self, input: OpenDraft) -> Result<ConversationId, ChatError> {
        self.refuse_when_quitting()?;
        if input.home.is_some() && input.project_id.is_some() {
            return Err(ChatError::HomeConversation);
        }
        let conversation_id = {
            // Held from the look-up to the creation, so a place never gets
            // two drafts.
            let _opening = self.inner.opening_drafts.lock().await;
            let existing = self
                .inner
                .store
                .draft_conversation(input.project_id.as_ref(), input.home.as_ref())
                .await
                .map_err(store_failure)?;
            match existing {
                Some(id) => id,
                None => {
                    self.create_draft(input.project_id.clone(), input.home.clone())
                        .await?
                }
            }
        };
        let summary = self
            .inner
            .store
            .conversation_summary(&conversation_id)
            .await
            .map_err(store_failure)?
            .ok_or(ChatError::ConversationNotFound)?;
        for attachment in input.attachments {
            let held = summary
                .attachments
                .iter()
                .any(|held| held.provider == attachment.provider && held.key == attachment.key);
            if !held {
                self.attach_context(ChangeAttachment {
                    conversation_id: conversation_id.clone(),
                    attachment,
                })
                .await?;
            }
        }
        if let Some(agent_key) = input.agent_key {
            let current = summary.agent_key.as_deref().unwrap_or(CONVERSATION_AGENT);
            if agent_key != current {
                self.set_conversation_agent(SetConversationAgent {
                    conversation_id: conversation_id.clone(),
                    agent_key: Some(agent_key),
                })
                .await?;
            }
        }
        Ok(conversation_id)
    }

    /// Creates the draft of a place, as `start_conversation` would create
    /// its conversation, without a turn and with an empty title.
    async fn create_draft(
        &self,
        project_id: Option<ProjectId>,
        home: Option<AttachmentRef>,
    ) -> Result<ConversationId, ChatError> {
        // No profile is chosen yet, so a window without profiles still opens
        // a draft, and its message box says why it cannot send.
        self.refuse_project_being_removed(project_id.as_ref())?;
        let full_access = self.full_access_for_new_conversations().await;
        let (attachments, _mentions) = self
            .starting_attachments(home.as_ref(), Vec::new(), &[])
            .await?;
        let conversation_id = self.inner.ids.conversation();
        let mut changes = vec![
            Change::CreateConversation {
                conversation: NewConversation {
                    id: conversation_id.clone(),
                    title: String::new(),
                    profile_id: None,
                    project_id,
                    task: None,
                    home,
                    agent_key: None,
                },
            },
            Change::MarkDraft,
        ];
        if full_access {
            changes.push(Change::SetFullAccess { enabled: true });
        }
        changes.extend(
            attachments
                .into_iter()
                .map(|attachment| Change::Attach { attachment }),
        );
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
        tracing::info!(conversation = %conversation_id, "a draft conversation was created");
        Ok(conversation_id)
    }

    /// Adds the owner's message to a conversation and starts the reply. A
    /// mention whose provider gives context attaches what it names, and the
    /// message gathers from it (ADR-0050).
    pub async fn send_message(&self, input: SendMessage) -> Result<StartedReply, ChatError> {
        self.refuse_when_quitting()?;
        check_message(&input.text, !input.images.is_empty())?;
        let (attaching, mentions) = self.split_attaching(check_mentions(input.mentions)?);
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
        let (named, named_mentions) = self.attach_mentioned(&attaching).await?;
        let mut mentions = mentions;
        mentions.extend(named_mentions);
        let images = self.resolve_images(&image_refs).await?;
        let run = self.new_run(&profile, summary.agent_key.as_deref());
        let opening = self
            .opening_note(
                &input.conversation_id,
                self.agent_reads_commands(&run.agent),
            )
            .await
            .map_err(store_failure)?;
        let turn_id = self.inner.ids.turn();
        let mut changes: Vec<Change> = Vec::new();
        // The first message makes a draft an ordinary conversation, titled
        // as `start_conversation` titles one (ADR-0068, decision 4).
        if summary.draft {
            let mut title = title_from_message(&input.text);
            if title.is_empty()
                && let Some(first) = images.first()
            {
                title = title_from_message(&first.view.name);
            }
            changes.push(Change::Undraft { title });
        }
        // Before the turn, so the run that answers it gathers from them.
        changes.extend(
            named
                .into_iter()
                .map(|attachment| Change::Attach { attachment }),
        );
        changes.push(Change::StartTurn {
            turn_id: turn_id.clone(),
            user_part_id: self.inner.ids.part(),
            text: input.text,
            run: run.clone(),
            task: None,
            mentions,
            images,
        });
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
        let run = self.new_run(&profile, summary.agent_key.as_deref());
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

    /// Chooses the agent of a conversation's next replies, or the built-in one.
    /// An agent that names a profile sets the conversation's too, which the
    /// owner can still change (ADR-0059, decision 7).
    pub async fn set_conversation_agent(
        &self,
        input: SetConversationAgent,
    ) -> Result<(), ChatError> {
        self.refuse_when_task_conversation(&input.conversation_id)
            .await?;
        let agent_key = input.agent_key.filter(|key| key != CONVERSATION_AGENT);
        let mut profile_change = None;
        if let Some(key) = &agent_key {
            let agent = self
                .conversation_agent(key)
                .ok_or(ChatError::AgentNotFound)?;
            if let Some(profile) = agent.profile {
                let exists = self
                    .inner
                    .store
                    .profile(&profile)
                    .await
                    .map_err(|error| profile_store_failure(&error))?
                    .is_some();
                if exists {
                    profile_change = Some(Change::SetProfile {
                        profile_id: Some(profile),
                    });
                }
            }
        }
        let mut changes = vec![Change::SetAgent { agent_key }];
        changes.extend(profile_change);
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
        self.end_conversation_terminals(&forgotten).await;
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
