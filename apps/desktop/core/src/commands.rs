//! The IPC commands (ADR-0007), one module per area. `ipc` registers them with
//! tauri-specta, and each one needs a permission (`build.rs`) and a grant
//! (`capabilities/`), or Tauri refuses it (ADR-0004). The plugins' commands
//! are not here: they reach the host's dispatch through `plugin_ipc`
//! (ADR-0034, decision 8).

pub(crate) mod chat;
pub(crate) mod child_agents;
pub(crate) mod export;
pub(crate) mod interface;
pub(crate) mod links;
pub(crate) mod plugins;
pub(crate) mod projects;
pub(crate) mod providers;
pub(crate) mod startup;
pub(crate) mod tools;
#[cfg(feature = "validation")]
pub(crate) mod validation;
pub(crate) mod window;
