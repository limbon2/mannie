//! The health marker (docs/features/workers/compatibility-and-updates.md,
//! updating a worker): the running program writes `<data>/health` once it
//! opened its store and listens, and removes it when it stops. `update`
//! waits for the marker of the new version before it keeps that version.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The marker's file name in the data folder.
pub(crate) const HEALTH_FILE: &str = "health";

/// What the marker says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Health {
    /// The version of the program that runs.
    pub(crate) version: String,
    /// Its process.
    pub(crate) process_id: u32,
}

/// Writes the marker of this program.
pub(crate) fn write(data: &Path) -> io::Result<()> {
    let health = Health {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        process_id: std::process::id(),
    };
    let text = serde_json::to_vec(&health).map_err(io::Error::other)?;
    let path = data.join(HEALTH_FILE);
    let temporary = path.with_extension("new");
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, &path)
}

/// Removes the marker, if there is one.
pub(crate) fn remove(data: &Path) -> io::Result<()> {
    match std::fs::remove_file(data.join(HEALTH_FILE)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// The marker, while there is a valid one.
pub(crate) fn read(data: &Path) -> Option<Health> {
    let text = std::fs::read(data.join(HEALTH_FILE)).ok()?;
    serde_json::from_slice(&text).ok()
}
