//! Synthetic conversation events for the store validations (V3): the writer
//! throughput benchmark and the crash tests write them instead of real
//! transcripts, which arrive in M1.
//!
//! The table is created on demand and is not part of the migrated schema, so
//! it never appears in a real database: only the `validation` feature compiles
//! this module, and neither the desktop app nor the worker enables it.

use rusqlite::params;

use crate::error::StoreError;
use crate::{Durability, Store};

const CREATE_TABLE: &str = "CREATE TABLE IF NOT EXISTS validation_events ( \
     conversation TEXT NOT NULL, \
     sequence INTEGER NOT NULL, \
     payload TEXT NOT NULL, \
     created_at INTEGER NOT NULL, \
     PRIMARY KEY (conversation, sequence) \
     ) STRICT";
const INSERT_EVENT: &str = "INSERT INTO validation_events (conversation, sequence, payload, created_at) VALUES (?1, ?2, ?3, ?4)";
const SELECT_SEQUENCES: &str =
    "SELECT sequence FROM validation_events WHERE conversation = ?1 ORDER BY sequence";

/// One synthetic event, shaped like a streamed delta of a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationEvent {
    /// The conversation the event belongs to.
    pub conversation: String,
    /// The event's sequence number within its conversation.
    pub sequence: u32,
    /// The delta text.
    pub payload: String,
}

/// Access to the synthetic event table.
#[derive(Debug, Clone)]
pub struct ValidationEvents {
    store: Store,
}

impl ValidationEvents {
    /// Creates the table when it does not exist.
    pub async fn create(store: &Store) -> Result<Self, StoreError> {
        store
            .write(Durability::Full, |transaction| {
                transaction.execute_batch(CREATE_TABLE)
            })
            .await?;
        Ok(Self {
            store: store.clone(),
        })
    }

    /// Appends events in one transaction with the given durability.
    pub async fn append(
        &self,
        durability: Durability,
        events: Vec<ValidationEvent>,
    ) -> Result<(), StoreError> {
        let created_at = self.store.clock().now().as_millis();
        self.store
            .write(durability, move |transaction| {
                let mut insert = transaction.prepare_cached(INSERT_EVENT)?;
                for event in &events {
                    insert.execute(params![
                        event.conversation,
                        event.sequence,
                        event.payload,
                        created_at
                    ])?;
                }
                Ok(())
            })
            .await
    }

    /// The stored sequence numbers of one conversation, in order.
    pub async fn sequences(&self, conversation: &str) -> Result<Vec<u32>, StoreError> {
        let conversation = conversation.to_owned();
        self.store
            .read(move |connection| {
                let mut select = connection.prepare(SELECT_SEQUENCES)?;
                let rows = select.query_map([conversation], |row| row.get(0))?;
                rows.collect()
            })
            .await
    }

    /// Runs SQLite's integrity check and returns its findings; an intact
    /// database returns `["ok"]`.
    pub async fn integrity_check(&self) -> Result<Vec<String>, StoreError> {
        self.store
            .read(|connection| {
                let mut check = connection.prepare("PRAGMA integrity_check")?;
                let rows = check.query_map([], |row| row.get(0))?;
                rows.collect()
            })
            .await
    }
}
