//! Central runtime manager for the Proxmox plugin: coordinates SQLite state,
//! OS credential store passwords, per-server REST clients, TLS inspection and
//! pinning, fail-closed VM allowlist enforcement, live server broadcasts, and
//! VNC console sessions (ADR-0080).

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use mannie_plugin_sdk::contracts::{Clock, ConversationId, Secret};
use mannie_plugin_sdk::ipc::PluginStream;
use mannie_plugin_sdk::services::secrets::SecretsApi;
use mannie_plugin_sdk::services::store::{ModuleStoreApi, StoreError};
use tokio::sync::watch;

use crate::api::{IsoListResult, ProxmoxApiClient, ProxmoxVmInfo};
use crate::keysyms::KeyStroke;
use crate::model::{
    AddServer, AllowedVmRecord, CertificateInfo, DiscoverableVmView, PinCertificate, ProxmoxError,
    ServerRecord, ServerState, SetAllowedVms, SetVncControl, TestConnectionResult, UpdateServer,
    VncControlOwner, VncKeyboardInput, VncPointerInput, VncSessionSummary, VncSessionTarget,
    VncSessionUpdate, normalize_host, normalize_sha256_hex,
};
use crate::store::{NAME_TAKEN, ProxmoxStore, ServerFields};
use crate::tls::probe_certificate;
use crate::vnc::{SessionKey, VncManager, VncScreenshot};

/// Timeout for owner-initiated TLS certificate inspection in Settings.
const CERT_PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// Secret key path in the OS credential store for a server's password.
fn secret_entry(server_id: &str) -> String {
    format!("server/{server_id}")
}

/// Shared runtime state of a running Proxmox plugin instance.
#[derive(Debug, Clone)]
pub(crate) struct ProxmoxManager {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    store: ProxmoxStore,
    secrets: Arc<dyn SecretsApi>,
    clock: Arc<dyn Clock>,
    /// Live list of all registered servers + their explicitly allowed VMs.
    states: watch::Sender<Vec<ServerState>>,
    /// Cached per-server REST API clients (invalidated on server edit/pin/remove).
    clients: Mutex<HashMap<String, Arc<ProxmoxApiClient>>>,
    /// Serializes Settings mutations to SQLite + secrets + cache.
    mutations: tokio::sync::Mutex<()>,
    /// Live VNC sessions across conversations.
    vnc: VncManager,
}

impl ProxmoxManager {
    /// Loads all servers and their allowed VMs from SQLite and starts the manager.
    pub(crate) async fn load(
        store: Arc<dyn ModuleStoreApi>,
        secrets: Arc<dyn SecretsApi>,
        clock: Arc<dyn Clock>,
        idle_timeout: Duration,
    ) -> Result<Self, StoreError> {
        let store = ProxmoxStore::new(store);
        let initial = store.load_all().await?;
        let (states, _rx) = watch::channel(initial);
        Ok(Self {
            inner: Arc::new(Inner {
                store,
                secrets,
                clock,
                states,
                clients: Mutex::new(HashMap::new()),
                mutations: tokio::sync::Mutex::new(()),
                vnc: VncManager::new(idle_timeout),
            }),
        })
    }

    /// Current snapshot of all servers and their explicitly allowed VMs.
    pub(crate) fn states(&self) -> Vec<ServerState> {
        self.inner.states.borrow().clone()
    }

    /// Subscribes to live server + allowlist updates for Settings UI.
    pub(crate) fn follow_servers(&self) -> watch::Receiver<Vec<ServerState>> {
        self.inner.states.subscribe()
    }

    /// Accesses the VNC manager for idle reaping, work reporting and lifecycle teardown.
    pub(crate) fn vnc(&self) -> &VncManager {
        &self.inner.vnc
    }

    /// Looks up a server by ID or case-insensitive name.
    pub(crate) fn find_server(&self, server_ref: &str) -> Option<ServerState> {
        let needle = server_ref.trim();
        let states = self.inner.states.borrow();
        states
            .iter()
            .find(|state| {
                state.server.id == needle || state.server.name.eq_ignore_ascii_case(needle)
            })
            .cloned()
    }

    /// Enforces the fail-closed VM allowlist: returns `(ServerState, AllowedVmRecord)`
    /// ONLY if `server_ref` exists and `vmid` is explicitly in `proxmox_allowed_vms`.
    pub(crate) fn resolve_allowed_vm(
        &self,
        server_ref: &str,
        vmid: u32,
    ) -> Result<(ServerRecord, AllowedVmRecord), ProxmoxError> {
        let state = self
            .find_server(server_ref)
            .ok_or(ProxmoxError::ServerNotFound)?;
        let vm = state
            .allowed_vms
            .iter()
            .find(|allowed| allowed.vmid == vmid)
            .cloned()
            .ok_or(ProxmoxError::VmNotAllowed)?;
        Ok((state.server, vm))
    }

    /// Checks whether at least one allowed VM on `server_ref` resides on `node`.
    pub(crate) fn ensure_node_has_allowed_vm(
        &self,
        server_ref: &str,
        node: &str,
    ) -> Result<ServerRecord, ProxmoxError> {
        let state = self
            .find_server(server_ref)
            .ok_or(ProxmoxError::ServerNotFound)?;
        let allowed_on_node = state.allowed_vms.iter().any(|vm| vm.node == node);
        if !allowed_on_node {
            return Err(ProxmoxError::VmNotAllowed);
        }
        Ok(state.server)
    }

    /// Adds a new Proxmox server with zero allowed VMs by default (fail closed).
    pub(crate) async fn add_server(&self, input: AddServer) -> Result<ServerState, ProxmoxError> {
        let _guard = self.inner.mutations.lock().await;
        let host = normalize_host(&input.host).ok_or_else(|| ProxmoxError::InvalidRequest {
            detail: "invalid host".to_owned(),
        })?;
        let fields = ServerFields {
            name: input.name.trim().to_owned(),
            host,
            port: input.port,
            username: input.username.trim().to_owned(),
            note: input.note.trim().to_owned(),
        };
        let id = uuid::Uuid::now_v7().to_string();
        let now = self.inner.clock.now();

        self.inner
            .store
            .insert_server(&id, &fields, now)
            .await
            .map_err(map_store_error)?;

        if let Err(error) = self
            .inner
            .secrets
            .save(&secret_entry(&id), input.password)
            .await
        {
            tracing::warn!(%error, server = %id, "could not store Proxmox password");
            drop(self.inner.store.delete_server(&id).await);
            return Err(ProxmoxError::CredentialsFailed);
        }

        if let Err(error) = self.inner.store.set_password_set(&id, true, now).await {
            tracing::error!(%error, server = %id, "could not record Proxmox password flag");
            if let Err(cleanup) = self.inner.secrets.delete(&secret_entry(&id)).await {
                tracing::warn!(%cleanup, server = %id, "rollback could not remove Proxmox password");
            }
            drop(self.inner.store.delete_server(&id).await);
            return Err(ProxmoxError::StorageFailed);
        }

        self.reload_states().await?;
        self.find_server(&id).ok_or(ProxmoxError::ServerNotFound)
    }

    /// Updates an existing Proxmox server. Changing `host` or `port` clears
    /// BOTH its pinned TLS certificate AND its VM allowlist and closes all
    /// live VNC sessions on that server.
    pub(crate) async fn update_server(
        &self,
        input: UpdateServer,
    ) -> Result<ServerState, ProxmoxError> {
        let _guard = self.inner.mutations.lock().await;
        let existing = self
            .find_server(&input.server_id)
            .ok_or(ProxmoxError::ServerNotFound)?;
        let host = normalize_host(&input.host).ok_or_else(|| ProxmoxError::InvalidRequest {
            detail: "invalid host".to_owned(),
        })?;
        let fields = ServerFields {
            name: input.name.trim().to_owned(),
            host,
            port: input.port,
            username: input.username.trim().to_owned(),
            note: input.note.trim().to_owned(),
        };
        let endpoint_changed =
            fields.host != existing.server.host || fields.port != existing.server.port;
        let now = self.inner.clock.now();

        self.inner
            .store
            .update_server(&input.server_id, &fields, endpoint_changed, now)
            .await
            .map_err(map_store_error)?;

        if let Some(password) = input.password {
            self.inner
                .secrets
                .save(&secret_entry(&input.server_id), password)
                .await
                .map_err(|error| {
                    tracing::warn!(%error, server = %input.server_id, "could not update Proxmox password");
                    ProxmoxError::CredentialsFailed
                })?;
            self.inner
                .store
                .set_password_set(&input.server_id, true, now)
                .await
                .map_err(map_store_error)?;
        }

        // Invalidate cached API client/ticket and close live VNC sessions on edit.
        self.drop_client(&input.server_id).await;
        self.inner
            .vnc
            .close_server(&input.server_id, "server_updated");

        self.reload_states().await?;
        self.find_server(&input.server_id)
            .ok_or(ProxmoxError::ServerNotFound)
    }

    /// Removes a Proxmox server, deletes its credential store entry, clears
    /// its cached ticket, and closes any active VNC sessions on it.
    pub(crate) async fn remove_server(&self, server_id: &str) -> Result<(), ProxmoxError> {
        let _guard = self.inner.mutations.lock().await;
        if self.find_server(server_id).is_none() {
            return Err(ProxmoxError::ServerNotFound);
        }

        self.inner.vnc.close_server(server_id, "server_removed");
        self.drop_client(server_id).await;

        self.inner
            .store
            .delete_server(server_id)
            .await
            .map_err(map_store_error)?;

        if let Err(error) = self.inner.secrets.delete(&secret_entry(server_id)).await {
            tracing::warn!(%error, server = %server_id, "could not delete Proxmox password from keyring");
        }

        self.reload_states().await
    }

    /// Inspects the server's presented leaf TLS certificate via a TLS-only
    /// handshake without sending any HTTP request or credentials.
    pub(crate) async fn inspect_certificate(
        &self,
        server_id: &str,
    ) -> Result<CertificateInfo, ProxmoxError> {
        let state = self
            .find_server(server_id)
            .ok_or(ProxmoxError::ServerNotFound)?;
        probe_certificate(
            &state.server.host,
            state.server.port,
            state.server.pinned_cert_sha256.as_deref(),
            self.inner.clock.now(),
            CERT_PROBE_TIMEOUT,
        )
        .await
    }

    /// Pins the server's leaf certificate ONLY if a live TLS inspection right
    /// now presents the exact SHA-256 fingerprint the owner approved.
    pub(crate) async fn pin_certificate(
        &self,
        input: PinCertificate,
    ) -> Result<ServerState, ProxmoxError> {
        let _guard = self.inner.mutations.lock().await;
        let state = self
            .find_server(&input.server_id)
            .ok_or(ProxmoxError::ServerNotFound)?;
        let expected =
            normalize_sha256_hex(&input.sha256).ok_or_else(|| ProxmoxError::InvalidRequest {
                detail: "invalid SHA-256 fingerprint".to_owned(),
            })?;

        let live = probe_certificate(
            &state.server.host,
            state.server.port,
            state.server.pinned_cert_sha256.as_deref(),
            self.inner.clock.now(),
            CERT_PROBE_TIMEOUT,
        )
        .await?;

        if live.sha256 != expected {
            return Err(ProxmoxError::PinMismatch);
        }

        if !live.valid_now {
            return Err(ProxmoxError::CertificateExpired);
        }

        let subject = if live.subject.is_empty() {
            state.server.host.clone()
        } else {
            live.subject
        };
        let now = self.inner.clock.now();
        self.inner
            .store
            .set_pinned_cert(&input.server_id, &expected, &subject, now)
            .await
            .map_err(map_store_error)?;

        self.drop_client(&input.server_id).await;
        self.inner
            .vnc
            .close_server(&input.server_id, "certificate_changed");
        self.reload_states().await?;
        self.find_server(&input.server_id)
            .ok_or(ProxmoxError::ServerNotFound)
    }

    /// Removes the pinned leaf certificate, returning the server to OS trust.
    pub(crate) async fn clear_certificate_pin(
        &self,
        server_id: &str,
    ) -> Result<ServerState, ProxmoxError> {
        let _guard = self.inner.mutations.lock().await;
        if self.find_server(server_id).is_none() {
            return Err(ProxmoxError::ServerNotFound);
        }

        let now = self.inner.clock.now();
        self.inner
            .store
            .clear_pinned_cert(server_id, now)
            .await
            .map_err(map_store_error)?;

        self.drop_client(server_id).await;
        self.inner
            .vnc
            .close_server(server_id, "certificate_changed");
        self.reload_states().await?;
        self.find_server(server_id)
            .ok_or(ProxmoxError::ServerNotFound)
    }

    /// Tests TLS trust + Proxmox login + QEMU VM visibility for Settings.
    pub(crate) async fn test_connection(
        &self,
        server_id: &str,
    ) -> Result<TestConnectionResult, ProxmoxError> {
        let state = self
            .find_server(server_id)
            .ok_or(ProxmoxError::ServerNotFound)?;
        let (api, password) = self.api_and_password(&state.server).await?;
        let vms = api.list_qemu_vms(&password).await?;
        Ok(TestConnectionResult {
            endpoint: state.server.endpoint(),
            visible_vm_count: u32::try_from(vms.len()).unwrap_or(u32::MAX),
            allowed_vm_count: u32::try_from(state.allowed_vms.len()).unwrap_or(u32::MAX),
        })
    }

    /// Queries all QEMU VMs visible to the server's Proxmox account for the
    /// Settings allowlist pre-configure dialog (never exposed to agents).
    pub(crate) async fn discover_server_vms(
        &self,
        server_id: &str,
    ) -> Result<Vec<DiscoverableVmView>, ProxmoxError> {
        let state = self
            .find_server(server_id)
            .ok_or(ProxmoxError::ServerNotFound)?;
        let allowed_set: BTreeSet<u32> = state.allowed_vms.iter().map(|vm| vm.vmid).collect();
        let (api, password) = self.api_and_password(&state.server).await?;
        let vms = api.list_qemu_vms(&password).await?;

        Ok(vms
            .into_iter()
            .map(|vm| DiscoverableVmView {
                allowed: allowed_set.contains(&vm.vmid),
                vmid: vm.vmid,
                node: vm.node,
                name: vm.name,
                status: vm.status,
                maxcpu: vm.maxcpu,
                maxmem_mib: vm
                    .maxmem
                    .and_then(|bytes| u32::try_from(bytes / (1024 * 1024)).ok()),
            })
            .collect())
    }

    /// Replaces the owner's explicit VM allowlist for a server and immediately
    /// closes any active VNC session for a VM that was unchecked.
    pub(crate) async fn set_allowed_vms(
        &self,
        input: SetAllowedVms,
    ) -> Result<ServerState, ProxmoxError> {
        let _guard = self.inner.mutations.lock().await;
        let existing = self
            .find_server(&input.server_id)
            .ok_or(ProxmoxError::ServerNotFound)?;
        let prev_ids: BTreeSet<u32> = existing.allowed_vms.iter().map(|vm| vm.vmid).collect();
        let next_ids: BTreeSet<u32> = input.vms.iter().map(|vm| vm.vmid).collect();

        let now = self.inner.clock.now();
        self.inner
            .store
            .replace_allowed_vms(&input.server_id, &input.vms, now)
            .await
            .map_err(map_store_error)?;

        // Close any active VNC session on VMs removed from the allowlist.
        for removed_vmid in prev_ids.difference(&next_ids) {
            self.inner
                .vnc
                .close_vm(&input.server_id, *removed_vmid, "vm_disallowed");
        }

        self.reload_states().await?;
        self.find_server(&input.server_id)
            .ok_or(ProxmoxError::ServerNotFound)
    }

    /// Fetches live status for explicitly allowed VMs on one or all servers.
    /// Unselected VMs are strictly filtered out in Rust.
    pub(crate) async fn list_allowed_vms_live(
        &self,
        server_filter: Option<&str>,
    ) -> Result<Vec<(ServerRecord, Vec<ProxmoxVmInfo>)>, ProxmoxError> {
        let targets: Vec<ServerState> = match server_filter {
            Some(name_or_id) => {
                let state = self
                    .find_server(name_or_id)
                    .ok_or(ProxmoxError::ServerNotFound)?;
                vec![state]
            }
            None => self.states(),
        };

        let mut results = Vec::with_capacity(targets.len());
        for state in targets {
            if state.allowed_vms.is_empty() {
                results.push((state.server, Vec::new()));
                continue;
            }

            let allowed_ids: BTreeSet<u32> = state.allowed_vms.iter().map(|vm| vm.vmid).collect();
            let (api, password) = self.api_and_password(&state.server).await?;
            let all_live = api.list_qemu_vms(&password).await?;
            let filtered: Vec<ProxmoxVmInfo> = all_live
                .into_iter()
                .filter(|vm| allowed_ids.contains(&vm.vmid))
                .collect();

            // Refresh cached node/name metadata for allowed VMs if changed.
            let mut changed_meta = false;
            for live_vm in &filtered {
                if let Some(stored) = state.allowed_vms.iter().find(|v| v.vmid == live_vm.vmid)
                    && (stored.node != live_vm.node || stored.name != live_vm.name)
                {
                    drop(
                        self.inner
                            .store
                            .update_allowed_vm_metadata(
                                &state.server.id,
                                live_vm.vmid,
                                &live_vm.node,
                                &live_vm.name,
                            )
                            .await,
                    );
                    changed_meta = true;
                }
            }
            if changed_meta {
                drop(self.reload_states().await);
            }

            results.push((state.server, filtered));
        }

        Ok(results)
    }

    /// Lists ISOs on `node` for `server` (caller must already have verified allowlist).
    pub(crate) async fn list_isos(
        &self,
        server: &ServerRecord,
        node: &str,
        storage_filter: Option<&str>,
    ) -> Result<IsoListResult, ProxmoxError> {
        let (api, password) = self.api_and_password(server).await?;
        api.list_isos(node, storage_filter, &password).await
    }

    /// Returns a server's cached REST API client and live password from the OS keyring.
    pub(crate) async fn api_and_password(
        &self,
        server: &ServerRecord,
    ) -> Result<(Arc<ProxmoxApiClient>, Secret), ProxmoxError> {
        if !server.password_set {
            return Err(ProxmoxError::PasswordMissing);
        }

        let password = self
            .inner
            .secrets
            .load(&secret_entry(&server.id))
            .await
            .map_err(|error| {
                tracing::warn!(%error, server = %server.id, "could not read Proxmox password");
                ProxmoxError::CredentialsFailed
            })?
            .ok_or(ProxmoxError::PasswordMissing)?;

        let client = {
            let mut map = lock(&self.inner.clients);
            if let Some(existing) = map.get(&server.id) {
                Arc::clone(existing)
            } else {
                let created = Arc::new(ProxmoxApiClient::new(server)?);
                map.insert(server.id.clone(), Arc::clone(&created));
                created
            }
        };

        Ok((client, password))
    }

    /// Owner opens (or attaches to) a live VNC session from the conversation UI.
    /// Newly opened owner sessions start in `Owner` mode; existing sessions keep
    /// their current control state until the owner clicks Take over / Resume.
    pub(crate) async fn open_vnc_session_for_owner(
        &self,
        target: VncSessionTarget,
    ) -> Result<VncSessionSummary, ProxmoxError> {
        let (server, vm) = self.resolve_allowed_vm(&target.server_id, target.vmid)?;
        let (api, password) = self.api_and_password(&server).await?;
        self.inner
            .vnc
            .ensure_session(
                target.conversation_id,
                &server,
                &vm.node,
                vm.vmid,
                &vm.name,
                &api,
                &password,
                VncControlOwner::Owner,
                false,
            )
            .await
    }

    /// Agent opens or reuses a live VNC session (starts in `Agent` mode; fails
    /// with `OwnerInControl` if the owner currently controls an existing session).
    pub(crate) async fn ensure_vnc_session_for_agent(
        &self,
        conversation_id: ConversationId,
        server: &ServerRecord,
        vm: &AllowedVmRecord,
    ) -> Result<SessionKey, ProxmoxError> {
        let (api, password) = self.api_and_password(server).await?;
        let _summary = self
            .inner
            .vnc
            .ensure_session(
                conversation_id.clone(),
                server,
                &vm.node,
                vm.vmid,
                &vm.name,
                &api,
                &password,
                VncControlOwner::Agent,
                true,
            )
            .await?;
        Ok(SessionKey {
            conversation_id,
            server_id: server.id.clone(),
            vmid: vm.vmid,
        })
    }

    /// Captures a screenshot for an agent VNC tool call.
    pub(crate) async fn agent_vnc_screenshot(
        &self,
        conversation_id: ConversationId,
        server: &ServerRecord,
        vm: &AllowedVmRecord,
        delay_ms: Option<u64>,
        wait_for_change_ms: Option<u64>,
    ) -> Result<VncScreenshot, ProxmoxError> {
        let key = self
            .ensure_vnc_session_for_agent(conversation_id, server, vm)
            .await?;
        self.inner
            .vnc
            .agent_screenshot(&key, delay_ms, wait_for_change_ms)
            .await
    }

    /// Sends pointer input for an agent VNC tool call.
    #[expect(
        clippy::too_many_arguments,
        reason = "agent pointer tool arguments passed through to VncManager"
    )]
    pub(crate) async fn agent_vnc_pointer(
        &self,
        conversation_id: ConversationId,
        server: &ServerRecord,
        vm: &AllowedVmRecord,
        action: &str,
        x: u16,
        y: u16,
        button: Option<&str>,
        scroll_delta: Option<i32>,
        delay_ms: Option<u64>,
        wait_for_change_ms: Option<u64>,
        take_screenshot: bool,
    ) -> Result<Option<VncScreenshot>, ProxmoxError> {
        let key = self
            .ensure_vnc_session_for_agent(conversation_id, server, vm)
            .await?;
        self.inner
            .vnc
            .agent_pointer(
                &key,
                action,
                x,
                y,
                button,
                scroll_delta,
                delay_ms,
                wait_for_change_ms,
                take_screenshot,
            )
            .await
    }

    /// Sends keyboard input for an agent VNC tool call.
    #[expect(
        clippy::too_many_arguments,
        reason = "agent keyboard tool arguments passed through to VncManager"
    )]
    pub(crate) async fn agent_vnc_keyboard(
        &self,
        conversation_id: ConversationId,
        server: &ServerRecord,
        vm: &AllowedVmRecord,
        strokes: &[KeyStroke],
        delay_ms: Option<u64>,
        wait_for_change_ms: Option<u64>,
        take_screenshot: bool,
    ) -> Result<Option<VncScreenshot>, ProxmoxError> {
        let key = self
            .ensure_vnc_session_for_agent(conversation_id, server, vm)
            .await?;
        self.inner
            .vnc
            .agent_keyboard(&key, strokes, delay_ms, wait_for_change_ms, take_screenshot)
            .await
    }

    /// Closes a live VNC session from an agent tool (refused if owner holds takeover).
    pub(crate) fn agent_vnc_close(
        &self,
        conversation_id: ConversationId,
        server_id: &str,
        vmid: u32,
    ) -> Result<(), ProxmoxError> {
        let key = SessionKey {
            conversation_id,
            server_id: server_id.to_owned(),
            vmid,
        };
        self.inner.vnc.agent_close(&key)
    }

    /// Lists active VNC sessions in one conversation.
    pub(crate) fn list_vnc_sessions(
        &self,
        conversation_id: &ConversationId,
    ) -> Vec<VncSessionSummary> {
        self.inner.vnc.list_for_conversation(conversation_id)
    }

    /// Subscribes to live PNG updates for an active VNC session.
    pub(crate) fn follow_vnc_session(
        &self,
        target: VncSessionTarget,
    ) -> Result<PluginStream<VncSessionUpdate>, ProxmoxError> {
        let key = SessionKey {
            conversation_id: target.conversation_id,
            server_id: target.server_id,
            vmid: target.vmid,
        };
        self.inner.vnc.follow_session(&key)
    }

    /// Switches live VNC control between `owner` and `agent`.
    pub(crate) fn set_vnc_control(
        &self,
        input: SetVncControl,
    ) -> Result<VncSessionSummary, ProxmoxError> {
        let key = SessionKey {
            conversation_id: input.conversation_id,
            server_id: input.server_id,
            vmid: input.vmid,
        };
        self.inner.vnc.set_control(&key, input.control)
    }

    /// Sends owner pointer input (requires `Owner` takeover mode).
    pub(crate) async fn owner_vnc_pointer(
        &self,
        input: VncPointerInput,
    ) -> Result<(), ProxmoxError> {
        let key = SessionKey {
            conversation_id: input.conversation_id,
            server_id: input.server_id,
            vmid: input.vmid,
        };
        self.inner
            .vnc
            .owner_pointer(&key, input.x, input.y, input.button_mask)
            .await
    }

    /// Sends owner keyboard input (requires `Owner` takeover mode).
    pub(crate) async fn owner_vnc_keyboard(
        &self,
        input: VncKeyboardInput,
    ) -> Result<(), ProxmoxError> {
        let key = SessionKey {
            conversation_id: input.conversation_id,
            server_id: input.server_id,
            vmid: input.vmid,
        };
        self.inner
            .vnc
            .owner_keyboard(&key, input.keysym, input.down)
            .await
    }

    /// Sends `Ctrl+Alt+Delete` for the owner (requires `Owner` takeover mode).
    pub(crate) async fn owner_vnc_ctrl_alt_del(
        &self,
        target: VncSessionTarget,
    ) -> Result<(), ProxmoxError> {
        let key = SessionKey {
            conversation_id: target.conversation_id,
            server_id: target.server_id,
            vmid: target.vmid,
        };
        self.inner.vnc.owner_ctrl_alt_del(&key).await
    }

    /// Closes a live VNC session on behalf of the owner.
    pub(crate) fn owner_vnc_close(&self, target: VncSessionTarget) {
        let key = SessionKey {
            conversation_id: target.conversation_id,
            server_id: target.server_id,
            vmid: target.vmid,
        };
        self.inner.vnc.owner_close(&key);
    }

    /// Releases all live VNC sessions and cached API tickets on plugin stop.
    pub(crate) async fn shutdown(&self) {
        self.inner.vnc.close_all();
        let clients: Vec<Arc<ProxmoxApiClient>> = {
            let mut map = lock(&self.inner.clients);
            map.drain().map(|(_, c)| c).collect()
        };
        for client in clients {
            client.clear_ticket().await;
        }
    }

    async fn drop_client(&self, server_id: &str) {
        let removed = lock(&self.inner.clients).remove(server_id);
        if let Some(client) = removed {
            client.clear_ticket().await;
        }
    }

    async fn reload_states(&self) -> Result<(), ProxmoxError> {
        let fresh = self.inner.store.load_all().await.map_err(map_store_error)?;
        let _previous = self.inner.states.send_replace(fresh);
        Ok(())
    }
}

fn map_store_error(error: StoreError) -> ProxmoxError {
    match error {
        StoreError::Constraint { code } if code == NAME_TAKEN => ProxmoxError::NameTaken,
        other => {
            tracing::error!(%other, error.code = other.code(), "Proxmox store failed");
            ProxmoxError::StorageFailed
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
