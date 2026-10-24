//! The images of tool results (ADR-0044). A tool call's document names its
//! images, and this table keeps their bytes, which the model's context, the
//! call's details and the conversation's export read.

use mannie_types::ids::{ConversationId, PartId};
use mannie_types::images::{ImageBytes, ImageFormat};
use rusqlite::{OptionalExtension, Transaction, params};

use super::{NewToolImage, RuntimeStoreError};
use crate::Store;

const INSERT_TOOL_IMAGE: &str = "INSERT INTO runtime_tool_images \
     (part_id, position, conversation_id, format, width, height, data, created_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)";
const SELECT_TOOL_IMAGE: &str = "SELECT format, data FROM runtime_tool_images \
     WHERE part_id = ?1 AND position = ?2 AND conversation_id = ?3";
const SELECT_PART_TOOL_IMAGES: &str = "SELECT position, format, width, height, data \
     FROM runtime_tool_images WHERE part_id = ?1 AND conversation_id = ?2 ORDER BY position";

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    INSERT_TOOL_IMAGE,
    SELECT_TOOL_IMAGE,
    SELECT_PART_TOOL_IMAGES,
];

/// A row of the images of a part: the position, the format's name, the
/// width, the height and the bytes.
type ImageRow = (u32, String, u32, u32, Vec<u8>);

/// An image of a tool's result as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredToolImage {
    /// Its position among the result's images.
    pub position: u32,
    /// The format of the bytes.
    pub format: ImageFormat,
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
    /// The bytes.
    pub data: ImageBytes,
}

/// Stores the images of a call's result with its part, in the transaction
/// that completes the call.
pub(crate) fn insert(
    transaction: &Transaction<'_>,
    conversation: &str,
    part: &str,
    image: &NewToolImage,
    now: i64,
) -> Result<(), RuntimeStoreError> {
    transaction
        .prepare_cached(INSERT_TOOL_IMAGE)?
        .execute(params![
            part,
            image.position,
            conversation,
            image.format.as_str(),
            image.width,
            image.height,
            image.data.as_slice(),
            now
        ])?;
    Ok(())
}

/// The format that the store names, or an error for one that this version
/// does not know.
fn format_of(name: &str) -> Result<ImageFormat, RuntimeStoreError> {
    ImageFormat::parse(name)
        .ok_or_else(|| RuntimeStoreError::invalid(format!("unknown image format {name:?}")))
}

impl Store {
    /// The bytes of an image of a tool call's result, by its position, or
    /// `None` when the conversation has no such image.
    pub async fn tool_image(
        &self,
        conversation_id: &ConversationId,
        part_id: &PartId,
        position: u32,
    ) -> Result<Option<(ImageFormat, ImageBytes)>, RuntimeStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        let part = part_id.as_str().to_owned();
        let raw: Option<(String, Vec<u8>)> = self
            .read(move |connection| {
                connection
                    .prepare_cached(SELECT_TOOL_IMAGE)?
                    .query_row(params![part, position, conversation], |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })
                    .optional()
            })
            .await?;
        raw.map(|(format, data)| Ok((format_of(&format)?, ImageBytes::from(data))))
            .transpose()
    }

    /// The images of the results of these tool call parts, in each part's
    /// order, for the parts of the conversation that have any.
    pub async fn tool_images(
        &self,
        conversation_id: &ConversationId,
        part_ids: &[PartId],
    ) -> Result<Vec<(PartId, Vec<StoredToolImage>)>, RuntimeStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        let parts = part_ids.to_vec();
        let raw: Vec<(PartId, Vec<ImageRow>)> = self
            .read(move |connection| {
                let mut statement = connection.prepare_cached(SELECT_PART_TOOL_IMAGES)?;
                let mut found = Vec::new();
                for part in parts {
                    let rows = statement
                        .query_map(params![part.as_str(), conversation], |row| {
                            Ok((
                                row.get(0)?,
                                row.get(1)?,
                                row.get(2)?,
                                row.get(3)?,
                                row.get(4)?,
                            ))
                        })?
                        .collect::<Result<Vec<_>, _>>()?;
                    if !rows.is_empty() {
                        found.push((part, rows));
                    }
                }
                Ok(found)
            })
            .await?;
        raw.into_iter()
            .map(|(part, rows)| {
                let images = rows
                    .into_iter()
                    .map(|(position, format, width, height, data)| {
                        Ok(StoredToolImage {
                            position,
                            format: format_of(&format)?,
                            width,
                            height,
                            data: ImageBytes::from(data),
                        })
                    })
                    .collect::<Result<Vec<_>, RuntimeStoreError>>()?;
                Ok((part, images))
            })
            .collect()
    }
}
