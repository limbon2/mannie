//! The commands of the Images plugin (docs/features/images.md, contracts):
//! choosing files in the system's dialog, preparing a chosen, pasted or
//! dropped image, dropping a waiting one, and their errors.

use mannie_types::images::{ImageFormat, MAX_IMAGE_BYTES};
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

/// The argument of `pick_images`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PickImages {
    /// The name of the dialog's filter, such as "Images", from the UI's
    /// catalog (ADR-0019).
    pub filter_name: String,
}

impl PickImages {
    /// Checks the filter's name, which the system's dialog shows: not blank,
    /// at most [`MAX_FILTER_NAME_CHARACTERS`] characters, and no control
    /// characters.
    pub fn check(&self) -> Result<&str, ImagesError> {
        let name = self.filter_name.trim();
        let invalid = name.is_empty()
            || name.chars().count() > MAX_FILTER_NAME_CHARACTERS
            || name.chars().any(char::is_control);
        if invalid {
            return Err(ImagesError::FilterNameInvalid);
        }
        Ok(name)
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
#[serde(rename_all = "camelCase")]
pub struct PickedImageRef {
    /// The file's ID, from `pick_images`.
    pub picked_id: String,
}

/// The argument of `prepare_image`: a file that the owner pasted or dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PrepareImage {
    /// The file's name, as the webview knows it.
    pub name: String,
    /// The file's bytes, as base64.
    pub data: String,
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
#[serde(rename_all = "camelCase")]
pub struct PreparedImageRef {
    /// The image's key.
    pub key: String,
}

/// Why a command of the Images plugin failed. The UI shows the message for
/// the code from its catalog (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ImagesError {
    /// The owner turned the Images plugin off (ADR-0030, decision 5).
    #[error("the Images plugin is off")]
    PluginOff,
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
    /// The name of the dialog's filter is blank, too long or has control
    /// characters.
    #[error("the name of the dialog's filter is not valid")]
    FilterNameInvalid,
    /// The core started without its database.
    #[error("the database is not open")]
    StoreUnavailable,
}

impl ImagesError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::PluginOff => "images.plugin_off",
            Self::TooLarge { .. } => "images.too_large",
            Self::TooWide { .. } => "images.too_wide",
            Self::TooManyPixels { .. } => "images.too_many_pixels",
            Self::CannotShrink { .. } => "images.cannot_shrink",
            Self::UnsupportedFormat => "images.unsupported_format",
            Self::Unreadable => "images.unreadable",
            Self::FileUnreadable => "images.file_unreadable",
            Self::NotFound => "images.not_found",
            Self::DialogFailed => "images.dialog_failed",
            Self::FilterNameInvalid => "images.filter_name_invalid",
            Self::StoreUnavailable => "images.store_unavailable",
        }
    }

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

#[cfg(test)]
mod tests {
    use super::{ImagesError, MAX_FILTER_NAME_CHARACTERS, PickImages};

    fn pick(name: &str) -> PickImages {
        PickImages {
            filter_name: name.to_owned(),
        }
    }

    #[test]
    fn the_filter_name_is_trimmed_and_must_be_short_plain_text() {
        assert_eq!(pick(" Images ").check(), Ok("Images"));
        assert_eq!(
            pick(&"é".repeat(MAX_FILTER_NAME_CHARACTERS))
                .check()
                .map(str::len),
            Ok(MAX_FILTER_NAME_CHARACTERS * 2)
        );
        for invalid in [
            String::new(),
            " \t".to_owned(),
            "Images\n(png)".to_owned(),
            "x".repeat(MAX_FILTER_NAME_CHARACTERS + 1),
        ] {
            assert_eq!(
                pick(&invalid).check(),
                Err(ImagesError::FilterNameInvalid),
                "{invalid:?}"
            );
        }
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
        assert_eq!(ImagesError::PluginOff.code(), "images.plugin_off");
    }
}
