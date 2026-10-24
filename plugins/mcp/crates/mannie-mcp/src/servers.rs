//! The registered servers (plugins/mcp/docs/README.md): the owner's
//! commands, the connection that the plugin keeps to each server that is
//! on, and the lists that the channel, the tool source, the section, the
//! mentions and the tools read.
//!
//! A server's secrets live only in the credential store (`secrets.rs`).
//! Adding a server stores its row first, then its secrets, then counts them
//! in the row, so no secret is kept for a server that is not registered;
//! removing one deletes its secrets first for the same reason.
//!
//! The plugin connects to a server when it starts, when the owner adds,
//! changes, turns on or restarts one, and when an authorized call needs a
//! connection (ADR-0043, decision 7). One task, the runner (`runner.rs`),
//! owns each connection, and connects again after a connection ended, at
//! most [`MAX_RESTARTS`] times in [`RESTART_WINDOW`]. A runner's updates
//! carry its generation, so a runner that the owner replaced changes
//! nothing. The views of the settings come from `views.rs`.
//!
//! The owner's changes run one at a time. A listing writes the tools under
//! a lock of its own, which the owner's switches of tools share, so a switch
//! is never lost to a listing that read the tools before it.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use mannie_plugin_sdk::contracts::{Clock, Secret, UtcMillis};
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::services::local_servers::{
    LocalServerError, LocalServerSpec, LocalServersApi,
};
use mannie_plugin_sdk::services::secrets::{SecretsApi, SecretsError};
use mannie_plugin_sdk::services::store::{ModuleStoreApi, StoreError};
use reqwest::header::{HeaderName, HeaderValue};
use rmcp::model::{Resource, ResourceTemplate, Tool};
use rmcp::service::{Peer, RoleClient};
use tokio::sync::watch;
use tokio::task::{AbortHandle, JoinHandle};

use crate::client::{self, RemoteClient, ServerInfo, Target};
use crate::commands::{
    LogEvent, MAX_ENABLED_TOOLS, McpError, SecretValue, ServerFailure, ServerInput,
    ServerListUpdate, ServerStatus, ServerTransport, ServerView, ToolView, UpdateServer,
};
use crate::log::{Log, Redactions};
use crate::naming::suggested_prefix;
use crate::offered::{OfferedTool, offered_tools};
use crate::secrets::{Family, Vault, decode_values, encode_values, part_count};
use crate::store::{
    McpStore, NAME_TAKEN, OAuthState, PREFIX_TAKEN, ServerFields, ServerRecord, StoredEndpoint,
    ToolRecord,
};

mod runner;
mod views;

use runner::run;
use views::{tool_record, view};

/// How long a local server may take to start and answer: `npx` may first
/// download it.
const LOCAL_START_TIMEOUT: Duration = Duration::from_secs(120);

/// How long a remote server may take to answer.
const REMOTE_START_TIMEOUT: Duration = Duration::from_secs(30);

/// How often a server may end in [`RESTART_WINDOW`] before it shows as
/// failed.
pub(crate) const MAX_RESTARTS: usize = 5;

/// The window of [`MAX_RESTARTS`].
pub(crate) const RESTART_WINDOW: Duration = Duration::from_mins(5);

/// How long a stop waits for a runner to close its connection.
const RUNNER_STOP_WAIT: Duration = Duration::from_secs(6);

/// The registered servers. Cloning is cheap; clones share the servers.
#[derive(Debug, Clone)]
pub(crate) struct Servers {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    plugin: PluginId,
    store: McpStore,
    vault: Vault,
    local: Arc<dyn LocalServersApi>,
    http: reqwest::Client,
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
    /// Counts the changes of the state, which the channel, the tool source
    /// and the waiting calls follow.
    revision: watch::Sender<u64>,
    /// Held by each of the owner's changes, so they run one at a time.
    changes: tokio::sync::Mutex<()>,
    /// Held by each write of the tools.
    tools: tokio::sync::Mutex<()>,
    /// The calls of the servers' tools in progress.
    calls: AtomicU32,
    /// The sign-ins that wait for the browser, by server.
    sign_ins: Mutex<HashMap<String, AbortHandle>>,
}

#[derive(Debug, Default)]
struct State {
    /// The servers, by name, as the store orders them.
    entries: Vec<Entry>,
    /// Whether the plugin stops, so no runner starts.
    stopping: bool,
}

#[derive(Debug)]
struct Entry {
    record: ServerRecord,
    tools: Vec<ToolRecord>,
    live: Live,
}

impl Entry {
    fn new(record: ServerRecord, tools: Vec<ToolRecord>) -> Self {
        Self {
            record,
            tools,
            live: Live::default(),
        }
    }
}

/// What lives of a server while the plugin runs.
#[derive(Debug)]
struct Live {
    status: ServerStatus,
    failure: Option<ServerFailure>,
    info: Option<ServerInfo>,
    peer: Option<Peer<RoleClient>>,
    redactions: Arc<Redactions>,
    resources: Vec<Resource>,
    templates: Vec<ResourceTemplate>,
    runner: Option<Runner>,
    /// The generation of the current runner.
    generation: u64,
    log: Log,
    /// When the server's connections ended, within the window of restarts.
    restarts: VecDeque<Instant>,
}

impl Default for Live {
    fn default() -> Self {
        Self {
            status: ServerStatus::Off,
            failure: None,
            info: None,
            peer: None,
            redactions: Arc::new(Redactions::default()),
            resources: Vec::new(),
            templates: Vec::new(),
            runner: None,
            generation: 0,
            log: Log::default(),
            restarts: VecDeque::new(),
        }
    }
}

impl Live {
    /// Forgets the connection and what it listed.
    fn disconnect(&mut self) {
        self.peer = None;
        self.info = None;
        self.resources.clear();
        self.templates.clear();
    }
}

/// The task that owns a server's connection.
#[derive(Debug)]
struct Runner {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

/// A connection that a call uses.
#[derive(Debug, Clone)]
pub(crate) struct LiveConnection {
    pub(crate) peer: Peer<RoleClient>,
    pub(crate) redactions: Arc<Redactions>,
    pub(crate) record: ServerRecord,
}

/// Why a call has no connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Unavailable {
    /// The owner removed the server.
    Removed,
    /// The owner turned the server off, or the plugin stops.
    Off,
    /// The server failed.
    Failed(Option<ServerFailure>),
    /// The server asks for a sign-in.
    SignInNeeded,
    /// The server asks for more permissions.
    AccessNeeded,
    /// The server did not connect in time.
    TimedOut,
}

/// A server of the section and the mentions: what its connection listed.
#[derive(Debug, Clone)]
pub(crate) struct ConnectedServer {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) prefix: String,
    pub(crate) instructions: Option<String>,
    pub(crate) resources: Vec<Resource>,
    pub(crate) templates: Vec<ResourceTemplate>,
}

/// A call of a server's tool in progress, which `work` counts until it is
/// dropped.
#[derive(Debug)]
pub(crate) struct CallGuard {
    inner: Arc<Inner>,
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        self.inner.calls.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Servers {
    /// The servers that the store keeps, without connections yet.
    pub(crate) async fn load(
        plugin: PluginId,
        store: Arc<dyn ModuleStoreApi>,
        secrets: Arc<dyn SecretsApi>,
        local: Arc<dyn LocalServersApi>,
        http: reqwest::Client,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, StoreError> {
        let store = McpStore::new(store);
        let records = store.servers().await?;
        let mut tools = store.tools().await?;
        let entries = records
            .into_iter()
            .map(|record| {
                let own: Vec<ToolRecord> = tools
                    .extract_if(.., |(server_id, _tool)| *server_id == record.id)
                    .map(|(_server_id, tool)| tool)
                    .collect();
                Entry::new(record, own)
            })
            .collect();
        Ok(Self {
            inner: Arc::new(Inner {
                plugin,
                store,
                vault: Vault::new(secrets),
                local,
                http,
                clock,
                state: Mutex::new(State {
                    entries,
                    stopping: false,
                }),
                revision: watch::Sender::new(0),
                changes: tokio::sync::Mutex::new(()),
                tools: tokio::sync::Mutex::new(()),
                calls: AtomicU32::new(0),
                sign_ins: Mutex::new(HashMap::new()),
            }),
        })
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn now(&self) -> UtcMillis {
        self.inner.clock.now()
    }

    /// Tells everyone who follows the servers that they changed.
    fn notify(&self) {
        self.inner.revision.send_modify(|revision| *revision += 1);
    }

    /// The changes of the servers from now on, until the plugin stops.
    pub(crate) fn follow(&self) -> watch::Receiver<u64> {
        self.inner.revision.subscribe()
    }

    /// Connects to every server that is on, as the plugin starts.
    pub(crate) fn start_all(&self) {
        let ids: Vec<String> = self
            .state()
            .entries
            .iter()
            .filter(|entry| entry.record.enabled)
            .map(|entry| entry.record.id.clone())
            .collect();
        for id in ids {
            self.start_runner(&id);
        }
        self.notify();
    }

    /// Stops every connection, as the plugin stops.
    pub(crate) async fn stop_all(&self) {
        let runners: Vec<Runner> = {
            let mut state = self.state();
            state.stopping = true;
            state
                .entries
                .iter_mut()
                .filter_map(|entry| {
                    entry.live.status = ServerStatus::Off;
                    entry.live.disconnect();
                    entry.live.runner.take()
                })
                .collect()
        };
        let sign_ins: Vec<AbortHandle> = self
            .inner
            .sign_ins
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain()
            .map(|(_server_id, sign_in)| sign_in)
            .collect();
        for sign_in in sign_ins {
            sign_in.abort();
        }
        for runner in &runners {
            runner.stop.send_replace(true);
        }
        for runner in runners {
            wait_for(runner.task).await;
        }
        self.notify();
    }

    /// The work in progress: the servers that are connected and the calls
    /// of their tools.
    pub(crate) fn work(&self) -> (u32, u32) {
        let connected = self
            .state()
            .entries
            .iter()
            .filter(|entry| entry.live.status == ServerStatus::Connected)
            .count();
        (
            u32::try_from(connected).unwrap_or(u32::MAX),
            self.inner.calls.load(Ordering::SeqCst),
        )
    }

    /// Counts a call of a server's tool until the guard is dropped.
    pub(crate) fn call_started(&self) -> CallGuard {
        self.inner.calls.fetch_add(1, Ordering::SeqCst);
        CallGuard {
            inner: Arc::clone(&self.inner),
        }
    }

    // ----- What the plugin's parts read -----

    /// The servers as the channel carries them.
    pub(crate) fn list_update(&self) -> ServerListUpdate {
        let state = self.state();
        let servers = state
            .entries
            .iter()
            .map(|entry| view(&self.inner.plugin, entry))
            .collect();
        ServerListUpdate::Servers {
            servers,
            enabled_tools: enabled_tool_count(&state.entries),
        }
    }

    /// The view of one server.
    fn view_of(&self, server_id: &str) -> Result<ServerView, McpError> {
        self.state()
            .entries
            .iter()
            .find(|entry| entry.record.id == server_id)
            .map(|entry| view(&self.inner.plugin, entry))
            .ok_or(McpError::ServerNotFound)
    }

    /// The tools that agents are offered: the tools that the owner turned
    /// on, of the servers that are on and connected or connecting, in the
    /// order of the servers' names and lists.
    pub(crate) fn offered(&self) -> Vec<OfferedTool> {
        let state = self.state();
        state
            .entries
            .iter()
            .filter(|entry| {
                let is_live = matches!(
                    entry.live.status,
                    ServerStatus::Connected | ServerStatus::Starting
                );
                entry.record.enabled && is_live
            })
            .flat_map(|entry| {
                offered_tools(&self.inner.plugin, &entry.record, &entry.tools)
                    .into_iter()
                    .filter_map(|(_tool, offered)| offered.ok())
                    .filter(|offered| offered.enabled)
            })
            .take(MAX_ENABLED_TOOLS as usize)
            .collect()
    }

    /// The connected servers, for the section and the mentions.
    pub(crate) fn connected(&self) -> Vec<ConnectedServer> {
        self.state()
            .entries
            .iter()
            .filter(|entry| entry.record.enabled && entry.live.status == ServerStatus::Connected)
            .map(|entry| ConnectedServer {
                id: entry.record.id.clone(),
                name: entry.record.name.clone(),
                prefix: entry.record.prefix.clone(),
                instructions: entry
                    .live
                    .info
                    .as_ref()
                    .filter(|_info| entry.record.instructions)
                    .and_then(|info| info.instructions.as_deref())
                    .map(|text| entry.live.redactions.apply(text)),
                resources: entry.live.resources.clone(),
                templates: entry.live.templates.clone(),
            })
            .collect()
    }

    /// A server and one of its tools, as the store keeps them, for a call's
    /// checks.
    pub(crate) fn tool_of(
        &self,
        server_id: &str,
        tool: &str,
    ) -> Option<(ServerRecord, ToolRecord)> {
        let state = self.state();
        let entry = state
            .entries
            .iter()
            .find(|entry| entry.record.id == server_id)?;
        let tool = entry
            .tools
            .iter()
            .find(|candidate| candidate.name == tool)?;
        Some((entry.record.clone(), tool.clone()))
    }

    /// A server, as the store keeps it.
    pub(crate) fn record(&self, server_id: &str) -> Option<ServerRecord> {
        self.state()
            .entries
            .iter()
            .find(|entry| entry.record.id == server_id)
            .map(|entry| entry.record.clone())
    }

    /// The server whose prefix is `prefix`.
    pub(crate) fn by_prefix(&self, prefix: &str) -> Option<ServerRecord> {
        self.state()
            .entries
            .iter()
            .find(|entry| entry.record.prefix == prefix)
            .map(|entry| entry.record.clone())
    }

    /// The connection of a server for a call, once it connected. A server
    /// that is on and connecting is waited for; one that is on without a
    /// runner is connected now (ADR-0043, decision 7).
    pub(crate) async fn connection(&self, server_id: &str) -> Result<LiveConnection, Unavailable> {
        let mut revision = self.follow();
        let timeout = match self.record(server_id).map(|record| record.endpoint) {
            Some(StoredEndpoint::Program { .. }) => LOCAL_START_TIMEOUT,
            Some(StoredEndpoint::Url { .. }) => REMOTE_START_TIMEOUT,
            None => return Err(Unavailable::Removed),
        };
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match self.current_connection(server_id) {
                Ok(Some(connection)) => return Ok(connection),
                Ok(None) => {}
                Err(unavailable) => return Err(unavailable),
            }
            match tokio::time::timeout_at(deadline, revision.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_closed)) => return Err(Unavailable::Off),
                Err(_elapsed) => return Err(Unavailable::TimedOut),
            }
        }
    }

    /// The connection of a server if it has one, `None` while it connects.
    fn current_connection(&self, server_id: &str) -> Result<Option<LiveConnection>, Unavailable> {
        let needs_runner = {
            let state = self.state();
            if state.stopping {
                return Err(Unavailable::Off);
            }
            let Some(entry) = state
                .entries
                .iter()
                .find(|entry| entry.record.id == server_id)
            else {
                return Err(Unavailable::Removed);
            };
            if !entry.record.enabled {
                return Err(Unavailable::Off);
            }
            match entry.live.status {
                ServerStatus::Connected => {
                    if let Some(peer) = &entry.live.peer {
                        return Ok(Some(LiveConnection {
                            peer: peer.clone(),
                            redactions: Arc::clone(&entry.live.redactions),
                            record: entry.record.clone(),
                        }));
                    }
                    false
                }
                ServerStatus::Starting => false,
                ServerStatus::SignInNeeded => return Err(Unavailable::SignInNeeded),
                ServerStatus::AccessNeeded => return Err(Unavailable::AccessNeeded),
                ServerStatus::Failed => {
                    return Err(Unavailable::Failed(entry.live.failure.clone()));
                }
                ServerStatus::Off => entry.live.runner.is_none(),
            }
        };
        if needs_runner {
            self.start_runner(server_id);
            self.notify();
        }
        Ok(None)
    }

    /// A server's log.
    pub(crate) fn log(&self, server_id: &str) -> Result<crate::commands::ServerLog, McpError> {
        let state = self.state();
        let entry = state
            .entries
            .iter()
            .find(|entry| entry.record.id == server_id)
            .ok_or(McpError::ServerNotFound)?;
        let (entries, cut) = entry.live.log.entries();
        Ok(crate::commands::ServerLog { entries, cut })
    }

    /// The tools that a server listed last, as Show tools lists them.
    pub(crate) fn tools(&self, server_id: &str) -> Result<Vec<ToolView>, McpError> {
        let state = self.state();
        let entry = state
            .entries
            .iter()
            .find(|entry| entry.record.id == server_id)
            .ok_or(McpError::ServerNotFound)?;
        Ok(
            offered_tools(&self.inner.plugin, &entry.record, &entry.tools)
                .into_iter()
                .map(|(tool, offered)| {
                    let (model_name, problem) = match offered {
                        Ok(offered) => (offered.spec.name, None),
                        Err((model_name, problem)) => (model_name, Some(problem)),
                    };
                    ToolView {
                        name: tool.name.clone(),
                        model_name,
                        title: tool.title.clone(),
                        description: tool.description.clone(),
                        hints: tool.hints.clone(),
                        enabled: tool.enabled,
                        new: tool.new,
                        problem,
                    }
                })
                .collect(),
        )
    }

    // ----- The owner's changes -----

    /// Registers a server, keeps its secrets and connects to it.
    pub(crate) async fn add(&self, input: ServerInput) -> Result<ServerView, McpError> {
        let _change = self.inner.changes.lock().await;
        let id = self.insert(input, true).await?;
        self.reload().await?;
        self.start_runner(&id);
        self.notify();
        tracing::info!(server = %id, "the owner registered an MCP server");
        self.view_of(&id)
    }

    /// Stores a new server with its secrets, on or off, and returns its ID.
    /// The caller holds the lock of changes.
    async fn insert(&self, input: ServerInput, enabled: bool) -> Result<String, McpError> {
        let id = uuid::Uuid::now_v7().to_string();
        let taken: Vec<String> = self
            .state()
            .entries
            .iter()
            .map(|entry| entry.record.prefix.clone())
            .collect();
        let fields = fields(&input, &taken);
        self.inner
            .store
            .insert(&id, &fields, enabled, self.now())
            .await
            .map_err(taken_or_storage)?;
        let saved = self.save_new_secrets(&id, &input).await;
        if let Err(error) = saved {
            // No server stays registered without the secrets that the owner
            // gave, so adding it again works.
            self.forget_secrets(&id, &input).await;
            if let Err(store_error) = self.inner.store.delete(&id).await {
                tracing::warn!(server = %id, error.code = store_error.code(), "could not forget a server whose secrets were not kept");
            }
            return Err(error);
        }
        Ok(id)
    }

    /// Keeps the secrets of a new server and counts them in its row.
    async fn save_new_secrets(&self, id: &str, input: &ServerInput) -> Result<(), McpError> {
        let family = secrets_family(input.transport);
        let (values, missing) = given_secrets(&input.secrets, &[]);
        if !values.is_empty() || !missing.is_empty() {
            let parts = self
                .inner
                .vault
                .save(family, id, &encode_values(&values))
                .await
                .map_err(secrets_failed)?;
            self.inner
                .store
                .set_secrets(id, parts, &missing, self.now())
                .await
                .map_err(|error| storage_failed(&error))?;
        }
        if let Some(client_secret) = &input.client_secret {
            let state = OAuthBundle::with_client_secret(client_secret.clone());
            self.save_oauth(id, &state, 0).await?;
        }
        Ok(())
    }

    /// Deletes what a failed add may have stored of a server's secrets: the
    /// parts that their texts take.
    async fn forget_secrets(&self, id: &str, input: &ServerInput) {
        let (values, _missing) = given_secrets(&input.secrets, &[]);
        let secret_parts = part_count(&encode_values(&values));
        let oauth_parts = input.client_secret.clone().map_or(0, |client_secret| {
            part_count(&OAuthBundle::with_client_secret(client_secret).encode())
        });
        let family = secrets_family(input.transport);
        for (family, count) in [(family, secret_parts), (Family::OAuth, oauth_parts)] {
            if let Err(error) = self.inner.vault.delete_from(family, id, 0, count).await {
                tracing::warn!(server = %id, %error, "could not delete the secrets of a server that was not added");
            }
        }
    }

    /// Changes a server and its secrets, and connects to it again when it
    /// is on.
    pub(crate) async fn update(&self, input: UpdateServer) -> Result<ServerView, McpError> {
        let _change = self.inner.changes.lock().await;
        let current = self
            .record(&input.server_id)
            .ok_or(McpError::ServerNotFound)?;
        let server = input.server;
        if transport_of(&current.endpoint) != server.transport {
            return Err(McpError::TransportChanged);
        }
        let taken: Vec<String> = self
            .state()
            .entries
            .iter()
            .filter(|entry| entry.record.id != current.id)
            .map(|entry| entry.record.prefix.clone())
            .collect();
        let mut fields = fields(&server, &taken);
        if server.prefix.is_empty() {
            // A server keeps its prefix, so its tools keep their names and
            // the model's earlier calls still read right.
            fields.prefix.clone_from(&current.prefix);
        }
        let endpoint_changed = fields.endpoint != current.endpoint;
        let stored = self.stored_secrets(&current).await?;
        // A secret that the owner keeps without a stored value stays
        // missing, and the server fails to connect until the owner enters it.
        let (values, missing) = given_secrets(&server.secrets, &stored);
        let runner = self.take_runner(&current.id);
        stop_runner(runner).await;
        self.inner
            .store
            .update(&current.id, &fields, endpoint_changed, self.now())
            .await
            .map_err(|error| match error {
                StoreError::Expectation { .. } => McpError::ServerNotFound,
                other => taken_or_storage(other),
            })?;
        self.replace_secrets(&current, server.transport, &values, &missing)
            .await?;
        let signs_in_elsewhere =
            endpoint_changed || !fields.oauth || fields.client_id != current.client_id;
        self.update_oauth(&current, &server, signs_in_elsewhere)
            .await?;
        self.reload().await?;
        {
            let mut state = self.state();
            if let Some(entry) = find_mut(&mut state.entries, &current.id) {
                entry.live.restarts.clear();
                entry.live.failure = None;
                entry.live.status = ServerStatus::Off;
                entry.live.disconnect();
            }
        }
        let is_on = self
            .record(&current.id)
            .is_some_and(|record| record.enabled);
        if is_on {
            self.start_runner(&current.id);
        }
        self.notify();
        tracing::info!(server = %current.id, endpoint_changed, "the owner changed an MCP server");
        self.view_of(&current.id)
    }

    /// The stored secret values of a server, by name.
    async fn stored_secrets(
        &self,
        record: &ServerRecord,
    ) -> Result<Vec<(String, Secret)>, McpError> {
        if record.secret_parts == 0 {
            return Ok(Vec::new());
        }
        let family = secrets_family(transport_of(&record.endpoint));
        let text = self
            .inner
            .vault
            .load(family, &record.id, record.secret_parts)
            .await
            .map_err(secrets_failed)?;
        Ok(text.as_ref().and_then(decode_values).unwrap_or_default())
    }

    /// Writes a server's new secret values, counts them in its row, then
    /// deletes the parts past them that the old values had.
    async fn replace_secrets(
        &self,
        current: &ServerRecord,
        transport: ServerTransport,
        values: &[(String, Secret)],
        missing: &[String],
    ) -> Result<(), McpError> {
        let family = secrets_family(transport);
        let parts = if values.is_empty() {
            0
        } else {
            self.inner
                .vault
                .save(family, &current.id, &encode_values(values))
                .await
                .map_err(secrets_failed)?
        };
        self.inner
            .store
            .set_secrets(&current.id, parts, missing, self.now())
            .await
            .map_err(|error| storage_failed(&error))?;
        self.inner
            .vault
            .delete_from(family, &current.id, parts, current.secret_parts)
            .await
            .map_err(secrets_failed)
    }

    /// Keeps a new client secret, forgets the old one when the owner asks,
    /// and forgets the tokens when the server signs in elsewhere now.
    async fn update_oauth(
        &self,
        current: &ServerRecord,
        server: &ServerInput,
        signs_in_elsewhere: bool,
    ) -> Result<(), McpError> {
        let changes_client_secret = server.client_secret.is_some() || server.forget_client_secret;
        if !changes_client_secret && !signs_in_elsewhere {
            return Ok(());
        }
        let mut bundle = self.oauth_bundle(current).await?.unwrap_or_default();
        if let Some(client_secret) = &server.client_secret {
            bundle.client_secret = Some(client_secret.clone());
        } else if server.forget_client_secret || !server.oauth {
            bundle.client_secret = None;
        }
        if signs_in_elsewhere {
            bundle.credentials = None;
        }
        self.save_oauth(&current.id, &bundle, current.oauth_parts)
            .await
    }

    /// Deletes a server's secrets, then forgets the server.
    pub(crate) async fn remove(&self, server_id: &str) -> Result<(), McpError> {
        let _change = self.inner.changes.lock().await;
        let current = self.record(server_id).ok_or(McpError::ServerNotFound)?;
        let runner = self.take_runner(&current.id);
        stop_runner(runner).await;
        let family = secrets_family(transport_of(&current.endpoint));
        self.inner
            .vault
            .delete_from(family, &current.id, 0, current.secret_parts)
            .await
            .map_err(secrets_failed)?;
        self.inner
            .vault
            .delete_from(Family::OAuth, &current.id, 0, current.oauth_parts)
            .await
            .map_err(secrets_failed)?;
        self.inner
            .store
            .delete(&current.id)
            .await
            .map_err(|error| storage_failed(&error))?;
        self.state()
            .entries
            .retain(|entry| entry.record.id != current.id);
        self.notify();
        tracing::info!(server = %current.id, "the owner removed an MCP server");
        Ok(())
    }

    /// Turns a server on, which connects to it, or off.
    pub(crate) async fn set_enabled(
        &self,
        server_id: &str,
        enabled: bool,
    ) -> Result<ServerView, McpError> {
        let _change = self.inner.changes.lock().await;
        let current = self.record(server_id).ok_or(McpError::ServerNotFound)?;
        if current.enabled == enabled {
            return self.view_of(&current.id);
        }
        if !enabled {
            let runner = self.take_runner(&current.id);
            stop_runner(runner).await;
        }
        self.inner
            .store
            .set_enabled(&current.id, enabled, self.now())
            .await
            .map_err(|error| match error {
                StoreError::Expectation { .. } => McpError::ServerNotFound,
                other => storage_failed(&other),
            })?;
        {
            let mut state = self.state();
            if let Some(entry) = find_mut(&mut state.entries, &current.id) {
                entry.record.enabled = enabled;
                entry.live.status = ServerStatus::Off;
                entry.live.failure = None;
                entry.live.restarts.clear();
                entry.live.disconnect();
            }
        }
        if enabled {
            self.start_runner(&current.id);
        }
        self.notify();
        tracing::info!(server = %current.id, enabled, "the owner turned an MCP server on or off");
        self.view_of(&current.id)
    }

    /// Connects to a server that is on again, also after it failed.
    pub(crate) async fn restart(&self, server_id: &str) -> Result<(), McpError> {
        let _change = self.inner.changes.lock().await;
        let current = self.record(server_id).ok_or(McpError::ServerNotFound)?;
        let runner = self.take_runner(&current.id);
        stop_runner(runner).await;
        {
            let mut state = self.state();
            if let Some(entry) = find_mut(&mut state.entries, &current.id) {
                entry.live.status = ServerStatus::Off;
                entry.live.failure = None;
                entry.live.restarts.clear();
                entry.live.disconnect();
            }
        }
        if current.enabled {
            self.start_runner(&current.id);
        }
        self.notify();
        Ok(())
    }

    /// Lets agents use a tool or not, within [`MAX_ENABLED_TOOLS`].
    pub(crate) async fn set_tool_enabled(
        &self,
        server_id: &str,
        tool: &str,
        enabled: bool,
    ) -> Result<(), McpError> {
        let _tools = self.inner.tools.lock().await;
        {
            let state = self.state();
            let entry = state
                .entries
                .iter()
                .find(|entry| entry.record.id == server_id)
                .ok_or(McpError::ServerNotFound)?;
            let current = entry
                .tools
                .iter()
                .find(|candidate| candidate.name == tool)
                .ok_or(McpError::ToolNotFound)?;
            let turns_on = enabled && !current.enabled;
            if turns_on && enabled_tool_count(&state.entries) >= MAX_ENABLED_TOOLS {
                return Err(McpError::TooManyTools {
                    limit: MAX_ENABLED_TOOLS,
                });
            }
        }
        let found = self
            .inner
            .store
            .set_tool_enabled(server_id, tool, enabled)
            .await
            .map_err(|error| storage_failed(&error))?;
        if !found {
            return Err(McpError::ToolNotFound);
        }
        {
            let mut state = self.state();
            if let Some(entry) = find_mut(&mut state.entries, server_id)
                && let Some(record) = entry
                    .tools
                    .iter_mut()
                    .find(|candidate| candidate.name == tool)
            {
                record.enabled = enabled;
                record.new = false;
            }
        }
        self.notify();
        Ok(())
    }

    /// Clears the marks of a server's new tools.
    pub(crate) async fn acknowledge_tools(&self, server_id: &str) -> Result<(), McpError> {
        let _tools = self.inner.tools.lock().await;
        if self.record(server_id).is_none() {
            return Err(McpError::ServerNotFound);
        }
        self.inner
            .store
            .acknowledge_tools(server_id)
            .await
            .map_err(|error| storage_failed(&error))?;
        {
            let mut state = self.state();
            if let Some(entry) = find_mut(&mut state.entries, server_id) {
                for tool in &mut entry.tools {
                    tool.new = false;
                }
            }
        }
        self.notify();
        Ok(())
    }

    /// Where the core would find a program.
    pub(crate) async fn locate(
        &self,
        program: &str,
    ) -> Result<crate::commands::LocatedProgramView, McpError> {
        let located = self
            .inner
            .local
            .locate(program)
            .await
            .map_err(|error| match error {
                LocalServerError::ProgramNotFound { .. } => McpError::ProgramNotFound,
                _other => McpError::ProgramRefused,
            })?;
        Ok(crate::commands::LocatedProgramView {
            path: located.path.display().to_string(),
            in_project: located.in_project,
        })
    }

    /// The names of the servers, for the preview of a configuration.
    pub(crate) fn names(&self) -> Vec<String> {
        self.state()
            .entries
            .iter()
            .map(|entry| entry.record.name.clone())
            .collect()
    }

    /// Adds a server of a pasted configuration, on unless its secrets are
    /// missing or the configuration turns it off.
    pub(crate) async fn import(
        &self,
        input: ServerInput,
        enabled: bool,
    ) -> Result<ServerView, McpError> {
        let _change = self.inner.changes.lock().await;
        let id = self.insert(input, enabled).await?;
        self.reload().await?;
        if enabled {
            self.start_runner(&id);
        }
        self.notify();
        tracing::info!(server = %id, enabled, "the owner imported an MCP server");
        self.view_of(&id)
    }

    /// Reads the servers again, keeping what lives of each.
    async fn reload(&self) -> Result<(), McpError> {
        let records = self
            .inner
            .store
            .servers()
            .await
            .map_err(|error| storage_failed(&error))?;
        let mut tools = self
            .inner
            .store
            .tools()
            .await
            .map_err(|error| storage_failed(&error))?;
        let mut state = self.state();
        let mut old = std::mem::take(&mut state.entries);
        state.entries = records
            .into_iter()
            .map(|record| {
                let own: Vec<ToolRecord> = tools
                    .extract_if(.., |(server_id, _tool)| *server_id == record.id)
                    .map(|(_server_id, tool)| tool)
                    .collect();
                let live = old
                    .iter()
                    .position(|entry| entry.record.id == record.id)
                    .map(|index| old.swap_remove(index).live)
                    .unwrap_or_default();
                Entry {
                    record,
                    tools: own,
                    live,
                }
            })
            .collect();
        Ok(())
    }

    // ----- OAuth state -----

    /// A server's OAuth state as the credential store keeps it.
    pub(crate) async fn oauth_bundle(
        &self,
        record: &ServerRecord,
    ) -> Result<Option<OAuthBundle>, McpError> {
        if record.oauth_parts == 0 {
            return Ok(None);
        }
        let text = self
            .inner
            .vault
            .load(Family::OAuth, &record.id, record.oauth_parts)
            .await
            .map_err(secrets_failed)?;
        Ok(text.as_ref().and_then(OAuthBundle::decode))
    }

    /// Keeps a server's OAuth state, counts it in its row, then deletes the
    /// parts past it that the old state had.
    pub(crate) async fn save_oauth(
        &self,
        server_id: &str,
        bundle: &OAuthBundle,
        old_parts: u32,
    ) -> Result<(), McpError> {
        let parts = if bundle.is_empty() {
            0
        } else {
            self.inner
                .vault
                .save(Family::OAuth, server_id, &bundle.encode())
                .await
                .map_err(secrets_failed)?
        };
        let state = OAuthState {
            parts,
            client_secret_set: bundle.client_secret.is_some(),
            signed_in: bundle.credentials.is_some(),
        };
        self.inner
            .store
            .set_oauth_state(server_id, state, self.now())
            .await
            .map_err(|error| storage_failed(&error))?;
        self.inner
            .vault
            .delete_from(Family::OAuth, server_id, parts, old_parts)
            .await
            .map_err(secrets_failed)?;
        let mut state_guard = self.state();
        if let Some(entry) = find_mut(&mut state_guard.entries, server_id) {
            entry.record.oauth_parts = parts;
            entry.record.client_secret_set = state.client_secret_set;
            entry.record.signed_in = state.signed_in;
        }
        drop(state_guard);
        self.notify();
        Ok(())
    }

    /// The plugin's HTTP client.
    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.inner.http
    }

    /// Records an event of a server's sign-in in its log.
    pub(crate) fn log_event(&self, server_id: &str, event: LogEvent) {
        let now = self.now();
        let mut state = self.state();
        if let Some(entry) = find_mut(&mut state.entries, server_id) {
            entry.live.log.event(now, event);
        }
    }

    /// Keeps the task of a sign-in that waits for the browser, and ends the
    /// one before it for the same server.
    pub(crate) fn replace_sign_in(&self, server_id: &str, sign_in: AbortHandle) {
        let earlier = self
            .inner
            .sign_ins
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(server_id.to_owned(), sign_in);
        if let Some(earlier) = earlier {
            earlier.abort();
        }
    }

    /// Forgets a server's OAuth tokens and registration, keeps its client
    /// secret, and stops its connection, which then needs a sign-in.
    pub(crate) async fn sign_out(&self, server_id: &str) -> Result<ServerView, McpError> {
        let _change = self.inner.changes.lock().await;
        let current = self.record(server_id).ok_or(McpError::ServerNotFound)?;
        if !current.oauth {
            return Err(McpError::NoSignIn);
        }
        let runner = self.take_runner(&current.id);
        stop_runner(runner).await;
        let mut bundle = self.oauth_bundle(&current).await?.unwrap_or_default();
        bundle.credentials = None;
        self.save_oauth(&current.id, &bundle, current.oauth_parts)
            .await?;
        {
            let mut state = self.state();
            if let Some(entry) = find_mut(&mut state.entries, &current.id) {
                entry.live.status = ServerStatus::Off;
                entry.live.failure = None;
                entry.live.disconnect();
            }
        }
        self.log_event(&current.id, LogEvent::SignedOut);
        if current.enabled {
            self.start_runner(&current.id);
        }
        self.notify();
        tracing::info!(server = %current.id, "the owner signed out of an MCP server");
        self.view_of(&current.id)
    }

    // ----- Runners -----

    /// Starts a runner for a server that has none.
    fn start_runner(&self, server_id: &str) {
        let mut state = self.state();
        if state.stopping {
            return;
        }
        let Some(entry) = find_mut(&mut state.entries, server_id) else {
            return;
        };
        if entry.live.runner.is_some() || !entry.record.enabled {
            return;
        }
        entry.live.generation += 1;
        let generation = entry.live.generation;
        entry.live.status = ServerStatus::Starting;
        entry.live.failure = None;
        let (stop, stopped) = watch::channel(false);
        let task = tokio::spawn(run(self.clone(), server_id.to_owned(), generation, stopped));
        entry.live.runner = Some(Runner { stop, task });
    }

    /// Takes a server's runner, to stop it.
    fn take_runner(&self, server_id: &str) -> Option<Runner> {
        let mut state = self.state();
        let entry = find_mut(&mut state.entries, server_id)?;
        // A runner that ends now changes nothing more.
        entry.live.generation += 1;
        entry.live.runner.take()
    }

    /// Changes what lives of a server, when `generation` is its runner's.
    fn update_live(
        &self,
        server_id: &str,
        generation: u64,
        change: impl FnOnce(&mut Entry, UtcMillis),
    ) -> bool {
        let now = self.now();
        let changed = {
            let mut state = self.state();
            match find_mut(&mut state.entries, server_id) {
                Some(entry) if entry.live.generation == generation => {
                    change(entry, now);
                    true
                }
                _stale => false,
            }
        };
        if changed {
            self.notify();
        }
        changed
    }

    /// The target of a server's connection, from its row and its secrets.
    async fn target(
        &self,
        server_id: &str,
        generation: u64,
    ) -> Result<(Target, ServerRecord, Arc<Redactions>), Prepared> {
        let record = self.record(server_id).ok_or(Prepared::Removed)?;
        if !record.missing_secrets.is_empty() {
            return Err(Prepared::Failed(ServerFailure::SecretMissing));
        }
        let values = self
            .stored_secrets(&record)
            .await
            .map_err(|_error| Prepared::Failed(ServerFailure::CredentialStoreUnavailable))?;
        let has_every_secret = record
            .secret_names
            .iter()
            .all(|name| values.iter().any(|(stored, _value)| stored == name));
        if !has_every_secret {
            return Err(Prepared::Failed(ServerFailure::SecretMissing));
        }
        let mut redacted: Vec<Secret> = values.iter().map(|(_name, value)| value.clone()).collect();
        let target = match &record.endpoint {
            StoredEndpoint::Program {
                program,
                arguments,
                folder,
            } => {
                let mut spec = LocalServerSpec::new(program.clone(), arguments.clone());
                if let Some(folder) = folder {
                    spec = spec.in_folder(folder.clone());
                }
                for (name, value) in &record.plain {
                    spec = spec.with_variable(name.clone(), value.clone());
                }
                for (name, value) in values {
                    spec = spec.with_secret(name, value);
                }
                let servers = self.clone();
                let id = server_id.to_owned();
                Target::Local {
                    local: Arc::clone(&self.inner.local),
                    spec,
                    errors: Arc::new(move |line| servers.log_output(&id, generation, &line)),
                }
            }
            StoredEndpoint::Url { url } => {
                let mut headers = HashMap::new();
                for (name, value) in &record.plain {
                    if let (Ok(name), Ok(value)) = (
                        HeaderName::from_bytes(name.as_bytes()),
                        HeaderValue::from_str(value),
                    ) {
                        headers.insert(name, value);
                    }
                }
                for (name, value) in &values {
                    if let (Ok(name), Ok(mut value)) = (
                        HeaderName::from_bytes(name.as_bytes()),
                        HeaderValue::from_str(value.expose()),
                    ) {
                        value.set_sensitive(true);
                        headers.insert(name, value);
                    }
                }
                let client = if record.oauth {
                    let authorized = crate::oauth::authorized_client(self, &record)
                        .await
                        .map_err(Prepared::from_oauth)?;
                    redacted.extend(authorized.secrets);
                    RemoteClient::Authorized(authorized.client)
                } else {
                    RemoteClient::Plain(self.inner.http.clone())
                };
                Target::Remote {
                    client,
                    url: url.clone(),
                    headers,
                }
            }
        };
        Ok((target, record, Arc::new(Redactions::new(redacted))))
    }

    /// Adds values that a server's output must not show from now on, such
    /// as the tokens of a refresh.
    pub(crate) fn redact_too(&self, server_id: &str, secrets: Vec<Secret>) {
        let mut state = self.state();
        if let Some(entry) = find_mut(&mut state.entries, server_id) {
            entry.live.redactions = Arc::new(entry.live.redactions.with(secrets));
        }
    }

    /// Adds a line of a server's error output to its log.
    fn log_output(&self, server_id: &str, generation: u64, line: &str) {
        let now = self.now();
        let mut state = self.state();
        if let Some(entry) = find_mut(&mut state.entries, server_id)
            && entry.live.generation == generation
        {
            let redactions = Arc::clone(&entry.live.redactions);
            entry.live.log.output(now, line, &redactions);
        }
    }

    /// Stores a listing of a server's tools: tools that it listed before
    /// keep the owner's switches, the tools of a first listing start on
    /// within [`MAX_ENABLED_TOOLS`], and a tool that arrives later starts
    /// off and marked as new. A tool whose name holds one of the server's
    /// secrets is left out, since its name is what a call sends back.
    async fn record_tools(&self, server_id: &str, generation: u64, listed: Vec<Tool>) {
        let _tools = self.inner.tools.lock().await;
        let records =
            {
                let state = self.state();
                let Some(entry) = state.entries.iter().find(|entry| {
                    entry.record.id == server_id && entry.live.generation == generation
                }) else {
                    return;
                };
                let redactions = Arc::clone(&entry.live.redactions);
                let (listed, secret_names): (Vec<Tool>, Vec<Tool>) = listed
                    .into_iter()
                    .partition(|tool| !redactions.holds(&tool.name));
                if !secret_names.is_empty() {
                    tracing::warn!(
                        server = %server_id,
                        count = secret_names.len(),
                        "an MCP server named tools with its secret, which are left out"
                    );
                }
                let mut room = MAX_ENABLED_TOOLS.saturating_sub(enabled_tool_count(&state.entries));
                let first_listing = !entry.record.listed;
                listed
                    .into_iter()
                    .zip(0_u32..)
                    .map(|(tool, position)| {
                        let name = tool.name.to_string();
                        let earlier = entry.tools.iter().find(|known| known.name == name);
                        let (enabled, new) = match earlier {
                            Some(known) => (known.enabled, known.new),
                            None if first_listing => {
                                let enabled = room > 0;
                                room = room.saturating_sub(1);
                                (enabled, false)
                            }
                            None => (false, true),
                        };
                        tool_record(&tool, position, enabled, new, &redactions)
                    })
                    .collect::<Vec<ToolRecord>>()
            };
        if let Err(error) = self.inner.store.replace_tools(server_id, &records).await {
            // A server removed meanwhile keeps no tools.
            tracing::debug!(server = %server_id, error.code = error.code(), "the listing of an MCP server's tools was not stored");
            return;
        }
        let count = u32::try_from(records.len()).unwrap_or(u32::MAX);
        self.update_live(server_id, generation, |entry, now| {
            entry.tools = records;
            entry.record.listed = true;
            entry.live.log.event(now, LogEvent::Listed { tools: count });
        });
    }

    /// Lists a server's resources again, without its secrets.
    async fn record_resources(&self, server_id: &str, generation: u64, peer: &Peer<RoleClient>) {
        match client::list_resources(peer).await {
            Ok((resources, templates)) => {
                self.update_live(server_id, generation, |entry, _now| {
                    let redactions = Arc::clone(&entry.live.redactions);
                    entry.live.resources = without_secrets(resources, &redactions);
                    entry.live.templates = templates_without_secrets(templates, &redactions);
                });
            }
            Err(error) => {
                tracing::debug!(server = %server_id, %error, "an MCP server did not list its resources");
            }
        }
    }
}

/// Why a runner could not prepare its connection.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Prepared {
    Removed,
    SignInNeeded,
    Failed(ServerFailure),
}

impl Prepared {
    fn from_oauth(error: crate::oauth::ClientError) -> Self {
        match error {
            crate::oauth::ClientError::SignInNeeded => Self::SignInNeeded,
            crate::oauth::ClientError::Unreachable => Self::Failed(ServerFailure::Unreachable),
            crate::oauth::ClientError::CredentialStoreUnavailable => {
                Self::Failed(ServerFailure::CredentialStoreUnavailable)
            }
        }
    }
}

/// A server's resources without its secrets: a resource whose URI holds one
/// is left out, since its URI is what a read sends back, and the secrets in
/// its other texts are replaced.
fn without_secrets(resources: Vec<Resource>, redactions: &Redactions) -> Vec<Resource> {
    resources
        .into_iter()
        .filter(|resource| !redactions.holds(&resource.uri))
        .map(|mut resource| {
            resource.name = redactions.apply(&resource.name);
            resource.title = resource.title.map(|title| redactions.apply(&title));
            resource.description = resource.description.map(|text| redactions.apply(&text));
            resource
        })
        .collect()
}

/// A server's resource templates without its secrets, as
/// [`without_secrets`] keeps its resources.
fn templates_without_secrets(
    templates: Vec<ResourceTemplate>,
    redactions: &Redactions,
) -> Vec<ResourceTemplate> {
    templates
        .into_iter()
        .filter(|template| !redactions.holds(&template.uri_template))
        .map(|mut template| {
            template.name = redactions.apply(&template.name);
            template.title = template.title.map(|title| redactions.apply(&title));
            template.description = template.description.map(|text| redactions.apply(&text));
            template
        })
        .collect()
}

/// Stops a runner that was taken, and waits for it to close its connection.
async fn stop_runner(runner: Option<Runner>) {
    let Some(runner) = runner else {
        return;
    };
    runner.stop.send_replace(true);
    wait_for(runner.task).await;
}

/// Waits a while for a runner's task to end.
async fn wait_for(task: JoinHandle<()>) {
    if tokio::time::timeout(RUNNER_STOP_WAIT, task).await.is_err() {
        tracing::warn!("an MCP server's connection did not close in time");
    }
}

/// The entry of a server.
fn find_mut<'a>(entries: &'a mut [Entry], server_id: &str) -> Option<&'a mut Entry> {
    entries
        .iter_mut()
        .find(|entry| entry.record.id == server_id)
}

/// How many tools of all servers the owner turned on.
fn enabled_tool_count(entries: &[Entry]) -> u32 {
    let count = entries
        .iter()
        .flat_map(|entry| entry.tools.iter())
        .filter(|tool| tool.enabled)
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// The transport of an endpoint.
fn transport_of(endpoint: &StoredEndpoint) -> ServerTransport {
    match endpoint {
        StoredEndpoint::Program { .. } => ServerTransport::Stdio,
        StoredEndpoint::Url { .. } => ServerTransport::Http,
    }
}

/// The family of a server's secret values.
fn secrets_family(transport: ServerTransport) -> Family {
    match transport {
        ServerTransport::Stdio => Family::Variable,
        ServerTransport::Http => Family::Header,
    }
}

/// The values of a server's secrets: those that the input gives, and the
/// stored ones that it keeps, with the names of those that have none.
fn given_secrets(
    input: &[SecretValue],
    stored: &[(String, Secret)],
) -> (Vec<(String, Secret)>, Vec<String>) {
    let mut values = Vec::new();
    let mut missing = Vec::new();
    for secret in input {
        let value = secret.value.clone().or_else(|| {
            stored
                .iter()
                .find(|(name, _value)| *name == secret.name)
                .map(|(_name, value)| value.clone())
        });
        match value {
            Some(value) => values.push((secret.name.clone(), value)),
            None => missing.push(secret.name.clone()),
        }
    }
    (values, missing)
}

/// The fields of a server as the store keeps them. A prefix that the input
/// leaves empty is taken from the name, among those not `taken`.
fn fields(input: &ServerInput, taken: &[String]) -> ServerFields {
    let endpoint = match input.transport {
        ServerTransport::Stdio => StoredEndpoint::Program {
            program: input.program.clone(),
            arguments: input.arguments.clone(),
            folder: (!input.folder.is_empty()).then(|| input.folder.clone()),
        },
        ServerTransport::Http => StoredEndpoint::Url {
            url: input.url.clone(),
        },
    };
    let prefix = if input.prefix.is_empty() {
        suggested_prefix(&input.name, taken)
    } else {
        input.prefix.clone()
    };
    ServerFields {
        name: input.name.clone(),
        prefix,
        endpoint,
        plain: input
            .plain
            .iter()
            .map(|value| (value.name.clone(), value.value.clone()))
            .collect(),
        secret_names: input
            .secrets
            .iter()
            .map(|secret| secret.name.clone())
            .collect(),
        oauth: input.oauth,
        client_id: (!input.client_id.is_empty()).then(|| input.client_id.clone()),
        images: input.images,
        instructions: input.instructions,
        timeout_seconds: input.timeout_seconds,
    }
}

/// At most `limit` bytes of `text`, cut at a character.
pub(crate) fn cut(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// A server's OAuth state in the credential store: the client secret that
/// the owner entered, and what a sign-in stored (`oauth.rs`).
#[derive(Debug, Clone, Default)]
pub(crate) struct OAuthBundle {
    pub(crate) client_secret: Option<Secret>,
    pub(crate) credentials: Option<Secret>,
}

impl OAuthBundle {
    fn with_client_secret(client_secret: Secret) -> Self {
        Self {
            client_secret: Some(client_secret),
            credentials: None,
        }
    }

    fn is_empty(&self) -> bool {
        self.client_secret.is_none() && self.credentials.is_none()
    }

    fn encode(&self) -> Secret {
        let mut values = Vec::new();
        if let Some(client_secret) = &self.client_secret {
            values.push(("client_secret".to_owned(), client_secret.clone()));
        }
        if let Some(credentials) = &self.credentials {
            values.push(("credentials".to_owned(), credentials.clone()));
        }
        encode_values(&values)
    }

    fn decode(text: &Secret) -> Option<Self> {
        let values = decode_values(text)?;
        let take = |key: &str| {
            values
                .iter()
                .find(|(name, _value)| name == key)
                .map(|(_name, value)| value.clone())
        };
        Some(Self {
            client_secret: take("client_secret"),
            credentials: take("credentials"),
        })
    }
}

/// What a failed write of the store means for the owner.
fn storage_failed(error: &StoreError) -> McpError {
    tracing::warn!(error.code = error.code(), "the MCP plugin's store failed");
    McpError::StorageFailed
}

/// A taken name or prefix, or a failed write.
fn taken_or_storage(error: StoreError) -> McpError {
    match error {
        StoreError::Constraint { code: NAME_TAKEN } => McpError::NameTaken,
        StoreError::Constraint { code: PREFIX_TAKEN } => McpError::PrefixTaken,
        other => storage_failed(&other),
    }
}

/// What a failure of the credential store means for the owner.
fn secrets_failed(error: SecretsError) -> McpError {
    tracing::warn!(%error, "the credential store failed for the MCP plugin");
    McpError::CredentialStoreUnavailable
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::Secret;
    use rmcp::model::{Resource, ResourceTemplate};

    use super::{OAuthBundle, cut, given_secrets, templates_without_secrets, without_secrets};
    use crate::commands::SecretValue;
    use crate::log::Redactions;

    #[test]
    fn resources_are_kept_without_the_servers_secret_and_a_secret_uri_is_left_out() {
        let redactions = Redactions::new([Secret::new("planted-secret".to_owned())]);
        let resources = vec![
            Resource::new("test://readme", "readme planted-secret")
                .with_title("Read planted-secret")
                .with_description("About planted-secret."),
            Resource::new("test://planted-secret", "token"),
        ];
        let templates = vec![
            ResourceTemplate::new("test://notes/{id}", "notes planted-secret"),
            ResourceTemplate::new("test://planted-secret/{id}", "keys"),
        ];

        let resources = without_secrets(resources, &redactions);
        let templates = templates_without_secrets(templates, &redactions);

        let [resource] = resources.as_slice() else {
            panic!("one resource stays: {resources:?}");
        };
        assert_eq!(resource.uri, "test://readme");
        assert_eq!(resource.name, "readme [REDACTED]");
        assert_eq!(resource.title.as_deref(), Some("Read [REDACTED]"));
        assert_eq!(resource.description.as_deref(), Some("About [REDACTED]."));
        let [template] = templates.as_slice() else {
            panic!("one template stays: {templates:?}");
        };
        assert_eq!(template.name, "notes [REDACTED]");
    }

    #[test]
    fn a_server_keeps_the_stored_secrets_that_the_owner_does_not_replace() {
        let stored = vec![("TOKEN".to_owned(), Secret::new("stored-token".to_owned()))];
        let input = vec![
            SecretValue {
                name: "TOKEN".to_owned(),
                value: None,
            },
            SecretValue {
                name: "OTHER".to_owned(),
                value: None,
            },
            SecretValue {
                name: "NEW".to_owned(),
                value: Some(Secret::new("new-value".to_owned())),
            },
        ];

        let (values, missing) = given_secrets(&input, &stored);

        let names: Vec<&str> = values.iter().map(|(name, _value)| name.as_str()).collect();
        assert_eq!(names, ["TOKEN", "NEW"]);
        assert_eq!(values[0].1.expose(), "stored-token");
        assert_eq!(missing, ["OTHER"]);
    }

    #[test]
    fn a_text_is_cut_at_a_character() {
        assert_eq!(cut("héllo", 2), "h");
        assert_eq!(cut("hello", 10), "hello");
    }

    #[test]
    fn the_oauth_state_crosses_the_store_with_its_parts() {
        let bundle = OAuthBundle {
            client_secret: Some(Secret::new("client-secret".to_owned())),
            credentials: Some(Secret::new("{\"access\":\"x\"}".to_owned())),
        };

        let decoded = OAuthBundle::decode(&bundle.encode()).unwrap();

        assert_eq!(decoded.client_secret.unwrap().expose(), "client-secret");
        assert_eq!(decoded.credentials.unwrap().expose(), "{\"access\":\"x\"}");
        assert!(OAuthBundle::default().is_empty());
    }
}
