//! Images (ADR-0033; plugins/images/docs/README.md): what the owner attaches to a
//! message besides its text and mentions. This module holds the references
//! that a message carries, the formats and limits of a stored image, the view
//! of an image part and the argument and result of `read_image`. The images
//! come from the runtime's image sources, which plugins contribute.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::chat::ChatError;
use crate::ids::{ConversationId, PartId};

/// The most images that one message carries.
pub const MAX_IMAGES: usize = 10;

/// The longest side of a stored image, in pixels.
pub const MAX_IMAGE_SIDE: u32 = 2048;

/// The largest stored image, in bytes.
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

/// The longest side of a stored thumbnail, in pixels.
pub const MAX_THUMBNAIL_SIDE: u32 = 320;

/// The largest stored thumbnail, in bytes.
pub const MAX_THUMBNAIL_BYTES: usize = 512 * 1024;

/// The longest ID of an image source, in bytes.
pub const MAX_IMAGE_SOURCE_ID_BYTES: usize = 64;

/// The longest key of a reference, in bytes.
pub const MAX_IMAGE_KEY_BYTES: usize = 128;

/// The longest name of an image, in characters.
pub const MAX_IMAGE_NAME_CHARACTERS: usize = 255;

/// The name of an image whose own name is empty once cleaned.
pub const FALLBACK_IMAGE_NAME: &str = "image";

/// The code of a refusal for a key that no source could hold: empty or too
/// long.
pub const INVALID_IMAGE_KEY: &str = "invalid_key";

/// The code of a refusal for an image that a source returned outside the
/// core's limits.
pub const INVALID_IMAGE: &str = "invalid_image";

/// The format of a stored image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    /// PNG.
    Png,
    /// JPEG.
    Jpeg,
}

/// The first bytes of every PNG file.
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// The first bytes of every JPEG file: the start of image and the first
/// marker's prefix.
const JPEG_SIGNATURE: &[u8] = b"\xff\xd8\xff";

impl ImageFormat {
    /// Every format.
    pub const ALL: [Self; 2] = [Self::Png, Self::Jpeg];

    /// The format's name as the store keeps it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
        }
    }

    /// The format with this name, or `None` for one this version does not
    /// know.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|format| format.as_str() == name)
    }

    /// The media type of the format, as a `data:` URL names it.
    pub fn media_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }

    /// The format that the bytes' signature shows, or `None`.
    pub fn of(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(PNG_SIGNATURE) {
            Some(Self::Png)
        } else if bytes.starts_with(JPEG_SIGNATURE) {
            Some(Self::Jpeg)
        } else {
            None
        }
    }
}

/// The bytes of an image. `Debug` shows only their number, so that a log or a
/// failed test never prints megabytes.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ImageBytes(Arc<[u8]>);

impl ImageBytes {
    /// The bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// Their number.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Vec<u8>> for ImageBytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes.into())
    }
}

impl From<Arc<[u8]>> for ImageBytes {
    fn from(bytes: Arc<[u8]>) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for ImageBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "ImageBytes({} bytes)", self.0.len())
    }
}

/// An image that a message carries: which source, and its key there.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ImageRef {
    /// The source's ID.
    pub source: String,
    /// The key that the source gave the image.
    pub key: String,
}

/// A sent image as the owner sees it. Its bytes come from `read_image`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ImageView {
    /// The image's name, such as its file's name. It is content and is never
    /// translated.
    pub name: String,
    /// The format of the stored image.
    pub format: ImageFormat,
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
    /// The size of the stored image in bytes.
    pub bytes: u32,
}

/// Which bytes of an image `read_image` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ImageSize {
    /// The thumbnail, at most [`MAX_THUMBNAIL_SIDE`] pixels on its longer
    /// side.
    Thumbnail,
    /// The image as the model reads it.
    Full,
}

/// The argument of `read_image`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReadImage {
    /// The conversation that holds the image.
    pub conversation_id: ConversationId,
    /// The image's part.
    pub part_id: PartId,
    /// Which bytes.
    pub size: ImageSize,
}

/// The result of `read_image`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ImageData {
    /// The format of the bytes.
    pub format: ImageFormat,
    /// The bytes, as base64.
    pub data: String,
}

/// Checks the references of a message before any source reads them: at most
/// [`MAX_IMAGES`] different ones, with IDs and keys within their limits.
/// Returns them without repetitions, in the order of the message.
pub fn check_images(images: Vec<ImageRef>) -> Result<Vec<ImageRef>, ChatError> {
    let mut distinct: Vec<ImageRef> = Vec::with_capacity(images.len().min(MAX_IMAGES));
    for image in images {
        if image.source.is_empty() || image.source.len() > MAX_IMAGE_SOURCE_ID_BYTES {
            return Err(ChatError::ImageUnavailable {
                image_source: cut_at_char(&image.source, MAX_IMAGE_SOURCE_ID_BYTES),
            });
        }
        if image.key.is_empty() || image.key.len() > MAX_IMAGE_KEY_BYTES {
            return Err(ChatError::ImageRefused {
                image_source: image.source,
                reason: INVALID_IMAGE_KEY.to_owned(),
            });
        }
        if distinct.contains(&image) {
            continue;
        }
        if distinct.len() == MAX_IMAGES {
            return Err(ChatError::TooManyImages {
                max: u32::try_from(MAX_IMAGES).unwrap_or(u32::MAX),
            });
        }
        distinct.push(image);
    }
    Ok(distinct)
}

/// The name of an image as Mannie keeps it: the last part of a path, without
/// control characters and surrounding white space, at most
/// [`MAX_IMAGE_NAME_CHARACTERS`] characters, and [`FALLBACK_IMAGE_NAME`] when
/// nothing is left.
pub fn clean_image_name(name: &str) -> String {
    let last_part = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last_part
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_IMAGE_NAME_CHARACTERS)
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        FALLBACK_IMAGE_NAME.to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// The first bytes of a text, at most `max_bytes`, cut at a character.
fn cut_at_char(text: &str, max_bytes: usize) -> String {
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::{
        FALLBACK_IMAGE_NAME, INVALID_IMAGE_KEY, ImageBytes, ImageData, ImageFormat, ImageRef,
        ImageSize, MAX_IMAGE_KEY_BYTES, MAX_IMAGE_NAME_CHARACTERS, MAX_IMAGES, check_images,
        clean_image_name,
    };
    use crate::chat::ChatError;

    fn image(key: &str) -> ImageRef {
        ImageRef {
            source: "images".to_owned(),
            key: key.to_owned(),
        }
    }

    #[test]
    fn references_lose_their_repetitions_and_keep_their_order() {
        let checked = check_images(vec![image("b"), image("a"), image("b")]).unwrap();

        assert_eq!(checked, [image("b"), image("a")]);
    }

    #[test]
    fn a_message_carries_at_most_ten_different_images() {
        let ten: Vec<ImageRef> = (0..MAX_IMAGES).map(|n| image(&n.to_string())).collect();
        let mut repeated = ten.clone();
        repeated.push(image("0"));
        let mut too_many = ten.clone();
        too_many.push(image("extra"));

        assert_eq!(check_images(ten).unwrap().len(), MAX_IMAGES);
        assert_eq!(check_images(repeated).unwrap().len(), MAX_IMAGES);
        assert_eq!(
            check_images(too_many),
            Err(ChatError::TooManyImages { max: 10 })
        );
    }

    #[test]
    fn empty_or_oversized_ids_and_keys_are_refused() {
        let no_source = ImageRef {
            source: String::new(),
            key: "a".to_owned(),
        };
        let long_source = ImageRef {
            source: "é".repeat(40),
            key: "a".to_owned(),
        };

        assert_eq!(
            check_images(vec![no_source]),
            Err(ChatError::ImageUnavailable {
                image_source: String::new()
            })
        );
        assert_eq!(
            check_images(vec![long_source]),
            Err(ChatError::ImageUnavailable {
                image_source: "é".repeat(32)
            })
        );
        for key in [String::new(), "a".repeat(MAX_IMAGE_KEY_BYTES + 1)] {
            assert_eq!(
                check_images(vec![image(&key)]),
                Err(ChatError::ImageRefused {
                    image_source: "images".to_owned(),
                    reason: INVALID_IMAGE_KEY.to_owned()
                })
            );
        }
    }

    #[test]
    fn formats_are_known_by_their_signatures_and_names() {
        assert_eq!(
            ImageFormat::of(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            ImageFormat::of(b"\xff\xd8\xff\xe0"),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(ImageFormat::of(b"GIF89a"), None);
        assert_eq!(ImageFormat::of(b"\x89PN"), None);
        for format in ImageFormat::ALL {
            assert_eq!(ImageFormat::parse(format.as_str()), Some(format));
        }
        assert_eq!(ImageFormat::parse("webp"), None);
        assert_eq!(ImageFormat::Jpeg.media_type(), "image/jpeg");
    }

    #[test]
    fn bytes_show_only_their_number_when_debugged() {
        let bytes = ImageBytes::from(vec![7_u8; 1_000]);

        assert_eq!(format!("{bytes:?}"), "ImageBytes(1000 bytes)");
        assert_eq!(bytes.len(), 1_000);
        assert!(!bytes.is_empty());
        assert!(ImageBytes::default().is_empty());
    }

    #[test]
    fn names_keep_their_last_part_without_control_characters() {
        assert_eq!(clean_image_name("C:\\Users\\me\\shot.png"), "shot.png");
        assert_eq!(clean_image_name("/home/me/photo 1.jpg"), "photo 1.jpg");
        assert_eq!(clean_image_name("  a\u{7}b\n.png "), "ab.png");
        assert_eq!(clean_image_name(""), FALLBACK_IMAGE_NAME);
        assert_eq!(clean_image_name("folder/"), FALLBACK_IMAGE_NAME);
        assert_eq!(
            clean_image_name(&"x".repeat(400)).chars().count(),
            MAX_IMAGE_NAME_CHARACTERS
        );
    }

    #[test]
    fn sizes_and_data_serialize_for_the_ui() {
        let data = ImageData {
            format: ImageFormat::Png,
            data: "AAAA".to_owned(),
        };

        assert_eq!(
            serde_json::to_value(ImageSize::Thumbnail).unwrap(),
            serde_json::json!("thumbnail")
        );
        assert_eq!(
            serde_json::to_value(&data).unwrap(),
            serde_json::json!({ "format": "png", "data": "AAAA" })
        );
    }
}
