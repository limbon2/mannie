//! The plugin's image source (ADR-0033, decision 2): it gives the runtime the
//! prepared images that a message names, and lets them go once the message is
//! stored.

use std::sync::Arc;

use futures_util::FutureExt;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::points::images::{ImageSource, SourcedImage};

use crate::plugin::IMAGES;
use crate::shelf::Shelf;

/// The code of a key that names no waiting image, such as one that was
/// removed, or one from before the plugin was turned off.
pub(crate) const NOT_FOUND: &str = "not_found";

/// The image source of the plugin while it is on.
#[derive(Debug)]
pub(crate) struct ShelfSource {
    shelf: Arc<Shelf>,
}

impl ShelfSource {
    pub(crate) fn new(shelf: Arc<Shelf>) -> Self {
        Self { shelf }
    }
}

impl ImageSource for ShelfSource {
    fn id(&self) -> &str {
        IMAGES
    }

    fn resolve<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<SourcedImage, String>> {
        let image = self.shelf.get(key).ok_or_else(|| NOT_FOUND.to_owned());
        futures_util::future::ready(image).boxed()
    }

    fn release(&self, keys: &[String]) {
        self.shelf.discard(keys);
    }
}
