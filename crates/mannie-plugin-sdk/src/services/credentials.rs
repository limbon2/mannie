//! The service `credentials` (ADR-0034, decision 5; ADR-0010): the core
//! lends a plugin's executor the credential of a provider profile for one of
//! its tasks, such as a worker's lease of the profile that the task runs
//! with.
//!
//! The core binds the service to the executor kind that equals the plugin's
//! ID, and lends only for an active task of that kind whose run uses the
//! profile. It logs each lend with the plugin, the task and the profile,
//! never the credential.
//!
//! `tests/protocol.rs` lists this module as a known exception: a
//! [`LentCredential`] carries the credential as [`Secret`], which never
//! serializes.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::BoxFuture;
use crate::contracts::{ProfileId, Secret, TaskId};
use crate::service::Service;

/// The service `credentials`, major 1. It reaches provider credentials, so
/// it is sensitive.
#[derive(Debug)]
pub enum Credentials {}

impl Service for Credentials {
    const NAME: &'static str = "credentials";
    const MAJOR: u16 = 1;
    const SENSITIVE: bool = true;
    type Api = dyn CredentialsApi;
}

/// Lends the credentials of provider profiles for the tasks of a plugin's
/// executor.
pub trait CredentialsApi: Send + Sync + fmt::Debug {
    /// The credential of `profile`, with its binding, for `task`, an active
    /// task of the plugin's executor whose run uses `profile`. `None` for a
    /// profile without a credential. Any other task or profile is refused
    /// with [`LendError::NotPermitted`].
    fn lend(
        &self,
        task: TaskId,
        profile: ProfileId,
    ) -> BoxFuture<'static, Result<Option<LentCredential>, LendError>>;
}

/// A provider credential that the core lends for one task, with the binding
/// of the profile that it belongs to.
#[derive(Debug)]
#[non_exhaustive]
pub struct LentCredential {
    /// The profile and its configuration revision.
    pub binding: String,
    /// The credential.
    pub credential: Secret,
}

impl LentCredential {
    /// A credential with the binding of its profile.
    pub fn new(binding: String, credential: Secret) -> Self {
        Self {
            binding,
            credential,
        }
    }
}

/// Why the core lent no credential. The core adds reasons by addition, so a
/// plugin treats a reason that it does not know as a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LendError {
    /// The profile no longer exists.
    #[error("the profile no longer exists")]
    ProfileMissing,
    /// The credential could not be read.
    #[error("the credential could not be read")]
    CredentialUnavailable,
    /// The store failed.
    #[error("the store failed")]
    StorageFailed,
    /// The task is not an active task of the plugin's executor, or its run
    /// does not use the profile.
    #[error("the task may not lend the credential of the profile")]
    NotPermitted,
}

#[cfg(test)]
mod tests {
    use super::LentCredential;
    use crate::contracts::Secret;

    const PLANTED: &str = "sk-planted-lent-credential-0123456789";

    #[test]
    fn a_lent_credential_never_shows_its_value_when_formatted() {
        let lent = LentCredential::new(
            "0199e0f2-0000-7000-8000-0000000000bb:3".to_owned(),
            Secret::new(PLANTED.to_owned()),
        );

        let debug = format!("{lent:?}");

        assert!(!debug.contains(PLANTED), "{debug}");
        assert!(
            debug.contains("0199e0f2-0000-7000-8000-0000000000bb:3"),
            "{debug}"
        );
        assert_eq!(lent.credential.expose(), PLANTED);
    }
}
