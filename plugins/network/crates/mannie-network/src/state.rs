//! Runtime state manager of the Network Client plugin (ADR-0075).
//!
//! Coordinates SQLite persistence, chunked OS credential-store hydration and
//! cleanup, live state broadcasts for `follow_state`, ephemeral `OpenAPI` import
//! previews and cancellation of active owner workbench requests.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

use mannie_plugin_sdk::contracts::{Clock, Secret, UtcMillis};
use mannie_plugin_sdk::services::file_dialog::{FileDialogApi, FileFilter};
use mannie_plugin_sdk::services::secrets::SecretsApi;
use mannie_plugin_sdk::services::store::ModuleStoreApi;
use tokio::sync::{oneshot, watch};
use uuid::Uuid;

use crate::commands::{
    CollectionInput, CookieInput, EnvironmentInput, FolderCreateInput, ManualSendInput,
    OpenApiConfirmInput, RequestInput,
};
use crate::http::{HttpExchange, ParsedSetCookie, execute_http};
use crate::model::{
    AuthConfig, BodyConfig, CollectionView, CookieView, EnvironmentView, FolderView, KeyValueEntry,
    MAX_OWNER_RESPONSE_BYTES, MAX_SPEC_BYTES, ManualResponseView, NetworkError,
    NetworkStateSnapshot, OpenApiPreview, Protocol, RequestView, VariableEntry,
};
use crate::openapi::{ImportedCollection, parse_openapi};
use crate::redact::{REDACTED, Redactor, is_secret_header_name, is_secret_name};
use crate::secrets::{
    Family, Vault, hydrate_auth, hydrate_body, hydrate_key_values, hydrate_variables, strip_auth,
    strip_body, strip_key_values, strip_variables,
};
use crate::store::{
    CollectionDocument, CollectionRow, CookieRow, EnvironmentRow, FolderRow, NetworkStore,
    RequestDocument, RequestRow, VariablesDocument,
};
use crate::template::{
    OriginPolicy, ResolveError, ResolvedHttpRequest, RuntimeOverrides,
    redact_url_template_for_agent, resolve_http_request, sanitize_saved_url,
};

/// Ephemeral `OpenAPI` previews kept in memory at once.
const MAX_CACHED_PREVIEWS: usize = 8;

/// Vault slot and chunk count for one persisted entity so updates stage into a
/// new slot before SQLite commits and deletes clean up every previous chunk.
#[derive(Debug, Clone, Default)]
struct SecretMeta {
    slot: String,
    parts: u32,
}

impl SecretMeta {
    fn new(slot: String, parts: u32) -> Self {
        Self { slot, parts }
    }

    fn effective_slot<'a>(&'a self, fallback_id: &'a str) -> &'a str {
        effective_slot(fallback_id, &self.slot)
    }
}

fn effective_slot<'a>(fallback_id: &'a str, slot: &'a str) -> &'a str {
    if slot.is_empty() { fallback_id } else { slot }
}

/// Active vault slots and chunk counts kept in memory across entities.
#[derive(Debug, Default)]
struct SecretRegistry {
    collections: HashMap<String, SecretMeta>,
    environments: HashMap<String, SecretMeta>,
    requests: HashMap<String, SecretMeta>,
    cookies: HashMap<String, SecretMeta>,
    global_variables: SecretMeta,
}

#[derive(Debug)]
struct Inner {
    store: NetworkStore,
    vault: Vault,
    file_dialog: Arc<dyn FileDialogApi>,
    clock: Arc<dyn Clock>,
    client: reqwest::Client,
    snapshot: Mutex<NetworkStateSnapshot>,
    secret_registry: Mutex<SecretRegistry>,
    previews: Mutex<Vec<(String, ImportedCollection)>>,
    active_runs: Mutex<HashMap<String, oneshot::Sender<()>>>,
    revisions: watch::Sender<u64>,
}

/// Shared handle to the running Network Client plugin state.
#[derive(Debug, Clone)]
pub(crate) struct NetworkManager {
    inner: Arc<Inner>,
}

impl NetworkManager {
    /// Loads all tables from SQLite and hydrates secrets from the OS
    /// credential store for the owner workbench and in-memory request
    /// execution.
    pub(crate) async fn load(
        store_api: Arc<dyn ModuleStoreApi>,
        secrets_api: Arc<dyn SecretsApi>,
        file_dialog: Arc<dyn FileDialogApi>,
        clock: Arc<dyn Clock>,
        client: reqwest::Client,
    ) -> Result<Self, NetworkError> {
        let store = NetworkStore::new(store_api);
        let vault = Vault::new(secrets_api);
        let tables = store.load_all().await.map_err(|error| {
            tracing::error!(error.code = error.code(), "failed to load network tables");
            NetworkError::StorageFailed
        })?;
        let mut registry = SecretRegistry {
            global_variables: SecretMeta::new(
                tables.global_variables.secret_slot.clone(),
                tables.global_variables.secret_parts,
            ),
            ..SecretRegistry::default()
        };

        let global_slot = effective_slot("global", &tables.global_variables.secret_slot);
        let global_secrets = vault
            .load_map(
                Family::Global,
                global_slot,
                tables.global_variables.secret_parts,
            )
            .await
            .map_err(|_err| NetworkError::CredentialStoreUnavailable)?;
        let global_variables = hydrate_variables(
            "var",
            &tables.global_variables.document.variables,
            &global_secrets,
        );
        let cookies = load_cookies(&vault, tables.cookies, &mut registry).await?;
        let folders_by_col = group_folders(tables.folders);
        let envs_by_col = load_environments(&vault, tables.environments, &mut registry).await?;
        let reqs_by_col = load_requests(&vault, tables.requests, &mut registry).await?;
        let collections = load_collections(
            &vault,
            tables.collections,
            folders_by_col,
            envs_by_col,
            reqs_by_col,
            &mut registry,
        )
        .await?;

        let snapshot = NetworkStateSnapshot {
            collections,
            global_variables,
            cookies,
        };
        let (revisions, _) = watch::channel(0);

        Ok(Self {
            inner: Arc::new(Inner {
                store,
                vault,
                file_dialog,
                clock,
                client,
                snapshot: Mutex::new(snapshot),
                secret_registry: Mutex::new(registry),
                previews: Mutex::new(Vec::new()),
                active_runs: Mutex::new(HashMap::new()),
                revisions,
            }),
        })
    }

    /// Returns the live owner workbench snapshot (unredacted per ADR-0075,
    /// decision 5).
    pub(crate) fn snapshot(&self) -> NetworkStateSnapshot {
        self.inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Subscribes to revision notifications for `follow_state`.
    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.inner.revisions.subscribe()
    }

    /// Number of manual workbench requests currently in flight.
    pub(crate) fn active_run_count(&self) -> u32 {
        let count = self
            .inner
            .active_runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    /// Cancels all in-flight workbench requests (called on plugin stop).
    pub(crate) fn cancel_all_runs(&self) {
        let mut active = self
            .inner
            .active_runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        for (_id, sender) in active.drain() {
            if sender.send(()).is_err() {
                tracing::debug!("owner request finished before plugin stop cancel");
            }
        }
    }

    async fn cleanup_vault_parts(&self, family: Family, slot: &str, from: u32, to: u32) {
        if slot.is_empty() || from >= to {
            return;
        }

        if let Err(error) = self.inner.vault.delete_from(family, slot, from, to).await {
            tracing::warn!(%error, "could not delete stale credential store entries");
        }
    }

    async fn stage_secret_map(
        &self,
        family: Family,
        preferred_slot: Option<&str>,
        secrets: &BTreeMap<String, String>,
    ) -> Result<SecretMeta, NetworkError> {
        if secrets.is_empty() {
            return Ok(SecretMeta::default());
        }

        let slot = preferred_slot.map_or_else(|| Uuid::now_v7().to_string(), str::to_owned);
        let parts = self
            .inner
            .vault
            .save_map(family, &slot, secrets)
            .await
            .map_err(|_err| NetworkError::CredentialStoreUnavailable)?;
        Ok(SecretMeta::new(slot, parts))
    }

    async fn stage_secret_text(
        &self,
        family: Family,
        preferred_slot: Option<&str>,
        value: &str,
    ) -> Result<SecretMeta, NetworkError> {
        if value.is_empty() {
            return Ok(SecretMeta::default());
        }

        let slot = preferred_slot.map_or_else(|| Uuid::now_v7().to_string(), str::to_owned);
        let parts = self
            .inner
            .vault
            .save_text(family, &slot, &Secret::new(value.to_owned()))
            .await
            .map_err(|_err| NetworkError::CredentialStoreUnavailable)?;
        Ok(SecretMeta::new(slot, parts))
    }

    fn notify(&self) {
        self.inner.revisions.send_modify(|rev| {
            *rev = rev.wrapping_add(1);
        });
    }

    pub(crate) fn now(&self) -> UtcMillis {
        self.inner.clock.now()
    }

    pub(crate) async fn create_collection(
        &self,
        mut input: CollectionInput,
    ) -> Result<CollectionView, NetworkError> {
        let (clean_base_url, extracted_query) = sanitize_saved_url(&input.base_url)?;
        input.base_url = clean_base_url;
        merge_extracted_query_params(&mut input.query_params, extracted_query);

        let now = self.now();
        let id = Uuid::now_v7().to_string();
        let (doc, secrets) = strip_collection_doc(&input);
        let staged = self
            .stage_secret_map(Family::Collection, Some(&id), &secrets)
            .await?;
        let row = CollectionRow {
            id: id.clone(),
            name: input.name.trim().to_owned(),
            description: input.description.clone(),
            protocol: input.protocol,
            base_url: input.base_url.clone(),
            enabled: input.enabled,
            active_environment_id: None,
            auto_cookies: input.auto_cookies,
            document: doc,
            secret_parts: staged.parts,
            secret_slot: staged.slot.clone(),
            created_at: now,
            updated_at: now,
        };

        if let Err(error) = self.inner.store.insert_collection(&row).await {
            self.cleanup_vault_parts(Family::Collection, &staged.slot, 0, staged.parts)
                .await;
            return Err(error);
        }

        let view = CollectionView {
            id: id.clone(),
            name: row.name,
            description: row.description,
            protocol: row.protocol,
            base_url: row.base_url,
            enabled: row.enabled,
            active_environment_id: None,
            auto_cookies: row.auto_cookies,
            headers: normalize_kv(&input.headers, true),
            query_params: normalize_kv(&input.query_params, false),
            auth: input.auth,
            variables: normalize_vars(&input.variables),
            folders: Vec::new(),
            environments: Vec::new(),
            requests: Vec::new(),
            created_at: now,
            updated_at: now,
        };

        self.inner
            .secret_registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .collections
            .insert(id, staged);
        self.inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .collections
            .push(view.clone());
        self.notify();
        Ok(view)
    }

    pub(crate) async fn update_collection(
        &self,
        collection_id: &str,
        mut input: CollectionInput,
    ) -> Result<CollectionView, NetworkError> {
        let existing = self
            .snapshot()
            .collections
            .into_iter()
            .find(|c| c.id == collection_id)
            .ok_or(NetworkError::CollectionNotFound)?;
        let (clean_base_url, extracted_query) = sanitize_saved_url(&input.base_url)?;
        input.base_url = clean_base_url;
        merge_extracted_query_params(&mut input.query_params, extracted_query);

        let previous = self
            .inner
            .secret_registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .collections
            .get(collection_id)
            .cloned()
            .unwrap_or_default();
        let now = self.now();
        let (doc, secrets) = strip_collection_doc(&input);
        let staged = self
            .stage_secret_map(Family::Collection, None, &secrets)
            .await?;
        let row = CollectionRow {
            id: existing.id.clone(),
            name: input.name.trim().to_owned(),
            description: input.description.clone(),
            protocol: existing.protocol,
            base_url: input.base_url.clone(),
            enabled: input.enabled,
            active_environment_id: existing.active_environment_id.clone(),
            auto_cookies: input.auto_cookies,
            document: doc,
            secret_parts: staged.parts,
            secret_slot: staged.slot.clone(),
            created_at: existing.created_at,
            updated_at: now,
        };

        if let Err(error) = self.inner.store.update_collection(&row).await {
            self.cleanup_vault_parts(Family::Collection, &staged.slot, 0, staged.parts)
                .await;
            return Err(error);
        }

        if previous.parts > 0 {
            self.cleanup_vault_parts(
                Family::Collection,
                previous.effective_slot(collection_id),
                0,
                previous.parts,
            )
            .await;
        }

        let updated = CollectionView {
            name: row.name,
            description: row.description,
            base_url: row.base_url,
            enabled: row.enabled,
            auto_cookies: row.auto_cookies,
            headers: normalize_kv(&input.headers, true),
            query_params: normalize_kv(&input.query_params, false),
            auth: input.auth,
            variables: normalize_vars(&input.variables),
            updated_at: now,
            ..existing
        };

        self.inner
            .secret_registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .collections
            .insert(collection_id.to_owned(), staged);

        let mut snap = self
            .inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        if let Some(slot) = snap.collections.iter_mut().find(|c| c.id == collection_id) {
            *slot = updated.clone();
        }
        drop(snap);

        self.notify();
        Ok(updated)
    }

    pub(crate) async fn set_collection_enabled(
        &self,
        collection_id: &str,
        enabled: bool,
    ) -> Result<(), NetworkError> {
        let now = self.now();
        let exists = self
            .snapshot()
            .collections
            .iter()
            .any(|c| c.id == collection_id);

        if !exists {
            return Err(NetworkError::CollectionNotFound);
        }

        self.inner
            .store
            .set_collection_enabled(collection_id, enabled, now)
            .await?;

        let mut snap = self
            .inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        if let Some(col) = snap.collections.iter_mut().find(|c| c.id == collection_id) {
            col.enabled = enabled;
            col.updated_at = now;
        }
        drop(snap);

        self.notify();
        Ok(())
    }

    pub(crate) async fn delete_collection(&self, collection_id: &str) -> Result<(), NetworkError> {
        let removed = self
            .snapshot()
            .collections
            .into_iter()
            .find(|c| c.id == collection_id)
            .ok_or(NetworkError::CollectionNotFound)?;

        self.inner.store.delete_collection(collection_id).await?;

        // Clean up credential-store entries for the collection and every child
        // environment and request once SQLite has committed the deletion
        // (ADR-0075, ADR-0048).
        let (col_meta, env_metas, req_metas) = {
            let mut registry = self
                .inner
                .secret_registry
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let cm = registry
                .collections
                .remove(collection_id)
                .unwrap_or_default();
            let em: Vec<(String, SecretMeta)> = removed
                .environments
                .iter()
                .map(|e| {
                    (
                        e.id.clone(),
                        registry.environments.remove(&e.id).unwrap_or_default(),
                    )
                })
                .collect();
            let rm: Vec<(String, SecretMeta)> = removed
                .requests
                .iter()
                .map(|r| {
                    (
                        r.id.clone(),
                        registry.requests.remove(&r.id).unwrap_or_default(),
                    )
                })
                .collect();
            (cm, em, rm)
        };

        if col_meta.parts > 0 {
            self.cleanup_vault_parts(
                Family::Collection,
                col_meta.effective_slot(collection_id),
                0,
                col_meta.parts,
            )
            .await;
        }

        for (env_id, meta) in env_metas {
            if meta.parts > 0 {
                self.cleanup_vault_parts(
                    Family::Environment,
                    meta.effective_slot(&env_id),
                    0,
                    meta.parts,
                )
                .await;
            }
        }

        for (req_id, meta) in req_metas {
            if meta.parts > 0 {
                self.cleanup_vault_parts(
                    Family::Request,
                    meta.effective_slot(&req_id),
                    0,
                    meta.parts,
                )
                .await;
            }
        }

        self.inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .collections
            .retain(|c| c.id != collection_id);
        self.notify();
        Ok(())
    }

    pub(crate) async fn create_folder(
        &self,
        input: FolderCreateInput,
    ) -> Result<FolderView, NetworkError> {
        let col = self
            .snapshot()
            .collections
            .into_iter()
            .find(|c| c.id == input.collection_id)
            .ok_or(NetworkError::CollectionNotFound)?;

        if let Some(parent_id) = &input.parent_id
            && !col.folders.iter().any(|f| f.id == *parent_id)
        {
            return Err(NetworkError::FolderNotFound);
        }

        let now = self.now();
        let id = Uuid::now_v7().to_string();
        let sort_order = u32::try_from(col.folders.len()).unwrap_or(u32::MAX);
        let row = FolderRow {
            id: id.clone(),
            collection_id: input.collection_id.clone(),
            parent_id: input.parent_id.clone(),
            name: input.name.trim().to_owned(),
            sort_order,
            created_at: now,
            updated_at: now,
        };
        self.inner.store.insert_folder(&row).await?;
        let view = FolderView {
            id,
            collection_id: row.collection_id,
            parent_id: row.parent_id,
            name: row.name,
            sort_order: row.sort_order,
            created_at: now,
            updated_at: now,
        };

        let mut snap = self
            .inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        if let Some(target) = snap
            .collections
            .iter_mut()
            .find(|c| c.id == input.collection_id)
        {
            target.folders.push(view.clone());
        }
        drop(snap);

        self.notify();
        Ok(view)
    }

    pub(crate) async fn rename_folder(
        &self,
        folder_id: &str,
        name: &str,
    ) -> Result<(), NetworkError> {
        let trimmed = name.trim();
        let found = self
            .snapshot()
            .collections
            .iter()
            .any(|c| c.folders.iter().any(|f| f.id == folder_id));

        if !found {
            return Err(NetworkError::FolderNotFound);
        }

        let now = self.now();
        self.inner
            .store
            .rename_folder(folder_id, trimmed, now)
            .await?;

        let mut snap = self
            .inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        for col in &mut snap.collections {
            if let Some(folder) = col.folders.iter_mut().find(|f| f.id == folder_id) {
                trimmed.clone_into(&mut folder.name);
                folder.updated_at = now;
            }
        }
        drop(snap);

        self.notify();
        Ok(())
    }

    pub(crate) async fn delete_folder(&self, folder_id: &str) -> Result<(), NetworkError> {
        let found = self
            .snapshot()
            .collections
            .iter()
            .any(|c| c.folders.iter().any(|f| f.id == folder_id));

        if !found {
            return Err(NetworkError::FolderNotFound);
        }

        self.inner.store.delete_folder(folder_id).await?;

        let mut snap = self
            .inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        for col in &mut snap.collections {
            col.folders
                .retain(|f| f.id != folder_id && f.parent_id.as_deref() != Some(folder_id));

            for req in &mut col.requests {
                if req.folder_id.as_deref() == Some(folder_id) {
                    req.folder_id = None;
                }
            }
        }
        drop(snap);

        self.notify();
        Ok(())
    }

    pub(crate) async fn create_request(
        &self,
        collection_id: &str,
        mut input: RequestInput,
    ) -> Result<RequestView, NetworkError> {
        let col = self
            .snapshot()
            .collections
            .into_iter()
            .find(|c| c.id == collection_id)
            .ok_or(NetworkError::CollectionNotFound)?;

        if let Some(folder_id) = &input.folder_id
            && !col.folders.iter().any(|f| f.id == *folder_id)
        {
            return Err(NetworkError::FolderNotFound);
        }

        let (clean_url, extracted_query) = sanitize_saved_url(&input.url_template)?;
        input.url_template = clean_url;
        merge_extracted_query_params(&mut input.query_params, extracted_query);

        let now = self.now();
        let id = Uuid::now_v7().to_string();
        let sort_order = u32::try_from(col.requests.len()).unwrap_or(u32::MAX);
        let (doc, secrets) = strip_request_doc(&input);
        let staged = self
            .stage_secret_map(Family::Request, Some(&id), &secrets)
            .await?;
        let row = RequestRow {
            id: id.clone(),
            collection_id: collection_id.to_owned(),
            folder_id: input.folder_id.clone(),
            name: input.name.trim().to_owned(),
            description: input.description.clone(),
            protocol: input.protocol,
            method: input.method,
            url_template: input.url_template.clone(),
            timeout_seconds: input.timeout_seconds,
            use_cookies: input.use_cookies,
            sort_order,
            document: doc,
            secret_parts: staged.parts,
            secret_slot: staged.slot.clone(),
            created_at: now,
            updated_at: now,
        };

        if let Err(error) = self.inner.store.insert_request(&row).await {
            self.cleanup_vault_parts(Family::Request, &staged.slot, 0, staged.parts)
                .await;
            return Err(error);
        }

        let view = RequestView {
            id: id.clone(),
            collection_id: row.collection_id,
            folder_id: row.folder_id,
            name: row.name,
            description: row.description,
            protocol: row.protocol,
            method: row.method,
            url_template: row.url_template,
            path_params: normalize_kv(&input.path_params, false),
            query_params: normalize_kv(&input.query_params, false),
            headers: normalize_kv(&input.headers, true),
            auth: input.auth,
            body: normalize_body(&input.body),
            timeout_seconds: row.timeout_seconds,
            use_cookies: row.use_cookies,
            sort_order,
            created_at: now,
            updated_at: now,
        };

        self.inner
            .secret_registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .requests
            .insert(id, staged);

        let mut snap = self
            .inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        if let Some(target) = snap.collections.iter_mut().find(|c| c.id == collection_id) {
            target.requests.push(view.clone());
        }
        drop(snap);

        self.notify();
        Ok(view)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "stages replacement secrets, updates SQLite, cleans up the old slot and refreshes the snapshot"
    )]
    pub(crate) async fn update_request(
        &self,
        request_id: &str,
        mut input: RequestInput,
    ) -> Result<RequestView, NetworkError> {
        let snap = self.snapshot();
        let (col_id, existing, valid_folder) = snap
            .collections
            .iter()
            .find_map(|c| {
                let req = c.requests.iter().find(|r| r.id == request_id)?;
                let folder_ok = input
                    .folder_id
                    .as_ref()
                    .is_none_or(|fid| c.folders.iter().any(|f| f.id == *fid));
                Some((c.id.clone(), req.clone(), folder_ok))
            })
            .ok_or(NetworkError::RequestNotFound)?;

        if !valid_folder {
            return Err(NetworkError::FolderNotFound);
        }

        let (clean_url, extracted_query) = sanitize_saved_url(&input.url_template)?;
        input.url_template = clean_url;
        merge_extracted_query_params(&mut input.query_params, extracted_query);

        let previous = self
            .inner
            .secret_registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .requests
            .get(request_id)
            .cloned()
            .unwrap_or_default();
        let now = self.now();
        let (doc, secrets) = strip_request_doc(&input);
        let staged = self
            .stage_secret_map(Family::Request, None, &secrets)
            .await?;
        let row = RequestRow {
            id: existing.id.clone(),
            collection_id: col_id.clone(),
            folder_id: input.folder_id.clone(),
            name: input.name.trim().to_owned(),
            description: input.description.clone(),
            protocol: existing.protocol,
            method: input.method,
            url_template: input.url_template.clone(),
            timeout_seconds: input.timeout_seconds,
            use_cookies: input.use_cookies,
            sort_order: existing.sort_order,
            document: doc,
            secret_parts: staged.parts,
            secret_slot: staged.slot.clone(),
            created_at: existing.created_at,
            updated_at: now,
        };

        if let Err(error) = self.inner.store.update_request(&row).await {
            self.cleanup_vault_parts(Family::Request, &staged.slot, 0, staged.parts)
                .await;
            return Err(error);
        }

        if previous.parts > 0 {
            self.cleanup_vault_parts(
                Family::Request,
                previous.effective_slot(request_id),
                0,
                previous.parts,
            )
            .await;
        }

        let updated = RequestView {
            folder_id: row.folder_id,
            name: row.name,
            description: row.description,
            method: row.method,
            url_template: row.url_template,
            path_params: normalize_kv(&input.path_params, false),
            query_params: normalize_kv(&input.query_params, false),
            headers: normalize_kv(&input.headers, true),
            auth: input.auth,
            body: normalize_body(&input.body),
            timeout_seconds: row.timeout_seconds,
            use_cookies: row.use_cookies,
            updated_at: now,
            ..existing
        };

        self.inner
            .secret_registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .requests
            .insert(request_id.to_owned(), staged);

        let mut locked = self
            .inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        if let Some(col) = locked.collections.iter_mut().find(|c| c.id == col_id)
            && let Some(slot) = col.requests.iter_mut().find(|r| r.id == request_id)
        {
            *slot = updated.clone();
        }
        drop(locked);

        self.notify();
        Ok(updated)
    }

    pub(crate) async fn duplicate_request(
        &self,
        request_id: &str,
    ) -> Result<RequestView, NetworkError> {
        let existing = self
            .snapshot()
            .collections
            .into_iter()
            .find_map(|c| c.requests.into_iter().find(|r| r.id == request_id))
            .ok_or(NetworkError::RequestNotFound)?;
        let copy_name = format!("{} (copy)", existing.name);
        let input = RequestInput {
            folder_id: existing.folder_id,
            name: copy_name
                .chars()
                .take(crate::model::MAX_NAME_CHARS)
                .collect(),
            description: existing.description,
            protocol: existing.protocol,
            method: existing.method,
            url_template: existing.url_template,
            path_params: existing.path_params,
            query_params: existing.query_params,
            headers: existing.headers,
            auth: existing.auth,
            body: existing.body,
            timeout_seconds: existing.timeout_seconds,
            use_cookies: existing.use_cookies,
        };
        self.create_request(&existing.collection_id, input).await
    }

    pub(crate) async fn delete_request(&self, request_id: &str) -> Result<(), NetworkError> {
        let found = self
            .snapshot()
            .collections
            .iter()
            .any(|c| c.requests.iter().any(|r| r.id == request_id));

        if !found {
            return Err(NetworkError::RequestNotFound);
        }

        self.inner.store.delete_request(request_id).await?;

        let meta = self
            .inner
            .secret_registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .requests
            .remove(request_id)
            .unwrap_or_default();

        if meta.parts > 0 {
            self.cleanup_vault_parts(
                Family::Request,
                meta.effective_slot(request_id),
                0,
                meta.parts,
            )
            .await;
        }

        let mut snap = self
            .inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        for col in &mut snap.collections {
            col.requests.retain(|r| r.id != request_id);
        }
        drop(snap);

        self.notify();
        Ok(())
    }

    pub(crate) async fn save_environment(
        &self,
        input: EnvironmentInput,
    ) -> Result<EnvironmentView, NetworkError> {
        let snap = self.snapshot();
        let col = snap
            .collections
            .iter()
            .find(|c| c.id == input.collection_id)
            .ok_or(NetworkError::CollectionNotFound)?;
        let now = self.now();
        let mut secrets = BTreeMap::new();
        let stripped = strip_variables("var", &input.variables, &mut secrets);
        let doc = VariablesDocument {
            variables: stripped,
        };

        if let Some(env_id) = input.environment_id {
            let existing = col
                .environments
                .iter()
                .find(|e| e.id == env_id)
                .ok_or(NetworkError::EnvironmentNotFound)?
                .clone();
            self.update_environment(
                &input.collection_id,
                &env_id,
                existing,
                &input.name,
                &input.variables,
                doc,
                &secrets,
                now,
            )
            .await
        } else {
            self.create_environment(
                &input.collection_id,
                &input.name,
                &input.variables,
                doc,
                &secrets,
                now,
            )
            .await
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "update_environment receives the already-validated collection, existing view, stripped doc and secrets"
    )]
    async fn update_environment(
        &self,
        collection_id: &str,
        env_id: &str,
        existing: EnvironmentView,
        name: &str,
        variables: &[VariableEntry],
        doc: VariablesDocument,
        secrets: &BTreeMap<String, String>,
        now: UtcMillis,
    ) -> Result<EnvironmentView, NetworkError> {
        let previous = self
            .inner
            .secret_registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .environments
            .get(env_id)
            .cloned()
            .unwrap_or_default();
        let staged = self
            .stage_secret_map(Family::Environment, None, secrets)
            .await?;
        let row = EnvironmentRow {
            id: env_id.to_owned(),
            collection_id: collection_id.to_owned(),
            name: name.trim().to_owned(),
            document: doc,
            secret_parts: staged.parts,
            secret_slot: staged.slot.clone(),
            created_at: existing.created_at,
            updated_at: now,
        };

        if let Err(error) = self.inner.store.update_environment(&row).await {
            self.cleanup_vault_parts(Family::Environment, &staged.slot, 0, staged.parts)
                .await;
            return Err(error);
        }

        if previous.parts > 0 {
            self.cleanup_vault_parts(
                Family::Environment,
                previous.effective_slot(env_id),
                0,
                previous.parts,
            )
            .await;
        }

        let updated = EnvironmentView {
            name: row.name,
            variables: normalize_vars(variables),
            updated_at: now,
            ..existing
        };
        self.inner
            .secret_registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .environments
            .insert(env_id.to_owned(), staged);

        let mut locked = self
            .inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        if let Some(target) = locked
            .collections
            .iter_mut()
            .find(|c| c.id == collection_id)
            && let Some(slot) = target.environments.iter_mut().find(|e| e.id == env_id)
        {
            *slot = updated.clone();
        }
        drop(locked);

        self.notify();
        Ok(updated)
    }

    async fn create_environment(
        &self,
        collection_id: &str,
        name: &str,
        variables: &[VariableEntry],
        doc: VariablesDocument,
        secrets: &BTreeMap<String, String>,
        now: UtcMillis,
    ) -> Result<EnvironmentView, NetworkError> {
        let id = Uuid::now_v7().to_string();
        let staged = self
            .stage_secret_map(Family::Environment, Some(&id), secrets)
            .await?;
        let row = EnvironmentRow {
            id: id.clone(),
            collection_id: collection_id.to_owned(),
            name: name.trim().to_owned(),
            document: doc,
            secret_parts: staged.parts,
            secret_slot: staged.slot.clone(),
            created_at: now,
            updated_at: now,
        };

        if let Err(error) = self.inner.store.insert_environment(&row).await {
            self.cleanup_vault_parts(Family::Environment, &staged.slot, 0, staged.parts)
                .await;
            return Err(error);
        }

        let view = EnvironmentView {
            id: id.clone(),
            collection_id: row.collection_id,
            name: row.name,
            variables: normalize_vars(variables),
            created_at: now,
            updated_at: now,
        };
        self.inner
            .secret_registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .environments
            .insert(id, staged);

        let mut locked = self
            .inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        if let Some(target) = locked
            .collections
            .iter_mut()
            .find(|c| c.id == collection_id)
        {
            target.environments.push(view.clone());
        }
        drop(locked);

        self.notify();
        Ok(view)
    }

    pub(crate) async fn delete_environment(
        &self,
        environment_id: &str,
    ) -> Result<(), NetworkError> {
        let col_id = self
            .snapshot()
            .collections
            .iter()
            .find_map(|c| {
                c.environments
                    .iter()
                    .any(|e| e.id == environment_id)
                    .then(|| c.id.clone())
            })
            .ok_or(NetworkError::EnvironmentNotFound)?;
        self.inner.store.delete_environment(environment_id).await?;

        let meta = self
            .inner
            .secret_registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .environments
            .remove(environment_id)
            .unwrap_or_default();

        if meta.parts > 0 {
            self.cleanup_vault_parts(
                Family::Environment,
                meta.effective_slot(environment_id),
                0,
                meta.parts,
            )
            .await;
        }

        let now = self.now();
        let was_active = {
            let mut snap = self
                .inner
                .snapshot
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let mut cleared = false;

            if let Some(col) = snap.collections.iter_mut().find(|c| c.id == col_id) {
                col.environments.retain(|e| e.id != environment_id);

                if col.active_environment_id.as_deref() == Some(environment_id) {
                    col.active_environment_id = None;
                    cleared = true;
                }
            }
            cleared
        };

        if was_active
            && let Err(error) = self
                .inner
                .store
                .set_active_environment(&col_id, None, now)
                .await
        {
            tracing::warn!(%error, "could not clear active environment after deletion");
        }

        self.notify();
        Ok(())
    }

    pub(crate) async fn set_active_environment(
        &self,
        collection_id: &str,
        environment_id: Option<&str>,
    ) -> Result<(), NetworkError> {
        let snap = self.snapshot();
        let col = snap
            .collections
            .iter()
            .find(|c| c.id == collection_id)
            .ok_or(NetworkError::CollectionNotFound)?;

        if let Some(env_id) = environment_id
            && !col.environments.iter().any(|e| e.id == env_id)
        {
            return Err(NetworkError::EnvironmentNotFound);
        }

        let now = self.now();
        self.inner
            .store
            .set_active_environment(collection_id, environment_id, now)
            .await?;

        let mut locked = self
            .inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        if let Some(target) = locked
            .collections
            .iter_mut()
            .find(|c| c.id == collection_id)
        {
            target.active_environment_id = environment_id.map(str::to_owned);
            target.updated_at = now;
        }
        drop(locked);

        self.notify();
        Ok(())
    }

    pub(crate) async fn save_global_variables(
        &self,
        variables: Vec<VariableEntry>,
    ) -> Result<Vec<VariableEntry>, NetworkError> {
        let now = self.now();
        let previous = self
            .inner
            .secret_registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .global_variables
            .clone();
        let mut secrets = BTreeMap::new();
        let stripped = strip_variables("var", &variables, &mut secrets);
        let staged = self
            .stage_secret_map(Family::Global, None, &secrets)
            .await?;

        if let Err(error) = self
            .inner
            .store
            .save_global_variables(
                &VariablesDocument {
                    variables: stripped,
                },
                staged.parts,
                &staged.slot,
                now,
            )
            .await
        {
            self.cleanup_vault_parts(Family::Global, &staged.slot, 0, staged.parts)
                .await;
            return Err(error);
        }

        if previous.parts > 0 {
            self.cleanup_vault_parts(
                Family::Global,
                previous.effective_slot("global"),
                0,
                previous.parts,
            )
            .await;
        }

        let normalized = normalize_vars(&variables);
        self.inner
            .secret_registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .global_variables = staged;
        self.inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .global_variables
            .clone_from(&normalized);
        self.notify();
        Ok(normalized)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "handles both editing an existing cookie by ID and upserting a new cookie"
    )]
    pub(crate) async fn save_cookie(&self, input: CookieInput) -> Result<CookieView, NetworkError> {
        let now = self.now();
        let raw_domain = input.domain.trim();
        let leading_dot = raw_domain.starts_with('.');
        let domain = raw_domain.trim_start_matches('.').to_ascii_lowercase();
        let host_only = !leading_dot && input.host_only;
        let path = if input.path.trim().is_empty() {
            "/".to_owned()
        } else {
            input.path.trim().to_owned()
        };
        let name = input.name.trim().to_owned();

        if let Some(cookie_id) = input.cookie_id {
            let existing = self
                .snapshot()
                .cookies
                .into_iter()
                .find(|c| c.id == cookie_id)
                .ok_or(NetworkError::CookieNotFound)?;
            let previous = self
                .inner
                .secret_registry
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .cookies
                .get(&cookie_id)
                .cloned()
                .unwrap_or_default();
            let staged = self
                .stage_secret_text(Family::Cookie, None, &input.value)
                .await?;
            let row = CookieRow {
                id: cookie_id.clone(),
                name,
                domain,
                path,
                host_only,
                secure: input.secure,
                http_only: input.http_only,
                expires_at: input.expires_at,
                enabled: input.enabled,
                secret_parts: staged.parts,
                secret_slot: staged.slot.clone(),
                created_at: existing.created_at,
                updated_at: now,
            };

            if let Err(error) = self.inner.store.update_cookie(&row).await {
                self.cleanup_vault_parts(Family::Cookie, &staged.slot, 0, staged.parts)
                    .await;
                return Err(error);
            }

            if previous.parts > 0 {
                self.cleanup_vault_parts(
                    Family::Cookie,
                    previous.effective_slot(&cookie_id),
                    0,
                    previous.parts,
                )
                .await;
            }

            let view = CookieView {
                id: cookie_id.clone(),
                name: row.name,
                value: input.value,
                domain: row.domain,
                path: row.path,
                host_only: row.host_only,
                secure: row.secure,
                http_only: row.http_only,
                expires_at: row.expires_at,
                enabled: row.enabled,
                created_at: row.created_at,
                updated_at: now,
            };
            self.inner
                .secret_registry
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .cookies
                .insert(cookie_id.clone(), staged);

            let mut snap = self
                .inner
                .snapshot
                .lock()
                .unwrap_or_else(PoisonError::into_inner);

            if let Some(slot) = snap.cookies.iter_mut().find(|c| c.id == cookie_id) {
                *slot = view.clone();
            }
            drop(snap);

            self.notify();
            Ok(view)
        } else {
            self.upsert_cookie_entry(
                name,
                input.value,
                domain,
                path,
                host_only,
                input.secure,
                input.http_only,
                input.expires_at,
                input.enabled,
                now,
            )
            .await
        }
    }

    #[expect(
        clippy::too_many_arguments,
        clippy::fn_params_excessive_bools,
        reason = "cookie attributes mirror RFC 6265 fields plus enabled toggle"
    )]
    async fn upsert_cookie_entry(
        &self,
        name: String,
        value: String,
        domain: String,
        path: String,
        host_only: bool,
        secure: bool,
        http_only: bool,
        expires_at: Option<UtcMillis>,
        enabled: bool,
        now: UtcMillis,
    ) -> Result<CookieView, NetworkError> {
        let existing = self
            .snapshot()
            .cookies
            .into_iter()
            .find(|c| c.name == name && c.domain == domain && c.path == path);
        let is_new = existing.is_none();
        let id = existing
            .as_ref()
            .map_or_else(|| Uuid::now_v7().to_string(), |c| c.id.clone());
        let created_at = existing.as_ref().map_or(now, |c| c.created_at);
        let previous = self
            .inner
            .secret_registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cookies
            .get(&id)
            .cloned()
            .unwrap_or_default();
        let staged = self
            .stage_secret_text(Family::Cookie, is_new.then_some(id.as_str()), &value)
            .await?;
        let row = CookieRow {
            id: id.clone(),
            name,
            domain,
            path,
            host_only,
            secure,
            http_only,
            expires_at,
            enabled,
            secret_parts: staged.parts,
            secret_slot: staged.slot.clone(),
            created_at,
            updated_at: now,
        };

        if let Err(error) = self.inner.store.upsert_cookie(&row).await {
            self.cleanup_vault_parts(Family::Cookie, &staged.slot, 0, staged.parts)
                .await;
            return Err(error);
        }

        if !is_new && previous.parts > 0 {
            self.cleanup_vault_parts(
                Family::Cookie,
                previous.effective_slot(&id),
                0,
                previous.parts,
            )
            .await;
        }

        let view = CookieView {
            id: id.clone(),
            name: row.name,
            value,
            domain: row.domain,
            path: row.path,
            host_only: row.host_only,
            secure: row.secure,
            http_only: row.http_only,
            expires_at: row.expires_at,
            enabled: row.enabled,
            created_at,
            updated_at: now,
        };
        self.inner
            .secret_registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cookies
            .insert(id.clone(), staged);

        let mut snap = self
            .inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        if let Some(slot) = snap.cookies.iter_mut().find(|c| c.id == id) {
            *slot = view.clone();
        } else {
            snap.cookies.push(view.clone());
        }
        drop(snap);

        self.notify();
        Ok(view)
    }

    pub(crate) async fn delete_cookie(&self, cookie_id: &str) -> Result<(), NetworkError> {
        let found = self.snapshot().cookies.iter().any(|c| c.id == cookie_id);

        if !found {
            return Err(NetworkError::CookieNotFound);
        }

        self.inner.store.delete_cookie(cookie_id).await?;

        let meta = self
            .inner
            .secret_registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cookies
            .remove(cookie_id)
            .unwrap_or_default();

        if meta.parts > 0 {
            self.cleanup_vault_parts(
                Family::Cookie,
                meta.effective_slot(cookie_id),
                0,
                meta.parts,
            )
            .await;
        }

        self.inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cookies
            .retain(|c| c.id != cookie_id);
        self.notify();
        Ok(())
    }

    pub(crate) async fn clear_cookies(&self) -> Result<(), NetworkError> {
        self.inner.store.delete_all_cookies().await?;

        let all: Vec<(String, SecretMeta)> = {
            let mut registry = self
                .inner
                .secret_registry
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            registry.cookies.drain().collect()
        };

        for (id, meta) in all {
            if meta.parts > 0 {
                self.cleanup_vault_parts(Family::Cookie, meta.effective_slot(&id), 0, meta.parts)
                    .await;
            }
        }

        self.inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cookies
            .clear();
        self.notify();
        Ok(())
    }

    /// Applies `Set-Cookie` headers returned by an HTTP exchange into the
    /// cookie jar when `auto_cookies` is enabled.
    pub(crate) async fn apply_set_cookies(&self, set_cookies: &[ParsedSetCookie]) {
        let now = self.now();

        for cookie in set_cookies {
            if cookie.remove {
                let existing_id = self
                    .snapshot()
                    .cookies
                    .into_iter()
                    .find(|c| {
                        c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path
                    })
                    .map(|c| c.id);

                if let Some(id) = existing_id
                    && let Err(error) = self.delete_cookie(&id).await
                {
                    tracing::warn!(%error, "could not remove expired cookie from jar");
                }
            } else if let Err(error) = self
                .upsert_cookie_entry(
                    cookie.name.clone(),
                    cookie.value.clone(),
                    cookie.domain.clone(),
                    cookie.path.clone(),
                    cookie.host_only,
                    cookie.secure,
                    cookie.http_only,
                    cookie.expires_at,
                    true,
                    now,
                )
                .await
            {
                tracing::warn!(%error, "could not persist Set-Cookie update to jar");
            }
        }
    }

    /// Opens the system file dialog to pick an `OpenAPI` JSON or YAML file, reads
    /// it once and caches a preview. Returns `None` if the owner canceled.
    pub(crate) async fn pick_openapi_file(
        &self,
        filter_label: &str,
    ) -> Result<Option<OpenApiPreview>, NetworkError> {
        let filter = FileFilter::new(filter_label, ["json", "yaml", "yml"]);
        let chosen = self
            .inner
            .file_dialog
            .choose_files(filter)
            .await
            .map_err(|_err| NetworkError::FileDialogFailed)?;
        let Some(first) = chosen.into_iter().next() else {
            return Ok(None);
        };
        let bytes = self
            .inner
            .file_dialog
            .read(first.file, MAX_SPEC_BYTES as u64)
            .await
            .map_err(|error| match error {
                mannie_plugin_sdk::services::file_dialog::DialogError::TooLarge { .. } => {
                    NetworkError::SpecTooLarge
                }
                _other => NetworkError::FileUnreadable,
            })?;
        let text = String::from_utf8(bytes).map_err(|_err| NetworkError::FileUnreadable)?;
        self.preview_openapi_text(&text).map(Some)
    }

    /// Parses pasted `OpenAPI` JSON or YAML text and caches a preview.
    pub(crate) fn preview_openapi_text(&self, text: &str) -> Result<OpenApiPreview, NetworkError> {
        let imported = parse_openapi(text)?;
        let preview_id = Uuid::now_v7().to_string();
        let preview = imported.preview(preview_id.clone());

        let mut cache = self
            .inner
            .previews
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        if cache.len() >= MAX_CACHED_PREVIEWS {
            cache.remove(0);
        }

        cache.push((preview_id, imported));
        Ok(preview)
    }

    fn take_preview(&self, preview_id: &str) -> Result<ImportedCollection, NetworkError> {
        let mut cache = self
            .inner
            .previews
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let pos = cache
            .iter()
            .position(|(id, _)| id == preview_id)
            .ok_or(NetworkError::PreviewNotFound)?;
        Ok(cache.remove(pos).1)
    }

    /// Persists a previously previewed `OpenAPI` collection into SQLite and the
    /// OS credential store.
    pub(crate) async fn confirm_openapi_import(
        &self,
        input: OpenApiConfirmInput,
    ) -> Result<CollectionView, NetworkError> {
        let imported = self.take_preview(&input.preview_id)?;

        let now = self.now();
        let collection_id = Uuid::now_v7().to_string();
        let name = input
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or(&imported.title)
            .to_owned();
        let raw_base_url = input
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .unwrap_or(&imported.base_url);
        let (base_url, extracted_query) = sanitize_saved_url(raw_base_url)?;
        let mut collection_query_params = imported.query_params.clone();
        merge_extracted_query_params(&mut collection_query_params, extracted_query);

        let mut col_secrets = BTreeMap::new();
        let col_doc = CollectionDocument {
            headers: Vec::new(),
            query_params: strip_key_values(
                "query",
                &collection_query_params,
                false,
                &mut col_secrets,
            ),
            auth: strip_auth(&imported.auth, &mut col_secrets),
            variables: strip_variables("var", &imported.variables, &mut col_secrets),
        };
        let col_staged = self
            .stage_secret_map(Family::Collection, Some(&collection_id), &col_secrets)
            .await?;

        let (folder_rows, folder_views, folder_ids) =
            build_imported_folders(&collection_id, &imported.folders, now);
        let (env_rows, env_views, env_secret_metas) = self
            .stage_imported_environments(&collection_id, &imported.environments, now)
            .await?;
        let active_environment_id = env_views.first().map(|e| e.id.clone());
        let (req_rows, req_views, req_secret_metas) = self
            .stage_imported_requests(&collection_id, &folder_ids, &imported.requests, now)
            .await?;

        let col_row = CollectionRow {
            id: collection_id.clone(),
            name: name.clone(),
            description: imported.description.clone(),
            protocol: Protocol::Http,
            base_url: base_url.clone(),
            enabled: input.enabled,
            active_environment_id: active_environment_id.clone(),
            auto_cookies: true,
            document: col_doc,
            secret_parts: col_staged.parts,
            secret_slot: col_staged.slot.clone(),
            created_at: now,
            updated_at: now,
        };

        if let Err(error) = self
            .inner
            .store
            .insert_imported_collection(&col_row, &folder_rows, &env_rows, &req_rows)
            .await
        {
            self.rollback_imported_secrets(&col_staged, &env_secret_metas, &req_secret_metas)
                .await;
            return Err(error);
        }

        let view = CollectionView {
            id: collection_id.clone(),
            name,
            description: imported.description,
            protocol: Protocol::Http,
            base_url,
            enabled: input.enabled,
            active_environment_id,
            auto_cookies: true,
            headers: Vec::new(),
            query_params: normalize_kv(&collection_query_params, false),
            auth: imported.auth,
            variables: normalize_vars(&imported.variables),
            folders: folder_views,
            environments: env_views,
            requests: req_views,
            created_at: now,
            updated_at: now,
        };
        self.record_imported_collection(
            view.clone(),
            col_staged,
            env_secret_metas,
            req_secret_metas,
        );
        Ok(view)
    }

    fn record_imported_collection(
        &self,
        view: CollectionView,
        col_secret_meta: SecretMeta,
        env_secret_metas: Vec<(String, SecretMeta)>,
        req_secret_metas: Vec<(String, SecretMeta)>,
    ) {
        let mut registry = self
            .inner
            .secret_registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        registry
            .collections
            .insert(view.id.clone(), col_secret_meta);

        for (eid, meta) in env_secret_metas {
            registry.environments.insert(eid, meta);
        }

        for (rid, meta) in req_secret_metas {
            registry.requests.insert(rid, meta);
        }
        drop(registry);

        self.inner
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .collections
            .push(view);
        self.notify();
    }

    /// Runs a request directly on the owner's workbench action without an
    /// agent approval prompt (ADR-0075, decision 6).
    pub(crate) async fn send_owner_request(
        &self,
        input: ManualSendInput,
    ) -> Result<ManualResponseView, NetworkError> {
        let snap = self.snapshot();
        let col = snap
            .collections
            .iter()
            .find(|c| c.id == input.collection_id)
            .ok_or(NetworkError::CollectionNotFound)?;
        let env_id = input
            .environment_id
            .as_deref()
            .or(col.active_environment_id.as_deref());
        let env = match env_id {
            Some(id) => Some(
                col.environments
                    .iter()
                    .find(|e| e.id == id)
                    .ok_or(NetworkError::EnvironmentNotFound)?,
            ),
            None => None,
        };

        let request = match (input.request_id.as_deref(), input.draft) {
            (_, Some(draft)) => RequestView {
                id: input.request_id.unwrap_or_default(),
                collection_id: col.id.clone(),
                folder_id: draft.folder_id,
                name: draft.name,
                description: draft.description,
                protocol: draft.protocol,
                method: draft.method,
                url_template: draft.url_template,
                path_params: draft.path_params,
                query_params: draft.query_params,
                headers: draft.headers,
                auth: draft.auth,
                body: draft.body,
                timeout_seconds: draft.timeout_seconds,
                use_cookies: draft.use_cookies,
                sort_order: 0,
                created_at: self.now(),
                updated_at: self.now(),
            },
            (Some(req_id), None) => col
                .requests
                .iter()
                .find(|r| r.id == req_id)
                .cloned()
                .ok_or(NetworkError::RequestNotFound)?,
            (None, None) => return Err(NetworkError::RequestNotFound),
        };

        let resolved = resolve_http_request(
            col,
            &request,
            env,
            &snap.global_variables,
            &RuntimeOverrides::default(),
            OriginPolicy::OwnerWorkbench,
        )
        .map_err(|err| match err {
            ResolveError::InvalidUrl
            | ResolveError::SecretInOrigin
            | ResolveError::InvalidPathParam
            | ResolveError::OriginOverrideRefused => NetworkError::InvalidUrl,
        })?;

        let (cancel_tx, cancel_rx) = oneshot::channel();
        let run_id = input.run_id.clone();

        {
            let mut active = self
                .inner
                .active_runs
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            active.insert(run_id.clone(), cancel_tx);
        }

        let exchange = execute_http(
            &self.inner.client,
            request.method,
            &resolved,
            &snap.cookies,
            request.use_cookies,
            self.now(),
            MAX_OWNER_RESPONSE_BYTES,
            Some(cancel_rx),
        )
        .await;

        self.inner
            .active_runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&run_id);

        let exchange = exchange?;

        if col.auto_cookies && !exchange.set_cookies.is_empty() {
            self.apply_set_cookies(&exchange.set_cookies).await;
        }

        Ok(exchange.into_owner_view(
            request.protocol,
            request.method,
            resolved.real_url.to_string(),
        ))
    }

    /// Cancels an in-flight owner workbench request by its client `run_id`.
    pub(crate) fn cancel_owner_request(&self, run_id: &str) {
        let sender = self
            .inner
            .active_runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(run_id);

        if let Some(tx) = sender
            && tx.send(()).is_err()
        {
            tracing::debug!("owner request finished before cancel arrived");
        }
    }

    async fn stage_imported_environments(
        &self,
        collection_id: &str,
        environments: &[crate::openapi::ImportedEnvironment],
        now: UtcMillis,
    ) -> Result<
        (
            Vec<EnvironmentRow>,
            Vec<EnvironmentView>,
            Vec<(String, SecretMeta)>,
        ),
        NetworkError,
    > {
        let mut rows = Vec::new();
        let mut views = Vec::new();
        let mut secret_metas = Vec::new();

        for env in environments {
            let eid = Uuid::now_v7().to_string();
            let mut env_secrets = BTreeMap::new();
            let stripped_vars = strip_variables("var", &env.variables, &mut env_secrets);
            let staged = self
                .stage_secret_map(Family::Environment, Some(&eid), &env_secrets)
                .await?;
            rows.push(EnvironmentRow {
                id: eid.clone(),
                collection_id: collection_id.to_owned(),
                name: env.name.clone(),
                document: VariablesDocument {
                    variables: stripped_vars,
                },
                secret_parts: staged.parts,
                secret_slot: staged.slot.clone(),
                created_at: now,
                updated_at: now,
            });
            secret_metas.push((eid.clone(), staged));
            views.push(EnvironmentView {
                id: eid,
                collection_id: collection_id.to_owned(),
                name: env.name.clone(),
                variables: normalize_vars(&env.variables),
                created_at: now,
                updated_at: now,
            });
        }

        Ok((rows, views, secret_metas))
    }

    async fn stage_imported_requests(
        &self,
        collection_id: &str,
        folder_ids: &HashMap<String, String>,
        requests: &[crate::openapi::ImportedRequest],
        now: UtcMillis,
    ) -> Result<(Vec<RequestRow>, Vec<RequestView>, Vec<(String, SecretMeta)>), NetworkError> {
        let mut rows = Vec::new();
        let mut views = Vec::new();
        let mut secret_metas = Vec::new();

        for (index, req) in requests.iter().enumerate() {
            let rid = Uuid::now_v7().to_string();
            let sort_order = u32::try_from(index).unwrap_or(u32::MAX);
            let folder_id = req
                .folder
                .as_ref()
                .and_then(|fname| folder_ids.get(fname).cloned());
            let mut req_secrets = BTreeMap::new();
            let doc = RequestDocument {
                path_params: strip_key_values("path", &req.path_params, false, &mut req_secrets),
                query_params: strip_key_values("query", &req.query_params, false, &mut req_secrets),
                headers: strip_key_values("header", &req.headers, true, &mut req_secrets),
                auth: strip_auth(&req.auth, &mut req_secrets),
                body: strip_body(&req.body, &mut req_secrets),
            };
            let staged = self
                .stage_secret_map(Family::Request, Some(&rid), &req_secrets)
                .await?;
            rows.push(RequestRow {
                id: rid.clone(),
                collection_id: collection_id.to_owned(),
                folder_id: folder_id.clone(),
                name: req.name.clone(),
                description: req.description.clone(),
                protocol: Protocol::Http,
                method: req.method,
                url_template: req.url_template.clone(),
                timeout_seconds: req.timeout_seconds,
                use_cookies: true,
                sort_order,
                document: doc,
                secret_parts: staged.parts,
                secret_slot: staged.slot.clone(),
                created_at: now,
                updated_at: now,
            });
            secret_metas.push((rid.clone(), staged));
            views.push(RequestView {
                id: rid,
                collection_id: collection_id.to_owned(),
                folder_id,
                name: req.name.clone(),
                description: req.description.clone(),
                protocol: Protocol::Http,
                method: req.method,
                url_template: req.url_template.clone(),
                path_params: normalize_kv(&req.path_params, false),
                query_params: normalize_kv(&req.query_params, false),
                headers: normalize_kv(&req.headers, true),
                auth: req.auth.clone(),
                body: normalize_body(&req.body),
                timeout_seconds: req.timeout_seconds,
                use_cookies: true,
                sort_order,
                created_at: now,
                updated_at: now,
            });
        }

        Ok((rows, views, secret_metas))
    }

    async fn rollback_imported_secrets(
        &self,
        col_meta: &SecretMeta,
        env_metas: &[(String, SecretMeta)],
        req_metas: &[(String, SecretMeta)],
    ) {
        if col_meta.parts > 0 {
            self.cleanup_vault_parts(Family::Collection, &col_meta.slot, 0, col_meta.parts)
                .await;
        }

        for (_eid, meta) in env_metas {
            if meta.parts > 0 {
                self.cleanup_vault_parts(Family::Environment, &meta.slot, 0, meta.parts)
                    .await;
            }
        }

        for (_rid, meta) in req_metas {
            if meta.parts > 0 {
                self.cleanup_vault_parts(Family::Request, &meta.slot, 0, meta.parts)
                    .await;
            }
        }
    }

    /// Builds a [`Redactor`] holding every secret value and cookie across the
    /// collection, environment, request, global variables and cookie jar so
    /// agent outputs, archives and errors never leak a secret or cookie.
    pub(crate) fn redactor_for(
        &self,
        collection: &CollectionView,
        request: &RequestView,
        environment: Option<&EnvironmentView>,
        resolved: &ResolvedHttpRequest,
    ) -> Redactor {
        let snap = self.snapshot();
        let mut secrets: Vec<String> = Vec::new();

        collect_var_secrets(&snap.global_variables, &mut secrets);
        collect_var_secrets(&collection.variables, &mut secrets);
        collect_kv_secrets(&collection.headers, true, &mut secrets);
        collect_kv_secrets(&collection.query_params, false, &mut secrets);
        collect_auth_secrets(&collection.auth, &mut secrets);

        if let Some(env) = environment {
            collect_var_secrets(&env.variables, &mut secrets);
        }

        collect_kv_secrets(&request.path_params, false, &mut secrets);
        collect_kv_secrets(&request.query_params, false, &mut secrets);
        collect_kv_secrets(&request.headers, true, &mut secrets);
        collect_auth_secrets(&request.auth, &mut secrets);

        if let BodyConfig::FormUrlEncoded { entries } = &request.body {
            collect_kv_secrets(entries, false, &mut secrets);
        }

        for cookie in &snap.cookies {
            if !cookie.value.is_empty() {
                secrets.push(cookie.value.clone());
            }
        }

        for (_name, value, is_secret) in &resolved.headers {
            if *is_secret && !value.is_empty() {
                secrets.push(value.clone());

                if let Some(token) = value.strip_prefix("Bearer ") {
                    secrets.push(token.to_owned());
                }
            }
        }

        let refs: Vec<&str> = secrets.iter().map(String::as_str).collect();
        let basic: Vec<(&str, &str)> = resolved
            .basic_auth
            .as_ref()
            .map(|(u, p)| vec![(u.as_str(), p.as_str())])
            .unwrap_or_default();
        Redactor::new(refs, basic)
    }

    /// Runs an authorized agent HTTP request, updating the cookie jar when
    /// `auto_cookies` is enabled and returning the exchange + redactor.
    pub(crate) async fn execute_agent_http(
        &self,
        collection: &CollectionView,
        request: &RequestView,
        environment: Option<&EnvironmentView>,
        resolved: &ResolvedHttpRequest,
    ) -> Result<(HttpExchange, Redactor), (NetworkError, Redactor)> {
        let snap = self.snapshot();
        let mut redactor = self.redactor_for(collection, request, environment, resolved);
        let exchange = match execute_http(
            &self.inner.client,
            request.method,
            resolved,
            &snap.cookies,
            request.use_cookies,
            self.now(),
            crate::model::MAX_AGENT_RESPONSE_BYTES,
            None,
        )
        .await
        {
            Ok(exchange) => exchange,
            Err(error) => return Err((error, redactor)),
        };

        for set_cookie in &exchange.set_cookies {
            redactor.add_secret(&set_cookie.value);
        }

        if collection.auto_cookies && !exchange.set_cookies.is_empty() {
            self.apply_set_cookies(&exchange.set_cookies).await;
        }

        Ok((exchange, redactor))
    }

    /// Formats a collection's base URL with any secret variable or embedded
    /// credential redacted.
    pub(crate) fn redacted_base_url(&self, collection: &CollectionView) -> String {
        let snap = self.snapshot();
        let env = collection.active_environment_id.as_deref().and_then(|id| {
            collection
                .environments
                .iter()
                .find(|candidate| candidate.id == id)
        });
        let scope = crate::template::build_variable_scope(
            &snap.global_variables,
            &collection.variables,
            env,
            &BTreeMap::new(),
        );
        let rendered = crate::template::interpolate(collection.base_url.trim(), &scope).redacted;
        let scrubbed = redact_url_template_for_agent(&rendered);

        if scrubbed.contains('@') && !scrubbed.contains(&format!("{REDACTED}@")) {
            REDACTED.to_owned()
        } else {
            scrubbed
        }
    }
}

fn merge_extracted_query_params(existing: &mut Vec<KeyValueEntry>, extracted: Vec<KeyValueEntry>) {
    for entry in extracted {
        if let Some(slot) = existing
            .iter_mut()
            .find(|item| item.key.eq_ignore_ascii_case(&entry.key))
        {
            slot.value = entry.value;
            slot.enabled = true;
            slot.secret = true;
        } else {
            existing.push(entry);
        }
    }
}

fn strip_collection_doc(input: &CollectionInput) -> (CollectionDocument, BTreeMap<String, String>) {
    let mut secrets = BTreeMap::new();
    let doc = CollectionDocument {
        headers: strip_key_values("header", &input.headers, true, &mut secrets),
        query_params: strip_key_values("query", &input.query_params, false, &mut secrets),
        auth: strip_auth(&input.auth, &mut secrets),
        variables: strip_variables("var", &input.variables, &mut secrets),
    };
    (doc, secrets)
}

fn strip_request_doc(input: &RequestInput) -> (RequestDocument, BTreeMap<String, String>) {
    let mut secrets = BTreeMap::new();
    let doc = RequestDocument {
        path_params: strip_key_values("path", &input.path_params, false, &mut secrets),
        query_params: strip_key_values("query", &input.query_params, false, &mut secrets),
        headers: strip_key_values("header", &input.headers, true, &mut secrets),
        auth: strip_auth(&input.auth, &mut secrets),
        body: strip_body(&input.body, &mut secrets),
    };
    (doc, secrets)
}

fn normalize_kv(entries: &[KeyValueEntry], header_rules: bool) -> Vec<KeyValueEntry> {
    entries
        .iter()
        .map(|entry| {
            let secret = entry.secret
                || if header_rules {
                    is_secret_header_name(&entry.key)
                } else {
                    is_secret_name(&entry.key)
                };
            KeyValueEntry {
                secret,
                ..entry.clone()
            }
        })
        .collect()
}

fn normalize_vars(variables: &[VariableEntry]) -> Vec<VariableEntry> {
    variables
        .iter()
        .map(|entry| {
            let secret = entry.secret || is_secret_name(&entry.key);
            VariableEntry {
                secret,
                ..entry.clone()
            }
        })
        .collect()
}

fn normalize_body(body: &BodyConfig) -> BodyConfig {
    match body {
        BodyConfig::FormUrlEncoded { entries } => BodyConfig::FormUrlEncoded {
            entries: normalize_kv(entries, false),
        },
        other => other.clone(),
    }
}

fn collect_var_secrets(entries: &[VariableEntry], out: &mut Vec<String>) {
    for entry in entries {
        let secret = entry.secret || is_secret_name(&entry.key);

        if secret && !entry.value.is_empty() {
            out.push(entry.value.clone());
        }
    }
}

fn collect_kv_secrets(entries: &[KeyValueEntry], header_rules: bool, out: &mut Vec<String>) {
    for entry in entries {
        let secret = entry.secret
            || if header_rules {
                is_secret_header_name(&entry.key)
            } else {
                is_secret_name(&entry.key)
            };

        if secret && !entry.value.is_empty() {
            out.push(entry.value.clone());
        }
    }
}

fn collect_auth_secrets(auth: &AuthConfig, out: &mut Vec<String>) {
    match auth {
        AuthConfig::Bearer { token } if !token.is_empty() => out.push(token.clone()),
        AuthConfig::Basic { password, .. } if !password.is_empty() => out.push(password.clone()),
        AuthConfig::ApiKey { value, .. } if !value.is_empty() => out.push(value.clone()),
        _other => {}
    }
}

async fn load_cookies(
    vault: &Vault,
    rows: Vec<CookieRow>,
    registry: &mut SecretRegistry,
) -> Result<Vec<CookieView>, NetworkError> {
    let mut cookies = Vec::with_capacity(rows.len());

    for row in rows {
        let slot = effective_slot(&row.id, &row.secret_slot).to_owned();
        registry.cookies.insert(
            row.id.clone(),
            SecretMeta::new(slot.clone(), row.secret_parts),
        );
        let value = if row.secret_parts == 0 {
            String::new()
        } else {
            vault
                .load_text(Family::Cookie, &slot, row.secret_parts)
                .await
                .map_err(|_err| NetworkError::CredentialStoreUnavailable)?
                .map(|secret| secret.expose().to_owned())
                .unwrap_or_default()
        };
        cookies.push(CookieView {
            id: row.id,
            name: row.name,
            value,
            domain: row.domain,
            path: row.path,
            host_only: row.host_only,
            secure: row.secure,
            http_only: row.http_only,
            expires_at: row.expires_at,
            enabled: row.enabled,
            created_at: row.created_at,
            updated_at: row.updated_at,
        });
    }

    Ok(cookies)
}

fn group_folders(rows: Vec<FolderRow>) -> HashMap<String, Vec<FolderView>> {
    let mut folders_by_col: HashMap<String, Vec<FolderView>> = HashMap::new();

    for row in rows {
        folders_by_col
            .entry(row.collection_id.clone())
            .or_default()
            .push(FolderView {
                id: row.id,
                collection_id: row.collection_id,
                parent_id: row.parent_id,
                name: row.name,
                sort_order: row.sort_order,
                created_at: row.created_at,
                updated_at: row.updated_at,
            });
    }

    folders_by_col
}

async fn load_environments(
    vault: &Vault,
    rows: Vec<EnvironmentRow>,
    registry: &mut SecretRegistry,
) -> Result<HashMap<String, Vec<EnvironmentView>>, NetworkError> {
    let mut envs_by_col: HashMap<String, Vec<EnvironmentView>> = HashMap::new();

    for row in rows {
        let slot = effective_slot(&row.id, &row.secret_slot).to_owned();
        registry.environments.insert(
            row.id.clone(),
            SecretMeta::new(slot.clone(), row.secret_parts),
        );
        let env_secrets = vault
            .load_map(Family::Environment, &slot, row.secret_parts)
            .await
            .map_err(|_err| NetworkError::CredentialStoreUnavailable)?;
        let variables = hydrate_variables("var", &row.document.variables, &env_secrets);
        envs_by_col
            .entry(row.collection_id.clone())
            .or_default()
            .push(EnvironmentView {
                id: row.id,
                collection_id: row.collection_id,
                name: row.name,
                variables,
                created_at: row.created_at,
                updated_at: row.updated_at,
            });
    }

    Ok(envs_by_col)
}

async fn load_requests(
    vault: &Vault,
    rows: Vec<RequestRow>,
    registry: &mut SecretRegistry,
) -> Result<HashMap<String, Vec<RequestView>>, NetworkError> {
    let mut reqs_by_col: HashMap<String, Vec<RequestView>> = HashMap::new();

    for row in rows {
        let slot = effective_slot(&row.id, &row.secret_slot).to_owned();
        registry.requests.insert(
            row.id.clone(),
            SecretMeta::new(slot.clone(), row.secret_parts),
        );
        let req_secrets = vault
            .load_map(Family::Request, &slot, row.secret_parts)
            .await
            .map_err(|_err| NetworkError::CredentialStoreUnavailable)?;
        let path_params =
            hydrate_key_values("path", &row.document.path_params, false, &req_secrets);
        let query_params =
            hydrate_key_values("query", &row.document.query_params, false, &req_secrets);
        let headers = hydrate_key_values("header", &row.document.headers, true, &req_secrets);
        let auth = hydrate_auth(&row.document.auth, &req_secrets);
        let body = hydrate_body(&row.document.body, &req_secrets);
        reqs_by_col
            .entry(row.collection_id.clone())
            .or_default()
            .push(RequestView {
                id: row.id,
                collection_id: row.collection_id,
                folder_id: row.folder_id,
                name: row.name,
                description: row.description,
                protocol: row.protocol,
                method: row.method,
                url_template: row.url_template,
                path_params,
                query_params,
                headers,
                auth,
                body,
                timeout_seconds: row.timeout_seconds,
                use_cookies: row.use_cookies,
                sort_order: row.sort_order,
                created_at: row.created_at,
                updated_at: row.updated_at,
            });
    }

    Ok(reqs_by_col)
}

async fn load_collections(
    vault: &Vault,
    rows: Vec<CollectionRow>,
    mut folders_by_col: HashMap<String, Vec<FolderView>>,
    mut envs_by_col: HashMap<String, Vec<EnvironmentView>>,
    mut reqs_by_col: HashMap<String, Vec<RequestView>>,
    registry: &mut SecretRegistry,
) -> Result<Vec<CollectionView>, NetworkError> {
    let mut collections = Vec::with_capacity(rows.len());

    for row in rows {
        let slot = effective_slot(&row.id, &row.secret_slot).to_owned();
        registry.collections.insert(
            row.id.clone(),
            SecretMeta::new(slot.clone(), row.secret_parts),
        );
        let col_secrets = vault
            .load_map(Family::Collection, &slot, row.secret_parts)
            .await
            .map_err(|_err| NetworkError::CredentialStoreUnavailable)?;
        let headers = hydrate_key_values("header", &row.document.headers, true, &col_secrets);
        let query_params =
            hydrate_key_values("query", &row.document.query_params, false, &col_secrets);
        let auth = hydrate_auth(&row.document.auth, &col_secrets);
        let variables = hydrate_variables("var", &row.document.variables, &col_secrets);
        let folders = folders_by_col.remove(&row.id).unwrap_or_default();
        let environments = envs_by_col.remove(&row.id).unwrap_or_default();
        let requests = reqs_by_col.remove(&row.id).unwrap_or_default();
        collections.push(CollectionView {
            id: row.id,
            name: row.name,
            description: row.description,
            protocol: row.protocol,
            base_url: row.base_url,
            enabled: row.enabled,
            active_environment_id: row.active_environment_id,
            auto_cookies: row.auto_cookies,
            headers,
            query_params,
            auth,
            variables,
            folders,
            environments,
            requests,
            created_at: row.created_at,
            updated_at: row.updated_at,
        });
    }

    Ok(collections)
}

fn build_imported_folders(
    collection_id: &str,
    folder_names: &[String],
    now: UtcMillis,
) -> (Vec<FolderRow>, Vec<FolderView>, HashMap<String, String>) {
    let mut rows = Vec::new();
    let mut views = Vec::new();
    let mut ids = HashMap::new();

    for (index, folder_name) in folder_names.iter().enumerate() {
        let fid = Uuid::now_v7().to_string();
        let sort_order = u32::try_from(index).unwrap_or(u32::MAX);
        ids.insert(folder_name.clone(), fid.clone());
        rows.push(FolderRow {
            id: fid.clone(),
            collection_id: collection_id.to_owned(),
            parent_id: None,
            name: folder_name.clone(),
            sort_order,
            created_at: now,
            updated_at: now,
        });
        views.push(FolderView {
            id: fid,
            collection_id: collection_id.to_owned(),
            parent_id: None,
            name: folder_name.clone(),
            sort_order,
            created_at: now,
            updated_at: now,
        });
    }

    (rows, views, ids)
}
