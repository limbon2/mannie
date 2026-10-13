//! Writes the TypeScript IPC bindings (`task gen`, ADR-0007):
//! `mannie-desktop-bindings <folder>`.

use std::path::PathBuf;

use anyhow::Context as _;

fn main() -> anyhow::Result<()> {
    let directory = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .context("name the folder for the bindings: mannie-desktop-bindings <folder>")?;
    mannie_desktop::export_bindings(&directory)?;
    Ok(())
}
