//! The images that wait in message boxes while the plugin is on: the files
//! that the owner chose, until they are prepared, and the prepared images,
//! until their message is stored or the owner removes them
//! (plugins/images/docs/README.md, the plugin). Everything lives in memory, within
//! bounds, and the oldest go first. A chosen file is a handle of the service
//! `file-dialog`, which reads it once; its path stays in the core (T-34).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mannie_plugin_sdk::contracts::clean_image_name;
use mannie_plugin_sdk::points::images::SourcedImage;
use mannie_plugin_sdk::services::file_dialog::{
    ChosenFile, DialogError, FileDialogApi, FileFilter, FileHandle, MAX_CHOSEN_FILES,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::commands::{ImagesError, MAX_FILE_BYTES, PickedImage};
use crate::prepare::prepare;

/// The most chosen files that wait to be prepared.
const MAX_PICKED: usize = 40;

// The service keeps at least as many chosen files as the shelf, so a file
// that the shelf keeps can always be read.
const _: () = assert!(MAX_PICKED <= MAX_CHOSEN_FILES);

/// The most prepared images that wait for their message.
const MAX_PREPARED: usize = 40;

/// The most bytes of prepared images and thumbnails that wait.
const MAX_PREPARED_BYTES: usize = 160 * 1024 * 1024;

/// A file that the owner chose.
#[derive(Debug)]
struct Picked {
    id: String,
    file: FileHandle,
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
    /// One image is read and decoded at a time, so that a drop or a choice
    /// of many large files does not hold them all in memory at once (T-32).
    decoding: Arc<Semaphore>,
    /// The system's file dialog, bound to this start of the plugin.
    dialog: Arc<dyn FileDialogApi>,
}

fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

impl Shelf {
    /// An empty shelf, whose chosen files come from `dialog`.
    pub(crate) fn new(dialog: Arc<dyn FileDialogApi>) -> Self {
        Self {
            entries: Mutex::new(Entries::default()),
            decoding: Arc::new(Semaphore::new(1)),
            dialog,
        }
    }

    fn entries(&self) -> MutexGuard<'_, Entries> {
        // Every critical section only adds, takes or removes entries.
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Shows the system's file dialog with `filter`, and returns the files
    /// that the owner chose, none when the owner closed it.
    pub(crate) async fn choose(&self, filter: FileFilter) -> Result<Vec<ChosenFile>, ImagesError> {
        self.dialog
            .choose_files(filter)
            .await
            .map_err(|error| dialog_refusal(&error))
    }

    /// Keeps the files that the owner chose, and returns their IDs and names.
    pub(crate) fn pick(&self, files: Vec<ChosenFile>) -> Vec<PickedImage> {
        let mut entries = self.entries();
        let mut picked = Vec::with_capacity(files.len());
        for chosen in files {
            let name = clean_image_name(&chosen.name);
            let id = new_id();
            picked.push(PickedImage {
                picked_id: id.clone(),
                name: name.clone(),
            });
            entries.picked.push_back(Picked {
                id,
                file: chosen.file,
                name,
            });
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
        let Picked { file, name, .. } = picked;
        // The permit comes before the read, so one chosen file is in memory
        // at a time (T-32).
        let permit = self.permit().await?;
        let bytes = self
            .dialog
            .read(file, max_file_bytes())
            .await
            .map_err(|error| dialog_refusal(&error))?;
        self.decode_and_keep(permit, move || prepare(&name, &bytes))
            .await
    }

    /// Prepares the bytes of a pasted or dropped file.
    pub(crate) async fn prepare_bytes(
        &self,
        name: String,
        bytes: Vec<u8>,
    ) -> Result<(String, SourcedImage), ImagesError> {
        let permit = self.permit().await?;
        self.decode_and_keep(permit, move || prepare(&name, &bytes))
            .await
    }

    /// The turn to read and decode one image.
    async fn permit(&self) -> Result<OwnedSemaphorePermit, ImagesError> {
        // The semaphore is never closed, so this refusal stays theoretical.
        Arc::clone(&self.decoding)
            .acquire_owned()
            .await
            .map_err(|_closed| {
                tracing::error!("the turn to decode an image could not be taken");
                ImagesError::Unreadable
            })
    }

    /// Decodes on a blocking thread, under the turn that `permit` holds, and
    /// keeps what it prepared under a new key.
    async fn decode_and_keep<F>(
        &self,
        permit: OwnedSemaphorePermit,
        work: F,
    ) -> Result<(String, SourcedImage), ImagesError>
    where
        F: FnOnce() -> Result<SourcedImage, ImagesError> + Send + 'static,
    {
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

/// The most bytes of a chosen file that the plugin reads.
fn max_file_bytes() -> u64 {
    u64::try_from(MAX_FILE_BYTES).unwrap_or(u64::MAX)
}

/// The plugin's error for a failure of the file dialog or of a read. The
/// service refuses a file larger than the plugin reads before reading it.
fn dialog_refusal(error: &DialogError) -> ImagesError {
    match error {
        DialogError::Failed | DialogError::InvalidFilter => {
            tracing::warn!(
                error.code = error.code(),
                "the file dialog could not choose images"
            );
            ImagesError::DialogFailed
        }
        DialogError::TooLarge { .. } => ImagesError::file_too_large(),
        DialogError::UnknownFile => ImagesError::NotFound,
        DialogError::Unreadable => ImagesError::FileUnreadable,
        // A reason of a later SDK is a file that cannot be read.
        _other => ImagesError::FileUnreadable,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::Arc;
    use std::time::Duration;

    use image::{DynamicImage, ImageBuffer, ImageFormat as Format, Rgb};
    use mannie_plugin_sdk::contracts::{ImageBytes, ImageFormat};
    use mannie_plugin_sdk::points::images::SourcedImage;
    use mannie_plugin_sdk::services::file_dialog::FileFilter;
    use mannie_plugin_sdk::testing::{FakeFile, FakeFileDialog};

    use super::{MAX_PICKED, MAX_PREPARED, MAX_PREPARED_BYTES, Shelf};
    use crate::commands::{ImagesError, MAX_FILE_BYTES};

    fn shelf(fake: &FakeFileDialog) -> Arc<Shelf> {
        Arc::new(Shelf::new(fake.api()))
    }

    fn images() -> FileFilter {
        FileFilter::new("Images", ["png"])
    }

    /// A PNG of 4 by 4 pixels.
    fn png() -> Vec<u8> {
        let image = ImageBuffer::from_fn(4, 4, |_x, _y| Rgb([9_u8, 9, 9]));
        let mut png = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image)
            .write_to(&mut png, Format::Png)
            .unwrap();
        png.into_inner()
    }

    fn image(bytes: usize) -> SourcedImage {
        let name = "a.png".to_owned();
        let format = ImageFormat::Png;
        let width = 1;
        let height = 1;
        let data = ImageBytes::from(vec![0; bytes]);
        let thumbnail_format = ImageFormat::Png;
        let thumbnail = ImageBytes::from(Vec::new());
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
    fn the_oldest_images_go_first_beyond_the_count_or_the_bytes() {
        let shelf = Shelf::new(FakeFileDialog::new().api());
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
        let shelf = Shelf::new(FakeFileDialog::new().api());
        shelf.keep("a".to_owned(), image(1));
        shelf.keep("b".to_owned(), image(1));

        shelf.discard(&["a".to_owned(), "unknown".to_owned()]);

        assert!(shelf.get("a").is_none());
        assert!(shelf.get("b").is_some());
    }

    #[tokio::test]
    async fn chosen_files_keep_their_names_and_the_oldest_go_first() {
        let fake = FakeFileDialog::new();
        let shelf = shelf(&fake);
        let mut files: Vec<FakeFile> = (0..MAX_PICKED)
            .map(|number| FakeFile::new(format!("{number}.jpg"), Vec::new()))
            .collect();
        files.insert(1, FakeFile::new("", Vec::new()));
        fake.answer(files);

        let chosen = shelf.choose(images()).await.unwrap();
        let picked = shelf.pick(chosen);

        assert_eq!(picked.len(), MAX_PICKED + 1);
        assert_eq!(picked[0].name, "0.jpg");
        assert_eq!(picked[1].name, "image", "a blank name gets the default");
        assert_eq!(picked[2].name, "1.jpg");
        assert_eq!(shelf.counts().0, MAX_PICKED);
        assert_eq!(fake.filters(), [images()]);
    }

    #[tokio::test]
    async fn a_chosen_file_is_read_once_and_a_large_one_never() {
        let fake = FakeFileDialog::new();
        let shelf = shelf(&fake);
        fake.answer(vec![
            FakeFile::new("notes.png", b"not an image".to_vec()),
            FakeFile::new("large.png", vec![0; MAX_FILE_BYTES + 1]),
            FakeFile::unreadable("gone.png"),
        ]);
        let chosen = shelf.choose(images()).await.unwrap();
        let picked = shelf.pick(chosen);

        let not_image = shelf.prepare_picked(&picked[0].picked_id).await;
        let again = shelf.prepare_picked(&picked[0].picked_id).await;
        let too_large = shelf.prepare_picked(&picked[1].picked_id).await;
        let gone = shelf.prepare_picked(&picked[2].picked_id).await;
        let limit = u64::try_from(MAX_FILE_BYTES).unwrap();

        assert_eq!(not_image.unwrap_err(), ImagesError::UnsupportedFormat);
        assert_eq!(again.unwrap_err(), ImagesError::NotFound);
        assert_eq!(too_large.unwrap_err(), ImagesError::file_too_large());
        assert_eq!(gone.unwrap_err(), ImagesError::FileUnreadable);
        assert_eq!(
            fake.reads(),
            [
                ("notes.png".to_owned(), limit),
                ("large.png".to_owned(), limit),
                ("gone.png".to_owned(), limit)
            ],
            "the first file was read once"
        );
        assert_eq!(shelf.counts(), (0, 0));
    }

    #[tokio::test]
    async fn a_failed_dialog_is_dialog_failed_and_a_closed_one_keeps_nothing() {
        let fake = FakeFileDialog::new();
        let shelf = shelf(&fake);
        fake.fail_next();

        let failed = shelf.choose(images()).await;
        let closed = shelf.choose(images()).await.unwrap();
        let picked = shelf.pick(closed);

        assert_eq!(failed.unwrap_err(), ImagesError::DialogFailed);
        assert_eq!(picked, Vec::new());
        assert_eq!(shelf.counts(), (0, 0));
        assert_eq!(fake.opened(), 2);
    }

    #[tokio::test]
    async fn one_chosen_file_is_read_and_decoded_at_a_time() {
        let fake = FakeFileDialog::new();
        let shelf = shelf(&fake);
        fake.answer(vec![
            FakeFile::new("a.png", png()),
            FakeFile::new("b.png", png()),
        ]);
        let chosen = shelf.choose(images()).await.unwrap();
        let picked = shelf.pick(chosen);
        let gate = fake.hold();

        let first = tokio::spawn({
            let shelf = Arc::clone(&shelf);
            let id = picked[0].picked_id.clone();
            async move { shelf.prepare_picked(&id).await }
        });
        let second = tokio::spawn({
            let shelf = Arc::clone(&shelf);
            let id = picked[1].picked_id.clone();
            async move { shelf.prepare_picked(&id).await }
        });
        tokio::time::timeout(Duration::from_secs(10), async {
            while fake.reads().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // The second read waits for the first file's turn to end.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let reads_while_held = fake.reads().len();
        gate.open();
        let prepared = [first.await.unwrap(), second.await.unwrap()];

        assert_eq!(reads_while_held, 1);
        assert!(prepared.iter().all(Result::is_ok), "{prepared:?}");
        assert_eq!(fake.reads().len(), 2);
        assert_eq!(shelf.counts(), (0, 2));
    }
}
