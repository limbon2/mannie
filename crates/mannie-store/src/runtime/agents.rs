//! The owner's custom agents (docs/features/custom-agents.md, data): each is a
//! document under a key that never changes (ADR-0059). A conversation names an
//! agent by its key, and a key that no row has reads as the built-in agent.

use mannie_types::agents::AgentDraft;
use mannie_types::clock::UtcMillis;
use mannie_types::ids::ConversationId;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::RuntimeStoreError;
use super::rows::{millis, parse_id};
use crate::writer::TransactError;
use crate::{Durability, Store};

const SELECT_AGENTS: &str = "SELECT key, document, created_at, updated_at FROM runtime_agents \
     ORDER BY created_at, key";
const SELECT_AGENT_EXISTS: &str = "SELECT 1 FROM runtime_agents WHERE key = ?1";
const INSERT_AGENT: &str = "INSERT INTO runtime_agents (key, document, created_at, updated_at) \
     VALUES (?1, ?2, ?3, ?3)";
const UPDATE_AGENT: &str =
    "UPDATE runtime_agents SET document = ?2, updated_at = ?3 WHERE key = ?1";
const DELETE_AGENT: &str = "DELETE FROM runtime_agents WHERE key = ?1";
const SELECT_AGENT_CONVERSATIONS: &str =
    "SELECT id FROM runtime_conversations WHERE agent_key = ?1 ORDER BY created_at, id";

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    SELECT_AGENTS,
    SELECT_AGENT_EXISTS,
    INSERT_AGENT,
    UPDATE_AGENT,
    DELETE_AGENT,
    SELECT_AGENT_CONVERSATIONS,
];

/// A custom agent as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAgent {
    /// Its key.
    pub key: String,
    /// What the owner entered.
    pub draft: AgentDraft,
    /// When it was created.
    pub created_at: UtcMillis,
    /// When it last changed.
    pub updated_at: UtcMillis,
}

struct RawAgent {
    key: String,
    document: String,
    created_at: i64,
    updated_at: i64,
}

fn agent_from_row(row: &Row<'_>) -> rusqlite::Result<RawAgent> {
    Ok(RawAgent {
        key: row.get(0)?,
        document: row.get(1)?,
        created_at: row.get(2)?,
        updated_at: row.get(3)?,
    })
}

/// The stored agents, the oldest first. A document that this version cannot
/// read is left out and logged, so one bad row costs its own agent only.
fn agents_in(connection: &Connection) -> Result<Vec<StoredAgent>, RuntimeStoreError> {
    let raw: Vec<RawAgent> = {
        let mut select = connection.prepare_cached(SELECT_AGENTS)?;
        let rows = select.query_map([], agent_from_row)?;
        rows.collect::<Result<_, _>>()?
    };
    let mut agents = Vec::with_capacity(raw.len());
    for row in raw {
        match serde_json::from_str(&row.document) {
            Ok(draft) => agents.push(StoredAgent {
                key: row.key,
                draft,
                created_at: millis(row.created_at),
                updated_at: millis(row.updated_at),
            }),
            Err(error) => {
                tracing::error!(%error, agent = row.key, "a stored agent could not be read");
            }
        }
    }
    Ok(agents)
}

fn document_of(draft: &AgentDraft) -> Result<String, RuntimeStoreError> {
    serde_json::to_string(draft)
        .map_err(|error| RuntimeStoreError::invalid(format!("an agent's document: {error}")))
}

fn rejected(error: TransactError<RuntimeStoreError>) -> RuntimeStoreError {
    match error {
        TransactError::Rejected(error) => error,
        TransactError::Store(error) => RuntimeStoreError::Store(error),
    }
}

impl Store {
    /// The owner's agents, the oldest first.
    pub async fn custom_agents(&self) -> Result<Vec<StoredAgent>, RuntimeStoreError> {
        self.read(|connection| Ok(agents_in(connection))).await?
    }

    /// Stores a new agent.
    pub async fn create_custom_agent(
        &self,
        key: &str,
        draft: &AgentDraft,
    ) -> Result<(), RuntimeStoreError> {
        let key = key.to_owned();
        let document = document_of(draft)?;
        let now = self.clock().now().as_millis();
        self.transact(Durability::Full, move |transaction| {
            let exists = transaction
                .prepare_cached(SELECT_AGENT_EXISTS)?
                .query_row([&key], |_row| Ok(()))
                .optional()?
                .is_some();
            if exists {
                return Err(TransactError::Rejected(RuntimeStoreError::AgentKeyTaken));
            }
            transaction
                .prepare_cached(INSERT_AGENT)?
                .execute(params![key, document, now])?;
            Ok(())
        })
        .await
        .map_err(rejected)
    }

    /// Replaces an agent's document.
    pub async fn update_custom_agent(
        &self,
        key: &str,
        draft: &AgentDraft,
    ) -> Result<(), RuntimeStoreError> {
        let key = key.to_owned();
        let document = document_of(draft)?;
        let now = self.clock().now().as_millis();
        self.transact(Durability::Full, move |transaction| {
            let changed = transaction
                .prepare_cached(UPDATE_AGENT)?
                .execute(params![key, document, now])?;
            if changed == 0 {
                return Err(TransactError::Rejected(RuntimeStoreError::AgentNotFound));
            }
            Ok(())
        })
        .await
        .map_err(rejected)
    }

    /// Deletes an agent, and returns the conversations that name it, which the
    /// caller sets back one by one through their hubs so every window hears of
    /// it (ADR-0059, decision 6).
    pub async fn delete_custom_agent(
        &self,
        key: &str,
    ) -> Result<Vec<ConversationId>, RuntimeStoreError> {
        let key = key.to_owned();
        self.transact(Durability::Full, move |transaction| {
            let deleted = transaction.prepare_cached(DELETE_AGENT)?.execute([&key])?;
            if deleted == 0 {
                return Err(TransactError::Rejected(RuntimeStoreError::AgentNotFound));
            }
            let ids: Vec<String> = {
                let mut select = transaction.prepare_cached(SELECT_AGENT_CONVERSATIONS)?;
                let rows = select.query_map([&key], |row| row.get(0))?;
                rows.collect::<Result<_, _>>()?
            };
            ids.iter()
                .map(|id| parse_id(id, ConversationId::parse).map_err(TransactError::Rejected))
                .collect()
        })
        .await
        .map_err(rejected)
    }
}
