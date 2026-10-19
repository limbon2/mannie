//! The commands of the Images plugin (plugins/images/docs/README.md, contracts):
//! choosing files in the system's dialog, preparing a chosen, pasted or
//! dropped image, dropping a waiting one, and their errors. Each command is a
//! spec of the plugin SDK (ADR-0034, decision 8), so the host checks its
//! input before the plugin's handler runs, and answers `plugin_off` while
//! Images is off.

use mannie_plugin_sdk::contracts::{ImageFormat, MAX_IMAGE_BYTES};
use mannie_plugin_sdk::ipc::{CommandSpec, InputRefusal, Interface, PluginError, Validate};
use serde::{Deserialize, Serialize};
use specta::Type;

/// The largest file that the plugin reads, in bytes.
pub const MAX_FILE_BYTES: usize = 20 * 1024 * 1024;

/// The longest side of an image that the plugin decodes, in pixels.
pub const MAX_DECODED_SIDE: u32 = 12_000;

/// The most pixels of an image that the plugin decodes, in megapixels.
pub const MAX_DECODED_MEGAPIXELS: u32 = 64;

/// The extensions of the files that the dialog shows.
pub const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

/// The longest name of the dialog's filter, in characters.
pub const MAX_FILTER_NAME_CHARACTERS: usize = 100;

/// The longest name of a pasted or dropped file that the plugin takes, in
/// bytes. The plugin keeps a shorter, cleaned name.
pub const MAX_PASTED_NAME_BYTES: usize = 1024;

/// The longest base64 text of a file that the plugin reads.
pub(crate) fn max_base64_len() -> usize {
    MAX_FILE_BYTES.div_ceil(3) * 4
}

/// The argument of `pick_images`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PickImages {
    /// The name of the dialog's filter, such as "Images", from the UI's
    /// catalog (ADR-0019).
    pub filter_name: String,
}

impl PickImages {
    /// The filter's name without surrounding white space.
    pub fn filter_name(&self) -> &str {
        self.filter_name.trim()
    }
}

impl Validate for PickImages {
    /// The filter's name, which the system's dialog shows, is not blank, has
    /// at most [`MAX_FILTER_NAME_CHARACTERS`] characters, and no control
    /// characters.
    fn validate(&self) -> Result<(), InputRefusal> {
        let name = self.filter_name();
        let invalid = name.is_empty()
            || name.chars().count() > MAX_FILTER_NAME_CHARACTERS
            || self.filter_name.chars().any(char::is_control);
        if invalid {
            return Err(InputRefusal::new("filterName"));
        }
        Ok(())
    }
}

/// A file that the owner chose in the dialog, which `prepare_picked_image`
/// reads. Its path stays in the core (ADR-0004).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PickedImage {
    /// The file's ID while the plugin keeps it.
    pub picked_id: String,
    /// The file's name.
    pub name: String,
}

/// The argument of `prepare_picked_image`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PickedImageRef {
    /// The file's ID, from `pick_images`.
    pub picked_id: String,
}

impl Validate for PickedImageRef {
    /// Any ID decodes: one that the plugin does not keep is `not_found`.
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// The argument of `prepare_image`: a file that the owner pasted or dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareImage {
    /// The file's name, as the webview knows it.
    pub name: String,
    /// The file's bytes, as base64.
    pub data: String,
}

impl Validate for PrepareImage {
    /// The name has at most [`MAX_PASTED_NAME_BYTES`] bytes. The data's
    /// length is the plugin's own check, which names the file's limit.
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.name.len() > MAX_PASTED_NAME_BYTES {
            return Err(InputRefusal::new("name"));
        }
        Ok(())
    }
}

/// An image that the plugin prepared and keeps until its message is stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PreparedImage {
    /// The key that the message's reference names (ADR-0033, decision 1).
    pub key: String,
    /// The image's name.
    pub name: String,
    /// The format of the prepared image.
    pub format: ImageFormat,
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
    /// The size of the prepared image in bytes.
    pub bytes: u32,
    /// The format of the thumbnail.
    pub thumbnail_format: ImageFormat,
    /// The thumbnail's bytes, as base64.
    pub thumbnail: String,
}

/// The argument of `discard_prepared_image`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedImageRef {
    /// The image's key.
    pub key: String,
}

impl Validate for PreparedImageRef {
    /// Any key decodes: an unknown key changes nothing.
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// Why a command of the Images plugin failed. The UI shows the message for
/// the code from its catalog (ADR-0019), and the host's refusals, such as
/// `plugin_off`, come beside these (ADR-0034, decision 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ImagesError {
    /// The file is larger than the plugin reads.
    #[error("the file is larger than {max_bytes} bytes")]
    TooLarge {
        /// The limit.
        max_bytes: u32,
    },
    /// The image is wider or taller than the plugin decodes.
    #[error("the image is wider or taller than {max_side} pixels")]
    TooWide {
        /// The limit.
        max_side: u32,
    },
    /// The image has more pixels than the plugin decodes.
    #[error("the image has more than {max_megapixels} megapixels")]
    TooManyPixels {
        /// The limit.
        max_megapixels: u32,
    },
    /// The image stays larger than Mannie sends, even shrunk and as a JPEG.
    #[error("the image stays larger than {max_bytes} bytes")]
    CannotShrink {
        /// The limit.
        max_bytes: u32,
    },
    /// The file is not a PNG, JPEG, GIF or WebP image.
    #[error("the file is not an image that Mannie reads")]
    UnsupportedFormat,
    /// The image is damaged or cut short.
    #[error("the image could not be decoded")]
    Unreadable,
    /// The chosen file could not be read.
    #[error("the file could not be read")]
    FileUnreadable,
    /// The chosen file is no longer kept, for example because the plugin was
    /// turned off and on.
    #[error("the chosen file is no longer available")]
    NotFound,
    /// The system's file dialog failed.
    #[error("the file dialog failed")]
    DialogFailed,
}

impl PluginError for ImagesError {
    /// The stable code of the error, for logs.
    fn code(&self) -> &'static str {
        match self {
            Self::TooLarge { .. } => "images.too_large",
            Self::TooWide { .. } => "images.too_wide",
            Self::TooManyPixels { .. } => "images.too_many_pixels",
            Self::CannotShrink { .. } => "images.cannot_shrink",
            Self::UnsupportedFormat => "images.unsupported_format",
            Self::Unreadable => "images.unreadable",
            Self::FileUnreadable => "images.file_unreadable",
            Self::NotFound => "images.not_found",
            Self::DialogFailed => "images.dialog_failed",
        }
    }
}

impl ImagesError {
    /// The error of a file larger than [`MAX_FILE_BYTES`].
    pub(crate) fn file_too_large() -> Self {
        Self::TooLarge {
            max_bytes: limit(MAX_FILE_BYTES),
        }
    }

    /// The error of an image that stays larger than Mannie sends.
    pub(crate) fn cannot_shrink() -> Self {
        Self::CannotShrink {
            max_bytes: limit(MAX_IMAGE_BYTES),
        }
    }
}

/// A limit as a number for the UI's message.
fn limit(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// `pick_images`: shows the system's file dialog for images and keeps the
/// files that the owner chose.
#[derive(Debug)]
pub enum PickImagesCommand {}

impl CommandSpec for PickImagesCommand {
    const NAME: &'static str = "pick_images";
    const DOCS: &'static str = "Shows the system's file dialog for images and keeps the files that the owner chose, which `prepare_picked_image` then prepares one by one. None when the owner closed the dialog.";
    type Input = PickImages;
    type Output = Vec<PickedImage>;
    type Error = ImagesError;
}

/// `prepare_picked_image`: reads and prepares a file that the owner chose.
#[derive(Debug)]
pub enum PreparePickedImageCommand {}

impl CommandSpec for PreparePickedImageCommand {
    const NAME: &'static str = "prepare_picked_image";
    const DOCS: &'static str = "Reads and prepares an image file that the owner chose.";
    type Input = PickedImageRef;
    type Output = PreparedImage;
    type Error = ImagesError;
}

/// `prepare_image`: prepares a file that the owner pasted or dropped.
#[derive(Debug)]
pub enum PrepareImageCommand {}

impl CommandSpec for PrepareImageCommand {
    const NAME: &'static str = "prepare_image";
    const DOCS: &'static str =
        "Prepares an image that the owner pasted or dropped on the message box.";
    /// The base64 text of the largest file that the plugin reads, with room
    /// for its name, so the plugin's own check names the file's limit.
    const MAX_INPUT_BYTES: u32 = 28 * 1024 * 1024;
    type Input = PrepareImage;
    type Output = PreparedImage;
    type Error = ImagesError;
}

/// `discard_prepared_image`: drops a waiting image.
#[derive(Debug)]
pub enum DiscardPreparedImageCommand {}

impl CommandSpec for DiscardPreparedImageCommand {
    const NAME: &'static str = "discard_prepared_image";
    const DOCS: &'static str = "Drops a waiting image that the owner removed from the message box.";
    type Input = PreparedImageRef;
    type Output = ();
    type Error = ImagesError;
}

/// The plugin's commands and the limit that the UI checks before a file
/// crosses IPC.
pub fn interface() -> Interface {
    Interface::new()
        .command::<PickImagesCommand>()
        .command::<PreparePickedImageCommand>()
        .command::<PrepareImageCommand>()
        .command::<DiscardPreparedImageCommand>()
        .constant(
            "MAX_FILE_BYTES",
            "The largest file that the plugin reads, in bytes.",
            limit(MAX_FILE_BYTES),
        )
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::ipc::{CommandSpec, InputRefusal, PluginError, Validate};

    use super::{
        ImagesError, MAX_FILTER_NAME_CHARACTERS, MAX_PASTED_NAME_BYTES, PickImages, PrepareImage,
        PrepareImageCommand, max_base64_len,
    };

    fn pick(name: &str) -> PickImages {
        PickImages {
            filter_name: name.to_owned(),
        }
    }

    #[test]
    fn the_filter_name_is_trimmed_and_must_be_short_plain_text() {
        assert_eq!(pick(" Images ").validate(), Ok(()));
        assert_eq!(pick(" Images ").filter_name(), "Images");
        assert_eq!(
            pick(&"é".repeat(MAX_FILTER_NAME_CHARACTERS)).validate(),
            Ok(())
        );
        for invalid in [
            String::new(),
            " \t".to_owned(),
            "Images\n(png)".to_owned(),
            "Images\n".to_owned(),
            "x".repeat(MAX_FILTER_NAME_CHARACTERS + 1),
        ] {
            assert_eq!(
                pick(&invalid).validate(),
                Err(InputRefusal::new("filterName")),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn a_pasted_file_s_name_is_short_and_its_data_fits_the_host_s_limit() {
        let named = |name: String| PrepareImage {
            name,
            data: String::new(),
        };

        assert_eq!(named("a.png".to_owned()).validate(), Ok(()));
        assert_eq!(
            named("x".repeat(MAX_PASTED_NAME_BYTES + 1)).validate(),
            Err(InputRefusal::new("name"))
        );
        let largest_input = max_base64_len() + MAX_PASTED_NAME_BYTES + 64;
        assert!(largest_input < usize::try_from(PrepareImageCommand::MAX_INPUT_BYTES).unwrap());
    }

    #[test]
    fn errors_serialize_with_their_code_and_limits() {
        assert_eq!(
            serde_json::to_value(ImagesError::file_too_large()).unwrap(),
            serde_json::json!({ "code": "too_large", "maxBytes": 20_971_520 })
        );
        assert_eq!(
            serde_json::to_value(ImagesError::cannot_shrink()).unwrap(),
            serde_json::json!({ "code": "cannot_shrink", "maxBytes": 5_242_880 })
        );
        assert_eq!(ImagesError::NotFound.code(), "images.not_found");
    }
}
