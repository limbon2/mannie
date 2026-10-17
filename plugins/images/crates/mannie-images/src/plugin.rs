//! The Images plugin's definition (ADR-0033, decision 10;
//! docs/features/images.md): its ID, its default, its start and stop, and the
//! commands that the desktop app calls while it is on.
//!
//! Its waiting images live in memory and go when it stops: a key from before
//! is refused as `not_found`, and the message box says so.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_plugins::{Plugin, PluginStartError, StartContext};
use mannie_runtime::{PluginContributions, SourcedImage};
use mannie_store::{ModuleStore, StoreModule};

use crate::commands::{
    ImagesError, MAX_FILE_BYTES, PickedImage, PickedImageRef, PrepareImage, PreparedImage,
    PreparedImageRef,
};
use crate::shelf::Shelf;
use crate::source::ShelfSource;

/// The plugin's ID, which its image source shares.
pub const IMAGES: &str = "images";

/// The Images plugin. Cloning is cheap; clones share everything.
#[derive(Debug, Clone, Default)]
pub struct ImagesPlugin {
    /// The waiting images while the plugin is on.
    shelf: Arc<Mutex<Option<Arc<Shelf>>>>,
}

fn lock(shelf: &Mutex<Option<Arc<Shelf>>>) -> MutexGuard<'_, Option<Arc<Shelf>>> {
    // Every critical section only replaces the value.
    shelf.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The longest base64 text of a file that the plugin reads.
fn max_base64_len() -> usize {
    MAX_FILE_BYTES.div_ceil(3) * 4
}

fn prepared(key: String, image: SourcedImage) -> PreparedImage {
    PreparedImage {
        key,
        name: image.name,
        format: image.format,
        width: image.width,
        height: image.height,
        bytes: u32::try_from(image.data.len()).unwrap_or(u32::MAX),
        thumbnail_format: image.thumbnail_format,
        thumbnail: BASE64.encode(image.thumbnail.as_slice()),
    }
}

impl ImagesPlugin {
    /// The plugin, off.
    pub fn new() -> Self {
        Self::default()
    }

    /// The waiting images while the plugin is on; its commands refuse with
    /// `plugin_off` while it is off (ADR-0030, decision 5).
    fn shelf(&self) -> Result<Arc<Shelf>, ImagesError> {
        lock(&self.shelf).clone().ok_or(ImagesError::PluginOff)
    }

    /// Refuses with `plugin_off` while the plugin is off, so that the file
    /// dialog does not open for nothing.
    pub fn check_on(&self) -> Result<(), ImagesError> {
        self.shelf().map(drop)
    }

    /// Keeps the files that the owner chose in the dialog, until each is
    /// prepared.
    pub fn pick(&self, paths: Vec<PathBuf>) -> Result<Vec<PickedImage>, ImagesError> {
        Ok(self.shelf()?.pick(paths))
    }

    /// Reads and prepares a file that the owner chose.
    pub async fn prepare_picked(
        &self,
        input: PickedImageRef,
    ) -> Result<PreparedImage, ImagesError> {
        let (key, image) = self.shelf()?.prepare_picked(&input.picked_id).await?;
        Ok(prepared(key, image))
    }

    /// Prepares a file that the owner pasted or dropped. Its bytes come from
    /// the webview, and are checked like any file's.
    pub async fn prepare(&self, input: PrepareImage) -> Result<PreparedImage, ImagesError> {
        let waiting = self.shelf()?;
        if input.data.len() > max_base64_len() {
            return Err(ImagesError::file_too_large());
        }
        let bytes = BASE64
            .decode(input.data.as_bytes())
            .map_err(|_invalid| ImagesError::Unreadable)?;
        let (key, image) = waiting.prepare_bytes(input.name, bytes).await?;
        Ok(prepared(key, image))
    }

    /// Drops a waiting image that the owner removed from the message box.
    pub fn discard(&self, input: &PreparedImageRef) -> Result<(), ImagesError> {
        self.shelf()?.discard(std::slice::from_ref(&input.key));
        Ok(())
    }
}

impl Plugin for ImagesPlugin {
    fn id(&self) -> &'static str {
        IMAGES
    }

    fn store_modules(&self) -> Vec<StoreModule> {
        Vec::new()
    }

    /// Images starts off, like every plugin (D28, D31).
    fn default_on(&self, _stores: Vec<ModuleStore>) -> BoxFuture<'static, bool> {
        futures_util::future::ready(false).boxed()
    }

    fn start(
        &self,
        _context: StartContext,
    ) -> BoxFuture<'static, Result<PluginContributions, PluginStartError>> {
        let waiting = Arc::new(Shelf::default());
        *lock(&self.shelf) = Some(Arc::clone(&waiting));
        tracing::info!("the Images plugin started");
        futures_util::future::ready(Ok(PluginContributions {
            images: vec![Arc::new(ShelfSource::new(waiting))],
            ..PluginContributions::default()
        }))
        .boxed()
    }

    fn stop(&self) -> BoxFuture<'static, ()> {
        // The host took the source out of the runtime first; the waiting
        // images go with the shelf.
        lock(&self.shelf).take();
        tracing::info!("the Images plugin stopped");
        futures_util::future::ready(()).boxed()
    }
}
