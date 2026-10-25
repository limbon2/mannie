//! The attachments of conversations (ADR-0046, decision 1): keys of context
//! providers with their labels, which the runtime writes through
//! [`Change::Attach`] and [`Change::Detach`], and which a conversation's
//! summary reads through [`summary_columns`].
//!
//! [`Change::Attach`]: super::Change::Attach
//! [`Change::Detach`]: super::Change::Detach
//! [`summary_columns`]: super::rows::summary_columns

use mannie_types::context::{Attachment, AttachmentRef, MAX_ATTACHMENTS};
use rusqlite::{Transaction, params};

use super::RuntimeStoreError;

const RELABEL_ATTACHMENT: &str = "UPDATE runtime_conversation_attachments SET label = ?4 \
     WHERE conversation_id = ?1 AND provider = ?2 AND attachment_key = ?3";
const COUNT_ATTACHMENTS: &str =
    "SELECT COUNT(*) FROM runtime_conversation_attachments WHERE conversation_id = ?1";
const INSERT_ATTACHMENT: &str = "INSERT INTO runtime_conversation_attachments \
     (conversation_id, provider, attachment_key, position, label, created_at) \
     VALUES (?1, ?2, ?3, (SELECT COALESCE(MAX(position) + 1, 0) \
     FROM runtime_conversation_attachments WHERE conversation_id = ?1), ?4, ?5)";
const DELETE_ATTACHMENT: &str = "DELETE FROM runtime_conversation_attachments \
     WHERE conversation_id = ?1 AND provider = ?2 AND attachment_key = ?3";

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    RELABEL_ATTACHMENT,
    COUNT_ATTACHMENTS,
    INSERT_ATTACHMENT,
    DELETE_ATTACHMENT,
];

/// Attaches a provider's key to a conversation after its other attachments,
/// or gives an attachment that it holds already its new label, in the
/// transaction of the change. Refused when the conversation holds
/// [`MAX_ATTACHMENTS`] others.
pub(super) fn attach(
    transaction: &Transaction<'_>,
    conversation: &str,
    attachment: &Attachment,
    now: i64,
) -> Result<(), RuntimeStoreError> {
    let relabeled = transaction
        .prepare_cached(RELABEL_ATTACHMENT)?
        .execute(params![
            conversation,
            attachment.provider,
            attachment.key,
            attachment.label
        ])?;
    if relabeled > 0 {
        return Ok(());
    }
    let count: i64 = transaction
        .prepare_cached(COUNT_ATTACHMENTS)?
        .query_row([conversation], |row| row.get(0))?;
    let full = usize::try_from(count).map_or(true, |count| count >= MAX_ATTACHMENTS);
    if full {
        return Err(RuntimeStoreError::TooManyAttachments);
    }
    transaction
        .prepare_cached(INSERT_ATTACHMENT)?
        .execute(params![
            conversation,
            attachment.provider,
            attachment.key,
            attachment.label,
            now
        ])?;
    Ok(())
}

/// Detaches a provider's key from a conversation. Returns whether the
/// conversation held it.
pub(super) fn detach(
    transaction: &Transaction<'_>,
    conversation: &str,
    attachment: &AttachmentRef,
) -> Result<bool, RuntimeStoreError> {
    let deleted = transaction
        .prepare_cached(DELETE_ATTACHMENT)?
        .execute(params![conversation, attachment.provider, attachment.key])?;
    Ok(deleted > 0)
}
