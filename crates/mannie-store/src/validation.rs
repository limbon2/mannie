//! Data for the store's validations and performance checks.
//!
//! - Synthetic conversation events (V3): the writer throughput benchmark and
//!   the crash tests write them. Their table is created on demand and is not
//!   part of the migrated schema, so it never appears in a real database.
//! - Long conversations (NFR-PERF-04): [`seed_conversation`] writes real turns,
//!   so the snapshot benchmark and the end-to-end performance checks open a
//!   conversation of 10,000 parts, and [`seed_conversation_with_tool_calls`]
//!   adds a completed tool call to each turn.
//!
//! Only the `validation` feature compiles this module. Release builds of the
//! desktop app and the worker never enable it.

use std::fmt::Write as _;

use mannie_types::clock::UtcMillis;
use mannie_types::ids::{ConversationId, PartId, ProfileId, RunId, TurnId};
use mannie_types::run::{FinishReason, RunOutcome, RunState};
use mannie_types::tools::{
    AccessMode, Decision, ToolCallDocument, ToolCallStatus, ToolOutcome, ToolResult,
};
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

/// How many numbered lines the result of a seeded tool call holds.
const SEED_RESULT_LINES: usize = 60;

/// Seeds a conversation of `turns` turns, each the owner's message and a
/// completed reply, written with normal durability in batches. `next_id`
/// supplies a new UUID each time it is called.
pub async fn seed_conversation(
    store: &Store,
    conversation: &ConversationId,
    profile: &ProfileId,
    turns: u32,
    next_id: impl FnMut() -> String,
) -> Result<(), RuntimeStoreError> {
    seed(store, conversation, profile, turns, false, next_id).await
}

/// Seeds a conversation as [`seed_conversation`] does, with a completed
/// `read_file` call before each reply, whose result is a page of a file.
pub async fn seed_conversation_with_tool_calls(
    store: &Store,
    conversation: &ConversationId,
    profile: &ProfileId,
    turns: u32,
    next_id: impl FnMut() -> String,
) -> Result<(), RuntimeStoreError> {
    seed(store, conversation, profile, turns, true, next_id).await
}

/// The document of a seeded tool call: a read of a file, allowed by the
/// workspace, with a page of numbered lines as its result.
fn seeded_call(call_id: String) -> ToolCallDocument {
    let path = "/home/owner/projects/app/src/main.rs".to_owned();
    let mut output = format!("File: {path}\nRevision: sha256:{}\n", "0".repeat(64));
    for line in 1..=SEED_RESULT_LINES {
        writeln!(output, "{line:6}\tlet value_{line} = compute({line});")
            .expect("writing to a String cannot fail");
    }
    let at = UtcMillis::from_millis(1_791_892_800_000);
    let total_lines = u32::try_from(SEED_RESULT_LINES).unwrap_or(u32::MAX);
    ToolCallDocument {
        status: ToolCallStatus::Completed,
        accesses: vec![mannie_types::tools::Access::Path {
            path: path.clone(),
            mode: AccessMode::Read,
            folder: false,
            exists: true,
            identity: "2049:131074".to_owned(),
            entries: None,
            state: None,
        }],
        decision: Some(Decision::Workspace),
        decided_at: Some(at),
        started_at: Some(at),
        ended_at: Some(at),
        result: Some(ToolResult {
            outcome: ToolOutcome::Read {
                path,
                first_line: 1,
                last_line: total_lines,
                total_lines,
            },
            output,
            archives: Vec::new(),
            images: Vec::new(),
        }),
        ..ToolCallDocument::receiving(call_id, "read_file".to_owned())
    }
}

async fn seed(
    store: &Store,
    conversation: &ConversationId,
    profile: &ProfileId,
    turns: u32,
    tool_calls: bool,
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
                    project_id: None,
                    task: None,
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
                mentions: Vec::new(),
                images: Vec::new(),
                text: format!("Question number {written}: how does this work?"),
                run: NewRun {
                    id: run_id.clone(),
                    agent: "conversation".to_owned(),
                    profile_id: profile.clone(),
                    binding: format!("{profile}:1"),
                },
                task: None,
            });
            changes.push(Change::StartRun {
                run_id: run_id.clone(),
            });
            if tool_calls {
                let call_id = next_id();
                changes.push(Change::AddPart {
                    part: NewPart {
                        id: PartId::parse(&next_id())?,
                        turn_id: turn_id.clone(),
                        run_id: run_id.clone(),
                        attempt: 1,
                        kind: PartKind::ToolCall,
                        failure: None,
                        tool_call: Some(seeded_call(format!("call_{call_id}"))),
                    },
                    text: r#"{"path":"src/main.rs","offset":null,"limit":null}"#.to_owned(),
                });
            }
            changes.push(Change::AddPart {
                part: NewPart {
                    id: reply_id,
                    turn_id,
                    run_id: run_id.clone(),
                    attempt: if tool_calls { 2 } else { 1 },
                    kind: PartKind::Text,
                    failure: None,
                    tool_call: None,
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
                task: None,
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
