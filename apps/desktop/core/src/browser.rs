//! Desktop binding of the Browser service (ADR-0064, ADR-0065).
//!
//! A session's tabs are WebView2 webviews that the core hosts beside the
//! pane ([`engine`]). The driver in `mannie-tools` checks every top-level
//! request against the conversation's approvals through the
//! [`NavigationGate`], and the pane places the native views through the
//! status stream, which is their lease.

mod actions;
mod engine;
#[cfg(windows)]
mod webview2;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_util::FutureExt as _;
use mannie_plugin_sdk::contracts::ConversationId;
use mannie_plugin_sdk::host::{BindRefusal, PluginBinding, ServiceProvider};
use mannie_plugin_sdk::ids::Contributor;
use mannie_plugin_sdk::points::tools::{
    BrowserAccess, BrowserAction, BrowserAuthorization, BrowserControlAccess,
};
use mannie_plugin_sdk::service::{BoundService, Service};
use mannie_plugin_sdk::services::browser::{
    Browser, BrowserActivity, BrowserApi, BrowserControl, BrowserControlMode, BrowserCredential,
    BrowserCredentialRef, BrowserError, BrowserFrame, BrowserPlacement, BrowserRequest,
    BrowserResult, BrowserStatus,
};
use mannie_plugin_sdk::{BoxFuture, BoxStream};
use mannie_store::Store;
use mannie_tools::browser::{
    Chrome, ChromeHost, ChromeNavigation, ChromeRequest, cleanup_browser_profiles,
};
use serde_json::json;
use tauri::AppHandle;
use tokio::sync::{Mutex as AsyncMutex, watch};
use zeroize::Zeroizing;

use self::actions::{error, stable_revision};
#[cfg(feature = "validation")]
pub(crate) use self::engine::SurfaceReport;
pub(crate) use self::engine::clear_engine_variables;
use self::engine::{Engine, SessionHost};

const MAX_SESSIONS: usize = 8;
const MAX_CREDENTIALS: usize = 16;
/// How often a status stream compares the session's status with the last
/// one it sent.
const STATUS_INTERVAL: Duration = Duration::from_millis(100);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A browser session is private to a plugin binding and a conversation.
#[derive(Debug, Clone)]
pub(crate) struct BrowserProvider {
    registry: Arc<Registry>,
}

#[derive(Debug)]
struct Registry {
    store: Store,
    engine: Engine,
    generation: AtomicU64,
    bindings: Mutex<Bindings>,
    startup: AsyncMutex<()>,
    /// Changes each time the app page loads, which ends every lease of a
    /// native view (ADR-0064, decision 5).
    page: watch::Sender<u64>,
}

#[derive(Debug, Default)]
struct Bindings {
    generations: HashMap<Contributor, u64>,
    sessions: HashMap<(Contributor, ConversationId), Arc<Slot>>,
}

impl BrowserProvider {
    pub(crate) fn new(store: Store, app: AppHandle, root: std::path::PathBuf) -> Self {
        cleanup_browser_profiles(&root);
        let (page, _) = watch::channel(0);
        Self {
            registry: Arc::new(Registry {
                store,
                engine: Engine::new(app, root),
                generation: AtomicU64::new(0),
                bindings: Mutex::new(Bindings::default()),
                startup: AsyncMutex::new(()),
                page,
            }),
        }
    }

    pub(crate) async fn deleted(&self, conversation: &ConversationId) {
        let sessions: Vec<_> = {
            let _startup = self.registry.startup.lock().await;
            lock(&self.registry.bindings)
                .sessions
                .extract_if(|(_, id), _| id == conversation)
                .map(|(_, session)| session)
                .collect()
        };
        for session in sessions {
            session.retire().await;
        }
    }

    /// The app page loaded again: every lease ends, so no page stays over
    /// the app until a pane follows its conversation again.
    pub(crate) fn page_loaded(&self) {
        self.registry.page.send_modify(|page| *page += 1);
        let slots: Vec<_> = lock(&self.registry.bindings)
            .sessions
            .values()
            .cloned()
            .collect();
        for slot in slots {
            if let Some(views) = slot.views() {
                views.hide_all();
            }
        }
    }

    /// Ends every session before Mannie quits: their processes first, then
    /// their folders (ADR-0064, decision 4).
    pub(crate) async fn shutdown(&self) {
        let sessions: Vec<_> = lock(&self.registry.bindings)
            .sessions
            .drain()
            .map(|(_, session)| session)
            .collect();
        for session in sessions {
            session.retire().await;
        }
    }
}

#[cfg(feature = "validation")]
impl BrowserProvider {
    /// The views of a conversation's session, whichever plugin binding owns it.
    fn validation_views(&self, conversation: &ConversationId) -> Option<Arc<dyn SessionHost>> {
        lock(&self.registry.bindings)
            .sessions
            .iter()
            .find(|((_, id), _)| id == conversation)
            .and_then(|(_, slot)| slot.views())
    }

    /// Where each tab's window of a conversation's session is (V13).
    pub(crate) async fn validation_surfaces(
        &self,
        conversation: &ConversationId,
    ) -> Vec<SurfaceReport> {
        match self.validation_views(conversation) {
            Some(views) => views.surfaces().await,
            None => Vec::new(),
        }
    }

    /// Evaluates an expression in a tab's page and returns its value as JSON
    /// (V13). It runs in the page, so only validation builds have it.
    pub(crate) async fn validation_evaluate(
        &self,
        conversation: &ConversationId,
        tab: &str,
        expression: &str,
    ) -> Option<String> {
        let views = self.validation_views(conversation)?;
        let params =
            json!({ "expression": expression, "returnByValue": true, "awaitPromise": true });
        let reply = views
            .send(tab, "Runtime.evaluate", Zeroizing::new(params.to_string()))
            .ok()?;
        let result = reply.await.ok()?.ok()?;
        Some(result["result"]["value"].to_string())
    }
}

impl ServiceProvider for BrowserProvider {
    fn name(&self) -> &'static str {
        Browser::NAME
    }

    fn major(&self) -> u16 {
        Browser::MAJOR
    }

    fn bind(&self, binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        let owner = binding.contributor();
        let generation = self.registry.generation.fetch_add(1, Ordering::Relaxed) + 1;
        lock(&self.registry.bindings)
            .generations
            .insert(owner.clone(), generation);
        Ok(BoundService::new::<Browser>(Arc::new(BoundBrowser {
            registry: Arc::clone(&self.registry),
            owner,
            generation,
        })))
    }

    fn release(&self, contributor: &Contributor) -> BoxFuture<'static, ()> {
        let registry = Arc::clone(&self.registry);
        let owner = contributor.clone();
        Box::pin(async move {
            let sessions: Vec<_> = {
                let mut bindings = lock(&registry.bindings);
                bindings.generations.remove(&owner);
                bindings
                    .sessions
                    .extract_if(|(contributor, _), _| contributor == &owner)
                    .map(|(_, session)| session)
                    .collect()
            };
            for session in sessions {
                session.retire().await;
            }
        })
    }
}

#[derive(Debug, Clone)]
struct BoundBrowser {
    registry: Arc<Registry>,
    owner: Contributor,
    generation: u64,
}

#[derive(Default)]
struct Session {
    chrome: Option<Arc<Chrome>>,
    views: Option<Arc<dyn SessionHost>>,
    active: Option<String>,
    epoch: u64,
    next_credential: u64,
    credentials: HashMap<String, Credential>,
    needs_observation: bool,
    last_observation: Option<actions::TargetObservation>,
    failure: Option<BrowserError>,
    followers: usize,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Session").finish_non_exhaustive()
    }
}

struct Credential {
    descriptor: BrowserCredentialRef,
    value: Zeroizing<String>,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Credential").finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct Slot {
    operation: AsyncMutex<()>,
    state: Mutex<Session>,
    gate: Arc<NavigationGate>,
    retired: AtomicBool,
}

impl Slot {
    fn new() -> Self {
        let (gate, _) = watch::channel(GateState::default());
        Self {
            operation: AsyncMutex::new(()),
            state: Mutex::new(Session::default()),
            gate: Arc::new(NavigationGate(gate)),
            retired: AtomicBool::new(false),
        }
    }

    fn ensure_live(&self) -> Result<(), BrowserError> {
        if self.retired.load(Ordering::Acquire) {
            Err(BrowserError::NotFound)
        } else {
            Ok(())
        }
    }

    fn chrome(&self) -> Result<Arc<Chrome>, BrowserError> {
        lock(&self.state)
            .chrome
            .clone()
            .ok_or(BrowserError::SessionMissing)
    }

    fn views(&self) -> Option<Arc<dyn SessionHost>> {
        lock(&self.state).views.clone()
    }

    fn interrupt(&self) {
        self.gate.invalidate();
        if let Ok(chrome) = self.chrome() {
            chrome.invalidate_targets();
            chrome.cancel_pending();
        }
    }

    async fn retire(&self) {
        self.retired.store(true, Ordering::Release);
        self.interrupt();
        let _operation = self.operation.lock().await;
        self.end().await;
    }

    async fn end(&self) {
        let chrome = {
            let mut state = lock(&self.state);
            state.epoch += 1;
            state.active = None;
            state.credentials.clear();
            state.needs_observation = false;
            state.last_observation = None;
            state.views = None;
            state.chrome.take()
        };
        if let Some(chrome) = chrome {
            chrome.end().await;
        }
        self.gate.set_mode(BrowserControlMode::Agent);
    }

    fn active(&self, chrome: &Chrome) -> Option<String> {
        let current = lock(&self.state).active.clone();
        let tabs = chrome.tabs();
        current
            .filter(|id| tabs.iter().any(|tab| tab.id == *id))
            .or_else(|| tabs.first().map(|tab| tab.id.clone()))
    }

    /// Gives the pages to the owner or to the agent, in the driver and in
    /// the native views.
    fn set_control(&self, chrome: &Chrome, mode: BrowserControlMode) -> Result<(), BrowserError> {
        let owner = mode == BrowserControlMode::Owner;
        if let Some(views) = self.views() {
            views.set_owner(owner).map_err(error)?;
        }
        chrome.set_owner_control(owner);
        chrome.invalidate_targets();
        lock(&self.state).last_observation = None;
        self.gate.set_mode(mode);
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct ActiveAuthorization {
    ticket: BrowserAuthorization,
    access: BrowserAccess,
}

#[derive(Debug, Clone)]
struct GateState {
    mode: BrowserControlMode,
    revision: u64,
    authorized: Option<ActiveAuthorization>,
}

impl Default for GateState {
    fn default() -> Self {
        Self {
            mode: BrowserControlMode::Agent,
            revision: 0,
            authorized: None,
        }
    }
}

#[derive(Debug)]
struct NavigationGate(watch::Sender<GateState>);

impl NavigationGate {
    fn invalidate(&self) {
        self.0.send_modify(|state| {
            state.revision += 1;
            state.authorized = None;
        });
    }

    fn set_mode(&self, mode: BrowserControlMode) {
        self.0.send_modify(|state| {
            state.revision += 1;
            state.mode = mode;
            state.authorized = None;
        });
    }

    fn resume_if_owner(&self, revision: u64) -> Result<(), BrowserError> {
        // Take Over and End interrupt the gate before taking the operation lock.
        let resumed = self.0.send_if_modified(|state| {
            if state.mode != BrowserControlMode::Owner || state.revision != revision {
                return false;
            }
            state.revision += 1;
            state.mode = BrowserControlMode::Agent;
            state.authorized = None;
            true
        });
        if resumed {
            Ok(())
        } else {
            Err(BrowserError::StaleTarget)
        }
    }

    fn authorize(
        &self,
        authorized: ActiveAuthorization,
        revision: u64,
    ) -> Result<AuthorizationScope<'_>, BrowserError> {
        let applied = self.0.send_if_modified(|state| {
            if state.revision != revision || state.mode != BrowserControlMode::Agent {
                return false;
            }
            state.authorized = Some(authorized);
            true
        });
        if applied {
            Ok(AuthorizationScope(self))
        } else {
            Err(BrowserError::StaleTarget)
        }
    }
}

struct AuthorizationScope<'a>(&'a NavigationGate);

impl Drop for AuthorizationScope<'_> {
    fn drop(&mut self) {
        self.0.0.send_modify(|state| state.authorized = None);
    }
}

impl ChromeNavigation for NavigationGate {
    fn approved(&self, request: &ChromeRequest) -> Option<bool> {
        let current = self.0.borrow();
        // The owner's own navigation in takeover grants the agent nothing
        // and needs no approval (ADR-0065, decision 3).
        if current.mode == BrowserControlMode::Owner {
            return Some(true);
        }
        let active = current.authorized.as_ref()?;
        (active.access.origin == request.origin).then(|| active.ticket.permits(&active.access))
    }

    fn allow(&self, request: ChromeRequest) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            let mut state = self.0.subscribe();
            let current = state.borrow_and_update().clone();
            if current.mode == BrowserControlMode::Owner {
                return true;
            }
            let Some(active) = current.authorized else {
                return false;
            };
            if active.access.origin == request.origin {
                return active.ticket.permits(&active.access);
            }
            let access = BrowserAccess::new(
                request.origin,
                active.access.session,
                stable_revision(&(current.revision, request.tab, request.revision)),
                BrowserAction::ContinueNavigation,
            );
            tokio::select! {
                biased;
                _ = state.changed() => false,
                allowed = active.ticket.authorize(access) => allowed,
            }
        })
    }
}

/// A status stream's hold on its conversation's native views. When the last
/// one ends, the pages hide (ADR-0064, decision 5).
struct Lease {
    slot: Arc<Slot>,
}

impl Lease {
    fn new(slot: Arc<Slot>) -> Self {
        lock(&slot.state).followers += 1;
        Self { slot }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let last = {
            let mut state = lock(&self.slot.state);
            state.followers = state.followers.saturating_sub(1);
            state.followers == 0
        };
        if last && let Some(views) = self.slot.views() {
            views.hide_all();
        }
    }
}

impl BoundBrowser {
    fn ensure_bound(&self) -> Result<(), BrowserError> {
        if lock(&self.registry.bindings).generations.get(&self.owner) == Some(&self.generation) {
            Ok(())
        } else {
            Err(BrowserError::SessionMissing)
        }
    }

    fn existing(&self, conversation: &ConversationId) -> Option<Arc<Slot>> {
        lock(&self.registry.bindings)
            .sessions
            .get(&(self.owner.clone(), conversation.clone()))
            .cloned()
    }

    async fn slot(&self, conversation: ConversationId) -> Result<Arc<Slot>, BrowserError> {
        self.ensure_bound()?;
        if let Some(slot) = self.existing(&conversation) {
            return Ok(slot);
        }
        let _startup = self.registry.startup.lock().await;
        self.ensure_bound()?;
        if self
            .registry
            .store
            .conversation_summary(&conversation)
            .await
            .ok()
            .flatten()
            .is_none()
        {
            return Err(BrowserError::NotFound);
        }
        self.ensure_bound()?;
        let mut bindings = lock(&self.registry.bindings);
        if bindings.generations.get(&self.owner) != Some(&self.generation) {
            return Err(BrowserError::SessionMissing);
        }
        let slot = bindings
            .sessions
            .entry((self.owner.clone(), conversation))
            .or_insert_with(|| Arc::new(Slot::new()));
        Ok(Arc::clone(slot))
    }

    async fn start(
        &self,
        slot: &Arc<Slot>,
        mode: BrowserControlMode,
    ) -> Result<Arc<Chrome>, BrowserError> {
        if let Ok(chrome) = slot.chrome() {
            return Ok(chrome);
        }
        let _startup = self.registry.startup.lock().await;
        slot.ensure_live()?;
        self.ensure_bound()?;
        let running = lock(&self.registry.bindings)
            .sessions
            .values()
            .filter(|slot| lock(&slot.state).chrome.is_some())
            .count();
        if running >= MAX_SESSIONS {
            return Err(BrowserError::LimitReached);
        }
        let started = Engine::start(&self.registry.engine).await.map_err(error)?;
        let views = started.host;
        let host: Arc<dyn ChromeHost> = Arc::<dyn SessionHost>::clone(&views);
        let gate: Arc<dyn ChromeNavigation> = Arc::<NavigationGate>::clone(&slot.gate);
        let chrome = Arc::new(Chrome::start(host, started.events, gate));
        if self.ensure_bound().is_err() || slot.ensure_live().is_err() {
            chrome.end().await;
            return Err(BrowserError::SessionMissing);
        }
        let epoch = {
            let mut state = lock(&slot.state);
            state.chrome = Some(Arc::clone(&chrome));
            state.views = Some(Arc::clone(&views));
            state.failure = None;
            state.epoch
        };
        let owner = mode == BrowserControlMode::Owner;
        views.set_owner(owner).map_err(error)?;
        chrome.set_owner_control(owner);
        if slot.gate.0.borrow().mode != mode {
            slot.gate.set_mode(mode);
        }
        tokio::spawn(watch_end(Arc::clone(slot), Arc::clone(&chrome), epoch));
        Ok(chrome)
    }

    fn read_status(&self, conversation: &ConversationId) -> Result<BrowserStatus, BrowserError> {
        self.ensure_bound()?;
        let Some(slot) = self.existing(conversation) else {
            return Ok(BrowserStatus::default());
        };
        Ok(status(&slot))
    }

    async fn owner_control(
        &self,
        conversation: ConversationId,
        tab: Option<String>,
        action: BrowserControl,
    ) -> Result<(), BrowserError> {
        let slot = self.slot(conversation).await?;
        // Takeover can interrupt an operation before taking its lock. Check
        // an addressed tab first so an obsolete pane cannot interrupt it.
        if let (BrowserControl::TakeOver, Some(tab)) = (&action, tab.as_deref())
            && !slot.chrome()?.tabs().iter().any(|open| open.id == tab)
        {
            return Err(BrowserError::NotFound);
        }
        if matches!(action, BrowserControl::TakeOver | BrowserControl::End) {
            slot.interrupt();
        }
        let _operation = slot.operation.lock().await;
        slot.ensure_live()?;
        self.ensure_bound()?;
        match action {
            BrowserControl::Start => {
                let chrome = self.start(&slot, BrowserControlMode::Owner).await?;
                // The owner starts with a blank tab to type an address into.
                if chrome.tabs().is_empty() {
                    actions::owner_input(&slot, BrowserControl::OpenTab).await?;
                }
            }
            BrowserControl::End => slot.end().await,
            BrowserControl::TakeOver | BrowserControl::Resume => {
                let chrome = slot.chrome()?;
                let mode = if matches!(action, BrowserControl::TakeOver) {
                    BrowserControlMode::Owner
                } else {
                    BrowserControlMode::Agent
                };
                if slot.gate.0.borrow().mode == mode {
                    return Ok(());
                }
                if let Some(tab) = tab {
                    actions::select(&slot, &chrome, tab)?;
                }
                slot.set_control(&chrome, mode)?;
            }
            action => {
                if slot.gate.0.borrow().mode != BrowserControlMode::Owner {
                    return Err(BrowserError::AgentControl);
                }
                let acts_on_current_tab = !matches!(
                    action,
                    BrowserControl::OpenTab
                        | BrowserControl::SelectTab { .. }
                        | BrowserControl::CloseTab { .. }
                );
                if let Some(tab) = tab.filter(|_| acts_on_current_tab) {
                    let chrome = slot.chrome()?;
                    actions::select(&slot, &chrome, tab)?;
                }
                actions::owner_input(&slot, action).await?;
            }
        }
        Ok(())
    }

    async fn change_credential(
        &self,
        conversation: ConversationId,
        action: BrowserCredential,
    ) -> Result<(), BrowserError> {
        self.ensure_bound()?;
        let slot = self
            .existing(&conversation)
            .ok_or(BrowserError::SessionMissing)?;
        let _operation = slot.operation.lock().await;
        slot.ensure_live()?;
        let mut state = lock(&slot.state);
        if state.chrome.is_none() {
            return Err(BrowserError::SessionMissing);
        }
        match action {
            BrowserCredential::Set {
                origin: site,
                label,
                value,
            } => {
                let value = Zeroizing::new(value);
                let origin = actions::credential_origin(&site)?;
                if label.is_empty()
                    || label.len() > 80
                    || value.is_empty()
                    || value.len() > 8192
                    || label.contains(value.as_str())
                {
                    return Err(BrowserError::InvalidBrowserInput);
                }
                if state.credentials.len() >= MAX_CREDENTIALS {
                    return Err(BrowserError::LimitReached);
                }
                state.next_credential += 1;
                let reference = format!("credential-{}", state.next_credential);
                let descriptor = BrowserCredentialRef {
                    reference: reference.clone(),
                    origin,
                    label,
                };
                state
                    .credentials
                    .insert(reference, Credential { descriptor, value });
            }
            BrowserCredential::Remove { reference } => {
                if state.credentials.remove(&reference).is_none() {
                    return Err(BrowserError::CredentialMissing);
                }
            }
        }
        Ok(())
    }

    fn access(
        &self,
        slot: Option<&Slot>,
        request: &BrowserRequest,
    ) -> Result<Option<BrowserAccess>, BrowserError> {
        self.ensure_bound()?;
        if matches!(request, BrowserRequest::Tabs) {
            return if slot
                .is_some_and(|slot| slot.gate.0.borrow().mode == BrowserControlMode::Owner)
            {
                Err(BrowserError::OwnerControl)
            } else {
                Ok(None)
            };
        }
        let (session_epoch, gate_revision, tab) = if let Some(slot) = slot {
            slot.ensure_live()?;
            let gate = slot.gate.0.borrow().clone();
            if gate.mode == BrowserControlMode::Owner {
                return Err(BrowserError::OwnerControl);
            }
            let session_epoch = lock(&slot.state).epoch;
            let tab = actions::target_tab(slot, request)?;
            (session_epoch, gate.revision, tab)
        } else {
            (0, 0, None)
        };
        let (site, action) = actions::site_and_action(request, tab.as_ref())?;
        let tab_revision = tab
            .as_ref()
            .map(|tab| (tab.id.as_str(), tab.revision.as_str()));
        let revision = stable_revision(&(gate_revision, tab_revision, request));
        let session = format!("{}.{}", self.generation, session_epoch);
        Ok(Some(BrowserAccess::new(site, session, revision, action)))
    }

    fn control_access(&self, slot: &Slot) -> Result<Option<BrowserControlAccess>, BrowserError> {
        self.ensure_bound()?;
        slot.ensure_live()?;
        let state = lock(&slot.state);
        if state.chrome.is_none() {
            return Err(BrowserError::SessionMissing);
        }
        let gate = slot.gate.0.borrow();
        Ok((gate.mode == BrowserControlMode::Owner).then(|| {
            BrowserControlAccess::new(
                format!("{}.{}", self.generation, state.epoch),
                gate.revision.to_string(),
            )
        }))
    }

    async fn return_control(
        &self,
        conversation: ConversationId,
        authorization: Option<BrowserAuthorization>,
    ) -> Result<(), BrowserError> {
        self.ensure_bound()?;
        let slot = self
            .existing(&conversation)
            .ok_or(BrowserError::SessionMissing)?;
        let _operation = slot.operation.lock().await;
        let revision = slot.gate.0.borrow().revision;
        let Some(access) = self.control_access(&slot)? else {
            return Ok(());
        };
        let ticket = authorization.ok_or(BrowserError::OwnerControl)?;
        if ticket.conversation_id() != &conversation
            || ticket.plugin_id() != self.owner.plugin_id().as_str()
            || !ticket.permits_control(&access)
        {
            return Err(BrowserError::StaleTarget);
        }
        let chrome = slot.chrome()?;
        // Focus leaves the pages and they stop taking input before the
        // agent gets them (ADR-0064, decision 6).
        if let Some(views) = slot.views() {
            views.set_owner(false).map_err(error)?;
        }
        slot.gate.resume_if_owner(revision)?;
        chrome.set_owner_control(false);
        chrome.invalidate_targets();
        lock(&slot.state).last_observation = None;
        Ok(())
    }

    async fn agent_request(
        &self,
        conversation: ConversationId,
        request: BrowserRequest,
        authorization: Option<BrowserAuthorization>,
    ) -> Result<BrowserResult, BrowserError> {
        self.ensure_bound()?;
        if matches!(request, BrowserRequest::Tabs) {
            let slot = self.existing(&conversation);
            if slot
                .as_ref()
                .is_some_and(|slot| slot.gate.0.borrow().mode == BrowserControlMode::Owner)
            {
                return Err(BrowserError::OwnerControl);
            }
            let chrome = slot.as_ref().and_then(|slot| slot.chrome().ok());
            let active = slot
                .as_ref()
                .zip(chrome.as_ref())
                .and_then(|(slot, chrome)| slot.active(chrome));
            let tabs: Vec<_> = chrome
                .into_iter()
                .flat_map(|chrome| chrome.tabs())
                .map(|tab| tab.id)
                .collect();
            return Ok(BrowserResult {
                data: json!({ "tabs": tabs, "activeTab": active }),
                frame: None,
                failure: None,
            });
        }
        let ticket = authorization.ok_or(BrowserError::NavigationDenied)?;
        if ticket.conversation_id() != &conversation
            || ticket.plugin_id() != self.owner.plugin_id().as_str()
        {
            return Err(BrowserError::NavigationDenied);
        }
        let slot = self.slot(conversation).await?;
        let _operation = slot.operation.lock().await;
        let gate_revision = slot.gate.0.borrow().revision;
        let access = self
            .access(Some(&slot), &request)?
            .ok_or(BrowserError::NavigationDenied)?;
        if !ticket.permits(&access) {
            return Err(BrowserError::StaleTarget);
        }
        if lock(&slot.state).needs_observation && !matches!(request, BrowserRequest::Observe { .. })
        {
            return Err(BrowserError::OperationUncertain);
        }
        let chrome = match slot.chrome() {
            Ok(chrome) => chrome,
            Err(_) => self.start(&slot, BrowserControlMode::Agent).await?,
        };
        let observing = matches!(request, BrowserRequest::Observe { .. });
        let scope = if observing {
            None
        } else {
            Some(slot.gate.authorize(
                ActiveAuthorization {
                    ticket,
                    access: access.clone(),
                },
                gate_revision,
            )?)
        };
        let result = actions::agent_input(&slot, &chrome, request, &access.origin).await;
        drop(scope);
        if observing && gate_revision != slot.gate.0.borrow().revision {
            return Err(BrowserError::StaleTarget);
        }
        match &result {
            Err(BrowserError::OperationUncertain) => {
                lock(&slot.state).needs_observation = true;
            }
            Ok(output) if output.failure == Some(BrowserError::OperationUncertain) => {
                lock(&slot.state).needs_observation = true;
            }
            Ok(output) if output.data.get("revision").is_some() => {
                lock(&slot.state).needs_observation = false;
            }
            _ => {}
        }
        result
    }

    fn place_view(
        &self,
        conversation: &ConversationId,
        placement: BrowserPlacement,
    ) -> Result<(), BrowserError> {
        self.ensure_bound()?;
        let slot = self
            .existing(conversation)
            .ok_or(BrowserError::SessionMissing)?;
        // Only a pane that follows the conversation may show its pages.
        if lock(&slot.state).followers == 0 {
            return Err(BrowserError::SessionMissing);
        }
        let views = slot.views().ok_or(BrowserError::SessionMissing)?;
        views.place(placement).map_err(error)
    }

    fn focus_page(&self, conversation: &ConversationId, tab: &str) -> Result<(), BrowserError> {
        self.ensure_bound()?;
        let slot = self
            .existing(conversation)
            .ok_or(BrowserError::SessionMissing)?;
        if slot.gate.0.borrow().mode != BrowserControlMode::Owner {
            return Err(BrowserError::AgentControl);
        }
        let views = slot.views().ok_or(BrowserError::SessionMissing)?;
        views.focus(tab).map_err(error)
    }

    async fn read_still(
        &self,
        conversation: ConversationId,
        tab: String,
    ) -> Result<Option<BrowserFrame>, BrowserError> {
        self.ensure_bound()?;
        let Some(views) = self.existing(&conversation).and_then(|slot| slot.views()) else {
            return Ok(None);
        };
        actions::still(views.as_ref(), &tab).await
    }

    fn follow(&self, conversation: ConversationId) -> BoxStream<'static, BrowserStatus> {
        let browser = self.clone();
        let page = self.registry.page.subscribe();
        let stream = futures_util::stream::unfold(
            (
                browser,
                conversation,
                page,
                None::<BrowserStatus>,
                None::<Lease>,
            ),
            |(browser, conversation, mut page, last, mut lease)| async move {
                loop {
                    if browser.ensure_bound().is_err() {
                        return None;
                    }
                    if lease.is_none() {
                        lease = browser.existing(&conversation).map(Lease::new);
                    }
                    let current = browser.read_status(&conversation).ok()?;
                    if last.as_ref() != Some(&current) {
                        return Some((
                            current.clone(),
                            (browser, conversation, page, Some(current), lease),
                        ));
                    }
                    tokio::select! {
                        // The app page loaded again: this stream's pane is
                        // gone (ADR-0064, decision 5).
                        _ = page.changed() => return None,
                        () = tokio::time::sleep(STATUS_INTERVAL) => {}
                    }
                }
            },
        );
        Box::pin(stream)
    }
}

/// The pane's status of a session.
fn status(slot: &Slot) -> BrowserStatus {
    let (chrome, views, failure, mut credentials) = {
        let state = lock(&slot.state);
        let credentials: Vec<_> = state
            .credentials
            .values()
            .map(|secret| secret.descriptor.clone())
            .collect();
        (
            state.chrome.clone(),
            state.views.clone(),
            state.failure,
            credentials,
        )
    };
    let Some(chrome) = chrome else {
        return BrowserStatus {
            failure,
            ..BrowserStatus::default()
        };
    };
    credentials.sort_by(|left, right| left.reference.cmp(&right.reference));
    let active_tab = slot.active(&chrome);
    let control = slot.gate.0.borrow().mode;
    let activity = (control == BrowserControlMode::Agent)
        .then(|| active_tab.as_deref().and_then(|tab| chrome.activity(tab)))
        .flatten()
        .map(|activity| BrowserActivity {
            kind: activity.kind.to_owned(),
            sequence: activity.sequence,
        });
    BrowserStatus {
        control: Some(control),
        tabs: chrome
            .tabs()
            .into_iter()
            .map(actions::browser_tab)
            .collect(),
        active_tab,
        activity,
        focus_return: views.and_then(|views| views.focus_return()),
        failure: None,
        credentials,
    }
}

/// Records a session that ended on its own, such as after its browser
/// process exited, so the pane can say so.
async fn watch_end(slot: Arc<Slot>, chrome: Arc<Chrome>, epoch: u64) {
    chrome.closed().await;
    let ended_alone = {
        let mut state = lock(&slot.state);
        let same = state.epoch == epoch
            && state
                .chrome
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &chrome));
        if same {
            state.failure = Some(BrowserError::BrowserDisconnected);
            state.epoch += 1;
            state.views = None;
            state.chrome = None;
            state.credentials.clear();
            state.active = None;
        }
        same
    };
    if ended_alone {
        tracing::warn!("a browser session ended on its own");
        chrome.end().await;
        slot.gate.set_mode(BrowserControlMode::Agent);
    }
}

impl BrowserApi for BoundBrowser {
    fn supported(&self) -> bool {
        Engine::supported()
    }

    fn status(
        &self,
        conversation: ConversationId,
    ) -> BoxFuture<'static, Result<BrowserStatus, BrowserError>> {
        std::future::ready(self.read_status(&conversation)).boxed()
    }

    fn watch(&self, conversation: ConversationId) -> BoxStream<'static, BrowserStatus> {
        self.follow(conversation)
    }

    fn place(
        &self,
        conversation: ConversationId,
        placement: BrowserPlacement,
    ) -> BoxFuture<'static, Result<(), BrowserError>> {
        std::future::ready(self.place_view(&conversation, placement)).boxed()
    }

    fn focus(
        &self,
        conversation: ConversationId,
        tab: String,
    ) -> BoxFuture<'static, Result<(), BrowserError>> {
        std::future::ready(self.focus_page(&conversation, &tab)).boxed()
    }

    fn still(
        &self,
        conversation: ConversationId,
        tab: String,
    ) -> BoxFuture<'static, Result<Option<BrowserFrame>, BrowserError>> {
        let browser = self.clone();
        async move { browser.read_still(conversation, tab).await }.boxed()
    }

    fn control(
        &self,
        conversation: ConversationId,
        tab: Option<String>,
        action: BrowserControl,
    ) -> BoxFuture<'static, Result<(), BrowserError>> {
        let browser = self.clone();
        async move { browser.owner_control(conversation, tab, action).await }.boxed()
    }

    fn credential(
        &self,
        conversation: ConversationId,
        action: BrowserCredential,
    ) -> BoxFuture<'static, Result<(), BrowserError>> {
        let browser = self.clone();
        async move { browser.change_credential(conversation, action).await }.boxed()
    }

    fn prepare(
        &self,
        conversation: ConversationId,
        request: BrowserRequest,
    ) -> BoxFuture<'static, Result<Vec<BrowserAccess>, BrowserError>> {
        let slot = self.existing(&conversation);
        std::future::ready(
            self.access(slot.as_deref(), &request)
                .map(|access| access.into_iter().collect()),
        )
        .boxed()
    }

    fn prepare_control(
        &self,
        conversation: ConversationId,
    ) -> BoxFuture<'static, Result<Option<BrowserControlAccess>, BrowserError>> {
        let result = self
            .existing(&conversation)
            .ok_or(BrowserError::SessionMissing)
            .and_then(|slot| self.control_access(&slot));
        std::future::ready(result).boxed()
    }

    fn request_control(
        &self,
        conversation: ConversationId,
        authorization: Option<BrowserAuthorization>,
    ) -> BoxFuture<'static, Result<(), BrowserError>> {
        let browser = self.clone();
        async move { browser.return_control(conversation, authorization).await }.boxed()
    }

    fn execute(
        &self,
        conversation: ConversationId,
        request: BrowserRequest,
        authorization: Option<BrowserAuthorization>,
    ) -> BoxFuture<'static, Result<BrowserResult, BrowserError>> {
        let browser = self.clone();
        async move {
            browser
                .agent_request(conversation, request, authorization)
                .await
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_owner_consent_cannot_resume_a_new_takeover() {
        let (state, _) = watch::channel(GateState::default());
        let gate = NavigationGate(state);
        gate.set_mode(BrowserControlMode::Owner);
        let old_revision = gate.0.borrow().revision;
        gate.invalidate();

        assert_eq!(
            gate.resume_if_owner(old_revision),
            Err(BrowserError::StaleTarget)
        );
        assert_eq!(gate.0.borrow().mode, BrowserControlMode::Owner);
        let current_revision = gate.0.borrow().revision;
        gate.resume_if_owner(current_revision).unwrap();
        assert_eq!(gate.0.borrow().mode, BrowserControlMode::Agent);
    }

    #[test]
    fn the_gate_decides_at_once_only_for_the_owner_and_the_authorized_origin() {
        let (state, _) = watch::channel(GateState::default());
        let gate = NavigationGate(state);
        let request = ChromeRequest {
            tab: "tab-1".to_owned(),
            origin: "https://example.com".to_owned(),
            revision: "1".to_owned(),
        };
        // Without an authorization the owner must decide.
        assert_eq!(gate.approved(&request), None);
        gate.set_mode(BrowserControlMode::Owner);
        assert_eq!(gate.approved(&request), Some(true));
    }
}
