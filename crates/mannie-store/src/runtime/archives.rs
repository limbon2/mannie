//! Archived outputs of tool calls: outputs too large for the transcript,
//! which the model and the owner read back in pages
//! (docs/architecture/agent-runtime.md, context management).

use mannie_types::ids::{ArchiveId, ConversationId};
use rusqlite::{OptionalExtension, Transaction, params};

use super::rows::position;
use super::{NewArchive, RuntimeStoreError};
use crate::Store;

const INSERT_ARCHIVE: &str = "INSERT INTO runtime_archives \
     (id, conversation_id, part_id, name, content, bytes, lines, created_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)";
const SELECT_ARCHIVE: &str = "SELECT name, content, lines FROM runtime_archives \
     WHERE id = ?1 AND conversation_id = ?2";

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[INSERT_ARCHIVE, SELECT_ARCHIVE];

/// An archived output as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredArchive {
    /// What it holds, such as `stdout`.
    pub name: String,
    /// The output.
    pub content: String,
    /// The number of its lines.
    pub lines: u32,
}

/// The number of lines of a text: a last line without a line break counts.
pub(crate) fn count_lines(text: &str) -> usize {
    let breaks = text.bytes().filter(|byte| *byte == b'\n').count();
    if text.is_empty() || text.ends_with('\n') {
        breaks
    } else {
        breaks + 1
    }
}

/// Stores an archive with the tool call's part, in the transaction that
/// updates the part.
pub(crate) fn insert(
    transaction: &Transaction<'_>,
    conversation: &str,
    part_id: &str,
    archive: &NewArchive,
    now: i64,
) -> Result<(), RuntimeStoreError> {
    let bytes = i64::try_from(archive.content.len())
        .map_err(|_out_of_range| RuntimeStoreError::invalid("an archive is too large"))?;
    let lines = i64::try_from(count_lines(&archive.content))
        .map_err(|_out_of_range| RuntimeStoreError::invalid("an archive has too many lines"))?;
    transaction
        .prepare_cached(INSERT_ARCHIVE)?
        .execute(params![
            archive.id.as_str(),
            conversation,
            part_id,
            archive.name,
            archive.content,
            bytes,
            lines,
            now
        ])?;
    Ok(())
}

impl Store {
    /// An archived output of a conversation, or `None` when the conversation
    /// has no such archive.
    pub async fn archive(
        &self,
        conversation_id: &ConversationId,
        archive_id: &ArchiveId,
    ) -> Result<Option<StoredArchive>, RuntimeStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        let archive = archive_id.as_str().to_owned();
        let raw: Option<(String, String, i64)> = self
            .read(move |connection| {
                connection
                    .prepare_cached(SELECT_ARCHIVE)?
                    .query_row([archive, conversation], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })
                    .optional()
            })
            .await?;
        raw.map(|(name, content, lines)| {
            Ok(StoredArchive {
                name,
                content,
                lines: position(lines, "an archive's lines")?,
            })
        })
        .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::count_lines;

    #[test]
    fn lines_count_a_last_line_without_a_break() {
        assert_eq!(count_lines(""), 0);
        assert_eq!(count_lines("one"), 1);
        assert_eq!(count_lines("one\n"), 1);
        assert_eq!(count_lines("one\ntwo"), 2);
        assert_eq!(count_lines("\n\n"), 2);
    }
}
