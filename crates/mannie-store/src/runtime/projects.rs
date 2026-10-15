//! Projects (docs/features/chat/projects.md, data): the folders the owner
//! added, whose conversations work in them (ADR-0023). A conversation refers
//! to its project, so a project is deleted only after its conversations.

use mannie_types::ids::{ConversationId, ProjectId};
use mannie_types::projects::{ProjectView, folder_name};
use mannie_types::transcript::ConversationSummary;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::rows::{millis, parse_id};
use super::{RuntimeStoreError, tasks};
use crate::writer::TransactError;
use crate::{Durability, Store};

const SELECT_PROJECTS: &str =
    "SELECT id, name, folder, created_at FROM runtime_projects ORDER BY created_at, id";
const SELECT_PROJECT: &str =
    "SELECT id, name, folder, created_at FROM runtime_projects WHERE id = ?1";
const SELECT_PROJECT_BY_FOLDER: &str =
    "SELECT id, name, folder, created_at FROM runtime_projects WHERE folder = ?1";
const INSERT_PROJECT: &str = "INSERT INTO runtime_projects (id, name, folder, created_at, updated_at) \
     VALUES (?1, NULL, ?2, ?3, ?3)";
const RENAME_PROJECT: &str = "UPDATE runtime_projects SET name = ?2, updated_at = ?3 WHERE id = ?1";
const SELECT_PROJECT_CONVERSATIONS: &str =
    "SELECT id FROM runtime_conversations WHERE project_id = ?1 ORDER BY created_at, id";
const DELETE_PROJECT_CONVERSATION: &str =
    "DELETE FROM runtime_conversations WHERE id = ?1 AND project_id = ?2";
const SELECT_PROJECT_IN_USE: &str =
    "SELECT EXISTS (SELECT 1 FROM runtime_conversations WHERE project_id = ?1)";
const DELETE_PROJECT: &str = "DELETE FROM runtime_projects WHERE id = ?1";

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    SELECT_PROJECTS,
    SELECT_PROJECT,
    SELECT_PROJECT_BY_FOLDER,
    INSERT_PROJECT,
    RENAME_PROJECT,
    SELECT_PROJECT_CONVERSATIONS,
    DELETE_PROJECT_CONVERSATION,
    SELECT_PROJECT_IN_USE,
    DELETE_PROJECT,
];

/// The project that [`Store::add_project`] returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedProject {
    /// The new project, or the one that already had the folder.
    pub project: ProjectView,
    /// Whether the project is new.
    pub created: bool,
}

/// What [`Store::delete_project`] deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDeletion {
    /// The conversations it deleted.
    pub conversations: Vec<ConversationId>,
    /// The task conversations of those conversations, which were deleted
    /// with them.
    pub task_conversations: Vec<ConversationId>,
    /// Whether it deleted the project too: not when a conversation joined
    /// the project meanwhile.
    pub project_deleted: bool,
}

/// The conversation list as one read saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationList {
    /// Every conversation, the most recent first.
    pub conversations: Vec<ConversationSummary>,
    /// Every project, the oldest first.
    pub projects: Vec<ProjectView>,
}

struct RawProject {
    id: String,
    name: Option<String>,
    folder: String,
    created_at: i64,
}

fn project_from_row(row: &Row<'_>) -> rusqlite::Result<RawProject> {
    Ok(RawProject {
        id: row.get(0)?,
        name: row.get(1)?,
        folder: row.get(2)?,
        created_at: row.get(3)?,
    })
}

impl RawProject {
    fn into_view(self) -> Result<ProjectView, RuntimeStoreError> {
        Ok(ProjectView {
            id: parse_id(&self.id, ProjectId::parse)?,
            name: self.name.unwrap_or_else(|| folder_name(&self.folder)),
            folder: self.folder,
            created_at: millis(self.created_at),
        })
    }
}

/// Every project, the oldest first.
pub(super) fn projects_in(connection: &Connection) -> Result<Vec<ProjectView>, RuntimeStoreError> {
    let raw: Vec<RawProject> = {
        let mut select = connection.prepare_cached(SELECT_PROJECTS)?;
        let rows = select.query_map([], project_from_row)?;
        rows.collect::<Result<_, _>>()?
    };
    raw.into_iter().map(RawProject::into_view).collect()
}

fn project_in(
    connection: &Connection,
    project_id: &str,
) -> Result<Option<ProjectView>, RuntimeStoreError> {
    connection
        .prepare_cached(SELECT_PROJECT)?
        .query_row([project_id], project_from_row)
        .optional()?
        .map(RawProject::into_view)
        .transpose()
}

fn rejected(error: TransactError<RuntimeStoreError>) -> RuntimeStoreError {
    match error {
        TransactError::Rejected(error) => error,
        TransactError::Store(error) => RuntimeStoreError::Store(error),
    }
}

impl Store {
    /// Every project, the oldest first.
    pub async fn projects(&self) -> Result<Vec<ProjectView>, RuntimeStoreError> {
        self.read(|connection| Ok(projects_in(connection))).await?
    }

    /// One project.
    pub async fn project(
        &self,
        project_id: &ProjectId,
    ) -> Result<Option<ProjectView>, RuntimeStoreError> {
        let project = project_id.as_str().to_owned();
        self.read(move |connection| Ok(project_in(connection, &project)))
            .await?
    }

    /// Adds a project for a folder, with full durability. A folder that
    /// already is a project returns that project instead; the caller passes
    /// the folder with its links resolved, so one folder has one form.
    pub async fn add_project(
        &self,
        project_id: &ProjectId,
        folder: &str,
    ) -> Result<AddedProject, RuntimeStoreError> {
        let id = project_id.as_str().to_owned();
        let folder = folder.to_owned();
        let now = self.clock().now().as_millis();
        self.transact(Durability::Full, move |transaction| {
            let existing = transaction
                .prepare_cached(SELECT_PROJECT_BY_FOLDER)?
                .query_row([&folder], project_from_row)
                .optional()?;
            if let Some(existing) = existing {
                let project = existing.into_view().map_err(TransactError::Rejected)?;
                return Ok(AddedProject {
                    project,
                    created: false,
                });
            }
            transaction
                .prepare_cached(INSERT_PROJECT)?
                .execute(params![id, folder, now])?;
            let project = project_in(transaction, &id)
                .map_err(TransactError::Rejected)?
                .ok_or(TransactError::Rejected(RuntimeStoreError::ProjectNotFound))?;
            Ok(AddedProject {
                project,
                created: true,
            })
        })
        .await
        .map_err(rejected)
    }

    /// Gives a project the name the owner chose, already checked. Returns the
    /// project as it is now, or `None` when it does not exist.
    pub async fn rename_project(
        &self,
        project_id: &ProjectId,
        name: &str,
    ) -> Result<Option<ProjectView>, RuntimeStoreError> {
        let id = project_id.as_str().to_owned();
        let name = name.to_owned();
        let now = self.clock().now().as_millis();
        self.transact(Durability::Full, move |transaction| {
            let changed = transaction
                .prepare_cached(RENAME_PROJECT)?
                .execute(params![id, name, now])?;
            if changed == 0 {
                return Ok(None);
            }
            project_in(transaction, &id).map_err(TransactError::Rejected)
        })
        .await
        .map_err(rejected)
    }

    /// The conversations of a project, the oldest first.
    pub async fn project_conversations(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<ConversationId>, RuntimeStoreError> {
        let project = project_id.as_str().to_owned();
        let ids: Vec<String> = self
            .read(move |connection| {
                let mut select = connection.prepare_cached(SELECT_PROJECT_CONVERSATIONS)?;
                let rows = select.query_map([project], |row| row.get(0))?;
                rows.collect()
            })
            .await?;
        ids.iter()
            .map(|id| parse_id(id, ConversationId::parse))
            .collect()
    }

    /// Deletes a project with its conversations, or returns `None` when the
    /// project does not exist.
    ///
    /// Each conversation is deleted in a transaction of its own, so a large
    /// project never holds the writer for long, and the project last. A
    /// conversation that joined the project meanwhile keeps the project, which
    /// the result says. The writer's `secure_delete` overwrites the freed
    /// pages, and emptying the write-ahead log once at the end removes the old
    /// pages from it too.
    pub async fn delete_project(
        &self,
        project_id: &ProjectId,
    ) -> Result<Option<ProjectDeletion>, RuntimeStoreError> {
        let Some(_project) = self.project(project_id).await? else {
            return Ok(None);
        };
        let conversations = self.project_conversations(project_id).await?;
        let mut deleted = Vec::with_capacity(conversations.len());
        let mut task_conversations = Vec::new();
        for conversation in conversations {
            let id = conversation.as_str().to_owned();
            let project = project_id.as_str().to_owned();
            // A conversation's task conversations go with it, through the
            // foreign key; the caller ends their subscriptions too.
            let (count, children) = self
                .transact(Durability::Full, move |transaction| {
                    let children = tasks::task_conversations_in(transaction, &id)
                        .map_err(TransactError::Rejected)?;
                    let count = transaction
                        .prepare_cached(DELETE_PROJECT_CONVERSATION)?
                        .execute([id, project])?;
                    Ok((count, children))
                })
                .await
                .map_err(rejected)?;
            if count > 0 {
                deleted.push(conversation);
                task_conversations.extend(children);
            }
        }
        let project = project_id.as_str().to_owned();
        let project_deleted = self
            .write(Durability::Full, move |transaction| {
                let in_use: bool = transaction
                    .prepare_cached(SELECT_PROJECT_IN_USE)?
                    .query_row([&project], |row| row.get(0))?;
                if in_use {
                    return Ok(false);
                }
                let count = transaction
                    .prepare_cached(DELETE_PROJECT)?
                    .execute([&project])?;
                Ok(count > 0)
            })
            .await?;
        self.flush().await?;
        Ok(Some(ProjectDeletion {
            conversations: deleted,
            task_conversations,
            project_deleted,
        }))
    }
}
