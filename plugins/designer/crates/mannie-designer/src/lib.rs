//! The Designer plugin (plugins/designer/docs/README.md) keeps the owner's
//! projects and exposes them to the Designer view through the plugin SDK.

pub mod commands;
pub mod plugin;
mod store;

pub use plugin::{DESIGNER, DesignerPlugin};

mannie_plugin_sdk::export_plugin!(DesignerPlugin::new);
