//! Plans (docs/features/plans.md, data): what an agent saved in a conversation,
//! which outlives it. The tool's write goes through a [`super::Change`], so it
//! shares a transaction with the call that records it (ADR-0060, decision 3).

use mannie_types::clock::UtcMillis;
use mannie_types::ids::{ConversationId, ProjectId};
use rusqlite::{OptionalExtension, Row, params};

use super::RuntimeStoreError;
use super::rows::{millis, parse_id};
use crate::writer::TransactError;
use crate::{Durability, Store};

const SELECT_PLANS: &str = "SELECT number, conversation_id, project_id, title, body, owner_edited, \
     created_at, updated_at FROM runtime_plans ORDER BY updated_at DESC, number DESC";
const SELECT_PLAN: &str = "SELECT number, conversation_id, project_id, title, body, owner_edited, \
     created_at, updated_at FROM runtime_plans WHERE number = ?1";
const SELECT_CONVERSATION_PLAN: &str = "SELECT number, conversation_id, project_id, title, body, \
     owner_edited, created_at, updated_at FROM runtime_plans WHERE conversation_id = ?1";
const EDIT_PLAN: &str = "UPDATE runtime_plans SET title = ?2, body = ?3, owner_edited = 1, \
     updated_at = ?4 WHERE number = ?1";
const DELETE_PLAN: &str = "DELETE FROM runtime_plans WHERE number = ?1";
pub(super) const SAVE_PLAN: &str = "INSERT INTO runtime_plans \
     (conversation_id, project_id, title, body, owner_edited, created_at, updated_at) \
     VALUES (?1, (SELECT project_id FROM runtime_conversations WHERE id = ?1), ?2, ?3, 0, ?4, ?4) \
     ON CONFLICT (conversation_id) DO UPDATE SET title = excluded.title, body = excluded.body, \
     owner_edited = 0, updated_at = excluded.updated_at";
pub(super) const PLAN_EDIT_SEEN: &str =
    "UPDATE runtime_plans SET owner_edited = 0 WHERE conversation_id = ?1";
pub(super) const MOVE_PLAN: &str =
    "UPDATE runtime_plans SET project_id = ?2 WHERE conversation_id = ?1";

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    SELECT_PLANS,
    SELECT_PLAN,
    SELECT_CONVERSATION_PLAN,
    EDIT_PLAN,
    DELETE_PLAN,
    SAVE_PLAN,
    PLAN_EDIT_SEEN,
    MOVE_PLAN,
];

/// A plan as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPlan {
    /// Its number.
    pub number: u32,
    /// The conversation that saved it, while that exists.
    pub conversation_id: Option<ConversationId>,
    /// That conversation's project, while that exists.
    pub project_id: Option<ProjectId>,
    /// Its title.
    pub title: String,
    /// Its text.
    pub body: String,
    /// Whether the owner edited it since the model last saved it.
    pub owner_edited: bool,
    /// When it was first saved.
    pub created_at: UtcMillis,
    /// When it last changed.
    pub updated_at: UtcMillis,
}

struct RawPlan {
    number: i64,
    conversation_id: Option<String>,
    project_id: Option<String>,
    title: String,
    body: String,
    owner_edited: bool,
    created_at: i64,
    updated_at: i64,
}

fn plan_from_row(row: &Row<'_>) -> rusqlite::Result<RawPlan> {
    Ok(RawPlan {
        number: row.get(0)?,
        conversation_id: row.get(1)?,
        project_id: row.get(2)?,
        title: row.get(3)?,
        body: row.get(4)?,
        owner_edited: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

impl RawPlan {
    fn into_plan(self) -> Result<StoredPlan, RuntimeStoreError> {
        Ok(StoredPlan {
            number: u32::try_from(self.number)
                .map_err(|_negative| RuntimeStoreError::invalid("a plan's number"))?,
            conversation_id: self
                .conversation_id
                .as_deref()
                .map(|id| parse_id(id, ConversationId::parse))
                .transpose()?,
            project_id: self
                .project_id
                .as_deref()
                .map(|id| parse_id(id, ProjectId::parse))
                .transpose()?,
            title: self.title,
            body: self.body,
            owner_edited: self.owner_edited,
            created_at: millis(self.created_at),
            updated_at: millis(self.updated_at),
        })
    }
}

fn rejected(error: TransactError<RuntimeStoreError>) -> RuntimeStoreError {
    match error {
        TransactError::Rejected(error) => error,
        TransactError::Store(error) => RuntimeStoreError::Store(error),
    }
}

impl Store {
    /// Every plan, the one that changed last first.
    pub async fn plans(&self) -> Result<Vec<StoredPlan>, RuntimeStoreError> {
        self.read(|connection| {
            let raw: Vec<RawPlan> = {
                let mut select = connection.prepare_cached(SELECT_PLANS)?;
                let rows = select.query_map([], plan_from_row)?;
                rows.collect::<Result<_, _>>()?
            };
            Ok(raw.into_iter().map(RawPlan::into_plan).collect())
        })
        .await?
    }

    /// One plan.
    pub async fn plan(&self, number: u32) -> Result<Option<StoredPlan>, RuntimeStoreError> {
        self.read(move |connection| {
            let raw = connection
                .prepare_cached(SELECT_PLAN)?
                .query_row([number], plan_from_row)
                .optional()?;
            Ok(raw.map(RawPlan::into_plan).transpose())
        })
        .await?
    }

    /// The plan of a conversation.
    pub async fn conversation_plan(
        &self,
        conversation: &ConversationId,
    ) -> Result<Option<StoredPlan>, RuntimeStoreError> {
        let conversation = conversation.as_str().to_owned();
        self.read(move |connection| {
            let raw = connection
                .prepare_cached(SELECT_CONVERSATION_PLAN)?
                .query_row([conversation], plan_from_row)
                .optional()?;
            Ok(raw.map(RawPlan::into_plan).transpose())
        })
        .await?
    }

    /// The owner's edit of a plan, and whether the plan exists.
    pub async fn edit_plan(
        &self,
        number: u32,
        title: &str,
        body: &str,
    ) -> Result<bool, RuntimeStoreError> {
        let title = title.to_owned();
        let body = body.to_owned();
        let now = self.clock().now().as_millis();
        self.transact(Durability::Full, move |transaction| {
            let changed = transaction
                .prepare_cached(EDIT_PLAN)?
                .execute(params![number, title, body, now])?;
            Ok(changed > 0)
        })
        .await
        .map_err(rejected)
    }

    /// Deletes a plan, and returns whether it existed.
    pub async fn delete_plan(&self, number: u32) -> Result<bool, RuntimeStoreError> {
        self.transact(Durability::Full, move |transaction| {
            let changed = transaction.prepare_cached(DELETE_PLAN)?.execute([number])?;
            Ok(changed > 0)
        })
        .await
        .map_err(rejected)
    }
}
