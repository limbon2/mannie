//! The IPC commands (ADR-0007), one module per area. `ipc` registers them with
//! tauri-specta, and each one needs a permission (`build.rs`) and a grant
//! (`capabilities/`), or Tauri refuses it (ADR-0004).

pub(crate) mod interface;
pub(crate) mod startup;
#[cfg(feature = "validation")]
pub(crate) mod validation;
