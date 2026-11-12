//! Browser tabs, committed documents and checked top-level requests over a
//! [`ChromeHost`] (ADR-0063, decisions 2 and 8).
//!
//! A paused top-level request to an approved origin continues at once. Any
//! other request fails at once, the host asks the owner, and an approved
//! request is replayed with its original method, body and headers. No request
//! waits inside the engine for the owner, so no order of process deaths can
//! release one (ADR-0063, validation, check 4).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use super::host::{ChromeEvent, ChromeHost};
use super::wire::Wire;
use super::{
    ChromeActivity, ChromeError, ChromeNavigation, ChromeNavigationAction, ChromeRequest,
    ChromeTab, chrome_origin, lock,
};

const MAX_TABS: usize = 12;
const MAX_PENDING_APPROVALS: usize = 12;
const MAX_APPROVED_ORIGINS: usize = 32;
const MAX_REPLAY_BODY: usize = 64 * 1024;
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(170);

/// One conversation's browser session and its guarded tabs.
pub struct Chrome {
    pub(super) inner: Arc<Driver>,
}

impl fmt::Debug for Chrome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Chrome").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;

pub(super) struct Driver {
    pub(super) wire: Wire,
    pub(super) state: Mutex<State>,
    navigation: Arc<dyn ChromeNavigation>,
    opening: AtomicUsize,
}

#[derive(Default)]
pub(super) struct State {
    pub(super) tabs: BTreeMap<String, Page>,
    pub(super) protected_values: Vec<Zeroizing<String>>,
    pub(super) tab_generation: u64,
    committed_hosts: HashSet<String>,
    pending: HashMap<String, CancellationToken>,
    next_pending: u64,
    owner: bool,
}

impl State {
    fn new_host_discloses_credential(&self, origin: &str) -> bool {
        // A previously displayed host carries no new credential information.
        // Match only the hostname, excluding the scheme and port.
        origin_host(origin).is_some_and(|host| {
            !self.committed_hosts.contains(&host)
                && self
                    .protected_values
                    .iter()
                    .any(|value| !value.is_empty() && host.contains(&value.to_ascii_lowercase()))
        })
    }
}

/// The page's visual viewport, for checking points and scaling images.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Viewport {
    /// Width in CSS pixels.
    pub(super) width: f64,
    /// Height in CSS pixels.
    pub(super) height: f64,
    /// Device pixels per CSS pixel.
    pub(super) scale: f64,
    /// Visual viewport origin in document CSS pixels.
    pub(super) document_x: f64,
    pub(super) document_y: f64,
}

impl Viewport {
    pub(super) fn contains(self, x: f64, y: f64) -> bool {
        (0.0..self.width).contains(&x) && (0.0..self.height).contains(&y)
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
            scale: 1.0,
            document_x: 0.0,
            document_y: 0.0,
        }
    }
}

/// A request that the host approved, which the next top-level request of the
/// tab to the same address continues as.
struct Continuation {
    origin: String,
    url: Option<String>,
    method: Option<String>,
    headers: Option<Value>,
    body: Option<ReplayBody>,
}

/// What becomes of a paused request.
enum Resolution {
    /// The engine does not send it.
    Fail,
    /// The engine sends it as it is.
    Continue,
    /// The engine sends it as an approved request was.
    Replay(Continuation),
}

/// The body of a request to replay.
enum ReplayBody {
    /// Text from the paused request.
    Text(Zeroizing<String>),
    /// Base64, as `Fetch.getRequestPostData` returns it.
    Base64(Zeroizing<String>),
}

pub(super) struct Page {
    pub(super) main_frame: String,
    pub(super) tab: ChromeTab,
    pub(super) revision: u64,
    pub(super) elements: HashMap<String, super::observation::ObservedElement>,
    pub(super) screenshot_size: Option<(u32, u32)>,
    pub(super) viewport: Viewport,
    pub(super) sensitive_documents: HashSet<String>,
    pub(super) document: String,
    pub(super) needs_observation: bool,
    activity: Option<ChromeActivity>,
    activity_sequence: u32,
    approved: Vec<String>,
    next: Option<Continuation>,
    fresh: bool,
    owner_used: bool,
    /// The data of the page's last drag, which interception kept from the
    /// operating system for an agent's drag.
    pub(super) drag_data: Option<Value>,
    issue: Option<ChromeError>,
    phase: PagePhase,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PagePhase {
    Attaching { loading: bool },
    Idle,
    Loading,
}

impl Page {
    fn new(id: &str, fresh: bool) -> Self {
        Self {
            main_frame: String::new(),
            tab: ChromeTab {
                id: id.to_owned(),
                url: "about:blank".to_owned(),
                title: String::new(),
                revision: "1".to_owned(),
                back: None,
                forward: None,
                loading: false,
            },
            revision: 1,
            elements: HashMap::new(),
            screenshot_size: None,
            viewport: Viewport::default(),
            sensitive_documents: HashSet::new(),
            document: String::new(),
            needs_observation: false,
            activity: None,
            activity_sequence: 0,
            approved: Vec::new(),
            next: None,
            fresh,
            owner_used: false,
            drag_data: None,
            issue: None,
            phase: PagePhase::Attaching { loading: false },
        }
    }

    pub(super) fn settled(&self) -> bool {
        self.phase == PagePhase::Idle
    }

    pub(super) fn is_current(&self, context: &Context) -> bool {
        self.tab.revision == context.revision
    }

    pub(super) fn record_activity(&mut self, kind: &'static str) {
        self.activity_sequence = self.activity_sequence.wrapping_add(1);
        self.activity = Some(ChromeActivity {
            kind,
            sequence: self.activity_sequence,
        });
    }

    pub(super) fn invalidate(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.tab.revision = self.revision.to_string();
        self.elements.clear();
        self.screenshot_size = None;
    }

    fn approve(&mut self, origin: String) {
        self.approved.retain(|known| *known != origin);
        if self.approved.len() >= MAX_APPROVED_ORIGINS {
            self.approved.remove(0);
        }
        self.approved.push(origin);
    }
}

#[derive(Clone)]
pub(super) struct Context {
    pub(super) id: String,
    pub(super) main_frame: String,
    pub(super) revision: String,
    pub(super) origin: Option<String>,
    pub(super) batch_tab_generation: Option<u64>,
    document: String,
    url: String,
}

impl Chrome {
    /// Drives a session whose tabs `host` holds. `events` carries what the
    /// host reports about them; the session ends when it closes.
    #[must_use]
    pub fn start(
        host: Arc<dyn ChromeHost>,
        events: mpsc::Receiver<ChromeEvent>,
        navigation: Arc<dyn ChromeNavigation>,
    ) -> Self {
        let inner = Arc::new(Driver {
            wire: Wire::new(host),
            navigation,
            state: Mutex::new(State::default()),
            opening: AtomicUsize::new(0),
        });
        tokio::spawn(Arc::clone(&inner).events(events));
        Self { inner }
    }

    /// A cached view of guarded tabs and their history origins.
    #[must_use]
    pub fn tabs(&self) -> Vec<ChromeTab> {
        if self.inner.wire.closed.is_cancelled() {
            return Vec::new();
        }
        lock(&self.inner.state)
            .tabs
            .values()
            .filter(|page| !matches!(page.phase, PagePhase::Attaching { .. }))
            .map(|page| page.tab.clone())
            .collect()
    }

    /// Whether the session has ended.
    #[must_use]
    pub fn ended(&self) -> bool {
        self.inner.wire.closed.is_cancelled()
    }

    /// Waits until the session ends for any reason.
    pub async fn closed(&self) {
        self.inner.wire.closed.cancelled().await;
    }

    /// The agent's latest gesture in a tab, for the owner's activity line.
    #[must_use]
    pub fn activity(&self, tab: &str) -> Option<ChromeActivity> {
        if self.inner.wire.closed.is_cancelled() {
            return None;
        }
        lock(&self.inner.state).tabs.get(tab)?.activity
    }

    /// Opens a fresh blank tab with interception installed.
    pub async fn open_tab(&self) -> Result<String, ChromeError> {
        if self.tabs().len() >= MAX_TABS {
            return Err(ChromeError::Limit);
        }
        let id = self.inner.wire.host().create_tab().await?;
        if let Err(error) = self.inner.setup_page(&id, false).await {
            let _closed = self.inner.wire.host().close_tab(&id).await;
            return Err(error);
        }
        Ok(id)
    }

    /// Closes a tab and invalidates its pending work.
    pub async fn close_tab(&self, tab: &str) -> Result<(), ChromeError> {
        self.inner.context(tab, None)?;
        self.cancel_pending();
        {
            let mut state = lock(&self.inner.state);
            state.tabs.remove(tab).ok_or(ChromeError::NoTab)?;
            state.tab_generation = state.tab_generation.wrapping_add(1);
        }
        self.inner.wire.host().close_tab(tab).await
    }

    /// Cancels approvals that wait before control changes or a run stops.
    pub fn cancel_pending(&self) {
        let mut state = lock(&self.inner.state);
        for pending in state.pending.values() {
            pending.cancel();
        }
        for page in state.tabs.values_mut() {
            page.next = None;
        }
    }

    /// Invalidates observations when the owner and agent exchange control.
    pub fn invalidate_targets(&self) {
        for page in lock(&self.inner.state).tabs.values_mut() {
            page.invalidate();
            page.activity = None;
        }
    }

    /// Records who controls the session. When the owner returns control,
    /// the current document of every tab that the owner used is withheld
    /// from model screenshots until the tab navigates (ADR-0064, decision 4).
    pub fn set_owner_control(&self, owner: bool) {
        let mut state = lock(&self.inner.state);
        state.owner = owner;
        for page in state.tabs.values_mut() {
            if !owner && page.owner_used {
                let document = page.document.clone();
                page.sensitive_documents.insert(document);
                page.screenshot_size = None;
            }
            page.owner_used = false;
        }
    }

    /// Ends the session: its processes, then its folder.
    pub async fn end(&self) {
        self.cancel_pending();
        self.inner.wire.closed.cancel();
        self.inner.wire.host().end().await;
    }

    /// Executes a checked navigation.
    pub async fn navigate(
        &self,
        tab: &str,
        action: ChromeNavigationAction,
    ) -> Result<(), ChromeError> {
        let context = self.inner.context(tab, None)?;
        self.inner.begin_input(&context, false)?;
        let destination = match &action {
            ChromeNavigationAction::Go(url) => {
                Some(chrome_origin(url).ok_or(ChromeError::InvalidAddress)?)
            }
            _ => None,
        };
        let result = match action {
            ChromeNavigationAction::Go(url) => {
                self.inner
                    .command(&context, "Page.navigate", json!({ "url": url }))
                    .await
            }
            ChromeNavigationAction::Reload => {
                context.origin.as_ref().ok_or(ChromeError::InvalidAddress)?;
                self.inner.command(&context, "Page.reload", json!({})).await
            }
            ChromeNavigationAction::Back | ChromeNavigationAction::Forward => {
                self.inner
                    .navigate_history(&context, matches!(action, ChromeNavigationAction::Back))
                    .await
            }
        };
        let result = result.map_err(|error| self.inner.dispatched(&context, error))?;
        self.inner
            .settle()
            .await
            .map_err(|error| self.inner.dispatched(&context, error))?;
        self.inner.outcome()?;
        if result["errorText"].is_string() {
            // A request that waited for the owner failed in the engine and
            // was replayed after approval. The navigation succeeded when the
            // tab now shows the destination.
            let replayed = destination.is_some_and(|destination| {
                lock(&self.inner.state)
                    .tabs
                    .get(tab)
                    .and_then(|page| chrome_origin(&page.tab.url))
                    .is_some_and(|current| current == destination)
            });
            if !replayed {
                return Err(ChromeError::Rejected);
            }
        }
        self.inner.refresh_title(tab).await;
        Ok(())
    }
}

impl Drop for Chrome {
    fn drop(&mut self) {
        if !self.inner.wire.closed.is_cancelled() {
            self.inner.wire.closed.cancel();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(self.inner.wire.host().end());
            }
        }
    }
}

impl Driver {
    pub(super) fn context(&self, id: &str, expected: Option<&str>) -> Result<Context, ChromeError> {
        if self.wire.closed.is_cancelled() {
            return Err(ChromeError::Disconnected);
        }
        let state = lock(&self.state);
        let page = state
            .tabs
            .get(id)
            .filter(|page| !matches!(page.phase, PagePhase::Attaching { .. }))
            .ok_or(ChromeError::NoTab)?;
        if expected.is_some_and(|expected| expected != page.tab.revision) {
            return Err(ChromeError::Stale);
        }
        Ok(Context {
            id: id.to_owned(),
            main_frame: page.main_frame.clone(),
            revision: page.tab.revision.clone(),
            origin: chrome_origin(&page.tab.url),
            batch_tab_generation: None,
            document: page.document.clone(),
            url: page.tab.url.clone(),
        })
    }

    pub(super) fn revalidate(&self, context: &Context) -> Result<(), ChromeError> {
        let current = self.context(&context.id, Some(&context.revision))?;
        if current.origin == context.origin {
            return Ok(());
        }
        Err(ChromeError::Stale)
    }

    pub(super) async fn command(
        &self,
        context: &Context,
        method: &str,
        params: Value,
    ) -> Result<Value, ChromeError> {
        self.wire.command(&context.id, method, params).await
    }

    pub(super) fn begin_input(
        &self,
        context: &Context,
        check_observation: bool,
    ) -> Result<(), ChromeError> {
        let mut state = lock(&self.state);
        let page = state.tabs.get(&context.id).ok_or(ChromeError::NoTab)?;
        if context.batch_tab_generation.is_some_and(|original| {
            original != state.tab_generation || !page.settled() || !page.is_current(context)
        }) || check_observation && page.needs_observation
        {
            return Err(ChromeError::Stale);
        }
        for page in state.tabs.values_mut() {
            page.issue = None;
        }
        Ok(())
    }

    pub(super) fn outcome(&self) -> Result<(), ChromeError> {
        let state = lock(&self.state);
        if let Some(issue) = state.tabs.values().find_map(|page| page.issue) {
            return Err(issue);
        }
        Ok(())
    }

    pub(super) fn dispatched(&self, context: &Context, error: ChromeError) -> ChromeError {
        if matches!(error, ChromeError::Disconnected | ChromeError::Uncertain) {
            if let Some(page) = lock(&self.state).tabs.get_mut(&context.id) {
                page.needs_observation = true;
            }
            return ChromeError::Uncertain;
        }
        error
    }

    async fn setup_page(&self, tab: &str, fresh: bool) -> Result<(), ChromeError> {
        {
            let mut state = lock(&self.state);
            if state.tabs.len() >= MAX_TABS {
                return Err(ChromeError::Limit);
            }
            state.tabs.insert(tab.to_owned(), Page::new(tab, fresh));
            state.tab_generation = state.tab_generation.wrapping_add(1);
        }
        let result = self.enable(tab).await;
        let mut state = lock(&self.state);
        match result {
            Ok(main_frame) => {
                if let Some(page) = state.tabs.get_mut(tab) {
                    if !fresh {
                        page.main_frame = main_frame;
                    }
                    page.phase = if matches!(page.phase, PagePhase::Attaching { loading: true }) {
                        PagePhase::Loading
                    } else {
                        PagePhase::Idle
                    };
                }
                Ok(())
            }
            Err(error) => {
                state.tabs.remove(tab);
                Err(error)
            }
        }
    }

    /// Installs request interception before the tab can navigate, then the
    /// page events that the commit check needs.
    async fn enable(&self, tab: &str) -> Result<String, ChromeError> {
        self.wire
            .command(
                tab,
                "Fetch.enable",
                json!({ "patterns": [{ "resourceType": "Document", "requestStage": "Request" }], "handleAuthRequests": true }),
            )
            .await?;
        let worker_bypass = self.wire.submit(
            tab,
            "Network.setBypassServiceWorker",
            json!({ "bypass": true }),
        )?;
        let page_events = self.wire.submit(tab, "Page.enable", json!({}))?;
        let owner = lock(&self.state).owner;
        let file_chooser = self.wire.submit(
            tab,
            "Page.setInterceptFileChooserDialog",
            json!({ "enabled": !owner }),
        )?;
        for command in [worker_bypass, page_events, file_chooser] {
            command.wait().await?;
        }
        let frames = self
            .wire
            .command(tab, "Page.getFrameTree", json!({}))
            .await?;
        Ok(frames["frameTree"]["frame"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_owned())
    }

    async fn navigate_history(&self, context: &Context, back: bool) -> Result<Value, ChromeError> {
        let history = self
            .command(context, "Page.getNavigationHistory", json!({}))
            .await?;
        let entry = history_entry(&history, back).ok_or(ChromeError::Rejected)?;
        let destination = entry["url"]
            .as_str()
            .and_then(chrome_origin)
            .ok_or(ChromeError::InvalidAddress)?;
        let expected = {
            let state = lock(&self.state);
            let page = state.tabs.get(&context.id).ok_or(ChromeError::NoTab)?;
            if back {
                page.tab.back.clone()
            } else {
                page.tab.forward.clone()
            }
        };
        if expected.as_ref() != Some(&destination) {
            return Err(ChromeError::Stale);
        }
        let request = ChromeRequest {
            tab: context.id.clone(),
            origin: destination.clone(),
            revision: context.revision.clone(),
        };
        let allowed = match self.navigation.approved(&request) {
            Some(allowed) => allowed,
            None => self.navigation.allow(request).await,
        };
        if !allowed {
            return Err(ChromeError::Rejected);
        }
        self.revalidate(context)?;
        if let Some(page) = lock(&self.state).tabs.get_mut(&context.id) {
            page.approve(destination.clone());
            page.next = Some(Continuation {
                origin: destination,
                url: None,
                method: None,
                headers: None,
                body: None,
            });
        }
        self.command(
            context,
            "Page.navigateToHistoryEntry",
            json!({ "entryId": entry["id"] }),
        )
        .await
    }

    async fn events(self: Arc<Self>, mut events: mpsc::Receiver<ChromeEvent>) {
        loop {
            let event = tokio::select! {
                () = self.wire.closed.cancelled() => break,
                event = events.recv() => event,
            };
            let Some(event) = event else { break };
            if self.event(event).await.is_err() {
                break;
            }
        }
        // A host that stops reporting, or an event that breaks the commit
        // rules, ends the whole session rather than leaving a tab ungoverned.
        if !self.wire.closed.is_cancelled() {
            self.wire.closed.cancel();
            for pending in lock(&self.state).pending.values() {
                pending.cancel();
            }
            self.wire.host().end().await;
        }
    }

    async fn event(self: &Arc<Self>, event: ChromeEvent) -> Result<(), ChromeError> {
        match event {
            ChromeEvent::Protocol {
                tab,
                method,
                params,
            } => self.protocol(&tab, &method, &params).await,
            ChromeEvent::Popup { popup, opener } => {
                self.popup(popup, &opener);
                Ok(())
            }
            ChromeEvent::Title { tab, title } => {
                let mut state = lock(&self.state);
                let State {
                    tabs,
                    protected_values,
                    ..
                } = &mut *state;
                if let Some(page) = tabs.get_mut(&tab) {
                    page.tab.title = sanitized_title(&title, protected_values);
                }
                Ok(())
            }
            ChromeEvent::Focused { tab } => {
                let mut state = lock(&self.state);
                if state.owner
                    && let Some(page) = state.tabs.get_mut(&tab)
                {
                    page.owner_used = true;
                }
                Ok(())
            }
            ChromeEvent::TabClosed { tab } => {
                let mut state = lock(&self.state);
                if state.tabs.remove(&tab).is_some() {
                    state.tab_generation = state.tab_generation.wrapping_add(1);
                }
                Ok(())
            }
        }
    }

    async fn protocol(
        self: &Arc<Self>,
        tab: &str,
        method: &str,
        params: &Value,
    ) -> Result<(), ChromeError> {
        match method {
            "Fetch.requestPaused" => self.paused(tab, params).await,
            "Fetch.authRequired" => self.deny_auth(tab, params).await,
            "Page.frameNavigated" => self.committed(tab, &params["frame"]),
            "Page.navigatedWithinDocument" => {
                self.same_document(tab, params);
                Ok(())
            }
            "Page.frameStartedLoading" => {
                drop(self.loading(tab, params, true));
                Ok(())
            }
            "Page.frameStoppedLoading" => {
                if let Some(tab) = self.loading(tab, params, false) {
                    // The engine can report a commit before its history has
                    // the new entry, so the history is read again here.
                    tokio::spawn(Arc::clone(self).refresh_history(tab.clone()));
                    let driver = Arc::clone(self);
                    tokio::spawn(async move { driver.refresh_title(&tab).await });
                }
                Ok(())
            }
            "Page.javascriptDialogOpening" => {
                // The owner answers dialogs in takeover (ADR-0064, decision 4).
                if lock(&self.state).owner {
                    return Ok(());
                }
                self.dismiss_prompt(tab).await
            }
            "Input.dragIntercepted" => {
                if let Some(page) = lock(&self.state).tabs.get_mut(tab) {
                    page.drag_data = Some(params["data"].clone());
                }
                Ok(())
            }
            "Page.fileChooserOpened" => {
                self.mark_prompt(tab);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn popup(self: &Arc<Self>, popup: String, opener: &str) {
        let known = lock(&self.state).tabs.contains_key(opener);
        if !known || self.wire.closed.is_cancelled() {
            self.wire.host().refuse_popup(&popup);
            return;
        }
        self.opening.fetch_add(1, Ordering::AcqRel);
        let driver = Arc::clone(self);
        tokio::spawn(async move {
            let host = driver.wire.host();
            let created = match host.create_tab().await {
                Ok(tab) => match driver.setup_page(&tab, true).await {
                    Ok(()) => Ok(tab),
                    Err(error) => {
                        let _closed = host.close_tab(&tab).await;
                        Err(error)
                    }
                },
                Err(error) => Err(error),
            };
            match created {
                Ok(tab) => {
                    if host.release_popup(&popup, &tab).is_err() {
                        lock(&driver.state).tabs.remove(&tab);
                        let _closed = host.close_tab(&tab).await;
                    }
                }
                Err(_) => host.refuse_popup(&popup),
            }
            driver.opening.fetch_sub(1, Ordering::AcqRel);
        });
    }

    async fn paused(self: &Arc<Self>, tab: &str, params: &Value) -> Result<(), ChromeError> {
        let id = string(&params["requestId"])?;
        let frame = params["frameId"].as_str().unwrap_or_default();
        let target = {
            let state = lock(&self.state);
            state.tabs.get(tab).map(|page| {
                (
                    page.fresh || page.main_frame.is_empty() || page.main_frame == frame,
                    page.tab.revision.clone(),
                )
            })
        };
        let Some((is_top, revision)) = target else {
            return self.resolve_request(tab, id, Resolution::Fail).await;
        };
        let url = params["request"]["url"].as_str().unwrap_or_default();
        let Some(origin) = chrome_origin(url) else {
            return self.resolve_request(tab, id, Resolution::Fail).await;
        };
        if !is_top {
            // Frames load under the engine's rules (ADR-0063, decision 8).
            return self.resolve_request(tab, id, Resolution::Continue).await;
        }
        let continuation = {
            let mut state = lock(&self.state);
            if state.new_host_discloses_credential(&origin) {
                if let Some(page) = state.tabs.get_mut(tab) {
                    page.issue = Some(ChromeError::Rejected);
                }
                None
            } else {
                state.tabs.get_mut(tab).and_then(|page| {
                    let matches = page.next.as_ref().is_some_and(|next| {
                        next.origin == origin && next.url.as_ref().is_none_or(|known| known == url)
                    });
                    if matches { page.next.take() } else { None }
                })
            }
        };
        if let Some(continuation) = continuation {
            return self
                .resolve_request(tab, id, Resolution::Replay(continuation))
                .await;
        }
        if lock(&self.state)
            .tabs
            .get(tab)
            .is_some_and(|page| page.issue == Some(ChromeError::Rejected))
        {
            return self.resolve_request(tab, id, Resolution::Fail).await;
        }

        let request = ChromeRequest {
            tab: tab.to_owned(),
            origin: origin.clone(),
            revision,
        };
        match self.navigation.approved(&request) {
            Some(true) => {
                if let Some(page) = lock(&self.state).tabs.get_mut(tab) {
                    page.approve(origin);
                }
                self.resolve_request(tab, id, Resolution::Continue).await
            }
            Some(false) => {
                self.reject(tab);
                self.resolve_request(tab, id, Resolution::Fail).await
            }
            None => self.ask(tab, id, url, params, request).await,
        }
    }

    /// Fails the request now and asks the owner. An approval replays it.
    async fn ask(
        self: &Arc<Self>,
        tab: &str,
        id: &str,
        url: &str,
        params: &Value,
        request: ChromeRequest,
    ) -> Result<(), ChromeError> {
        let body = self.request_body(tab, id, params).await;
        let cancellation = CancellationToken::new();
        let key = {
            let mut state = lock(&self.state);
            if state.pending.len() >= MAX_PENDING_APPROVALS {
                None
            } else {
                state.next_pending = state.next_pending.wrapping_add(1);
                let key = state.next_pending.to_string();
                state.pending.insert(key.clone(), cancellation.clone());
                Some(key)
            }
        };
        self.resolve_request(tab, id, Resolution::Fail).await?;
        let (Some(key), Ok(body)) = (key, body) else {
            self.reject(tab);
            return Ok(());
        };
        let continuation = Continuation {
            origin: request.origin.clone(),
            url: Some(url.to_owned()),
            method: params["request"]["method"].as_str().map(str::to_owned),
            headers: Some(headers(&params["request"]["headers"])),
            body,
        };
        tokio::spawn(Arc::clone(self).replay(key, request, continuation, cancellation));
        Ok(())
    }

    async fn request_body(
        &self,
        tab: &str,
        id: &str,
        params: &Value,
    ) -> Result<Option<ReplayBody>, ChromeError> {
        if let Some(body) = params["request"]["postData"].as_str() {
            if body.len() > MAX_REPLAY_BODY {
                return Err(ChromeError::Limit);
            }
            return Ok(Some(ReplayBody::Text(Zeroizing::new(body.to_owned()))));
        }
        if params["request"]["hasPostData"].as_bool() != Some(true) {
            return Ok(None);
        }
        let body = self
            .wire
            .command(tab, "Fetch.getRequestPostData", json!({ "requestId": id }))
            .await?;
        match body["postData"].as_str() {
            Some(encoded) if encoded.len() <= MAX_REPLAY_BODY * 4 / 3 + 4 => {
                Ok(Some(ReplayBody::Base64(Zeroizing::new(encoded.to_owned()))))
            }
            _ => Err(ChromeError::Limit),
        }
    }

    async fn replay(
        self: Arc<Self>,
        key: String,
        request: ChromeRequest,
        continuation: Continuation,
        cancellation: CancellationToken,
    ) {
        let allowed = tokio::select! {
            biased;
            () = cancellation.cancelled() => false,
            () = self.wire.closed.cancelled() => false,
            allowed = tokio::time::timeout(APPROVAL_TIMEOUT, self.navigation.allow(request.clone())) => allowed.unwrap_or(false),
        };
        let navigate = {
            let mut state = lock(&self.state);
            let navigate = match state.tabs.get_mut(&request.tab) {
                Some(page) if allowed && !cancellation.is_cancelled() => {
                    page.approve(request.origin.clone());
                    let url = continuation.url.clone();
                    page.next = Some(continuation);
                    url
                }
                Some(page) => {
                    page.issue = Some(ChromeError::Rejected);
                    None
                }
                None => None,
            };
            state.pending.remove(&key);
            navigate
        };
        if let Some(url) = navigate
            && self
                .wire
                .command(&request.tab, "Page.navigate", json!({ "url": url }))
                .await
                .is_err()
            && let Some(page) = lock(&self.state).tabs.get_mut(&request.tab)
        {
            page.next = None;
            page.issue = Some(ChromeError::Rejected);
        }
    }

    fn reject(&self, tab: &str) {
        if let Some(page) = lock(&self.state).tabs.get_mut(tab) {
            page.issue = Some(ChromeError::Rejected);
        }
    }

    /// Fails, continues or replays a paused request.
    async fn resolve_request(
        &self,
        tab: &str,
        id: &str,
        resolution: Resolution,
    ) -> Result<(), ChromeError> {
        let (method, params) = match resolution {
            Resolution::Fail => (
                "Fetch.failRequest",
                json!({ "requestId": id, "errorReason": "BlockedByClient" }),
            ),
            Resolution::Continue => ("Fetch.continueRequest", json!({ "requestId": id })),
            Resolution::Replay(continuation) => {
                let mut params = json!({ "requestId": id });
                if let Some(method) = continuation.method {
                    params["method"] = Value::String(method);
                }
                if let Some(headers) = continuation.headers {
                    params["headers"] = headers;
                }
                if let Some(body) = continuation.body {
                    params["postData"] = Value::String(encoded_body(&body));
                }
                ("Fetch.continueRequest", params)
            }
        };
        match self.wire.command(tab, method, params).await {
            // A tab that closed meanwhile took its request with it.
            Ok(_) | Err(ChromeError::NoTab) => Ok(()),
            Err(ChromeError::Protocol) if !lock(&self.state).tabs.contains_key(tab) => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn committed(self: &Arc<Self>, tab: &str, frame: &Value) -> Result<(), ChromeError> {
        if frame["parentId"].is_string() {
            return Ok(());
        }
        let mut state = lock(&self.state);
        let State {
            tabs,
            protected_values,
            committed_hosts,
            ..
        } = &mut *state;
        let Some(page) = tabs.get_mut(tab) else {
            return Ok(());
        };
        let url = string(&frame["url"])?;
        let origin = chrome_origin(url);
        if let Some(origin) = &origin
            && !page.approved.contains(origin)
        {
            // A document that no checked request produced, such as one
            // restored from a cache that bypassed interception.
            return Err(ChromeError::Disconnected);
        }
        if origin.is_none() && url != "about:blank" && !url.starts_with("chrome-error:") {
            return Err(ChromeError::InvalidAddress);
        }
        if let Some(host) = origin.as_deref().and_then(origin_host) {
            committed_hosts.insert(host);
        }
        page.fresh = false;
        string(&frame["id"])?.clone_into(&mut page.main_frame);
        frame["loaderId"]
            .as_str()
            .unwrap_or_default()
            .clone_into(&mut page.document);
        page.tab.url = sanitized_address(url, protected_values);
        page.tab.title.clear();
        page.invalidate();
        let id = page.tab.id.clone();
        drop(state);
        tokio::spawn(Arc::clone(self).refresh_history(id));
        Ok(())
    }

    fn same_document(self: &Arc<Self>, tab: &str, params: &Value) {
        let mut state = lock(&self.state);
        let State {
            tabs,
            protected_values,
            ..
        } = &mut *state;
        let Some(page) = tabs.get_mut(tab) else {
            return;
        };
        if params["frameId"].as_str() != Some(page.main_frame.as_str()) {
            return;
        }
        if let Some(url) = params["url"].as_str() {
            if chrome_origin(url) != chrome_origin(&page.tab.url) {
                self.wire.closed.cancel();
                return;
            }
            page.tab.url = sanitized_address(url, protected_values);
            page.invalidate();
        }
        let id = page.tab.id.clone();
        drop(state);
        tokio::spawn(Arc::clone(self).refresh_history(id));
    }

    async fn refresh_history(self: Arc<Self>, id: String) {
        let Ok(context) = self.context(&id, None) else {
            return;
        };
        let Ok(history) = self
            .command(&context, "Page.getNavigationHistory", json!({}))
            .await
        else {
            return;
        };
        let mut state = lock(&self.state);
        let Some(page) = state.tabs.get_mut(&id) else {
            return;
        };
        if page.document == context.document && page.tab.url == context.url {
            let origin = |back| {
                history_entry(&history, back)
                    .and_then(|entry| entry["url"].as_str())
                    .and_then(chrome_origin)
            };
            page.tab.back = origin(true);
            page.tab.forward = origin(false);
        }
    }

    pub(super) async fn refresh_title(&self, id: &str) {
        let Ok(context) = self.context(id, None) else {
            return;
        };
        // The loaded document's AX root gives its title without selecting it.
        let Ok(tree) = self
            .command(
                &context,
                "Accessibility.getFullAXTree",
                json!({ "frameId": context.main_frame, "depth": 0 }),
            )
            .await
        else {
            return;
        };
        let Some(title) = tree["nodes"]
            .as_array()
            .and_then(|nodes| {
                nodes
                    .iter()
                    .find(|node| node["role"]["value"] == "RootWebArea")
            })
            .and_then(|root| root["name"]["value"].as_str())
        else {
            return;
        };
        let mut state = lock(&self.state);
        let State {
            tabs,
            protected_values,
            ..
        } = &mut *state;
        if let Some(page) = tabs.get_mut(id)
            && page.document == context.document
            && page.tab.url == context.url
        {
            page.tab.title = sanitized_title(title, protected_values);
        }
    }

    fn loading(&self, tab: &str, params: &Value, loading: bool) -> Option<String> {
        let mut state = lock(&self.state);
        let page = state.tabs.get_mut(tab)?;
        let main = page.fresh
            || page.main_frame.is_empty()
            || params["frameId"].as_str() == Some(page.main_frame.as_str());
        if !main {
            return None;
        }
        page.phase = match page.phase {
            PagePhase::Attaching { .. } => PagePhase::Attaching { loading },
            _ if loading => PagePhase::Loading,
            _ => PagePhase::Idle,
        };
        page.tab.loading = loading;
        (!loading).then(|| page.tab.id.clone())
    }

    pub(super) async fn settle(&self) -> Result<(), ChromeError> {
        // Give the engine a chance to report a navigation started by the
        // input, including a popup, before the operation loses its run scope.
        let settled = async {
            let mut last_activity = tokio::time::Instant::now();
            loop {
                let busy = {
                    let state = lock(&self.state);
                    !state.pending.is_empty()
                        || state
                            .tabs
                            .values()
                            .any(|page| page.phase != PagePhase::Idle)
                } || self.opening.load(Ordering::Acquire) > 0;
                if busy {
                    last_activity = tokio::time::Instant::now();
                } else if last_activity.elapsed() >= Duration::from_millis(120) {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        tokio::select! {
            () = self.wire.closed.cancelled() => Err(ChromeError::Uncertain),
            result = tokio::time::timeout(APPROVAL_TIMEOUT, settled) => result.unwrap_or(Err(ChromeError::Uncertain)),
        }
    }

    fn mark_prompt(&self, tab: &str) {
        if let Some(page) = lock(&self.state).tabs.get_mut(tab) {
            page.issue = Some(ChromeError::NativePrompt);
        }
    }

    async fn deny_auth(&self, tab: &str, params: &Value) -> Result<(), ChromeError> {
        self.mark_prompt(tab);
        self.wire
            .command(
                tab,
                "Fetch.continueWithAuth",
                json!({
                    "requestId": string(&params["requestId"])?,
                    "authChallengeResponse": { "response": "CancelAuth" },
                }),
            )
            .await?;
        Ok(())
    }

    async fn dismiss_prompt(&self, tab: &str) -> Result<(), ChromeError> {
        self.mark_prompt(tab);
        self.wire
            .command(
                tab,
                "Page.handleJavaScriptDialog",
                json!({ "accept": false }),
            )
            .await?;
        Ok(())
    }
}

/// `Fetch.continueRequest` takes the body as base64.
fn encoded_body(body: &ReplayBody) -> String {
    match body {
        ReplayBody::Text(text) => base64::engine::general_purpose::STANDARD.encode(text.as_bytes()),
        ReplayBody::Base64(encoded) => encoded.as_str().to_owned(),
    }
}

fn headers(headers: &Value) -> Value {
    let entries = headers
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, value)| {
            value
                .as_str()
                .map(|value| json!({ "name": name, "value": value }))
        })
        .collect();
    Value::Array(entries)
}

fn sanitized_title(title: &str, protected_values: &[Zeroizing<String>]) -> String {
    redact(title, protected_values)
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect()
}

pub(super) fn redact(text: &str, values: &[Zeroizing<String>]) -> String {
    let mut text = text.to_owned();
    for value in values {
        if !value.is_empty() {
            text = text.replace(value.as_str(), "[redacted]");
        }
    }
    text
}

fn string(value: &Value) -> Result<&str, ChromeError> {
    value.as_str().ok_or(ChromeError::Protocol)
}

fn origin_host(address: &str) -> Option<String> {
    url::Url::parse(address).ok()?.host_str().map(str::to_owned)
}

fn contains_protected(text: &str, values: &[Zeroizing<String>]) -> bool {
    values
        .iter()
        .any(|value| !value.is_empty() && text.contains(value.as_str()))
}

fn sanitized_address(address: &str, protected_values: &[Zeroizing<String>]) -> String {
    let Ok(mut url) = url::Url::parse(address) else {
        return String::new();
    };
    if chrome_origin(address).is_none() {
        return if address == "about:blank" {
            "about:blank".to_owned()
        } else {
            String::new()
        };
    }
    url.set_query(None);
    url.set_fragment(None);
    if contains_protected(url.path(), protected_values) {
        url.set_path("/");
    }
    url.into()
}

fn history_entry(history: &Value, back: bool) -> Option<&Value> {
    let index = usize::try_from(history["currentIndex"].as_i64()?).ok()?;
    let index = if back {
        index.checked_sub(1)?
    } else {
        index.checked_add(1)?
    };
    history["entries"].as_array()?.get(index)
}
