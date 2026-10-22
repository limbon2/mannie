//! Background commands (ADR-0041, docs/features/system-tools/
//! background-commands.md): the record of each command beside the part of
//! the call that started it, which the runtime writes through
//! [`Change::StartBackgroundCommand`] and
//! [`Change::UpdateBackgroundCommand`], and the recovery of the commands
//! that ran when the process ended.
//!
//! [`Change::StartBackgroundCommand`]: super::Change::StartBackgroundCommand
//! [`Change::UpdateBackgroundCommand`]: super::Change::UpdateBackgroundCommand

use mannie_types::ids::{ConversationId, PartId};
use mannie_types::tools::{
    BackgroundCommandDocument, BackgroundCommandView, BackgroundEnding, ProcessRecord,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::rows::{from_json, parse_id, position, to_json};
use super::{Change, DOCUMENT_VERSION, RuntimeStoreError};
use crate::{Durability, Store};

const INSERT_COMMAND: &str = "INSERT INTO runtime_background_commands \
     (conversation_id, number, part_id, source, running, version, document, created_at, updated_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)";
const UPDATE_COMMAND: &str = "UPDATE runtime_background_commands \
     SET running = ?3, version = ?4, document = ?5, updated_at = ?6 \
     WHERE conversation_id = ?1 AND number = ?2 RETURNING part_id";
const SELECT_COMMAND: &str = "SELECT number, part_id, source, version, document \
     FROM runtime_background_commands WHERE conversation_id = ?1 AND number = ?2";
const SELECT_COMMANDS: &str = "SELECT number, part_id, source, version, document \
     FROM runtime_background_commands WHERE conversation_id = ?1 ORDER BY number";
const SELECT_NEXT_NUMBER: &str = "SELECT COALESCE(MAX(number), 0) + 1 \
     FROM runtime_background_commands WHERE conversation_id = ?1";
const SELECT_RUNNING: &str = "SELECT conversation_id, number, part_id, source, version, document \
     FROM runtime_background_commands WHERE running = 1 ORDER BY created_at";

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    INSERT_COMMAND,
    UPDATE_COMMAND,
    SELECT_COMMAND,
    SELECT_COMMANDS,
    SELECT_NEXT_NUMBER,
    SELECT_RUNNING,
];

/// A background command as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBackgroundCommand {
    /// Its number in its conversation.
    pub number: u32,
    /// The part of the call that started it.
    pub part_id: PartId,
    /// `local`, or the plugin that runs it on a remote host.
    pub source: String,
    /// Its record.
    pub document: BackgroundCommandDocument,
}

impl StoredBackgroundCommand {
    /// The command's view.
    pub fn view(&self) -> BackgroundCommandView {
        self.document.view(self.number, self.part_id.clone())
    }
}

/// A background command that ran when the process ended, which the recovery
/// marked as interrupted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredCommand {
    /// Its conversation.
    pub conversation_id: ConversationId,
    /// Its number.
    pub number: u32,
    /// `local`, or the plugin that ran it on a remote host.
    pub source: String,
    /// The shell's process of a local command, whose tree may still run.
    pub process: Option<ProcessRecord>,
}

/// The source of a command that runs on the machine itself.
pub const LOCAL_SOURCE: &str = "local";

struct RawCommand {
    number: i64,
    part_id: String,
    source: String,
    version: i64,
    document: String,
}

fn raw_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawCommand> {
    Ok(RawCommand {
        number: row.get(0)?,
        part_id: row.get(1)?,
        source: row.get(2)?,
        version: row.get(3)?,
        document: row.get(4)?,
    })
}

impl RawCommand {
    fn into_command(self) -> Result<StoredBackgroundCommand, RuntimeStoreError> {
        if self.version != i64::from(DOCUMENT_VERSION) {
            return Err(RuntimeStoreError::invalid(format!(
                "background command {} has document version {}",
                self.number, self.version
            )));
        }
        Ok(StoredBackgroundCommand {
            number: position(self.number, "a background command's number")?,
            part_id: parse_id(&self.part_id, PartId::parse)?,
            source: self.source,
            document: from_json(&self.document, "background command document")?,
        })
    }
}

/// Stores a command's record, in the transaction of its change.
pub(super) fn insert(
    transaction: &Transaction<'_>,
    conversation: &str,
    number: u32,
    part_id: &PartId,
    source: &str,
    document: &BackgroundCommandDocument,
    now: i64,
) -> Result<(), RuntimeStoreError> {
    transaction
        .prepare_cached(INSERT_COMMAND)?
        .execute(params![
            conversation,
            number,
            part_id.as_str(),
            source,
            document.ending.is_none(),
            DOCUMENT_VERSION,
            to_json(document)?,
            now
        ])?;
    Ok(())
}

/// Replaces a command's record, in the transaction of its change, and
/// returns the part of the call that started it.
pub(super) fn update(
    transaction: &Transaction<'_>,
    conversation: &str,
    number: u32,
    document: &BackgroundCommandDocument,
    now: i64,
) -> Result<String, RuntimeStoreError> {
    transaction
        .prepare_cached(UPDATE_COMMAND)?
        .query_row(
            params![
                conversation,
                number,
                document.ending.is_none(),
                DOCUMENT_VERSION,
                to_json(document)?,
                now
            ],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| {
            RuntimeStoreError::invalid(format!("background command {number} does not exist"))
        })
}

/// Marks a command's end as told to the model, in the transaction of the
/// note or the result that tells it.
pub(super) fn tell(
    transaction: &Transaction<'_>,
    conversation: &str,
    number: u32,
    now: i64,
) -> Result<(), RuntimeStoreError> {
    let raw = transaction
        .prepare_cached(SELECT_COMMAND)?
        .query_row(params![conversation, number], raw_from_row)
        .optional()?
        .ok_or_else(|| {
            RuntimeStoreError::invalid(format!("background command {number} does not exist"))
        })?;
    let mut document = raw.into_command()?.document;
    if document.told {
        return Ok(());
    }
    document.told = true;
    update(transaction, conversation, number, &document, now).map(drop)
}

/// The view of one command of a conversation, for an event.
pub(super) fn view(
    connection: &Connection,
    conversation: &str,
    number: u32,
) -> Result<Option<BackgroundCommandView>, RuntimeStoreError> {
    let raw = connection
        .prepare_cached(SELECT_COMMAND)?
        .query_row(params![conversation, number], raw_from_row)
        .optional()?;
    raw.map(|raw| Ok(raw.into_command()?.view())).transpose()
}

/// Every command of a conversation, oldest first.
pub(super) fn commands(
    connection: &Connection,
    conversation: &str,
) -> Result<Vec<StoredBackgroundCommand>, RuntimeStoreError> {
    let mut select = connection.prepare_cached(SELECT_COMMANDS)?;
    let raw = select
        .query_map([conversation], raw_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    raw.into_iter().map(RawCommand::into_command).collect()
}

impl Store {
    /// The background commands of a conversation, oldest first.
    pub async fn background_commands(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<StoredBackgroundCommand>, RuntimeStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        self.read_runtime(move |connection| commands(connection, &conversation))
            .await
    }

    /// One background command of a conversation.
    pub async fn background_command(
        &self,
        conversation_id: &ConversationId,
        number: u32,
    ) -> Result<Option<StoredBackgroundCommand>, RuntimeStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        self.read_runtime(move |connection| {
            let raw = connection
                .prepare_cached(SELECT_COMMAND)?
                .query_row(params![conversation, number], raw_from_row)
                .optional()?;
            raw.map(RawCommand::into_command).transpose()
        })
        .await
    }

    /// The number that the conversation's next background command gets: one
    /// after the highest that it ever had, so a number is never reused.
    pub async fn next_background_number(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<u32, RuntimeStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        self.read_runtime(move |connection| {
            let next: i64 = connection
                .prepare_cached(SELECT_NEXT_NUMBER)?
                .query_row([conversation], |row| row.get(0))?;
            position(next, "a background command's number")
        })
        .await
    }

    /// Marks every background command that ran when the process ended as
    /// interrupted, untold, with an event in its conversation, and returns
    /// them. The runtime calls this once at startup, before any run starts,
    /// whatever its runs did.
    pub async fn recover_background_commands(
        &self,
    ) -> Result<Vec<RecoveredCommand>, RuntimeStoreError> {
        let running = self
            .read_runtime(|connection| {
                let mut select = connection.prepare(SELECT_RUNNING)?;
                let raw = select
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            RawCommand {
                                number: row.get(1)?,
                                part_id: row.get(2)?,
                                source: row.get(3)?,
                                version: row.get(4)?,
                                document: row.get(5)?,
                            },
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(raw)
            })
            .await?;
        let now = self.clock().now();
        let mut recovered = Vec::with_capacity(running.len());
        for (conversation, raw) in running {
            let conversation_id = parse_id(&conversation, ConversationId::parse)?;
            let command = raw.into_command()?;
            let mut document = command.document;
            document.ending = Some(BackgroundEnding::Interrupted);
            document.ended_at = Some(now);
            document.told = false;
            let process = document.process.clone();
            let change = Change::UpdateBackgroundCommand {
                number: command.number,
                document: Box::new(document),
                archive: None,
                announce: true,
            };
            self.apply(&conversation_id, vec![change], Durability::Full)
                .await?;
            tracing::info!(conversation = %conversation_id, number = command.number, "marked a background command as interrupted after a restart");
            recovered.push(RecoveredCommand {
                conversation_id,
                number: command.number,
                source: command.source,
                process,
            });
        }
        Ok(recovered)
    }

    /// Runs a read of the runtime's tables, mapping the store's errors.
    async fn read_runtime<T, F>(&self, read: F) -> Result<T, RuntimeStoreError>
    where
        F: FnOnce(&Connection) -> Result<T, RuntimeStoreError> + Send + 'static,
        T: Send + 'static,
    {
        self.read(move |connection| Ok(read(connection)))
            .await
            .map_err(RuntimeStoreError::Store)?
    }
}
