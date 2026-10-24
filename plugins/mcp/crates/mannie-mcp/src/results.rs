//! The results of the servers' tools as the model reads them
//! (plugins/mcp/docs/README.md, what the model reads): text as it is, PNG
//! and JPEG images as images (ADR-0044) when the owner lets the server's
//! images through, audio and other binary content as a note, resource links
//! with their URIs, and embedded text resources inline. Structured content
//! becomes JSON when the result has no text, and a result that the tool
//! marks as an error fails.
//!
//! Every secret of the server is replaced in the text before it leaves the
//! plugin. Text past the result's room is archived whole for
//! `read_tool_output`, and the result keeps its start and its end.

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use mannie_plugin_sdk::contracts::{ArchiveId, MAX_TOOL_IMAGE_BYTES, MAX_TOOL_IMAGES};
use mannie_plugin_sdk::points::tools::{
    MAX_TOOL_ARCHIVE_BYTES, ToolArchive, ToolImage, ToolResult,
};
use rmcp::model::{CallToolResult, ContentBlock, ReadResourceResult, ResourceContents};

use crate::log::Redactions;
use crate::servers::cut;
use crate::texts;

/// The share of the room that the start of a long text takes, in thirds.
const HEAD_THIRDS: usize = 2;

/// Where a result goes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Room<'a> {
    /// The most bytes of the result's text.
    pub(crate) result_limit: usize,
    /// The IDs that the core reserved for archives.
    pub(crate) archive_ids: &'a [ArchiveId],
}

/// What a result is made of, before it fits its room.
#[derive(Debug, Default)]
struct Parts {
    texts: Vec<String>,
    images: Vec<ToolImage>,
    has_text: bool,
}

impl Parts {
    fn text(&mut self, text: String) {
        self.has_text = true;
        self.texts.push(text);
    }

    fn note(&mut self, note: String) {
        self.texts.push(note);
    }

    /// An image for the model, or a note when it cannot go.
    fn image(&mut self, data: &str, mime_type: &str, images_allowed: bool) {
        if !images_allowed {
            self.note(texts::image_left_out(mime_type, texts::IMAGES_OFF));
            return;
        }
        if !matches!(mime_type, "image/png" | "image/jpeg" | "image/jpg") {
            self.note(texts::image_left_out(mime_type, texts::IMAGE_TYPE));
            return;
        }
        if self.images.len() >= MAX_TOOL_IMAGES {
            self.note(texts::image_left_out(
                mime_type,
                &texts::image_past_limit(MAX_TOOL_IMAGES),
            ));
            return;
        }
        let Some(bytes) = decode(data) else {
            self.note(texts::image_left_out(mime_type, texts::IMAGE_DATA));
            return;
        };
        if bytes.len() > MAX_TOOL_IMAGE_BYTES {
            // The core leaves it out with a note of its own.
            tracing::debug!(
                bytes = bytes.len(),
                "an MCP server's image is larger than a result takes"
            );
        }
        self.images.push(ToolImage::new(bytes));
    }

    /// One block of content.
    fn block(&mut self, block: ContentBlock, images_allowed: bool) {
        match block {
            ContentBlock::Text(text) => self.text(text.text),
            ContentBlock::Image(image) => self.image(&image.data, &image.mime_type, images_allowed),
            ContentBlock::Audio(audio) => {
                let bytes = decode(&audio.data).map_or(0, |bytes| bytes.len());
                self.note(texts::audio(&audio.mime_type, bytes));
            }
            ContentBlock::ResourceLink(resource) => {
                self.note(texts::resource_link(&resource.name, &resource.uri));
            }
            ContentBlock::Resource(embedded) => self.contents(embedded.resource, images_allowed),
            _other => {}
        }
    }

    /// The contents of a resource: text inline, an image as an image, and
    /// other binary content as a note.
    fn contents(&mut self, contents: ResourceContents, images_allowed: bool) {
        match contents {
            ResourceContents::TextResourceContents { uri, text, .. } => {
                self.text(texts::resource_text(&uri, &text));
            }
            ResourceContents::BlobResourceContents {
                uri,
                mime_type,
                blob,
                ..
            } => {
                let mime_type = mime_type.unwrap_or_else(|| "application/octet-stream".to_owned());
                if mime_type.starts_with("image/") {
                    self.image(&blob, &mime_type, images_allowed);
                } else {
                    let bytes = decode(&blob).map_or(0, |bytes| bytes.len());
                    self.note(texts::resource_binary(&uri, &mime_type, bytes));
                }
            }
            _other => {}
        }
    }
}

/// The bytes of base64 data, with or without padding.
fn decode(data: &str) -> Option<Vec<u8>> {
    let data = data.trim();
    STANDARD
        .decode(data)
        .or_else(|_padded| STANDARD_NO_PAD.decode(data))
        .ok()
}

/// A tool's result, as the model reads it.
pub(crate) fn render(
    result: CallToolResult,
    images_allowed: bool,
    redactions: &Redactions,
    room: Room<'_>,
) -> ToolResult {
    let is_error = result.is_error == Some(true);
    let mut parts = Parts::default();
    for block in result.content {
        parts.block(block, images_allowed);
    }
    if !parts.has_text
        && let Some(structured) = result.structured_content
    {
        let json = serde_json::to_string_pretty(&structured).unwrap_or_default();
        parts.text(format!("{}\n{json}", texts::STRUCTURED));
    }
    let mut lines = Vec::with_capacity(parts.texts.len() + 1);
    if is_error {
        lines.push(texts::TOOL_ERROR_TEXT.to_owned());
    }
    if parts.texts.is_empty() {
        lines.push(texts::NO_CONTENT.to_owned());
    }
    lines.extend(parts.texts);
    let result = fitted(&redactions.apply(&lines.join("\n")), room).with_images(parts.images);
    if is_error {
        result.with_failure(texts::TOOL_ERROR)
    } else {
        result
    }
}

/// A read resource, as the model reads it.
pub(crate) fn render_resource(
    read: ReadResourceResult,
    images_allowed: bool,
    redactions: &Redactions,
    room: Room<'_>,
) -> ToolResult {
    let mut parts = Parts::default();
    for contents in read.contents {
        parts.contents(contents, images_allowed);
    }
    if parts.texts.is_empty() && parts.images.is_empty() {
        parts.note(texts::NO_CONTENT.to_owned());
    }
    fitted(&redactions.apply(&parts.texts.join("\n")), room).with_images(parts.images)
}

/// A text that fits its room: whole, or its start and its end with the whole
/// text archived under the first reserved ID.
pub(crate) fn fitted(text: &str, room: Room<'_>) -> ToolResult {
    if text.len() <= room.result_limit {
        return ToolResult::new(text);
    }
    let Some(id) = room.archive_ids.first() else {
        let (head, tail, left_out) = ends(text, room.result_limit.saturating_sub(64));
        return ToolResult::new(format!(
            "{head}\n{}\n{tail}",
            texts::left_out_unarchived(left_out)
        ));
    };
    let note_room = 160;
    let (head, tail, left_out) = ends(text, room.result_limit.saturating_sub(note_room));
    let output = format!(
        "{head}\n{}\n{tail}",
        texts::left_out(left_out, &id.to_string())
    );
    let archive = ToolArchive::new(id.clone(), "output", cut(text, MAX_TOOL_ARCHIVE_BYTES));
    ToolResult::new(output).with_archives(vec![archive])
}

/// The start and the end of a text in `room` bytes, cut at characters, and
/// how many bytes lie between them.
fn ends(text: &str, room: usize) -> (&str, &str, usize) {
    let head_room = room / 3 * HEAD_THIRDS;
    let tail_room = room - head_room;
    let mut head_end = head_room.min(text.len());
    while !text.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = text.len().saturating_sub(tail_room).max(head_end);
    while !text.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    (
        &text[..head_end],
        &text[tail_start..],
        tail_start - head_end,
    )
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::{ArchiveId, Secret};
    use rmcp::model::{CallToolResult, ContentBlock, Resource, ResourceContents};
    use serde_json::json;

    use super::{Room, fitted, render};
    use crate::log::Redactions;
    use crate::texts;

    const PIXEL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

    fn room(limit: usize, ids: &[ArchiveId]) -> Room<'_> {
        Room {
            result_limit: limit,
            archive_ids: ids,
        }
    }

    #[test]
    fn text_images_and_notes_reach_the_model_with_secrets_replaced() {
        let result = CallToolResult::success(vec![
            ContentBlock::text("token planted-secret-value"),
            ContentBlock::image(PIXEL, "image/png"),
            ContentBlock::image(PIXEL, "image/gif"),
            ContentBlock::audio("UklGRg==", "audio/wav"),
            ContentBlock::ResourceLink(Resource::new("test://readme", "readme")),
            ContentBlock::resource(ResourceContents::text("Embedded.", "test://embedded")),
        ]);
        let redactions = Redactions::new([Secret::new("planted-secret-value".to_owned())]);

        let rendered = render(result, true, &redactions, room(64 * 1024, &[]));

        assert!(
            rendered.output.contains("token [REDACTED]"),
            "{}",
            rendered.output
        );
        assert!(!rendered.output.contains("planted-secret-value"));
        assert_eq!(rendered.images.len(), 1);
        assert!(rendered.output.contains("image/gif is left out"));
        assert!(rendered.output.contains("Audio of type audio/wav, 4 bytes"));
        assert!(rendered.output.contains("test://readme"));
        assert!(
            rendered
                .output
                .contains("Resource test://embedded:\nEmbedded.")
        );
        assert_eq!(rendered.failure, None);
    }

    #[test]
    fn images_stay_out_when_the_owner_says_so_and_errors_fail() {
        let mut result = CallToolResult::error(vec![ContentBlock::image(PIXEL, "image/png")]);
        result.is_error = Some(true);

        let rendered = render(result, false, &Redactions::default(), room(64 * 1024, &[]));

        assert_eq!(rendered.images.len(), 0);
        assert!(rendered.output.starts_with(texts::TOOL_ERROR_TEXT));
        assert_eq!(rendered.failure.as_deref(), Some(texts::TOOL_ERROR));
    }

    #[test]
    fn structured_content_speaks_when_there_is_no_text() {
        let mut result = CallToolResult::success(Vec::new());
        result.structured_content = Some(json!({ "sum": 3 }));

        let rendered = render(result, true, &Redactions::default(), room(64 * 1024, &[]));

        assert!(
            rendered.output.contains("\"sum\": 3"),
            "{}",
            rendered.output
        );
        let empty = render(
            CallToolResult::success(Vec::new()),
            true,
            &Redactions::default(),
            room(64 * 1024, &[]),
        );
        assert_eq!(empty.output, texts::NO_CONTENT);
    }

    #[test]
    fn a_long_text_keeps_its_ends_and_is_archived_whole() {
        let ids = [ArchiveId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap()];
        let text = format!("{}{}", "a".repeat(10_000), "é".repeat(5_000));

        let fitted_result = fitted(&text, room(2_000, &ids));

        assert!(fitted_result.output.len() <= 2_000);
        assert!(fitted_result.output.starts_with("aaa"));
        assert!(fitted_result.output.ends_with('é'));
        assert!(fitted_result.output.contains("read_tool_output"));
        assert_eq!(fitted_result.archives.len(), 1);
        assert_eq!(fitted_result.archives[0].content, text);
    }
}
