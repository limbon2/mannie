//! The Images plugin's definition (ADR-0033, decision 10;
//! plugins/images/docs/README.md): its interface, its start and stop, and the
//! handlers of its commands. Its manifest, `plugins/images/plugin.toml`,
//! gives its ID, and it is off by default, like every plugin (D28, D31).
//!
//! Each start keeps its waiting images on a shelf of its own, which the
//! start's handlers and image source share, and which goes when the plugin
//! stops: a key or a chosen file from before is refused as `not_found`, and
//! the message box says so. The host answers `plugin_off` while Images is
//! off (ADR-0034, decision 8).

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::StartContext;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::error::StartFailure;
use mannie_plugin_sdk::ipc::{Handlers, Interface};
use mannie_plugin_sdk::plugin::{Instance, Plugin, Started};
use mannie_plugin_sdk::points::images::{ImageSources, SourcedImage};
use mannie_plugin_sdk::services::file_dialog::{FileDialog, FileFilter};

use crate::commands::{
    DiscardPreparedImageCommand, EXTENSIONS, ImagesError, PickImages, PickImagesCommand,
    PickedImage, PickedImageRef, PrepareImage, PrepareImageCommand, PreparePickedImageCommand,
    PreparedImage, PreparedImageRef, interface, max_base64_len,
};
use crate::shelf::Shelf;
use crate::source::ShelfSource;

/// The plugin's ID, which its image source shares.
pub const IMAGES: &str = "images";

/// The Images plugin.
#[derive(Debug, Clone, Copy, Default)]
pub struct ImagesPlugin;

impl ImagesPlugin {
    /// The plugin, off.
    pub fn new() -> Self {
        Self
    }
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

/// Shows the system's file dialog for images and keeps the files that the
/// owner chose until each is prepared; none when the owner closed the
/// dialog.
async fn pick(shelf: Arc<Shelf>, input: PickImages) -> Result<Vec<PickedImage>, ImagesError> {
    let filter = FileFilter::new(input.filter_name(), EXTENSIONS.iter().copied());
    let chosen = shelf.choose(filter).await?;
    Ok(shelf.pick(chosen))
}

/// Reads and prepares a file that the owner chose.
async fn prepare_picked(
    shelf: Arc<Shelf>,
    input: PickedImageRef,
) -> Result<PreparedImage, ImagesError> {
    let (key, image) = shelf.prepare_picked(&input.picked_id).await?;
    Ok(prepared(key, image))
}

/// Prepares a file that the owner pasted or dropped. Its bytes come from the
/// webview, and are checked like any file's.
async fn prepare(shelf: Arc<Shelf>, input: PrepareImage) -> Result<PreparedImage, ImagesError> {
    if input.data.len() > max_base64_len() {
        return Err(ImagesError::file_too_large());
    }
    let bytes = BASE64
        .decode(input.data.as_bytes())
        .map_err(|_invalid| ImagesError::Unreadable)?;
    let (key, image) = shelf.prepare_bytes(input.name, bytes).await?;
    Ok(prepared(key, image))
}

/// The handlers of a start, which reach its shelf.
fn handlers(shelf: &Arc<Shelf>) -> Handlers {
    let picking = Arc::clone(shelf);
    let reading = Arc::clone(shelf);
    let preparing = Arc::clone(shelf);
    let discarding = Arc::clone(shelf);
    Handlers::new()
        .command::<PickImagesCommand, _, _>(move |input| pick(Arc::clone(&picking), input))
        .command::<PreparePickedImageCommand, _, _>(move |input| {
            prepare_picked(Arc::clone(&reading), input)
        })
        .command::<PrepareImageCommand, _, _>(move |input| prepare(Arc::clone(&preparing), input))
        .command::<DiscardPreparedImageCommand, _, _>(move |input: PreparedImageRef| {
            discarding.discard(std::slice::from_ref(&input.key));
            std::future::ready(Ok(()))
        })
}

impl ImagesPlugin {
    /// Starts the plugin on the file dialog that the host bound to it.
    fn start_now(context: &StartContext) -> Result<Started, StartFailure> {
        let shelf = Arc::new(Shelf::new(context.service::<FileDialog>()?));
        tracing::info!("the Images plugin started");
        let source = Arc::new(ShelfSource::new(Arc::clone(&shelf)));
        let handlers = handlers(&shelf);
        Ok(Started::new(ImagesInstance { shelf })
            .with_contributions(Contributions::new().add::<ImageSources>(source))
            .with_handlers(handlers))
    }
}

impl Plugin for ImagesPlugin {
    fn interface(&self) -> Interface {
        interface()
    }

    fn start(&self, context: StartContext) -> BoxFuture<'static, Result<Started, StartFailure>> {
        Box::pin(std::future::ready(Self::start_now(&context)))
    }
}

/// Images while it is on: the shelf of the waiting images, which goes when
/// it stops.
#[derive(Debug)]
struct ImagesInstance {
    shelf: Arc<Shelf>,
}

impl Instance for ImagesInstance {
    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        // The host took the source out of the runtime and ended the calls of
        // this start first; the waiting images go with the shelf.
        drop(self.shelf);
        tracing::info!("the Images plugin stopped");
        Box::pin(std::future::ready(()))
    }
}
