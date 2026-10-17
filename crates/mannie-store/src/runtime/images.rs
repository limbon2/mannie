//! The bytes of the images that the owner's messages hold (ADR-0033). An
//! image part's document says what the image is, and this table keeps its
//! bytes and its thumbnail's, which the model's context and `read_image` read.

use mannie_types::ids::{ConversationId, PartId};
use mannie_types::images::{ImageBytes, ImageFormat, ImageSize};
use rusqlite::{OptionalExtension, Transaction, params};

use super::{NewImage, RuntimeStoreError};
use crate::Store;

const INSERT_IMAGE: &str = "INSERT INTO runtime_images \
     (part_id, conversation_id, format, data, thumbnail_format, thumbnail, created_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";
const SELECT_FULL: &str = "SELECT format, data FROM runtime_images \
     WHERE part_id = ?1 AND conversation_id = ?2";
const SELECT_THUMBNAIL: &str = "SELECT thumbnail_format, thumbnail FROM runtime_images \
     WHERE part_id = ?1 AND conversation_id = ?2";

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[INSERT_IMAGE, SELECT_FULL, SELECT_THUMBNAIL];

/// An image's bytes as the store keeps them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredImage {
    /// The format of the bytes.
    pub format: ImageFormat,
    /// The bytes.
    pub data: ImageBytes,
}

/// Stores an image's bytes with its part, in the transaction that starts the
/// turn.
pub(crate) fn insert(
    transaction: &Transaction<'_>,
    conversation: &str,
    image: &NewImage,
    now: i64,
) -> Result<(), RuntimeStoreError> {
    transaction.prepare_cached(INSERT_IMAGE)?.execute(params![
        image.part_id.as_str(),
        conversation,
        image.view.format.as_str(),
        image.data.as_slice(),
        image.thumbnail_format.as_str(),
        image.thumbnail.as_slice(),
        now
    ])?;
    Ok(())
}

impl Store {
    /// The bytes of an image of a conversation, or `None` when the
    /// conversation has no image with this part.
    pub async fn image(
        &self,
        conversation_id: &ConversationId,
        part_id: &PartId,
        size: ImageSize,
    ) -> Result<Option<StoredImage>, RuntimeStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        let part = part_id.as_str().to_owned();
        let statement = match size {
            ImageSize::Thumbnail => SELECT_THUMBNAIL,
            ImageSize::Full => SELECT_FULL,
        };
        let raw: Option<(String, Vec<u8>)> = self
            .read(move |connection| {
                connection
                    .prepare_cached(statement)?
                    .query_row([part, conversation], |row| Ok((row.get(0)?, row.get(1)?)))
                    .optional()
            })
            .await?;
        raw.map(|(format, data)| {
            let format = ImageFormat::parse(&format).ok_or_else(|| {
                RuntimeStoreError::invalid(format!("unknown image format {format:?}"))
            })?;
            Ok(StoredImage {
                format,
                data: ImageBytes::from(data),
            })
        })
        .transpose()
    }
}
