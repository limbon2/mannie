//! The Proxmox plugin (`plugins/proxmox/`): manages explicitly allowed Proxmox
//! VE QEMU virtual machines and live VNC consoles under the master's policy
//! and journal (ADR-0080).

mod api;
pub mod commands;
mod keysyms;
mod management;
mod mentions;
pub mod model;
mod plugin;
mod rfb;
mod section;
mod state;
pub mod store;
mod tls;
mod tools;
mod vnc;
mod ws;

pub use model::PROXMOX;
pub use plugin::{ProxmoxPlugin, STORAGE_FAILED, WORK_VNC_SESSIONS};
pub use store::MODULE;
pub use tools::{
    TOOL_ATTACH_ISO, TOOL_EJECT_ISO, TOOL_GET_VM_CONFIG, TOOL_LIST_ISOS, TOOL_LIST_VMS,
    TOOL_SET_BOOT_ORDER, TOOL_VM_POWER, TOOL_VNC_CLOSE, TOOL_VNC_KEYBOARD, TOOL_VNC_POINTER,
    TOOL_VNC_SCREENSHOT,
};

mannie_plugin_sdk::export_plugin!(ProxmoxPlugin::new);
