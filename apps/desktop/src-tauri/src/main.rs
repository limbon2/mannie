//! The Mannie desktop app.

// Release builds are GUI programs without a console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    mannie_desktop::run()
}
