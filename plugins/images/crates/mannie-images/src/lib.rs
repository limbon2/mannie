//! The Images plugin (ADR-0033; plugins/images/docs/README.md): the images that the
//! owner attaches to a message in the message box, read from files that the
//! owner chose in the system's dialog or from a paste or a drop, prepared for
//! the model, and kept until their message is stored. Its image source gives
//! them to the runtime.
//!
//! The core never depends on this crate (ADR-0030, decision 10). The crate
//! is the plugin's entry crate: it exports the plugin with its manifest,
//! `plugins/images/plugin.toml`, for the list of plugins (ADR-0034, decision
//! 4), and the plugin host starts and stops it and dispatches its commands,
//! which reach it through its inlined Tauri plugin (decision 8). The core's
//! file dialog gives it the chosen files as handles.

pub mod commands;
pub mod plugin;
mod prepare;
mod shelf;
mod source;

pub use plugin::{IMAGES, ImagesPlugin};

mannie_plugin_sdk::export_plugin!(ImagesPlugin::new);
