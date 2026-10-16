//! The project commands of the chat feature (docs/features/chat/projects.md,
//! contracts): adding, renaming and removing projects, and the move of a
//! general conversation into a project that the tool `move_to_project`
//! makes (ADR-0023).
//!
//! The conversation list's lock is held while a project changes, so the list
//! receives the changes of projects in the order the store made them. A
//! conversation's lock is never taken while the list's is held: `Hubs::apply`
//! takes the conversation's first.

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_store::{Change, Durability, RuntimeStoreError};
use mannie_tools::{FolderError, MoveFailure, ProjectMoves, workspace_folder};
use mannie_types::chat::ChatError;
use mannie_types::ids::{ConversationId, ProjectId};
use mannie_types::projects::{ProjectView, RemoveProject, RenameProject, check_project_name};

use crate::{Busy, Runtime, lock};

/// How often a removal goes through the project's conversations, when
/// conversations joined the project while it deleted them.
const REMOVAL_ROUNDS: usize = 3;

fn store_failure(error: RuntimeStoreError) -> ChatError {
    match error {
        RuntimeStoreError::ProjectNotFound => ChatError::ProjectNotFound,
        RuntimeStoreError::ConversationNotFound => ChatError::ConversationNotFound,
        other => {
            tracing::error!(error = %other, error.code = other.code(), "a project could not be read or saved");
            ChatError::StorageFailed
        }
    }
}

/// The chat error of a folder that cannot be a project's.
pub(crate) fn folder_failure(error: FolderError) -> ChatError {
    match error {
        FolderError::Refused { reason } => ChatError::PathRefused { reason },
        FolderError::NotFound => ChatError::FolderNotFound,
        FolderError::NotAFolder => ChatError::NotAFolder,
        FolderError::Unreadable { kind } => ChatError::FolderUnreadable { kind },
    }
}

/// Checks a folder that the owner chose and resolves its links, on a
/// blocking thread.
async fn resolve_folder(text: String) -> Result<String, ChatError> {
    tokio::task::spawn_blocking(move || workspace_folder(&text))
        .await
        .unwrap_or_else(|error| {
            Err(FolderError::Unreadable {
                kind: error.to_string(),
            })
        })
        .map_err(folder_failure)
}

/// A project that is being removed. New conversations and moves into it are
/// refused until the removal ends, and dropping this ends it.
struct Removal<'a> {
    runtime: &'a Runtime,
    project: ProjectId,
}

impl Drop for Removal<'_> {
    fn drop(&mut self) {
        lock(&self.runtime.inner.removing_projects).remove(&self.project);
    }
}

/// The moves of `move_to_project`: a change of the conversation, stored and
/// published through its hub.
pub(crate) struct Moves {
    pub(crate) runtime: Runtime,
}

impl ProjectMoves for Moves {
    fn move_into(
        &self,
        conversation: ConversationId,
        project: ProjectId,
    ) -> BoxFuture<'static, Result<ProjectView, MoveFailure>> {
        let runtime = self.runtime.clone();
        async move { runtime.move_into(&conversation, &project).await }.boxed()
    }
}

impl Runtime {
    /// Whether a project is being removed.
    pub(crate) fn is_being_removed(&self, project: &ProjectId) -> bool {
        lock(&self.inner.removing_projects).contains(project)
    }

    /// Adds a folder as a project, or returns the project that has the
    /// folder already. The folder must exist, and it is stored with its links
    /// resolved.
    pub async fn add_project(&self, folder: String) -> Result<ProjectView, ChatError> {
        let folder = resolve_folder(folder).await?;
        let mut changes = self.inner.hubs.project_changes().await;
        let added = self
            .inner
            .store
            .add_project(&self.inner.ids.project(), &folder)
            .await
            .map_err(store_failure)?;
        if added.created {
            changes.upserted(added.project.clone());
            tracing::info!(project = %added.project.id, "added a project");
        }
        Ok(added.project)
    }

    /// Gives a project the name the owner chose.
    pub async fn rename_project(&self, input: RenameProject) -> Result<ProjectView, ChatError> {
        let name = check_project_name(&input.name)?;
        let mut changes = self.inner.hubs.project_changes().await;
        let project = self
            .inner
            .store
            .rename_project(&input.project_id, &name)
            .await
            .map_err(store_failure)?
            .ok_or(ChatError::ProjectNotFound)?;
        changes.upserted(project.clone());
        tracing::info!(project = %project.id, "renamed a project");
        Ok(project)
    }

    /// Removes a project with its conversations, when it has as many as the
    /// owner confirmed. A conversation whose reply is starting or being
    /// compacted keeps the project until that ends; a reply that is being
    /// written is stopped, as deleting one conversation stops it.
    pub async fn remove_project(&self, input: RemoveProject) -> Result<(), ChatError> {
        let project = input.project_id;
        let _removal = self.begin_removal(&project)?;
        self.inner
            .store
            .project(&project)
            .await
            .map_err(store_failure)?
            .ok_or(ChatError::ProjectNotFound)?;
        let conversations = self
            .inner
            .store
            .project_conversations(&project)
            .await
            .map_err(store_failure)?;
        let count = u32::try_from(conversations.len()).unwrap_or(u32::MAX);
        if count != input.conversations {
            return Err(ChatError::ProjectChanged {
                conversations: count,
            });
        }
        for _round in 0..REMOVAL_ROUNDS {
            let conversations = self
                .inner
                .store
                .project_conversations(&project)
                .await
                .map_err(store_failure)?;
            self.stop_for_removal(&conversations).await?;
            let deletion = self
                .inner
                .store
                .delete_project(&project)
                .await
                .map_err(store_failure)?
                .ok_or(ChatError::ProjectNotFound)?;
            for conversation in &deletion.conversations {
                self.inner.hubs.deleted(conversation).await;
            }
            for conversation in &deletion.task_conversations {
                self.inner.hubs.ended_task_conversation(conversation).await;
            }
            // The deletions of the conversations of executors' tasks wait
            // in the outbox until each executor takes them.
            self.inner.executors.notify_outbox();
            if deletion.project_deleted {
                self.inner
                    .hubs
                    .project_changes()
                    .await
                    .removed(project.clone());
                tracing::info!(project = %project, conversations = count, "removed a project");
                return Ok(());
            }
            tracing::info!(project = %project, "conversations joined a project while it was removed");
        }
        Err(ChatError::ProjectBusy)
    }

    fn begin_removal(&self, project: &ProjectId) -> Result<Removal<'_>, ChatError> {
        if !lock(&self.inner.removing_projects).insert(project.clone()) {
            return Err(ChatError::ProjectBusy);
        }
        Ok(Removal {
            runtime: self,
            project: project.clone(),
        })
    }

    /// Stops the replies of conversations that a removal deletes. A reply
    /// that is starting, or a compaction, refuses the removal before anything
    /// stops.
    async fn stop_for_removal(&self, conversations: &[ConversationId]) -> Result<(), ChatError> {
        let running: Vec<ConversationId> = {
            let busy = lock(&self.inner.busy);
            let refused = conversations.iter().any(|conversation| {
                matches!(
                    busy.get(conversation),
                    Some(Busy::Starting | Busy::Compacting)
                )
            });
            if refused {
                return Err(ChatError::ProjectBusy);
            }
            conversations
                .iter()
                .filter(|conversation| matches!(busy.get(*conversation), Some(Busy::Running(_))))
                .cloned()
                .collect()
        };
        for conversation in &running {
            self.stop_reply(conversation).await;
        }
        Ok(())
    }

    /// Moves a general conversation into a project, for `move_to_project`.
    async fn move_into(
        &self,
        conversation: &ConversationId,
        project: &ProjectId,
    ) -> Result<ProjectView, MoveFailure> {
        if self.is_being_removed(project) {
            return Err(MoveFailure::ProjectNotFound);
        }
        let applied = self
            .inner
            .hubs
            .apply(
                &self.inner.store,
                conversation,
                vec![Change::SetProject {
                    project_id: project.clone(),
                }],
                Durability::Full,
            )
            .await;
        match applied {
            Ok(_applied) => {}
            Err(RuntimeStoreError::ProjectNotFound) => return Err(MoveFailure::ProjectNotFound),
            Err(RuntimeStoreError::AlreadyInProject) => {
                return Err(MoveFailure::AlreadyInProject {
                    name: self.project_name_of(conversation).await,
                });
            }
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), "could not move a conversation into a project");
                return Err(MoveFailure::Storage);
            }
        }
        tracing::info!(conversation = %conversation, project = %project, "moved a conversation into a project");
        match self.inner.store.project(project).await {
            Ok(Some(view)) => Ok(view),
            Ok(None) => Err(MoveFailure::ProjectNotFound),
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), "could not read the project of a move");
                Err(MoveFailure::Storage)
            }
        }
    }

    /// The name of a conversation's project, for a move that it refused.
    async fn project_name_of(&self, conversation: &ConversationId) -> String {
        let store = &self.inner.store;
        let project = match store.conversation_summary(conversation).await {
            Ok(Some(summary)) => summary.project_id,
            _ => None,
        };
        let view = match project {
            Some(id) => store.project(&id).await.ok().flatten(),
            None => None,
        };
        view.map(|view| view.name).unwrap_or_default()
    }
}
