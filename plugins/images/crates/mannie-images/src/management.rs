//! Self-management provider for the Images plugin (ADR-0081).
//!
//! Images keeps only an in-memory shelf of waiting attachments cleared when the
//! plugin stops, so this provider is read-only.

use std::sync::Arc;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::MAX_IMAGE_BYTES;
use mannie_plugin_sdk::points::management::{
    ActionResult, ExecuteActionRequest, ManagementError, ManagementGuide, ManagementProvider,
    ManagementSnapshot, PrepareActionRequest, PreparedAction,
};
use serde_json::json;

use crate::commands::{EXTENSIONS, MAX_DECODED_MEGAPIXELS, MAX_DECODED_SIDE, MAX_FILE_BYTES};
use crate::shelf::Shelf;

/// Read-only management provider for the Images plugin.
#[derive(Debug, Clone)]
pub(crate) struct ImagesManagement {
    shelf: Arc<Shelf>,
}

impl ImagesManagement {
    /// Creates a management provider backed by the running shelf.
    pub(crate) fn new(shelf: Arc<Shelf>) -> Self {
        Self { shelf }
    }
}

impl ManagementProvider for ImagesManagement {
    fn guide(&self) -> ManagementGuide {
        ManagementGuide {
            summary: "Lets the owner attach PNG, JPEG, GIF, and WebP images to chat messages by \
                picking files through the system dialog, pasting, or dropping them into the \
                message box. Decodes and resizes images in Rust before handing them to the \
                runtime."
                .to_owned(),
            storage: "Stores no SQLite tables, settings, or credentials. Pending picked files and \
                prepared images stay in an in-memory shelf that is cleared whenever the plugin \
                stops; once sent, message attachments are stored by the core."
                .to_owned(),
            troubleshooting:
                "Files larger than 20 MiB, images wider or taller than 12,000 pixels, \
                or images above 64 megapixels are rejected before delivery. If a chosen image \
                reports not_found, the plugin was restarted or the shelf evicted the oldest entry \
                and the owner must attach the image again."
                    .to_owned(),
            actions: Vec::new(),
        }
    }

    fn inspect(
        &self,
        query: Option<String>,
    ) -> BoxFuture<'static, Result<ManagementSnapshot, ManagementError>> {
        let storage = Arc::clone(&self.shelf);

        Box::pin(async move {
            if let Some(query) = query
                && !query.trim().is_empty()
            {
                return Err(ManagementError::invalid_field(
                    "query",
                    format!("images does not support inspect query {query:?}"),
                ));
            }

            let (picked_count, prepared_count) = storage.counts();
            let summary = format!(
                "Images plugin is on ({picked_count} picked file(s), {prepared_count} prepared image(s) waiting on shelf)."
            );

            let data = json!({
                "supportedExtensions": EXTENSIONS,
                "maxFileBytes": MAX_FILE_BYTES,
                "maxEncodedImageBytes": MAX_IMAGE_BYTES,
                "maxDecodedSidePixels": MAX_DECODED_SIDE,
                "maxDecodedMegapixels": MAX_DECODED_MEGAPIXELS,
                "pickedCount": picked_count,
                "preparedCount": prepared_count,
                "storesCredentials": false,
                "storesTables": false,
            });

            Ok(ManagementSnapshot::new(summary, data))
        })
    }

    fn prepare_action(
        &self,
        request: PrepareActionRequest,
    ) -> BoxFuture<'static, Result<PreparedAction, ManagementError>> {
        Box::pin(std::future::ready(Err(ManagementError::unknown_action(
            &request.action,
        ))))
    }

    fn execute_action(
        &self,
        request: ExecuteActionRequest,
    ) -> BoxFuture<'static, Result<ActionResult, ManagementError>> {
        Box::pin(std::future::ready(Err(ManagementError::unknown_action(
            &request.action,
        ))))
    }
}
