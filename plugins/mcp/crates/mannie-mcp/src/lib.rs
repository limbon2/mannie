//! The MCP plugin (ADR-0043; plugins/mcp/docs/README.md): tool servers of
//! the Model Context Protocol that the owner registers by hand, local
//! programs that the core starts and remote URLs of Streamable HTTP, with
//! secret variables and headers and OAuth sign-in. Agents use the servers'
//! tools under the master's policy and journal, read their resources with
//! `mcp_read_resource`, and the owner mentions resources with "@".
//!
//! The plugin speaks MCP through rmcp in both protocol eras, the stateless
//! era of 2026-07-28 and the `initialize` handshake before it
//! (plugins/mcp/docs/adr/0001-speak-mcp-through-rmcp-in-both-protocol-
//! eras.md), and signs in with OAuth through rmcp's `auth`
//! (plugins/mcp/docs/adr/0002-sign-in-to-remote-servers-with-oauth.md). The
//! servers' secrets live in the credential store.

mod callback;
mod client;
pub mod commands;
mod config;
mod imports;
mod log;
mod mentions;
mod naming;
mod oauth;
mod offered;
pub mod plugin;
mod resources;
mod results;
pub mod rules;
mod schema;
mod secrets;
mod section;
mod servers;
mod source;
mod store;
mod texts;
mod tool;

pub use plugin::{MCP, McpPlugin};
pub use store::MODULE;

mannie_plugin_sdk::export_plugin!(McpPlugin::new);
