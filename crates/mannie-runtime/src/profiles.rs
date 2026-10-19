//! Provider profiles and their credentials (docs/features/chat/providers.md).
//! The credential of a profile lives in a [`CredentialVault`]: on the master,
//! the operating system's credential store (ADR-0010). The store keeps only the
//! name of the vault's entry.

use std::fmt;
use std::sync::Arc;

use futures_util::future::BoxFuture;

use mannie_provider::{TestInput, Url, check_base_url, credential_transport_allowed, test_profile};
use mannie_store::{ProfileFields, ProfileStoreError, StoredProfile};
use mannie_types::ids::ProfileId;
use mannie_types::provider::{
    CheckOutcome, CreateProviderProfile, CredentialChange, CredentialStoreProblem, ProfileDraft,
    ProviderProfile, ProviderProfileError, ProviderTestReport, ServerKind, SkipReason,
    check_credential,
};
use mannie_types::run::RunFailure;
use mannie_types::secret::Secret;

use crate::Runtime;

/// Where credentials are kept. The calls may block; the runtime makes them
/// on blocking threads.
pub trait CredentialVault: Send + Sync + fmt::Debug + 'static {
    /// Stores a credential under `reference`, replacing what was there.
    fn save(&self, reference: &str, credential: &Secret) -> Result<(), VaultError>;
    /// Reads the credential stored under `reference`.
    fn read(&self, reference: &str) -> Result<Secret, VaultError>;
    /// Deletes the credential stored under `reference`.
    fn delete(&self, reference: &str) -> Result<(), VaultError>;
    /// A wait that ends once a credential is saved under `reference`, for a
    /// vault whose credentials arrive while runs wait for them: a worker's
    /// leases (plugins/workers/docs/credential-leases.md). `None`, the
    /// default, for a vault whose credentials never arrive by themselves.
    fn arrival(&self, _reference: &str) -> Option<BoxFuture<'static, ()>> {
        None
    }
}

/// Why the vault could not do what was asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum VaultError {
    /// No credential is stored under the reference.
    #[error("no credential is stored under the reference")]
    Missing,
    /// The vault cannot be used.
    #[error("the credential store cannot be used")]
    Unavailable(CredentialStoreProblem),
}

/// Runs a vault call on a blocking thread.
pub(crate) async fn in_vault<T, F>(
    vault: &Arc<dyn CredentialVault>,
    work: F,
) -> Result<T, VaultError>
where
    T: Send + 'static,
    F: FnOnce(&dyn CredentialVault) -> Result<T, VaultError> + Send + 'static,
{
    let vault = Arc::clone(vault);
    tokio::task::spawn_blocking(move || work(vault.as_ref()))
        .await
        .unwrap_or(Err(VaultError::Unavailable(CredentialStoreProblem::Other)))
}

fn vault_failure(error: VaultError) -> ProviderProfileError {
    let reason = match error {
        VaultError::Unavailable(problem) => problem,
        VaultError::Missing => CredentialStoreProblem::Other,
    };
    ProviderProfileError::CredentialStoreUnavailable { reason }
}

fn store_failure(error: &ProfileStoreError) -> ProviderProfileError {
    match error {
        ProfileStoreError::NameTaken => ProviderProfileError::NameTaken,
        ProfileStoreError::NotFound => ProviderProfileError::NotFound,
        ProfileStoreError::InvalidData { .. } | ProfileStoreError::Store(_) => {
            tracing::error!(error = %error, error.code = error.code(), "a provider profile could not be read or saved");
            ProviderProfileError::StorageFailed
        }
    }
}

/// The name of a profile's credential entry for one configuration revision.
fn credential_reference(profile: &ProfileId, revision: u32) -> String {
    format!("provider/{profile}/{revision}")
}

/// A draft's fields, checked.
struct Checked {
    name: String,
    model: String,
    base_url: Url,
}

fn check_draft(draft: &ProfileDraft) -> Result<Checked, ProviderProfileError> {
    let (name, model) = draft.check_fields()?;
    let base_url = check_base_url(&draft.base_url)?;
    Ok(Checked {
        name,
        model: model.as_str().to_owned(),
        base_url,
    })
}

fn fields(draft: &ProfileDraft, checked: Checked, credential_ref: Option<String>) -> ProfileFields {
    ProfileFields {
        name: checked.name,
        interface: draft.interface,
        base_url: checked.base_url.to_string(),
        model: checked.model,
        credential_ref,
        context_window: draft.context_window,
        output_limit: draft.output_limit,
        reasoning_effort: draft.reasoning_effort,
        use_tools: draft.use_tools,
    }
}

impl Runtime {
    /// Every profile, the oldest first.
    pub async fn list_profiles(&self) -> Result<Vec<ProviderProfile>, ProviderProfileError> {
        let profiles = self
            .inner
            .store
            .list_profiles()
            .await
            .map_err(|error| store_failure(&error))?;
        Ok(profiles.iter().map(StoredProfile::view).collect())
    }

    /// Adds a profile and stores its credential. The first profile becomes the
    /// default.
    pub async fn create_profile(
        &self,
        input: CreateProviderProfile,
    ) -> Result<ProviderProfile, ProviderProfileError> {
        let checked = check_draft(&input.draft)?;
        let credential = input
            .credential
            .as_ref()
            .map(check_credential)
            .transpose()?;
        if credential.is_some() && !credential_transport_allowed(&checked.base_url) {
            return Err(ProviderProfileError::InsecureCredentialTransport);
        }
        let id = self.inner.ids.profile();
        let reference = credential.as_ref().map(|_| credential_reference(&id, 1));
        if let (Some(reference), Some(credential)) = (reference.clone(), credential) {
            in_vault(&self.inner.vault, move |vault| {
                vault.save(&reference, &credential)
            })
            .await
            .map_err(vault_failure)?;
        }
        let stored = self
            .inner
            .store
            .insert_profile(&id, fields(&input.draft, checked, reference.clone()))
            .await;
        match stored {
            Ok(profile) => {
                tracing::info!(profile = %profile.id, interface = profile.interface.as_str(), "added a provider profile");
                Ok(profile.view())
            }
            Err(error) => {
                if let Some(reference) = reference {
                    self.forget_credential(reference).await;
                }
                Err(store_failure(&error))
            }
        }
    }

    /// Changes a profile, and keeps, replaces or removes its credential. A new
    /// credential is stored before the profile changes, and the old one is
    /// deleted after, so a failure in between never loses the credential that
    /// works.
    pub async fn update_profile(
        &self,
        id: &ProfileId,
        draft: ProfileDraft,
        change: CredentialChange,
    ) -> Result<ProviderProfile, ProviderProfileError> {
        let current = self
            .inner
            .store
            .profile(id)
            .await
            .map_err(|error| store_failure(&error))?
            .ok_or(ProviderProfileError::NotFound)?;
        let checked = check_draft(&draft)?;
        let (reference, new_credential) = match change {
            CredentialChange::Keep => (current.credential_ref.clone(), None),
            CredentialChange::Remove => (None, None),
            CredentialChange::Replace { credential } => {
                let credential = check_credential(&credential)?;
                let reference = credential_reference(id, current.config_revision.saturating_add(1));
                (Some(reference), Some(credential))
            }
        };
        if reference.is_some() && !credential_transport_allowed(&checked.base_url) {
            return Err(ProviderProfileError::InsecureCredentialTransport);
        }
        if let (Some(reference), Some(credential)) = (reference.clone(), new_credential.clone()) {
            in_vault(&self.inner.vault, move |vault| {
                vault.save(&reference, &credential)
            })
            .await
            .map_err(vault_failure)?;
        }
        let updated = self
            .inner
            .store
            .update_profile(id, fields(&draft, checked, reference.clone()))
            .await;
        match updated {
            Ok((before, after)) => {
                let replaced = before
                    .credential_ref
                    .filter(|old| Some(old) != reference.as_ref());
                if let Some(old) = replaced {
                    self.forget_credential(old).await;
                }
                tracing::info!(profile = %after.id, revision = after.config_revision, "changed a provider profile");
                Ok(after.view())
            }
            Err(error) => {
                if let (Some(reference), Some(_)) = (reference, new_credential) {
                    self.forget_credential(reference).await;
                }
                Err(store_failure(&error))
            }
        }
    }

    /// Deletes a profile and its credential. A profile that a running reply
    /// uses is kept until the reply ends.
    pub async fn delete_profile(&self, id: &ProfileId) -> Result<(), ProviderProfileError> {
        if self.profile_in_use(id) {
            return Err(ProviderProfileError::ProfileInUse);
        }
        let deleted = self
            .inner
            .store
            .delete_profile(id)
            .await
            .map_err(|error| store_failure(&error))?;
        if let Some(reference) = deleted.credential_ref {
            self.forget_credential(reference).await;
        }
        tracing::info!(profile = %id, "deleted a provider profile");
        Ok(())
    }

    /// Makes a profile the default.
    pub async fn set_default_profile(&self, id: &ProfileId) -> Result<(), ProviderProfileError> {
        self.inner
            .store
            .set_default_profile(id)
            .await
            .map_err(|error| store_failure(&error))
    }

    /// Runs the provider test of a profile and records the server it found.
    pub async fn test_profile(
        &self,
        id: &ProfileId,
    ) -> Result<ProviderTestReport, ProviderProfileError> {
        let profile = self
            .inner
            .store
            .profile(id)
            .await
            .map_err(|error| store_failure(&error))?
            .ok_or(ProviderProfileError::NotFound)?;
        let base_url = check_base_url(&profile.base_url)?;
        let (credential, credential_check) = match profile.credential_ref.clone() {
            None => (
                None,
                CheckOutcome::Skipped {
                    reason: SkipReason::NoCredential,
                },
            ),
            Some(reference) => {
                match in_vault(&self.inner.vault, move |vault| vault.read(&reference)).await {
                    Ok(secret) => (Some(secret), CheckOutcome::Passed),
                    Err(VaultError::Missing) => (
                        None,
                        CheckOutcome::Failed {
                            failure: RunFailure::CredentialMissing,
                        },
                    ),
                    Err(VaultError::Unavailable(_)) => (
                        None,
                        CheckOutcome::Failed {
                            failure: RunFailure::CredentialStoreUnavailable,
                        },
                    ),
                }
            }
        };
        let input = TestInput {
            interface: profile.interface,
            base_url,
            model: profile.model.clone(),
            credential,
            reasoning_effort: profile.reasoning_effort,
            output_limit: profile.output_limit,
            credential_check,
            use_tools: profile.use_tools,
        };
        let report = test_profile(&self.inner.client, &self.inner.registry, input).await;
        if let Some(server) = report.server
            && let Err(error) = self.inner.store.record_profile_server(id, server).await
        {
            store_failure(&error);
        }
        tracing::info!(profile = %id, server = report.server.map(ServerKind::as_str), "tested a provider profile");
        Ok(report)
    }

    /// Deletes a credential that no profile uses any more. A failure is
    /// logged; the entry is left behind in the credential store.
    async fn forget_credential(&self, reference: String) {
        let deleted = in_vault(&self.inner.vault, move |vault| vault.delete(&reference)).await;
        match deleted {
            Ok(()) | Err(VaultError::Missing) => {}
            Err(error) => {
                tracing::warn!(%error, "could not delete a credential that is no longer used");
            }
        }
    }
}
