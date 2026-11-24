//! The Network Client plugin (ADR-0075; plugins/network/docs/README.md):
//! Postman-style API collections with `OpenAPI` v2 and v3 JSON/YAML import,
//! custom variables, headers, query and path parameters, cookie jar, owner
//! workbench execution and redacted agent tools under per-endpoint approvals.

pub mod commands;
mod http;
mod mentions;
pub mod model;
mod openapi;
pub mod plugin;
mod redact;
mod secrets;
mod section;
mod state;
mod store;
mod template;
pub mod tools;

pub use plugin::{NETWORK, NetworkPlugin};
pub use store::MODULE;

mannie_plugin_sdk::export_plugin!(NetworkPlugin::new);
