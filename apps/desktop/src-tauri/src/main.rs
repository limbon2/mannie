//! The Mannie desktop app: Tauri's context, which the build script compiles
//! from `tauri.conf.json` and the capabilities, and the plugins that `task
//! gen` lists, handed to the desktop app's library (ADR-0035, decision 4).

// Release builds are GUI programs without a console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    mannie_desktop_core::run(tauri::generate_context!(), mannie_plugin_list::plugins())
}
