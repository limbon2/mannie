//! Unix paths: every name is an ordinary name, and a walked path is already
//! canonical.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

use mannie_types::tools::{PathRefusal, ToolFailure};

/// Unix has no names that reach other hosts or devices by their form alone.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the same function refuses paths on Windows"
)]
pub(super) fn check_text(_text: &str) -> Result<(), PathRefusal> {
    Ok(())
}

/// A link's target needs no conversion.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the same function refuses targets on Windows"
)]
pub(super) fn link_target(target: &Path) -> Result<PathBuf, PathRefusal> {
    Ok(target.to_path_buf())
}

/// A walked path is canonical: it has no links, no `.` and no `..`, and Unix
/// file systems have no second name for a file.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the same function canonicalizes on Windows"
)]
pub(super) fn canonical(
    path: &Path,
    _missing: usize,
    _keeps_link: bool,
) -> Result<PathBuf, ToolFailure> {
    Ok(path.to_path_buf())
}

/// Unix compares names exactly.
pub(super) fn same_name(left: &OsStr, right: &OsStr) -> bool {
    left == right
}

/// Unix reports files in use through their error kinds.
pub(super) fn is_sharing_violation(_error: &io::Error) -> bool {
    false
}
