//! The fake source of a validation build's updates (ADR-0063, decision 5).
//! The end-to-end tests set what it offers and read the installs that were
//! asked for. A validation build never contacts the releases and installs
//! nothing.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mannie_plugin_sdk::BoxFuture;
use mannie_types::updates::UpdateFailureCode;
use mannie_types::validation::ValidationUpdateOffer;

use super::source::{InstallError, Offer, Package, Progress, Source};

/// What the end-to-end tests set and read.
#[derive(Default)]
pub(crate) struct Controls {
    offer: Mutex<ValidationUpdateOffer>,
    installs: Mutex<Vec<String>>,
}

impl Controls {
    /// Sets what the next checks find.
    pub(crate) fn set_offer(&self, offer: ValidationUpdateOffer) {
        *lock(&self.offer) = offer;
    }

    /// Records an install that was asked for.
    pub(crate) fn record_install(&self, version: &str) {
        lock(&self.installs).push(version.to_owned());
    }

    /// The versions whose install was asked for, oldest first.
    pub(crate) fn installs(&self) -> Vec<String> {
        lock(&self.installs).clone()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Answers a check with what the controls hold.
pub(crate) struct FakeSource {
    controls: Arc<Controls>,
}

impl FakeSource {
    /// A source that reads `controls`.
    pub(crate) fn new(controls: Arc<Controls>) -> Self {
        Self { controls }
    }
}

impl Source for FakeSource {
    fn check(&self) -> BoxFuture<'static, Result<Option<Box<dyn Offer>>, UpdateFailureCode>> {
        let offer = lock(&self.controls.offer).clone();
        let found = match offer {
            ValidationUpdateOffer::Nothing => Ok(None),
            ValidationUpdateOffer::Version { version } => Ok(Some(FakeOffer::boxed(version, None))),
            ValidationUpdateOffer::CheckFails { code } => Err(code),
            ValidationUpdateOffer::DownloadFails { version, code } => {
                Ok(Some(FakeOffer::boxed(version, Some(code))))
            }
        };
        Box::pin(std::future::ready(found))
    }
}

struct FakeOffer {
    version: String,
    download_failure: Option<UpdateFailureCode>,
}

impl FakeOffer {
    fn boxed(version: String, download_failure: Option<UpdateFailureCode>) -> Box<dyn Offer> {
        Box::new(Self {
            version,
            download_failure,
        })
    }
}

impl Offer for FakeOffer {
    fn version(&self) -> &str {
        &self.version
    }

    fn download(
        self: Box<Self>,
        _progress: Progress,
    ) -> BoxFuture<'static, Result<Box<dyn Package>, UpdateFailureCode>> {
        let downloaded = if let Some(code) = self.download_failure {
            Err(code)
        } else {
            let package: Box<dyn Package> = Box::new(FakePackage {
                version: self.version,
            });
            Ok(package)
        };
        Box::pin(std::future::ready(downloaded))
    }
}

struct FakePackage {
    version: String,
}

impl Package for FakePackage {
    fn version(&self) -> &str {
        &self.version
    }

    fn install(self: Box<Self>) -> Result<(), InstallError> {
        // "Restart and update" records the request in a validation build and
        // never chooses this package, so nothing reaches this point.
        Err("a validation build installs nothing".into())
    }
}
