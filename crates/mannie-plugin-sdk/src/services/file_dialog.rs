//! The service `file-dialog` (ADR-0034, decision 5; ADR-0033, decision 10):
//! the system's file dialog, which the core shows over the main window.
//!
//! A plugin gets a handle for each file that the owner chose, never its
//! path, and reads the file through the service, once (ADR-0004, T-34). The
//! core keeps the paths in a table of the plugin's own binding, so a handle
//! names nothing in another plugin's binding or in a later start, and the
//! table drops its oldest files beyond [`MAX_CHOSEN_FILES`].
//!
//! `tests/protocol.rs` lists this module as a known exception: a handle
//! names a file in one binding's table and reads once, so it neither clones
//! nor crosses JSON. A filter and the errors do.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::BoxFuture;
use crate::service::Service;

/// The service `file-dialog`, major 1.
#[derive(Debug)]
pub enum FileDialog {}

impl Service for FileDialog {
    const NAME: &'static str = "file-dialog";
    const MAJOR: u16 = 1;
    type Api = dyn FileDialogApi;
}

/// The most chosen files that one binding keeps for its plugin. The oldest
/// go first, and their handles then name nothing.
pub const MAX_CHOSEN_FILES: usize = 64;

/// The system's file dialog, bound to one plugin.
pub trait FileDialogApi: Send + Sync + fmt::Debug {
    /// Shows the dialog with one filter and returns the files that the owner
    /// chose, none when the owner closed it.
    fn choose_files(
        &self,
        filter: FileFilter,
    ) -> BoxFuture<'static, Result<Vec<ChosenFile>, DialogError>>;

    /// Reads a chosen file, refusing one larger than `max_bytes` without
    /// reading it. The handle reads once.
    fn read(
        &self,
        file: FileHandle,
        max_bytes: u64,
    ) -> BoxFuture<'static, Result<Vec<u8>, DialogError>>;
}

/// What the dialog offers to choose: files with these extensions, under
/// this label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct FileFilter {
    /// The filter's name, which the dialog shows, such as "Images".
    pub label: String,
    /// The extensions without their dots, such as `png`.
    pub extensions: Vec<String>,
}

impl FileFilter {
    /// The filter `label` for `extensions`.
    pub fn new(
        label: impl Into<String>,
        extensions: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            label: label.into(),
            extensions: extensions.into_iter().map(Into::into).collect(),
        }
    }
}

/// A file that the owner chose.
#[derive(Debug)]
#[non_exhaustive]
pub struct ChosenFile {
    /// The handle that reads the file once.
    pub file: FileHandle,
    /// The file's name, without its folders, as the file system gives it.
    pub name: String,
}

impl ChosenFile {
    /// The chosen file of `file`, named `name`.
    pub fn new(file: FileHandle, name: String) -> Self {
        Self { file, name }
    }
}

/// A file that the owner chose, in the binding of the plugin that showed
/// the dialog. It is neither `Clone` nor `Copy`, and a read consumes it.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct FileHandle(u64);

#[cfg(feature = "host")]
impl FileHandle {
    /// The handle with the provider's number `raw`.
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The provider's number of the handle.
    pub fn raw(&self) -> u64 {
        self.0
    }
}

/// Why the dialog or a read failed. The core adds reasons by addition, so a
/// plugin treats a reason that it does not know as a file that cannot be
/// read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum DialogError {
    /// The dialog could not be shown.
    #[error("the file dialog could not be shown")]
    Failed,
    /// The filter's label or extensions are not valid.
    #[error("the filter of the file dialog is not valid")]
    InvalidFilter,
    /// The handle names no file of the plugin's binding: one that was read
    /// already, one of an earlier start, or one that the table dropped.
    #[error("the file is not one that the owner chose, or it was read already")]
    UnknownFile,
    /// The file is larger than the read allows.
    #[error("the file is larger than {max_bytes} bytes")]
    TooLarge {
        /// The read's limit.
        max_bytes: u64,
    },
    /// The file could not be read, such as one that was deleted.
    #[error("the file could not be read")]
    Unreadable,
}

impl DialogError {
    /// The error's stable code, which a plugin logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Failed => "file_dialog.failed",
            Self::InvalidFilter => "file_dialog.invalid_filter",
            Self::UnknownFile => "file_dialog.unknown_file",
            Self::TooLarge { .. } => "file_dialog.too_large",
            Self::Unreadable => "file_dialog.unreadable",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DialogError, FileFilter};

    #[test]
    fn a_filter_keeps_its_label_and_extensions() {
        let filter = FileFilter::new("Images", ["png", "jpg"]);

        assert_eq!(filter.label, "Images");
        assert_eq!(filter.extensions, ["png", "jpg"]);
    }

    #[test]
    fn every_error_has_a_code_of_its_own() {
        let codes = [
            DialogError::Failed,
            DialogError::InvalidFilter,
            DialogError::UnknownFile,
            DialogError::TooLarge { max_bytes: 1 },
            DialogError::Unreadable,
        ]
        .map(|error| error.code());

        let distinct: std::collections::BTreeSet<&str> = codes.iter().copied().collect();
        assert_eq!(distinct.len(), codes.len());
    }
}
