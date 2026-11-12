//! The source of updates in an installed release build (ADR-0063, decisions
//! 2 to 4): Tauri's updater plugin reads the manifest of the latest release
//! of Mannie's repository, downloads the installer of this installation's
//! kind and verifies its signature against the key in the app's
//! configuration. The webview has no permission of the plugin, so only this
//! module uses it.

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use mannie_plugin_sdk::BoxFuture;
use mannie_types::updates::UpdateFailureCode;
use tauri::utils::config::BundleType;
use tauri::{AppHandle, Runtime, Url};
use tauri_plugin_updater::{Error as PluginError, Update, Updater, UpdaterExt};

use super::source::{InstallError, Offer, Package, Progress, Source};

/// The manifest of the latest release. GitHub never serves a draft or a
/// pre-release under "latest", so publishing a release is what offers it.
const MANIFEST: &str = "https://github.com/limbon2/mannie/releases/latest/download/latest.json";

/// Where the files of Mannie's releases download from. A manifest that names
/// another address is refused (ADR-0063, decision 3).
const DOWNLOADS: &str = "https://github.com/limbon2/mannie/releases/download/";

/// How long the request for the manifest may take.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a download may take in all.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_mins(30);

/// How long a request may stay without data.
const STALL_TIMEOUT: Duration = Duration::from_secs(60);

/// The kinds of packages that update themselves. Tauri's bundler marks each
/// binary with the kind of the package that carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Installation {
    /// The Windows installer.
    Nsis,
    /// The package for Debian and Ubuntu.
    Deb,
    /// The package for Fedora and its relatives.
    Rpm,
}

impl Installation {
    /// The kind of the package that installed this binary. `None` for a
    /// binary that no package of these kinds carried, such as one that was
    /// built and started by hand.
    pub(crate) fn of_this_binary() -> Option<Self> {
        match tauri::utils::platform::bundle_type()? {
            BundleType::Nsis => Some(Self::Nsis),
            BundleType::Deb => Some(Self::Deb),
            BundleType::Rpm => Some(Self::Rpm),
            _ => None,
        }
    }

    /// The name in the keys of the manifest's platforms.
    fn name(self) -> &'static str {
        match self {
            Self::Nsis => "nsis",
            Self::Deb => "deb",
            Self::Rpm => "rpm",
        }
    }
}

/// The bundler's mark of this binary, for the start line of the log. The
/// install checks of the release workflow read it there, so a build whose
/// bundler did not mark it fails before its release.
pub(crate) fn bundle_mark() -> String {
    tauri::utils::platform::bundle_type()
        .map_or_else(|| "unmarked".to_owned(), |kind| kind.to_string())
}

/// The key of this installation in the manifest's platforms, such as
/// `windows-x86_64-nsis`. Naming it keeps the plugin from choosing another
/// package of the same system.
fn target(installation: Installation) -> Option<String> {
    let system = tauri_plugin_updater::target()?;
    Some(format!("{system}-{}", installation.name()))
}

/// The releases of Mannie's repository, read through the updater plugin.
pub(crate) struct ReleaseSource<R: Runtime> {
    app: AppHandle<R>,
    installation: Installation,
    manifest: String,
    downloads: String,
    /// Closes the log right before a started install ends the process on
    /// Windows (ADR-0063, decision 11).
    close_log: Arc<dyn Fn() + Send + Sync>,
}

impl<R: Runtime> ReleaseSource<R> {
    /// The source of an installed release build.
    pub(crate) fn new(
        app: AppHandle<R>,
        installation: Installation,
        close_log: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            app,
            installation,
            manifest: MANIFEST.to_owned(),
            downloads: DOWNLOADS.to_owned(),
            close_log,
        }
    }

    fn updater(&self) -> Result<Updater, UpdateFailureCode> {
        let manifest = Url::parse(&self.manifest).map_err(|error| {
            tracing::error!(%error, "the address of the update manifest is not a URL");
            UpdateFailureCode::NoUpdateInformation
        })?;
        let target = target(self.installation).ok_or_else(|| {
            tracing::warn!("no release of Mannie is built for this system");
            UpdateFailureCode::NoPackageForThisInstallation
        })?;
        let close_log = Arc::clone(&self.close_log);
        self.app
            .updater_builder()
            .endpoints(vec![manifest])
            .map_err(|error| check_failure(&error))?
            .target(target)
            .timeout(CHECK_TIMEOUT)
            .configure_client(|client| client.read_timeout(STALL_TIMEOUT))
            // The hook replaces the plugin's own, which only cleans up what
            // Tauri's event loop already cleaned up when it ended.
            .on_before_exit(move || close_log())
            .build()
            .map_err(|error| check_failure(&error))
    }
}

impl<R: Runtime> Source for ReleaseSource<R> {
    fn check(&self) -> BoxFuture<'static, Result<Option<Box<dyn Offer>>, UpdateFailureCode>> {
        let updater = self.updater();
        let downloads = self.downloads.clone();
        Box::pin(async move {
            let updater = updater?;
            let update = updater
                .check()
                .await
                .map_err(|error| check_failure(&error))?;
            let Some(update) = update else {
                return Ok(None);
            };
            // The signature decides what installs. The address is checked as
            // well, so that a manifest cannot make Mannie fetch from a place
            // that is not a release of its repository.
            if !update.download_url.as_str().starts_with(&downloads) {
                tracing::warn!(
                    version = %update.version,
                    "the update manifest names a download outside of Mannie's releases"
                );
                return Err(UpdateFailureCode::NoUpdateInformation);
            }
            let offer: Box<dyn Offer> = Box::new(ReleaseOffer { update });
            Ok(Some(offer))
        })
    }
}

/// A newer release that the manifest announced.
struct ReleaseOffer {
    update: Update,
}

impl Offer for ReleaseOffer {
    fn version(&self) -> &str {
        &self.update.version
    }

    fn download(
        mut self: Box<Self>,
        progress: Progress,
    ) -> BoxFuture<'static, Result<Box<dyn Package>, UpdateFailureCode>> {
        self.update.timeout = Some(DOWNLOAD_TIMEOUT);
        Box::pin(async move {
            let bytes = self
                .update
                .download(progress, || {})
                .await
                .map_err(|error| download_failure(&error))?;
            let package: Box<dyn Package> = Box::new(ReleasePackage {
                update: self.update,
                bytes,
            });
            Ok(package)
        })
    }
}

/// A downloaded installer whose signature the plugin verified.
struct ReleasePackage {
    update: Update,
    bytes: Vec<u8>,
}

impl Package for ReleasePackage {
    fn version(&self) -> &str {
        &self.update.version
    }

    fn install(self: Box<Self>) -> Result<(), InstallError> {
        self.update.install(&self.bytes)?;
        Ok(())
    }
}

/// The code of a failed check. A request without an answer is the only
/// failure that the owner's network explains; an answer that the plugin
/// cannot use is the release's.
fn check_failure(error: &PluginError) -> UpdateFailureCode {
    let code = match error {
        PluginError::Reqwest(request) if !request.is_decode() => UpdateFailureCode::Unreachable,
        PluginError::TargetNotFound(_)
        | PluginError::TargetsNotFound(_)
        | PluginError::UnsupportedArch
        | PluginError::UnsupportedOs => UpdateFailureCode::NoPackageForThisInstallation,
        _ => UpdateFailureCode::NoUpdateInformation,
    };
    tracing::warn!(%error, code = code.code(), "the check for a newer release failed");
    code
}

/// The code of a failed download: the signature's failures apart from the
/// transfer's.
fn download_failure(error: &PluginError) -> UpdateFailureCode {
    let code = match error {
        PluginError::Minisign(_)
        | PluginError::Base64(_)
        | PluginError::SignatureUtf8(_)
        | PluginError::SignedVersionMismatch { .. }
        | PluginError::MissingSignedVersion => UpdateFailureCode::SignatureInvalid,
        _ => UpdateFailureCode::DownloadFailed,
    };
    tracing::warn!(%error, code = code.code(), "the download of the update failed");
    code
}
