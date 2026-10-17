//! The images that wait in message boxes while the plugin is on: the files
//! that the owner chose, until they are prepared, and the prepared images,
//! until their message is stored or the owner removes them
//! (docs/features/images.md, the plugin). Everything lives in memory, within
//! bounds, and the oldest go first.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mannie_runtime::SourcedImage;
use mannie_types::images::clean_image_name;
use tokio::sync::Semaphore;

use crate::commands::{ImagesError, MAX_FILE_BYTES, PickedImage};
use crate::prepare::prepare;

/// The most chosen files that wait to be prepared.
const MAX_PICKED: usize = 40;

/// The most prepared images that wait for their message.
const MAX_PREPARED: usize = 40;

/// The most bytes of prepared images and thumbnails that wait.
const MAX_PREPARED_BYTES: usize = 160 * 1024 * 1024;

/// A file that the owner chose.
#[derive(Debug)]
struct Picked {
    id: String,
    path: PathBuf,
    name: String,
}

/// A prepared image and its key.
#[derive(Debug)]
struct Prepared {
    key: String,
    image: SourcedImage,
}

impl Prepared {
    fn bytes(&self) -> usize {
        self.image.data.len() + self.image.thumbnail.len()
    }
}

#[derive(Debug, Default)]
struct Entries {
    picked: VecDeque<Picked>,
    prepared: VecDeque<Prepared>,
}

/// The waiting images of the plugin while it is on.
#[derive(Debug)]
pub(crate) struct Shelf {
    entries: Mutex<Entries>,
    /// One image is decoded at a time, so that a drop of many large files
    /// does not hold them all in memory at once (T-32).
    decoding: Arc<Semaphore>,
}

impl Default for Shelf {
    fn default() -> Self {
        Self {
            entries: Mutex::new(Entries::default()),
            decoding: Arc::new(Semaphore::new(1)),
        }
    }
}

fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

impl Shelf {
    fn entries(&self) -> MutexGuard<'_, Entries> {
        // Every critical section only adds, takes or removes entries.
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps the files that the owner chose, and returns their IDs and names.
    pub(crate) fn pick(&self, paths: Vec<PathBuf>) -> Vec<PickedImage> {
        let mut entries = self.entries();
        let mut picked = Vec::with_capacity(paths.len());
        for path in paths {
            let name = clean_image_name(&path.file_name().unwrap_or_default().to_string_lossy());
            let id = new_id();
            picked.push(PickedImage {
                picked_id: id.clone(),
                name: name.clone(),
            });
            entries.picked.push_back(Picked { id, path, name });
            while entries.picked.len() > MAX_PICKED {
                entries.picked.pop_front();
            }
        }
        picked
    }

    /// Reads and prepares a chosen file, which then no longer waits as a
    /// file.
    pub(crate) async fn prepare_picked(
        &self,
        picked_id: &str,
    ) -> Result<(String, SourcedImage), ImagesError> {
        let picked = {
            let mut entries = self.entries();
            let index = entries
                .picked
                .iter()
                .position(|picked| picked.id == picked_id)
                .ok_or(ImagesError::NotFound)?;
            entries.picked.remove(index).ok_or(ImagesError::NotFound)?
        };
        let Picked { path, name, .. } = picked;
        self.decode_and_keep(move || {
            let bytes = read_file(&path)?;
            prepare(&name, &bytes)
        })
        .await
    }

    /// Prepares the bytes of a pasted or dropped file.
    pub(crate) async fn prepare_bytes(
        &self,
        name: String,
        bytes: Vec<u8>,
    ) -> Result<(String, SourcedImage), ImagesError> {
        self.decode_and_keep(move || prepare(&name, &bytes)).await
    }

    /// Runs one decode at a time on a blocking thread, and keeps what it
    /// prepared under a new key.
    async fn decode_and_keep<F>(&self, work: F) -> Result<(String, SourcedImage), ImagesError>
    where
        F: FnOnce() -> Result<SourcedImage, ImagesError> + Send + 'static,
    {
        let permit = Arc::clone(&self.decoding)
            .acquire_owned()
            .await
            .map_err(|_closed| ImagesError::PluginOff)?;
        let image = tokio::task::spawn_blocking(move || {
            let prepared = work();
            drop(permit);
            prepared
        })
        .await
        .map_err(|error| {
            tracing::error!(%error, "preparing an image ended without a result");
            ImagesError::Unreadable
        })??;
        let key = new_id();
        tracing::debug!(
            name = image.name,
            width = image.width,
            height = image.height,
            bytes = image.data.len(),
            "an image was prepared"
        );
        self.keep(key.clone(), image.clone());
        Ok((key, image))
    }

    fn keep(&self, key: String, image: SourcedImage) {
        let mut entries = self.entries();
        entries.prepared.push_back(Prepared { key, image });
        loop {
            let total: usize = entries.prepared.iter().map(Prepared::bytes).sum();
            let over = entries.prepared.len() > MAX_PREPARED || total > MAX_PREPARED_BYTES;
            if !over {
                break;
            }
            entries.prepared.pop_front();
        }
    }

    /// The prepared image with this key.
    pub(crate) fn get(&self, key: &str) -> Option<SourcedImage> {
        self.entries()
            .prepared
            .iter()
            .find(|prepared| prepared.key == key)
            .map(|prepared| prepared.image.clone())
    }

    /// Drops the prepared images with these keys; unknown keys change
    /// nothing.
    pub(crate) fn discard(&self, keys: &[String]) {
        self.entries()
            .prepared
            .retain(|prepared| !keys.contains(&prepared.key));
    }

    /// The number of waiting files and images, for the tests.
    #[cfg(test)]
    pub(crate) fn counts(&self) -> (usize, usize) {
        let entries = self.entries();
        (entries.picked.len(), entries.prepared.len())
    }
}

/// Reads a chosen file, refusing one larger than the plugin reads before
/// reading it.
fn read_file(path: &std::path::Path) -> Result<Vec<u8>, ImagesError> {
    let unreadable = |error: std::io::Error| {
        tracing::info!(kind = %error.kind(), "a chosen image file could not be read");
        ImagesError::FileUnreadable
    };
    let size = std::fs::metadata(path).map_err(unreadable)?.len();
    if size > u64::try_from(MAX_FILE_BYTES).unwrap_or(u64::MAX) {
        return Err(ImagesError::file_too_large());
    }
    std::fs::read(path).map_err(unreadable)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mannie_runtime::SourcedImage;
    use mannie_types::images::{ImageBytes, ImageFormat};

    use super::{MAX_PICKED, MAX_PREPARED, MAX_PREPARED_BYTES, Shelf};
    use crate::commands::ImagesError;

    fn image(bytes: usize) -> SourcedImage {
        SourcedImage {
            name: "a.png".to_owned(),
            format: ImageFormat::Png,
            width: 1,
            height: 1,
            data: ImageBytes::from(vec![0; bytes]),
            thumbnail_format: ImageFormat::Png,
            thumbnail: ImageBytes::from(Vec::new()),
        }
    }

    #[test]
    fn the_oldest_images_go_first_beyond_the_count_or_the_bytes() {
        let shelf = Shelf::default();
        for number in 0..=MAX_PREPARED {
            shelf.keep(number.to_string(), image(1));
        }
        assert_eq!(shelf.counts().1, MAX_PREPARED);
        assert!(shelf.get("0").is_none());
        assert!(shelf.get("1").is_some());

        shelf.keep("big".to_owned(), image(MAX_PREPARED_BYTES - 10));
        shelf.keep("small".to_owned(), image(20));
        assert!(shelf.get("big").is_none());
        assert!(shelf.get("small").is_some());
    }

    #[test]
    fn discarded_images_leave_and_unknown_keys_change_nothing() {
        let shelf = Shelf::default();
        shelf.keep("a".to_owned(), image(1));
        shelf.keep("b".to_owned(), image(1));

        shelf.discard(&["a".to_owned(), "unknown".to_owned()]);

        assert!(shelf.get("a").is_none());
        assert!(shelf.get("b").is_some());
    }

    #[test]
    fn chosen_files_keep_their_names_and_the_oldest_go_first() {
        let shelf = Shelf::default();
        let paths = (0..=MAX_PICKED)
            .map(|number| std::path::PathBuf::from(format!("/photos/{number}.jpg")))
            .collect();

        let picked = shelf.pick(paths);

        assert_eq!(picked.len(), MAX_PICKED + 1);
        assert_eq!(picked[1].name, "1.jpg");
        assert_eq!(shelf.counts().0, MAX_PICKED);
    }

    #[tokio::test]
    async fn a_chosen_file_is_read_once_and_a_large_one_never() {
        let folder = tempfile::tempdir().unwrap();
        let text = folder.path().join("notes.png");
        std::fs::write(&text, "not an image").unwrap();
        let large = folder.path().join("large.png");
        std::fs::File::create(&large)
            .unwrap()
            .set_len(21 * 1024 * 1024)
            .unwrap();
        let shelf = Arc::new(Shelf::default());
        let picked = shelf.pick(vec![text, large, folder.path().join("gone.png")]);

        let not_image = shelf.prepare_picked(&picked[0].picked_id).await;
        let again = shelf.prepare_picked(&picked[0].picked_id).await;
        let too_large = shelf.prepare_picked(&picked[1].picked_id).await;
        let gone = shelf.prepare_picked(&picked[2].picked_id).await;

        assert_eq!(not_image.unwrap_err(), ImagesError::UnsupportedFormat);
        assert_eq!(again.unwrap_err(), ImagesError::NotFound);
        assert_eq!(too_large.unwrap_err(), ImagesError::file_too_large());
        assert_eq!(gone.unwrap_err(), ImagesError::FileUnreadable);
        assert_eq!(shelf.counts(), (0, 0));
    }
}
