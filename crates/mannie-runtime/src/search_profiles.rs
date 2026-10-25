//! Search profiles and the calls of their models (ADR-0047): the owner's
//! embedding and rerank models, each on a server of the owner's choice, and
//! the service `models`, through which plugins call them. A profile's
//! credential lives in the credential vault as a chat profile's does, and a
//! plugin never sees it, the server or the prefixes.

use futures_util::FutureExt;
use futures_util::stream::{self, StreamExt};
use mannie_plugin_sdk::host::{BindRefusal, PluginBinding, ServiceProvider};
use mannie_plugin_sdk::service::{BoundService, Service};
use mannie_plugin_sdk::services::models::{
    InputKind, Models, ModelsApi, ModelsError, SearchModel, SearchModelKind, inputs_fit,
};
use mannie_plugin_sdk::{BoxFuture, BoxStream};
use mannie_provider::{
    ProviderError, SearchTarget, SearchTestInput, Url, check_base_url, credential_transport_allowed,
};
use mannie_store::{SearchProfileFields, StoredSearchProfile};
use mannie_types::ids::ProfileId;
use mannie_types::provider::{
    CheckOutcome, CreateSearchProfile, CredentialChange, ProviderProfileError, SearchKind,
    SearchProfile, SearchProfileDraft, SearchTestReport, ServerKind, SkipReason, check_credential,
};
use mannie_types::run::RunFailure;
use mannie_types::secret::Secret;
use sha2::{Digest, Sha256};

use crate::compaction::to_hex;
use crate::profiles::{VaultError, credential_reference, in_vault, store_failure, vault_failure};
use crate::retry::RetryDecision;
use crate::{Runtime, lock};

/// A draft's fields, checked.
struct Checked {
    name: String,
    model: String,
    base_url: Url,
    query_prefix: String,
    document_prefix: String,
}

fn check_draft(draft: &SearchProfileDraft) -> Result<Checked, ProviderProfileError> {
    let fields = draft.check_fields()?;
    let base_url = check_base_url(&draft.base_url)?;
    Ok(Checked {
        name: fields.name,
        model: fields.model.as_str().to_owned(),
        base_url,
        query_prefix: fields.query_prefix,
        document_prefix: fields.document_prefix,
    })
}

fn fields(
    draft: &SearchProfileDraft,
    checked: Checked,
    credential_ref: Option<String>,
) -> SearchProfileFields {
    SearchProfileFields {
        name: checked.name,
        kind: draft.kind,
        base_url: checked.base_url.to_string(),
        model: checked.model,
        credential_ref,
        query_prefix: checked.query_prefix,
        document_prefix: checked.document_prefix,
    }
}

/// What a document's vector means for a profile: a digest of its model and
/// its document prefix, the two that change a vector. A new server or a new
/// credential keeps it.
fn vector_key(profile: &StoredSearchProfile) -> String {
    let mut digest = Sha256::new();
    digest.update(profile.model.as_bytes());
    digest.update([0]);
    digest.update(profile.document_prefix.as_bytes());
    let bytes = digest.finalize();
    to_hex(&bytes[..16])
}

fn sdk_kind(kind: SearchKind) -> SearchModelKind {
    match kind {
        SearchKind::Embeddings => SearchModelKind::Embeddings,
        SearchKind::Rerank => SearchModelKind::Rerank,
    }
}

/// The failure of a call, as the service names it to a plugin: a refusal by
/// the code of its failure without the log's prefix, as the failure's own JSON
/// names it.
fn models_failure(error: &ProviderError) -> ModelsError {
    match error {
        ProviderError::Transient { .. } => ModelsError::Unreachable,
        ProviderError::ContextOverflow => ModelsError::InputTooLong,
        ProviderError::Permanent(RunFailure::InvalidResponse) => ModelsError::InvalidResponse,
        ProviderError::Permanent(failure) => {
            let code = failure.code();
            ModelsError::Refused {
                failure: code.strip_prefix("run.").unwrap_or(code).to_owned(),
            }
        }
    }
}

impl Runtime {
    /// Every search profile, the oldest first.
    pub async fn list_search_profiles(&self) -> Result<Vec<SearchProfile>, ProviderProfileError> {
        let profiles = self
            .inner
            .store
            .list_search_profiles()
            .await
            .map_err(|error| store_failure(&error))?;
        Ok(profiles.iter().map(StoredSearchProfile::view).collect())
    }

    /// Adds a search profile and stores its credential.
    pub async fn create_search_profile(
        &self,
        input: CreateSearchProfile,
    ) -> Result<SearchProfile, ProviderProfileError> {
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
            .insert_search_profile(&id, fields(&input.draft, checked, reference.clone()))
            .await;
        match stored {
            Ok(profile) => {
                tracing::info!(profile = %profile.id, kind = profile.kind.as_str(), "added a search profile");
                self.announce_search_profile(&profile.id);
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

    /// Changes a search profile, and keeps, replaces or removes its
    /// credential, in the order that chat profiles change theirs.
    pub async fn update_search_profile(
        &self,
        id: &ProfileId,
        draft: SearchProfileDraft,
        change: CredentialChange,
    ) -> Result<SearchProfile, ProviderProfileError> {
        let current = self
            .inner
            .store
            .search_profile(id)
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
            .update_search_profile(id, fields(&draft, checked, reference.clone()))
            .await;
        match updated {
            Ok((before, after)) => {
                let replaced = before
                    .credential_ref
                    .filter(|old| Some(old) != reference.as_ref());
                if let Some(old) = replaced {
                    self.forget_credential(old).await;
                }
                tracing::info!(profile = %after.id, revision = after.config_revision, "changed a search profile");
                self.announce_search_profile(&after.id);
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

    /// Deletes a search profile and its credential. A call that uses it now
    /// finishes, and later calls find no profile.
    pub async fn delete_search_profile(&self, id: &ProfileId) -> Result<(), ProviderProfileError> {
        let deleted = self
            .inner
            .store
            .delete_search_profile(id)
            .await
            .map_err(|error| store_failure(&error))?;
        if let Some(reference) = deleted.credential_ref {
            self.forget_credential(reference).await;
        }
        tracing::info!(profile = %id, "deleted a search profile");
        self.announce_search_profile(id);
        Ok(())
    }

    /// Runs the test of a search profile and records the server it found.
    pub async fn test_search_profile(
        &self,
        id: &ProfileId,
    ) -> Result<SearchTestReport, ProviderProfileError> {
        let profile = self
            .inner
            .store
            .search_profile(id)
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
        let input = SearchTestInput {
            kind: profile.kind,
            target: SearchTarget {
                base_url,
                model: profile.model.clone(),
                credential,
            },
            credential_check,
        };
        let report = mannie_provider::test_search_profile(&self.inner.client, input).await;
        if let Some(server) = report.server
            && let Err(error) = self
                .inner
                .store
                .record_search_profile_server(id, server)
                .await
        {
            store_failure(&error);
        }
        tracing::info!(profile = %id, server = report.server.map(ServerKind::as_str), "tested a search profile");
        Ok(report)
    }

    /// The search profiles of a kind, as the service names them to plugins.
    async fn search_models(&self, kind: SearchKind) -> Result<Vec<SearchModel>, ModelsError> {
        let profiles = self
            .inner
            .store
            .list_search_profiles()
            .await
            .map_err(|error| {
                store_failure(&error);
                ModelsError::Unavailable
            })?;
        Ok(profiles
            .iter()
            .filter(|profile| profile.kind == kind)
            .map(|profile| {
                SearchModel::new(
                    profile.id.as_str(),
                    profile.name.clone(),
                    profile.model.clone(),
                    sdk_kind(profile.kind),
                    vector_key(profile),
                )
            })
            .collect())
    }

    /// A profile of a kind and where its calls go, with its credential read
    /// from the vault.
    async fn search_call_target(
        &self,
        profile: &str,
        kind: SearchKind,
    ) -> Result<(StoredSearchProfile, SearchTarget), ModelsError> {
        let id = ProfileId::parse(profile).map_err(|_invalid| ModelsError::ProfileNotFound)?;
        let stored = self
            .inner
            .store
            .search_profile(&id)
            .await
            .map_err(|error| {
                store_failure(&error);
                ModelsError::Unavailable
            })?
            .ok_or(ModelsError::ProfileNotFound)?;
        if stored.kind != kind {
            return Err(ModelsError::WrongKind);
        }
        let base_url =
            check_base_url(&stored.base_url).map_err(|_invalid| ModelsError::InvalidInputs)?;
        let credential: Option<Secret> = match stored.credential_ref.clone() {
            None => None,
            Some(reference) => Some(
                in_vault(&self.inner.vault, move |vault| vault.read(&reference))
                    .await
                    .map_err(|_unreadable| ModelsError::CredentialUnavailable)?,
            ),
        };
        let target = SearchTarget {
            base_url,
            model: stored.model.clone(),
            credential,
        };
        Ok((stored, target))
    }

    /// Embeds texts with an embedding profile, putting its prefix before each.
    pub(crate) async fn embed_with(
        &self,
        profile: &str,
        kind: InputKind,
        inputs: Vec<String>,
    ) -> Result<Vec<Vec<f32>>, ModelsError> {
        if !inputs_fit(&inputs) {
            return Err(ModelsError::InvalidInputs);
        }
        let (stored, target) = self
            .search_call_target(profile, SearchKind::Embeddings)
            .await?;
        let prefix = match kind {
            InputKind::Query => stored.query_prefix.as_str(),
            // A kind that a later SDK adds is embedded as a document.
            _ => stored.document_prefix.as_str(),
        };
        let texts: Vec<String> = inputs
            .into_iter()
            .map(|input| format!("{prefix}{input}"))
            .collect();
        let client = self.inner.client.clone();
        self.with_retries(|| {
            let client = client.clone();
            let target = target.clone();
            let texts = texts.clone();
            async move { mannie_provider::embed(&client, &target, &texts).await }
        })
        .await
    }

    /// Scores passages against a query with a rerank profile.
    pub(crate) async fn rerank_with(
        &self,
        profile: &str,
        query: String,
        documents: Vec<String>,
    ) -> Result<Vec<f32>, ModelsError> {
        let query_fits = query.len() <= mannie_plugin_sdk::services::models::MAX_INPUT_BYTES;
        if !inputs_fit(&documents) || !query_fits {
            return Err(ModelsError::InvalidInputs);
        }
        let (_stored, target) = self.search_call_target(profile, SearchKind::Rerank).await?;
        let client = self.inner.client.clone();
        self.with_retries(|| {
            let client = client.clone();
            let target = target.clone();
            let query = query.clone();
            let documents = documents.clone();
            async move { mannie_provider::rerank(&client, &target, &query, &documents).await }
        })
        .await
    }

    /// Makes a call until it succeeds, fails for good, or the retries of the
    /// runtime's policy are used up, as a reply's model call does.
    async fn with_retries<T, F, Fut>(&self, call: F) -> Result<T, ModelsError>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<T, ProviderError>>,
    {
        let mut attempt = 1;
        loop {
            match call().await {
                Ok(value) => return Ok(value),
                Err(error @ ProviderError::Transient { retry_after, .. }) => {
                    let random = lock(&self.inner.random).f64();
                    match self.inner.config.retry.decide(attempt, retry_after, random) {
                        RetryDecision::After(delay) => {
                            tracing::debug!(
                                attempt,
                                error.code = error.failure().code(),
                                "a search call failed and is tried again"
                            );
                            tokio::time::sleep(delay).await;
                            attempt += 1;
                        }
                        RetryDecision::GiveUp => return Err(models_failure(&error)),
                    }
                }
                Err(error) => return Err(models_failure(&error)),
            }
        }
    }

    /// Tells the service's followers that a search profile changed or was
    /// deleted. Nobody follows while no plugin uses the service.
    fn announce_search_profile(&self, id: &ProfileId) {
        let _followers = self.inner.search_changes.send(id.as_str().to_owned());
    }
}

/// The runtime's provider of the service `models`.
#[derive(Debug)]
pub(crate) struct ModelsProvider {
    runtime: Runtime,
}

impl ModelsProvider {
    /// The provider of `runtime`'s search profiles.
    pub(crate) fn new(runtime: &Runtime) -> Self {
        Self {
            runtime: runtime.clone(),
        }
    }
}

impl ServiceProvider for ModelsProvider {
    fn name(&self) -> &'static str {
        Models::NAME
    }

    fn major(&self) -> u16 {
        Models::MAJOR
    }

    fn bind(&self, binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        Ok(BoundService::new::<Models>(
            self.runtime.bound_models(binding.id.as_str()),
        ))
    }
}

impl Runtime {
    /// The service `models` bound to a plugin, as its provider binds it.
    pub(crate) fn bound_models(&self, plugin: &str) -> std::sync::Arc<dyn ModelsApi> {
        std::sync::Arc::new(BoundModels {
            runtime: self.clone(),
            plugin: plugin.to_owned(),
        })
    }
}

/// The service bound to one plugin.
#[derive(Debug)]
struct BoundModels {
    runtime: Runtime,
    plugin: String,
}

impl ModelsApi for BoundModels {
    fn search_profiles(
        &self,
        kind: SearchModelKind,
    ) -> BoxFuture<'static, Result<Vec<SearchModel>, ModelsError>> {
        let runtime = self.runtime.clone();
        let kind = match kind {
            SearchModelKind::Rerank => SearchKind::Rerank,
            // A kind that a later SDK adds lists the embedding profiles.
            _ => SearchKind::Embeddings,
        };
        async move { runtime.search_models(kind).await }.boxed()
    }

    fn embed(
        &self,
        profile: &str,
        kind: InputKind,
        inputs: Vec<String>,
    ) -> BoxFuture<'static, Result<Vec<Vec<f32>>, ModelsError>> {
        let runtime = self.runtime.clone();
        let plugin = self.plugin.clone();
        let profile = profile.to_owned();
        async move {
            let count = inputs.len();
            let outcome = runtime.embed_with(&profile, kind, inputs).await;
            tracing::info!(
                plugin,
                profile,
                count,
                outcome = outcome.as_ref().err().map(ModelsError::code),
                "a plugin embedded texts"
            );
            outcome
        }
        .boxed()
    }

    fn rerank(
        &self,
        profile: &str,
        query: String,
        documents: Vec<String>,
    ) -> BoxFuture<'static, Result<Vec<f32>, ModelsError>> {
        let runtime = self.runtime.clone();
        let plugin = self.plugin.clone();
        let profile = profile.to_owned();
        async move {
            let count = documents.len();
            let outcome = runtime.rerank_with(&profile, query, documents).await;
            tracing::info!(
                plugin,
                profile,
                count,
                outcome = outcome.as_ref().err().map(ModelsError::code),
                "a plugin reranked passages"
            );
            outcome
        }
        .boxed()
    }

    fn changes(&self) -> BoxStream<'static, String> {
        let receiver = self.runtime.inner.search_changes.subscribe();
        stream::unfold(receiver, |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(id) => return Some((id, receiver)),
                    // A follower that fell behind reads every profile again
                    // at the next change it hears of; the stream goes on.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_skipped)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                }
            }
        })
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use mannie_provider::ProviderError;
    use mannie_types::clock::UtcMillis;
    use mannie_types::ids::ProfileId;
    use mannie_types::provider::{SearchKind, ServerKind};
    use mannie_types::run::RunFailure;

    use super::{StoredSearchProfile, models_failure, vector_key};
    use mannie_plugin_sdk::services::models::ModelsError;

    fn profile(model: &str, prefix: &str, base_url: &str) -> StoredSearchProfile {
        StoredSearchProfile {
            id: ProfileId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap(),
            name: "Embeddings".to_owned(),
            kind: SearchKind::Embeddings,
            base_url: base_url.to_owned(),
            model: model.to_owned(),
            credential_ref: None,
            config_revision: 1,
            query_prefix: "Query: ".to_owned(),
            document_prefix: prefix.to_owned(),
            server: Some(ServerKind::Other),
            created_at: UtcMillis::from_millis(0),
            updated_at: UtcMillis::from_millis(0),
        }
    }

    #[test]
    fn the_key_of_vectors_follows_the_model_and_the_document_prefix_only() {
        let here = vector_key(&profile("embedder", "", "http://127.0.0.1:8080/v1"));
        let elsewhere = vector_key(&profile("embedder", "", "https://search.example.com/v1"));
        let other_model = vector_key(&profile("embedder-2", "", "http://127.0.0.1:8080/v1"));
        let other_prefix = vector_key(&profile(
            "embedder",
            "passage: ",
            "http://127.0.0.1:8080/v1",
        ));
        // The separator keeps a model's end and a prefix's start apart.
        let shifted = vector_key(&profile(
            "embedder p",
            "assage: ",
            "http://127.0.0.1:8080/v1",
        ));

        assert_eq!(here, elsewhere);
        assert_eq!(here.len(), 32);
        assert_ne!(here, other_model);
        assert_ne!(here, other_prefix);
        assert_ne!(other_prefix, shifted);
    }

    #[test]
    fn failures_reach_plugins_by_their_class() {
        let transient = ProviderError::Transient {
            cause: RunFailure::ConnectionFailed,
            retry_after: None,
        };

        assert_eq!(models_failure(&transient), ModelsError::Unreachable);
        assert_eq!(
            models_failure(&ProviderError::ContextOverflow),
            ModelsError::InputTooLong
        );
        assert_eq!(
            models_failure(&ProviderError::Permanent(RunFailure::InvalidResponse)),
            ModelsError::InvalidResponse
        );
        assert_eq!(
            models_failure(&ProviderError::Permanent(RunFailure::InvalidCredential)),
            ModelsError::Refused {
                failure: "invalid_credential".to_owned()
            }
        );
    }
}
