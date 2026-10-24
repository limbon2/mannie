//! Complete, consistent saved conversation records for export. Display views
//! deliberately shorten some bodies, so export reads the original rows.

use mannie_types::export::{
    ConversationRecord, ExportError, ExportWarning, RECORD_VERSION, RecordArchive, RecordImage,
    RecordPart, RecordToolImage, RecordTurn,
};
use mannie_types::ids::{ArchiveId, ConversationId, PartId, RunId, TurnId};
use mannie_types::images::ImageFormat;
use mannie_types::transcript::{Checkpoint, PartState};
use rusqlite::{Connection, params};

use super::RuntimeStoreError;
use super::background::{self, StoredBackgroundCommand};
use super::rows::{self, millis, parse_id, part_from_row, position, turn_from_row};
use crate::Store;

const TURNS: &str = "SELECT id, position, current_run_id, created_at FROM runtime_turns WHERE conversation_id = ?1 ORDER BY position";
const PARTS: &str = "SELECT id, turn_id, run_id, position, kind, audience, state, text, document, created_at FROM runtime_parts WHERE turn_id = ?1 AND (run_id IS NULL OR run_id = ?2 OR run_id IN (SELECT id FROM runtime_runs WHERE turn_id = ?1 AND kept = 1)) AND state <> 'discarded' ORDER BY position";
const RETAINED_RUNS: &str = "SELECT id FROM runtime_runs WHERE turn_id = ?1 AND kept = 1 AND id <> ?2 ORDER BY created_at, id";
const SEQUENCE: &str = "SELECT last_seq FROM runtime_conversations WHERE id = ?1";
const CHECKPOINTS: &str = "SELECT id, summary FROM runtime_checkpoints WHERE conversation_id = ?1 ORDER BY created_at, id";
const IMAGES: &str = "SELECT part_id, format, data FROM runtime_images WHERE conversation_id = ?1";
const TOOL_IMAGES: &str = "SELECT part_id, position, format, data FROM runtime_tool_images \
     WHERE conversation_id = ?1 ORDER BY part_id, position";
const ARCHIVES: &str = "SELECT id, part_id, name, content FROM runtime_archives WHERE conversation_id = ?1 ORDER BY created_at, id";
const CHILDREN: &str =
    "SELECT id FROM runtime_conversations WHERE parent_conversation_id = ?1 ORDER BY task_number";

#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    TURNS,
    PARTS,
    RETAINED_RUNS,
    SEQUENCE,
    CHECKPOINTS,
    IMAGES,
    TOOL_IMAGES,
    ARCHIVES,
    CHILDREN,
];

impl Store {
    /// Captures all current saved content within one read transaction. A missing
    /// conversation returns `None`; missing attachments become explicit warnings.
    pub async fn conversation_record(
        &self,
        id: &ConversationId,
    ) -> Result<Option<ConversationRecord>, RuntimeStoreError> {
        let id = id.clone();
        let store = self.clone();
        self.read(move |connection| {
            let transaction = connection.unchecked_transaction()?;
            let record = record_in(&transaction, &id, store.clock().now());
            transaction.finish()?;
            Ok(record)
        })
        .await?
    }
}

fn record_in(
    connection: &Connection,
    id: &ConversationId,
    captured_at: mannie_types::clock::UtcMillis,
) -> Result<Option<ConversationRecord>, RuntimeStoreError> {
    let Some(conversation) = rows::conversation_summary(connection, id.as_str())? else {
        return Ok(None);
    };
    let through_seq = connection.query_row(SEQUENCE, [id.as_str()], |row| row.get::<_, i64>(0))?;
    let turns = record_turns(connection, id)?;
    let checkpoints = record_checkpoints(connection, id)?;
    let (images, archives) = record_assets(connection, id, &turns)?;
    let tool_images = record_tool_images(connection, id, &turns)?;
    let children = record_children(connection, id)?;
    let warnings = missing_attachments(id, &turns, &images, &archives, &tool_images)?;
    let background_commands = background::commands(connection, id.as_str())?
        .iter()
        .map(StoredBackgroundCommand::view)
        .collect();
    Ok(Some(ConversationRecord {
        version: RECORD_VERSION,
        conversation,
        captured_at,
        through_seq: position(through_seq, "export sequence")?,
        turns,
        checkpoints,
        images,
        tool_images,
        archives,
        children,
        warnings,
        background_commands,
    }))
}

fn record_turns(
    connection: &Connection,
    id: &ConversationId,
) -> Result<Vec<RecordTurn>, RuntimeStoreError> {
    let turn_rows = connection
        .prepare_cached(TURNS)?
        .query_map([id.as_str()], turn_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    let mut turns = Vec::with_capacity(turn_rows.len());
    for turn in turn_rows {
        let run = turn
            .current_run_id
            .as_deref()
            .map(|run| rows::run_view(connection, run))
            .transpose()?
            .flatten();
        let retained_ids = connection
            .prepare_cached(RETAINED_RUNS)?
            .query_map(params![turn.id, turn.current_run_id], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut retained_runs = Vec::new();
        for id in retained_ids {
            if let Some(run) = rows::run_view(connection, &id)? {
                retained_runs.push(run);
            }
        }
        let part_rows = connection
            .prepare_cached(PARTS)?
            .query_map(params![turn.id, turn.current_run_id], part_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        let mut parts = Vec::new();
        for part in part_rows {
            if !part.audience().includes_owner() {
                continue;
            }
            let text = if part.state()? == PartState::Streaming {
                rows::streaming_text(connection, &part.id)?
            } else {
                part.text
            };
            parts.push(RecordPart {
                id: parse_id(&part.id, PartId::parse)?,
                kind: part.kind,
                state: part.state,
                run_id: part
                    .run_id
                    .as_deref()
                    .map(|id| parse_id(id, RunId::parse))
                    .transpose()?,
                position: position(part.position, "export part position")?,
                text,
                document: part.document,
                created_at: millis(part.created_at),
            });
        }
        // The turn view contributes only metadata; complete bodies live beside it.
        let view = mannie_types::transcript::TurnView {
            id: parse_id(&turn.id, TurnId::parse)?,
            position: position(turn.position, "export turn position")?,
            run,
            parts: Vec::new(),
            created_at: millis(turn.created_at),
        };
        turns.push(RecordTurn {
            view,
            retained_runs,
            parts,
        });
    }
    Ok(turns)
}

fn record_checkpoints(
    connection: &Connection,
    id: &ConversationId,
) -> Result<Vec<Checkpoint>, RuntimeStoreError> {
    let mut checkpoints = Vec::new();
    let checkpoint_rows = connection
        .prepare_cached(CHECKPOINTS)?
        .query_map([id.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (checkpoint, summary) in checkpoint_rows {
        if let Some(view) = rows::checkpoint_view(connection, &checkpoint)? {
            checkpoints.push(Checkpoint { view, summary });
        }
    }
    Ok(checkpoints)
}

fn record_assets(
    connection: &Connection,
    id: &ConversationId,
    turns: &[RecordTurn],
) -> Result<(Vec<RecordImage>, Vec<RecordArchive>), RuntimeStoreError> {
    let included = |part: &str| {
        turns
            .iter()
            .any(|turn| turn.parts.iter().any(|entry| entry.id.as_str() == part))
    };
    let mut images = Vec::new();
    let mut select = connection.prepare_cached(IMAGES)?;
    let image_rows = select.query_map([id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Vec<u8>>(2)?,
        ))
    })?;
    for image in image_rows {
        let (part, format, data) = image?;
        if included(&part) {
            images.push(RecordImage {
                part_id: parse_id(&part, PartId::parse)?,
                format: ImageFormat::parse(&format)
                    .ok_or_else(|| RuntimeStoreError::invalid("unknown exported image format"))?,
                data,
            });
        }
    }
    let mut archives = Vec::new();
    let mut select = connection.prepare_cached(ARCHIVES)?;
    let archive_rows = select.query_map([id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    for archive in archive_rows {
        let (archive, part, name, content) = archive?;
        if included(&part) {
            archives.push(RecordArchive {
                id: parse_id(&archive, ArchiveId::parse)?,
                part_id: parse_id(&part, PartId::parse)?,
                name,
                content,
            });
        }
    }
    Ok((images, archives))
}

/// The images of the results of the included tool calls, by part and
/// position (ADR-0044, decision 7).
fn record_tool_images(
    connection: &Connection,
    id: &ConversationId,
    turns: &[RecordTurn],
) -> Result<Vec<RecordToolImage>, RuntimeStoreError> {
    let mut images = Vec::new();
    let mut select = connection.prepare_cached(TOOL_IMAGES)?;
    let rows = select.query_map([id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, u32>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Vec<u8>>(3)?,
        ))
    })?;
    for row in rows {
        let (part, position, format, data) = row?;
        let included = turns
            .iter()
            .any(|turn| turn.parts.iter().any(|entry| entry.id.as_str() == part));
        if included {
            images.push(RecordToolImage {
                part_id: parse_id(&part, PartId::parse)?,
                position,
                format: ImageFormat::parse(&format)
                    .ok_or_else(|| RuntimeStoreError::invalid("unknown exported image format"))?,
                data,
            });
        }
    }
    Ok(images)
}

fn record_children(
    connection: &Connection,
    id: &ConversationId,
) -> Result<Vec<mannie_types::transcript::ConversationSummary>, RuntimeStoreError> {
    let child_ids = connection
        .prepare_cached(CHILDREN)?
        .query_map([id.as_str()], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut children = Vec::new();
    for child in child_ids {
        if let Some(summary) = rows::conversation_summary(connection, &child)? {
            children.push(summary);
        }
    }
    Ok(children)
}

fn missing_attachments(
    id: &ConversationId,
    turns: &[RecordTurn],
    images: &[RecordImage],
    archives: &[RecordArchive],
    tool_images: &[RecordToolImage],
) -> Result<Vec<ExportWarning>, RuntimeStoreError> {
    let mut warnings = Vec::new();
    for part in turns.iter().flat_map(|turn| &turn.parts) {
        if part.kind == "image" && !images.iter().any(|image| image.part_id == part.id) {
            warnings.push(ExportWarning {
                conversation_id: id.clone(),
                reference: part.id.to_string(),
                reason: ExportError::AttachmentMissing,
            });
        }
        let document: serde_json::Value = rows::from_json(&part.document, "export part document")?;
        for references in ["/toolCall/result/archives", "/task/archives"]
            .into_iter()
            .filter_map(|pointer| {
                document
                    .pointer(pointer)
                    .and_then(serde_json::Value::as_array)
            })
        {
            for reference in references {
                if let Some(archive) = reference.get("id").and_then(serde_json::Value::as_str)
                    && !archives.iter().any(|entry| entry.id.as_str() == archive)
                {
                    warnings.push(ExportWarning {
                        conversation_id: id.clone(),
                        reference: archive.to_owned(),
                        reason: ExportError::AttachmentMissing,
                    });
                }
            }
        }
        let named_images = document
            .pointer("/toolCall/result/images")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten();
        for position in named_images.filter_map(|image| image.get("position")?.as_u64()) {
            let stored = tool_images
                .iter()
                .any(|image| image.part_id == part.id && u64::from(image.position) == position);
            if !stored {
                warnings.push(ExportWarning {
                    conversation_id: id.clone(),
                    reference: format!("{}#{position}", part.id),
                    reason: ExportError::AttachmentMissing,
                });
            }
        }
    }
    Ok(warnings)
}
