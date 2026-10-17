//! The Images plugin (ADR-0033; docs/features/images.md): the images that the
//! owner attaches to a message in the message box, read from files that the
//! owner chose in the system's dialog or from a paste or a drop, prepared for
//! the model, and kept until their message is stored. Its image source gives
//! them to the runtime.
//!
//! The core never depends on this crate (ADR-0030, decision 10). The desktop
//! app's composition root starts and stops the plugin, opens the file dialog
//! and calls into the plugin from its IPC commands.

pub mod commands;
pub mod plugin;
mod prepare;
mod shelf;
mod source;

pub use plugin::{IMAGES, ImagesPlugin};
