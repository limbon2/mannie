//! Writes the core's TypeScript IPC bindings, the envelopes and the transport
//! of plugin calls (`task gen`, ADR-0007, ADR-0034):
//! `mannie-desktop-bindings <folder>`. The plugin bindings program that `task
//! gen` generates writes each plugin's bindings (ADR-0035, decision 3).

use std::path::PathBuf;

use anyhow::Context as _;

fn main() -> anyhow::Result<()> {
    let directory = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .context("name the folder for the bindings: mannie-desktop-bindings <folder>")?;
    mannie_desktop_core::export_bindings(&directory)?;
    Ok(())
}
