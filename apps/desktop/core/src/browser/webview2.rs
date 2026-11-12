//! The WebView2 host of browser sessions (ADR-0064, decisions 2 to 6).
//!
//! The core creates these webviews itself with `webview2-com`, outside
//! Tauri's webview manager, so a page gets no IPC bridge, no host objects and
//! no script from Mannie. Each session has its own environment, user data
//! folder and kill-on-close job; each tab is a controller in a child window
//! of the main window.
//!
//! WebView2 is usable only on the thread that created it. Every call to it
//! runs on the main thread through [`UiThread::post`] and never waits there:
//! a command's completion handler answers a oneshot, and its events go into
//! one bounded channel. The COM objects live in a thread-local registry on
//! the main thread, and nothing borrows the registry across a call into
//! WebView2, whose handlers may run inside such a call.
#![expect(
    unsafe_code,
    reason = "WebView2's COM API is unsafe throughout; this module is the exception that the engineering standards and ADR-0064, decision 11, allow"
)]

mod cdp;
mod cursor;
mod environment;
mod surface;
mod tab;
#[cfg(test)]
mod tests;
#[cfg(feature = "validation")]
mod validation;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::services::browser::{BrowserFocusReturn, BrowserPlacement};
use mannie_tools::browser::{
    BrowserFolder, ChromeError, ChromeEvent, ChromeHost, ChromeReply, ProcessTreeJob,
    process_running, terminate_process,
};
use tauri::{AppHandle, Manager};
use tokio::sync::{mpsc, oneshot};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, ICoreWebView2Controller, ICoreWebView2Deferral, ICoreWebView2Environment,
    ICoreWebView2NewWindowRequestedEventArgs,
};
use windows::Win32::Foundation::HWND;
use zeroize::Zeroizing;

use super::engine::{PageViews, SessionHost, StartedSession};
use crate::window::MAIN_WINDOW;

/// The most events that wait for the driver. A full queue ends the session,
/// because a dropped `Fetch.requestPaused` would leave a request ungoverned.
const EVENTS: usize = 256;
/// How long teardown waits for a session's processes before it ends the
/// ones outside its job one by one.
const EXIT_GRACE: Duration = Duration::from_secs(3);

/// Removes the variables through which WebView2 would give a session another
/// folder, browser or debugging port (ADR-0064, decision 3). The app
/// webview read them when the main window was created, so tauri-driver still
/// reaches it.
pub(crate) fn clear_engine_variables() {
    let names: Vec<_> = std::env::vars_os()
        .map(|(name, _)| name)
        .filter(|name| {
            name.to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("WEBVIEW2_")
        })
        .collect();
    for name in names {
        // SAFETY: on Windows the standard library changes the environment
        // with SetEnvironmentVariableW under its own lock, and the system
        // serializes every reader and writer of the process environment, so
        // a concurrent reader sees the block before or after the change.
        unsafe { std::env::remove_var(&name) };
    }
}

/// Whether the Evergreen WebView2 runtime is installed.
pub(crate) fn supported() -> bool {
    environment::runtime_version().is_some()
}

/// The thread where every WebView2 call runs: the app's main thread, or a
/// test's own message loop.
pub(super) trait UiDispatch: Send + Sync {
    /// Runs `task` on the thread later. Fails once its loop has ended.
    fn post(&self, task: Box<dyn FnOnce() + Send>) -> Result<(), ChromeError>;
    /// The window that pages are children of, on the thread.
    fn main_window(&self) -> Option<HWND>;
}

/// The app's main thread through Tauri.
struct TauriUi {
    app: AppHandle,
}

impl UiDispatch for TauriUi {
    fn post(&self, task: Box<dyn FnOnce() + Send>) -> Result<(), ChromeError> {
        self.app
            .run_on_main_thread(task)
            .map_err(|_closed| ChromeError::Disconnected)
    }

    fn main_window(&self) -> Option<HWND> {
        let window = self.app.get_webview_window(MAIN_WINDOW)?;
        // Tauri's handle type may come from another version of the windows
        // crate; the raw pointer is the same handle.
        window.hwnd().ok().map(|hwnd| HWND(hwnd.0))
    }
}

/// The thread where every WebView2 call runs.
#[derive(Clone)]
pub(super) struct UiThread {
    dispatch: Arc<dyn UiDispatch>,
}

impl UiThread {
    pub(super) fn new(app: AppHandle) -> Self {
        Self {
            dispatch: Arc::new(TauriUi { app }),
        }
    }

    #[cfg(test)]
    fn with(dispatch: Arc<dyn UiDispatch>) -> Self {
        Self { dispatch }
    }

    /// Runs `task` on the UI thread later. Fails once its loop has ended, at
    /// quit.
    fn post(&self, task: impl FnOnce() + Send + 'static) -> Result<(), ChromeError> {
        self.dispatch.post(Box::new(task))
    }

    /// The main window's handle, on the UI thread.
    fn main_window(&self) -> Option<HWND> {
        self.dispatch.main_window()
    }
}

/// What a session shares between the main thread and the driver's tasks.
pub(super) struct Shared {
    id: u64,
    ui: UiThread,
    events: Mutex<Option<mpsc::Sender<ChromeEvent>>>,
    tabs: Mutex<HashSet<String>>,
    processes: Mutex<HashSet<u32>>,
    job: ProcessTreeJob,
    folder: Mutex<Option<BrowserFolder>>,
    ended: AtomicBool,
    owner: AtomicBool,
    next_tab: AtomicU64,
    next_popup: AtomicU64,
    focus_sequence: AtomicU32,
    focus_return: Mutex<Option<BrowserFocusReturn>>,
}

impl Shared {
    /// Hands an event to the driver. A full queue or a closed driver ends
    /// the session: closing the channel makes the driver end it.
    fn emit(&self, event: ChromeEvent) {
        let mut events = lock(&self.events);
        let delivered = events
            .as_ref()
            .is_some_and(|sender| sender.try_send(event).is_ok());
        if !delivered && events.take().is_some() {
            tracing::warn!("a browser session's events overflowed; the session ends");
        }
    }

    /// Ends the session from the engine's side, for example after its
    /// browser process exited.
    fn fail(&self) {
        lock(&self.events).take();
    }

    #[cfg(test)]
    fn folder_exists(&self) -> bool {
        lock(&self.folder).is_some()
    }

    /// Records that focus left a page and moves it to the app. Called from a
    /// WebView2 handler, while the browser process waits for it: moving the
    /// focus there would send a message to that waiting process's window, so
    /// the move runs after the handler returned.
    fn focus_left(&self, forward: bool) {
        let sequence = self.focus_sequence.fetch_add(1, Ordering::AcqRel) + 1;
        *lock(&self.focus_return) = Some(BrowserFocusReturn { sequence, forward });
        let ui = self.ui.clone();
        let posted = self.ui.post(move || surface::focus_main_window(&ui));
        if posted.is_err() {
            tracing::debug!("the event loop ended before focus returned to the app");
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The COM objects of a session, on the main thread.
struct SessionUi {
    environment: ICoreWebView2Environment,
    parent: HWND,
    tabs: HashMap<String, TabUi>,
    popups: HashMap<String, PendingPopup>,
}

#[derive(Clone)]
struct TabUi {
    container: HWND,
    controller: ICoreWebView2Controller,
    webview: ICoreWebView2,
    cursor: Option<HWND>,
}

struct PendingPopup {
    args: ICoreWebView2NewWindowRequestedEventArgs,
    deferral: ICoreWebView2Deferral,
}

thread_local! {
    /// The sessions' COM objects. Only the main thread touches them.
    static SESSIONS: RefCell<HashMap<u64, SessionUi>> = RefCell::new(HashMap::new());
}

/// Reads a session's tab without holding the registry across the caller's
/// WebView2 calls.
fn tab_ui(session: u64, tab: &str) -> Option<TabUi> {
    SESSIONS.with_borrow(|sessions| sessions.get(&session)?.tabs.get(tab).cloned())
}

fn session_tabs(session: u64) -> Vec<(String, TabUi)> {
    SESSIONS.with_borrow(|sessions| {
        sessions
            .get(&session)
            .map(|ui| {
                ui.tabs
                    .iter()
                    .map(|(id, tab)| (id.clone(), tab.clone()))
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// A browser session on WebView2.
pub(super) struct WebView2Session {
    shared: Arc<Shared>,
}

/// Creates the engine of new sessions.
pub(super) async fn start(ui: UiThread, root: &Path) -> Result<StartedSession, ChromeError> {
    let (session, events) = start_session(ui, root).await?;
    Ok(StartedSession {
        host: session,
        events,
    })
}

async fn start_session(
    ui: UiThread,
    root: &Path,
) -> Result<(Arc<WebView2Session>, mpsc::Receiver<ChromeEvent>), ChromeError> {
    static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);
    let folder = BrowserFolder::create(root).map_err(|error| {
        tracing::warn!(%error, "could not create a browser session folder");
        ChromeError::Profile
    })?;
    let job = ProcessTreeJob::new().map_err(|error| {
        tracing::warn!(%error, "could not create a browser session job");
        ChromeError::Launch
    })?;
    let (events, receiver) = mpsc::channel(EVENTS);
    let data = folder.data().to_path_buf();
    let shared = Arc::new(Shared {
        id: NEXT_SESSION.fetch_add(1, Ordering::Relaxed),
        ui: ui.clone(),
        events: Mutex::new(Some(events)),
        tabs: Mutex::new(HashSet::new()),
        processes: Mutex::new(HashSet::new()),
        job,
        folder: Mutex::new(Some(folder)),
        ended: AtomicBool::new(false),
        owner: AtomicBool::new(false),
        next_tab: AtomicU64::new(1),
        next_popup: AtomicU64::new(1),
        focus_sequence: AtomicU32::new(0),
        focus_return: Mutex::new(None),
    });
    let session = WebView2Session {
        shared: Arc::clone(&shared),
    };
    let (send, created) = oneshot::channel();
    let creating = Arc::clone(&shared);
    let posted = ui.post(move || environment::create(&creating, &data, send));
    let created = match posted {
        Ok(()) => created.await.unwrap_or(Err(ChromeError::Launch)),
        Err(error) => Err(error),
    };
    if let Err(error) = created {
        session.end().await;
        return Err(error);
    }
    let debugging = lock(&shared.folder)
        .as_ref()
        .is_some_and(BrowserFolder::has_debugging_port);
    if debugging {
        tracing::error!("a browser session opened a debugging port; it is refused");
        session.end().await;
        return Err(ChromeError::Incompatible);
    }
    Ok((Arc::new(session), receiver))
}

impl ChromeHost for WebView2Session {
    fn send(
        &self,
        tab: &str,
        method: &str,
        params: Zeroizing<String>,
    ) -> Result<ChromeReply, ChromeError> {
        if self.shared.ended.load(Ordering::Acquire) {
            return Err(ChromeError::Disconnected);
        }
        if !lock(&self.shared.tabs).contains(tab) {
            return Err(ChromeError::NoTab);
        }
        let (reply, answer) = oneshot::channel();
        let session = self.shared.id;
        let tab = tab.to_owned();
        let method = method.to_owned();
        self.shared
            .ui
            .post(move || cdp::call(session, &tab, &method, params, reply))?;
        Ok(answer)
    }

    fn create_tab(&self) -> BoxFuture<'static, Result<String, ChromeError>> {
        let shared = Arc::clone(&self.shared);
        Box::pin(async move {
            if shared.ended.load(Ordering::Acquire) {
                return Err(ChromeError::Disconnected);
            }
            let id = format!("tab-{}", shared.next_tab.fetch_add(1, Ordering::Relaxed));
            let (send, created) = oneshot::channel();
            let creating = Arc::clone(&shared);
            let tab = id.clone();
            shared.ui.post(move || tab::create(&creating, tab, send))?;
            created.await.unwrap_or(Err(ChromeError::Disconnected))?;
            Ok(id)
        })
    }

    fn release_popup(&self, popup: &str, tab: &str) -> Result<(), ChromeError> {
        let session = self.shared.id;
        let popup = popup.to_owned();
        let tab = tab.to_owned();
        self.shared
            .ui
            .post(move || tab::release_popup(session, &popup, Some(&tab)))
    }

    fn refuse_popup(&self, popup: &str) {
        let session = self.shared.id;
        let popup = popup.to_owned();
        let posted = self
            .shared
            .ui
            .post(move || tab::release_popup(session, &popup, None));
        if posted.is_err() {
            tracing::debug!("the event loop ended before a popup was refused");
        }
    }

    fn close_tab(&self, tab: &str) -> BoxFuture<'static, Result<(), ChromeError>> {
        lock(&self.shared.tabs).remove(tab);
        let session = self.shared.id;
        let tab = tab.to_owned();
        let posted = self.shared.ui.post(move || tab::close(session, &tab));
        Box::pin(std::future::ready(posted))
    }

    fn end(&self) -> BoxFuture<'static, ()> {
        let shared = Arc::clone(&self.shared);
        Box::pin(async move { end_session(&shared).await })
    }
}

/// Ends a session's processes first, then its views, then its folder. A
/// graceful close could let a request go, and at quit the event loop is
/// already gone, so the job ends before anything else (ADR-0064, decision 4).
async fn end_session(shared: &Arc<Shared>) {
    if shared.ended.swap(true, Ordering::AcqRel) {
        return;
    }
    lock(&shared.tabs).clear();
    shared.fail();
    if let Err(error) = shared.job.terminate() {
        tracing::warn!(%error, "could not terminate a browser session's job");
    }
    let processes: Vec<u32> = lock(&shared.processes).iter().copied().collect();
    let deadline = tokio::time::Instant::now() + EXIT_GRACE;
    loop {
        let job_empty = shared.job.active_processes().is_ok_and(|count| count == 0);
        let running: Vec<u32> = processes
            .iter()
            .copied()
            .filter(|process| process_running(*process))
            .collect();
        if job_empty && running.is_empty() {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            // The engine's sandboxed processes left the job; they follow
            // their browser process, and this ends any that did not.
            for process in running {
                terminate_process(process);
            }
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let session = shared.id;
    if shared.ui.post(move || tab::close_session(session)).is_err() {
        tracing::debug!("the event loop ended before a browser session's views closed");
    }
    let folder = lock(&shared.folder).take();
    if let Some(folder) = folder {
        folder.remove();
    }
}

impl PageViews for WebView2Session {
    fn place(&self, placement: BrowserPlacement) -> Result<(), ChromeError> {
        if !lock(&self.shared.tabs).contains(&placement.tab_id) {
            return Err(ChromeError::NoTab);
        }
        let session = self.shared.id;
        self.shared
            .ui
            .post(move || surface::place(session, &placement))
    }

    fn hide_all(&self) {
        let session = self.shared.id;
        if self
            .shared
            .ui
            .post(move || surface::hide_all(session))
            .is_err()
        {
            tracing::debug!("the event loop ended before the browser views hid");
        }
    }

    fn set_owner(&self, owner: bool) -> Result<(), ChromeError> {
        self.shared.owner.store(owner, Ordering::Release);
        let shared = Arc::clone(&self.shared);
        self.shared
            .ui
            .post(move || surface::set_owner(&shared, owner))
    }

    fn focus(&self, tab: &str) -> Result<(), ChromeError> {
        if !self.shared.owner.load(Ordering::Acquire) {
            return Err(ChromeError::Rejected);
        }
        if !lock(&self.shared.tabs).contains(tab) {
            return Err(ChromeError::NoTab);
        }
        let session = self.shared.id;
        let tab = tab.to_owned();
        self.shared.ui.post(move || surface::focus(session, &tab))
    }

    fn focus_return(&self) -> Option<BrowserFocusReturn> {
        *lock(&self.shared.focus_return)
    }

    #[cfg(feature = "validation")]
    fn surfaces(&self) -> BoxFuture<'static, Vec<super::engine::SurfaceReport>> {
        let (reply, reports) = oneshot::channel();
        let shared = Arc::clone(&self.shared);
        let posted = self
            .shared
            .ui
            .post(move || validation::surfaces(&shared, reply));
        Box::pin(async move {
            if posted.is_err() {
                return Vec::new();
            }
            reports.await.unwrap_or_default()
        })
    }
}

impl SessionHost for WebView2Session {}
