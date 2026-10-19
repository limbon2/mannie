//! Image sources (ADR-0033, decision 2): a plugin's source turns a key from
//! the message box into a prepared image, which the core checks against the
//! limits in [`contracts`](crate::contracts) and stores with the message
//! (decision 3).
//!
//! The source's ID follows the rule `Owned`, which
//! [`owned_by`](crate::ids::owned_by) checks: the plugin's ID, or the
//! plugin's ID, a dot and more (ADR-0034, decision 6).

use std::fmt;
use std::sync::Arc;

use crate::BoxFuture;
use crate::contracts::{ImageBytes, ImageFormat};
use crate::contributions::{ExtensionPoint, IdRule};

/// The point of plugins' image sources, whose entries are named by their
/// IDs.
#[derive(Debug)]
pub enum ImageSources {}

impl ExtensionPoint for ImageSources {
    const NAME: &'static str = "image-sources";
    const MAJOR: u16 = 1;
    const IDS: IdRule = IdRule::Owned;
    type Entry = Arc<dyn ImageSource>;

    fn id(entry: &Arc<dyn ImageSource>) -> &str {
        entry.id()
    }
}

/// A source of images (ADR-0033, decision 2).
pub trait ImageSource: Send + Sync + fmt::Debug + 'static {
    /// The source's ID: the plugin's ID, or one that starts with it and a dot.
    fn id(&self) -> &str;

    /// The image with this key. The key comes from the webview, so the source
    /// checks it as untrusted input, and refuses a key that it does not hold
    /// with its code.
    fn resolve<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<SourcedImage, String>>;

    /// The message that holds the images with these keys was stored, so the
    /// source may let them go.
    fn release(&self, keys: &[String]);
}

/// An image as a source gives it: prepared, within the core's limits, with
/// its thumbnail, which the source keeps within
/// [`MAX_THUMBNAIL_SIDE`](crate::contracts::MAX_THUMBNAIL_SIDE) pixels on
/// each side. The core refuses an image when:
///
/// - the image's bytes are not its `format`, or the thumbnail's bytes are
///   not its `thumbnail_format`;
/// - the image has more than
///   [`MAX_IMAGE_BYTES`](crate::contracts::MAX_IMAGE_BYTES), or the
///   thumbnail more than
///   [`MAX_THUMBNAIL_BYTES`](crate::contracts::MAX_THUMBNAIL_BYTES);
/// - the width or the height is 0 or more than
///   [`MAX_IMAGE_SIDE`](crate::contracts::MAX_IMAGE_SIDE).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SourcedImage {
    /// The image's name, such as its file's name.
    pub name: String,
    /// The format of the image.
    pub format: ImageFormat,
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
    /// The image.
    pub data: ImageBytes,
    /// The format of the thumbnail.
    pub thumbnail_format: ImageFormat,
    /// The thumbnail.
    pub thumbnail: ImageBytes,
}

impl SourcedImage {
    /// An image with its thumbnail. The two sides and the two pairs of format
    /// and bytes are easy to swap, so callers pass named locals.
    pub fn new(
        name: String,
        format: ImageFormat,
        width: u32,
        height: u32,
        data: ImageBytes,
        thumbnail_format: ImageFormat,
        thumbnail: ImageBytes,
    ) -> Self {
        Self {
            name,
            format,
            width,
            height,
            data,
            thumbnail_format,
            thumbnail,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SourcedImage;
    use crate::contracts::{ImageBytes, ImageFormat};

    #[test]
    fn the_constructor_keeps_each_argument_in_its_field() {
        let name = "shot.png".to_owned();
        let format = ImageFormat::Png;
        let width = 800;
        let height = 600;
        let data = ImageBytes::from(b"\x89PNG\r\n\x1a\nimage".to_vec());
        let thumbnail_format = ImageFormat::Jpeg;
        let thumbnail = ImageBytes::from(b"\xff\xd8\xffthumbnail".to_vec());

        let image = SourcedImage::new(
            name,
            format,
            width,
            height,
            data.clone(),
            thumbnail_format,
            thumbnail.clone(),
        );

        assert_eq!(image.name, "shot.png");
        assert_eq!(image.format, ImageFormat::Png);
        assert_eq!(image.width, 800);
        assert_eq!(image.height, 600);
        assert_eq!(image.data, data);
        assert_eq!(image.thumbnail_format, ImageFormat::Jpeg);
        assert_eq!(image.thumbnail, thumbnail);
    }
}
