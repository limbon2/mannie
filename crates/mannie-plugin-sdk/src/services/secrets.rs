//! The service `secrets` (ADR-0034, decision 5; ADR-0030, decision 13): the
//! operating system's credential store, limited to the entries that a
//! plugin's manifest lists under `[secrets]`.
//!
//! A plugin names an entry as its manifest lists it. The core stores an
//! entry of `entries` as `<id>/<entry>`, so two plugins never share one, and
//! an entry of `legacy` under its own name, which only a frozen list of the
//! core admits, such as the name that Workers' master key had in 0.2.0
//! (decision 10). An entry of a keyed family, `<family>/<key>`, is stored as
//! `<id>/<family>/<key>`, so a plugin keeps one for each thing that it
//! knows, such as a host's password (ADR-0036, decision 8), and deletes it
//! when the thing goes. Any other name is refused before the store is
//! touched.
//!
//! The calls of the credential store may block, so the core makes them on a
//! blocking thread and a plugin awaits them.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::BoxFuture;
use crate::contracts::Secret;
use crate::service::Service;

/// The service `secrets`, major 1. It reaches the credential store, so it is
/// sensitive.
#[derive(Debug)]
pub enum Secrets {}

impl Service for Secrets {
    const NAME: &'static str = "secrets";
    const MAJOR: u16 = 1;
    const SENSITIVE: bool = true;
    type Api = dyn SecretsApi;
}

/// The plugin's entries of the credential store.
pub trait SecretsApi: Send + Sync + fmt::Debug {
    /// The secret of one of the plugin's entries, named as the manifest
    /// lists it, or `None` when the store holds none.
    fn load(&self, entry: &str) -> BoxFuture<'static, Result<Option<Secret>, SecretsError>>;

    /// Keeps a secret under one of the plugin's entries, named as the
    /// manifest lists it.
    fn save(&self, entry: &str, secret: Secret) -> BoxFuture<'static, Result<(), SecretsError>>;

    /// Deletes one of the plugin's entries, named as for [`load`]
    /// (ADR-0036, decision 8). Deleting an entry that the store does not
    /// hold succeeds. A provider from before `delete` keeps this default,
    /// which deletes nothing and answers `Unavailable`.
    ///
    /// [`load`]: SecretsApi::load
    fn delete(&self, _entry: &str) -> BoxFuture<'static, Result<(), SecretsError>> {
        Box::pin(std::future::ready(Err(SecretsError::Unavailable)))
    }
}

/// Why a plugin's secret could not be read or kept. The core adds reasons by
/// addition, so a plugin treats a reason that it does not know as a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SecretsError {
    /// The credential store cannot be used.
    #[error("the credential store cannot be used")]
    Unavailable,
    /// The entry is not one that the plugin's manifest lists.
    #[error("the entry does not belong to the plugin")]
    NotPermitted,
}
