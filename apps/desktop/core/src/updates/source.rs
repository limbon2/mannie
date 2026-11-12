//! The ports of the updates module (ADR-0063): where a newer version comes
//! from, how its installer downloads and how it installs. The release build
//! implements them over Tauri's updater plugin, a validation build over a
//! fake that the end-to-end tests set, and the module's tests over fakes of
//! their own.

use mannie_plugin_sdk::BoxFuture;
use mannie_types::updates::UpdateFailureCode;

/// Hears about each part of a download: the part's length and the length of
/// the whole download when the server announced it.
pub(crate) type Progress = Box<dyn FnMut(usize, Option<u64>) + Send>;

/// Why an install did not happen. The text is only logged and noted in the
/// journal's record, never shown.
pub(crate) type InstallError = Box<dyn std::error::Error + Send + Sync>;

/// Where a newer version comes from.
pub(crate) trait Source: Send + Sync {
    /// Asks once. `None` means that no version is newer than the running one.
    fn check(&self) -> BoxFuture<'static, Result<Option<Box<dyn Offer>>, UpdateFailureCode>>;
}

/// A newer version that a source announced.
pub(crate) trait Offer: Send {
    /// The version that the source announced.
    fn version(&self) -> &str;

    /// Downloads the installer and verifies it.
    fn download(
        self: Box<Self>,
        progress: Progress,
    ) -> BoxFuture<'static, Result<Box<dyn Package>, UpdateFailureCode>>;
}

/// A downloaded and verified installer, kept in memory until Mannie installs
/// it or quits.
pub(crate) trait Package: Send {
    /// The version that the installer installs.
    fn version(&self) -> &str;

    /// Verifies the installer again and installs it, on the calling thread.
    /// On Windows a started install ends the process, so this returns only
    /// when the install could not start. On Linux it returns after the
    /// package manager did.
    fn install(self: Box<Self>) -> Result<(), InstallError>;
}
