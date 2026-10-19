//! Images (ADR-0033; plugins/images/docs/README.md): the core's side of the image
//! sources that plugins contribute through the SDK's `points::images`
//! (ADR-0034, decision 6), which turn a key from the message box into an
//! image: the check of their IDs, the core's checks of what they return, and
//! the bytes that the UI reads back.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use mannie_plugin_sdk::contributions::{Contributions, ExtensionPoint};
use mannie_plugin_sdk::host::{ContributionRefusal, ID_TAKEN};
use mannie_plugin_sdk::points::images::{ImageSource, ImageSources, SourcedImage};
use mannie_store::NewImage;
use mannie_types::chat::ChatError;
use mannie_types::images::{
    INVALID_IMAGE, ImageData, ImageFormat, ImageRef, ImageView, MAX_IMAGE_BYTES, MAX_IMAGE_SIDE,
    MAX_THUMBNAIL_BYTES, ReadImage, clean_image_name,
};
use sha2::{Digest, Sha256};

use crate::Runtime;
use crate::compaction::to_hex;
use crate::conversations::store_failure;
use crate::plugins::Snapshot;

/// Refuses a plugin's image sources whose IDs another plugin's sources have.
/// The coordinator of the SDK made sure that each ID is the plugin's (the
/// rule `Owned`), and the core has no sources of its own.
pub(crate) fn check_image_sources(
    snapshot: &Snapshot,
    contributions: &Contributions,
) -> Result<(), ContributionRefusal> {
    for source in contributions.entries::<ImageSources>() {
        let id = source.id();
        if snapshot.find::<ImageSources>(id).is_some() {
            return Err(ContributionRefusal::new(
                ImageSources::NAME,
                ID_TAKEN,
                format!("an image source with the ID {id} is registered already"),
            ));
        }
    }
    Ok(())
}

/// Why the core refuses what a source returned, for the log.
fn check_sourced(image: &SourcedImage) -> Result<(), &'static str> {
    if ImageFormat::of(image.data.as_slice()) != Some(image.format) {
        return Err("the image's bytes are not its format");
    }
    if ImageFormat::of(image.thumbnail.as_slice()) != Some(image.thumbnail_format) {
        return Err("the thumbnail's bytes are not its format");
    }
    if image.data.len() > MAX_IMAGE_BYTES {
        return Err("the image is too large");
    }
    if image.thumbnail.len() > MAX_THUMBNAIL_BYTES {
        return Err("the thumbnail is too large");
    }
    let sides = [image.width, image.height];
    if sides
        .iter()
        .any(|side| *side == 0 || *side > MAX_IMAGE_SIDE)
    {
        return Err("the image's dimensions are out of range");
    }
    Ok(())
}

/// The SHA-256 digest of bytes, in hexadecimal.
fn digest(bytes: &[u8]) -> String {
    to_hex(&Sha256::digest(bytes))
}

impl Runtime {
    /// The image sources of the plugins that are on, in the order in which
    /// the plugins started.
    fn image_sources(&self) -> Vec<Arc<dyn ImageSource>> {
        self.contributions()
            .items::<ImageSources>()
            .iter()
            .map(|registered| Arc::clone(&registered.entry))
            .collect()
    }

    /// Reads a message's images from their sources, which
    /// [`check_images`] checked, and checks what each source returned
    /// (ADR-0033, decisions 2 and 3).
    ///
    /// [`check_images`]: mannie_types::images::check_images
    pub(crate) async fn resolve_images(
        &self,
        images: &[ImageRef],
    ) -> Result<Vec<NewImage>, ChatError> {
        if images.is_empty() {
            return Ok(Vec::new());
        }
        let sources = self.image_sources();
        let mut resolved = Vec::with_capacity(images.len());
        for image in images {
            let source = sources
                .iter()
                .find(|source| source.id() == image.source)
                .ok_or_else(|| ChatError::ImageUnavailable {
                    image_source: image.source.clone(),
                })?;
            let prepared = source.resolve(&image.key).await.map_err(|code| {
                tracing::info!(source = source.id(), code, "an image was refused");
                ChatError::ImageRefused {
                    image_source: image.source.clone(),
                    reason: code,
                }
            })?;
            if let Err(problem) = check_sourced(&prepared) {
                tracing::error!(
                    source = source.id(),
                    problem,
                    "an image source returned an image outside the core's limits"
                );
                return Err(ChatError::ImageRefused {
                    image_source: image.source.clone(),
                    reason: INVALID_IMAGE.to_owned(),
                });
            }
            let bytes = u32::try_from(prepared.data.len()).unwrap_or(u32::MAX);
            resolved.push(NewImage {
                part_id: self.inner.ids.part(),
                source: image.source.clone(),
                view: ImageView {
                    name: clean_image_name(&prepared.name),
                    format: prepared.format,
                    width: prepared.width,
                    height: prepared.height,
                    bytes,
                },
                digest: digest(prepared.data.as_slice()),
                data: prepared.data,
                thumbnail_format: prepared.thumbnail_format,
                thumbnail: prepared.thumbnail,
            });
        }
        Ok(resolved)
    }

    /// Tells each source that the message that holds its images was stored.
    /// A source that stopped since is gone, and has dropped its images
    /// already.
    pub(crate) fn release_images(&self, images: &[ImageRef]) {
        if images.is_empty() {
            return;
        }
        for source in self.image_sources() {
            let keys: Vec<String> = images
                .iter()
                .filter(|image| image.source == source.id())
                .map(|image| image.key.clone())
                .collect();
            if !keys.is_empty() {
                source.release(&keys);
            }
        }
    }

    /// The bytes of an image of a conversation, the thumbnail or the full
    /// image, for the transcript (ADR-0033, decision 4). They stay readable
    /// while the plugin that added the image is off.
    pub async fn read_image(&self, input: ReadImage) -> Result<ImageData, ChatError> {
        let stored = self
            .inner
            .store
            .image(&input.conversation_id, &input.part_id, input.size)
            .await
            .map_err(store_failure)?
            .ok_or(ChatError::ImageNotFound)?;
        Ok(ImageData {
            format: stored.format,
            data: BASE64.encode(stored.data.as_slice()),
        })
    }
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::points::images::SourcedImage;
    use mannie_types::images::{ImageBytes, ImageFormat, MAX_IMAGE_BYTES};

    use super::{check_sourced, digest};

    fn png() -> SourcedImage {
        let name = "shot.png".to_owned();
        let format = ImageFormat::Png;
        let width = 800;
        let height = 600;
        let data = ImageBytes::from(b"\x89PNG\r\n\x1a\nrest".to_vec());
        let thumbnail_format = ImageFormat::Jpeg;
        let thumbnail = ImageBytes::from(b"\xff\xd8\xffrest".to_vec());
        SourcedImage::new(
            name,
            format,
            width,
            height,
            data,
            thumbnail_format,
            thumbnail,
        )
    }

    #[test]
    fn an_image_within_the_limits_passes() {
        assert_eq!(check_sourced(&png()), Ok(()));
    }

    #[test]
    fn bytes_that_are_not_their_format_or_too_large_are_refused() {
        let mut mislabeled = png();
        mislabeled.format = ImageFormat::Jpeg;
        let mut bad_thumbnail = png();
        bad_thumbnail.thumbnail = ImageBytes::from(b"GIF89a".to_vec());
        let mut huge = b"\x89PNG\r\n\x1a\n".to_vec();
        huge.resize(MAX_IMAGE_BYTES + 1, 0);
        let mut too_large = png();
        too_large.data = ImageBytes::from(huge);

        assert!(check_sourced(&mislabeled).is_err());
        assert!(check_sourced(&bad_thumbnail).is_err());
        assert!(check_sourced(&too_large).is_err());
    }

    #[test]
    fn dimensions_outside_one_to_2048_are_refused() {
        for (width, height) in [(0, 600), (800, 0), (2049, 600), (800, 4000)] {
            let mut image = png();
            image.width = width;
            image.height = height;
            assert!(check_sourced(&image).is_err(), "{width}x{height}");
        }
        let mut largest = png();
        largest.width = 2048;
        largest.height = 2048;
        assert_eq!(check_sourced(&largest), Ok(()));
    }

    #[test]
    fn digests_are_hexadecimal_sha_256() {
        assert_eq!(
            digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
