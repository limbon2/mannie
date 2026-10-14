//! Data for the store's validations and performance checks.
//!
//! - Synthetic conversation events (V3): the writer throughput benchmark and
//!   the crash tests write them. Their table is created on demand and is not
//!   part of the migrated schema, so it never appears in a real database.
//! - Long conversations (NFR-PERF-04): [`seed_conversation`] writes real turns,
//!   so the snapshot benchmark and the end-to-end performance checks open a
//!   conversation of 10,000 parts.
//!
//! Only the `validation` feature compiles this module. Release builds of the
//! desktop app and the worker never enable it.

use mannie_types::ids::{ConversationId, PartId, ProfileId, RunId, TurnId};
use mannie_types::run::{FinishReason, RunOutcome, RunState};
use mannie_types::transcript::PartKind;
use rusqlite::params;

use crate::error::StoreError;
use crate::runtime::{Change, NewConversation, NewPart, NewRun, RuntimeStoreError};
use crate::{Durability, Store};

/// How many turns one transaction of [`seed_conversation`] writes.
const SEED_TURNS_PER_BATCH: u32 = 100;

/// The reply that every seeded turn gets: Markdown of a realistic length.
const SEED_REPLY: &str = r#"Here is a short answer with **some emphasis**, a list and code:

- the first point, which explains a detail
- the second point, with `inline code`

```rust
fn main() {
    println!("hello");
}
```

A closing paragraph that repeats the idea in other words, so that the text is about as long as a typical reply of a chat model, with a [link](https://example.com) at the end."#;

/// Seeds a conversation of `turns` turns, each the owner's message and a
/// completed reply, written with normal durability in batches. `next_id`
/// supplies a new UUID each time it is called.
pub async fn seed_conversation(
    store: &Store,
    conversation: &ConversationId,
    profile: &ProfileId,
    turns: u32,
    mut next_id: impl FnMut() -> String,
) -> Result<(), RuntimeStoreError> {
    let mut written = 0;
    let mut created = false;
    while written < turns {
        let mut changes = Vec::new();
        if !created {
            changes.push(Change::CreateConversation {
                conversation: NewConversation {
                    id: conversation.clone(),
                    title: "A long conversation".to_owned(),
                    profile_id: None,
                },
            });
            created = true;
        }
        for _ in 0..SEED_TURNS_PER_BATCH.min(turns - written) {
            let turn_id = TurnId::parse(&next_id())?;
            let run_id = RunId::parse(&next_id())?;
            let user_part_id = PartId::parse(&next_id())?;
            let reply_id = PartId::parse(&next_id())?;
            changes.push(Change::StartTurn {
                turn_id: turn_id.clone(),
                user_part_id,
                text: format!("Question number {written}: how does this work?"),
                run: NewRun {
                    id: run_id.clone(),
                    agent: "conversation".to_owned(),
                    profile_id: profile.clone(),
                    binding: format!("{profile}:1"),
                },
            });
            changes.push(Change::StartRun {
                run_id: run_id.clone(),
            });
            changes.push(Change::AddPart {
                part: NewPart {
                    id: reply_id,
                    turn_id,
                    run_id: run_id.clone(),
                    attempt: 1,
                    kind: PartKind::Text,
                    failure: None,
                },
                text: SEED_REPLY.to_owned(),
            });
            changes.push(Change::EndRun {
                run_id,
                state: RunState::Completed,
                outcome: RunOutcome {
                    finish: Some(FinishReason::Stop),
                    ..RunOutcome::default()
                },
            });
            written += 1;
        }
        store
            .apply(conversation, changes, Durability::Normal)
            .await?;
    }
    Ok(())
}

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
