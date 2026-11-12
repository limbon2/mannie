//! The engine that hosts browser sessions: WebView2 on Windows, and none on
//! other systems, where the Browser plugin does not start (ADR-0064,
//! decision 1).

use std::path::PathBuf;
use std::sync::Arc;

use mannie_plugin_sdk::services::browser::{BrowserFocusReturn, BrowserPlacement};
use mannie_tools::browser::{ChromeError, ChromeEvent, ChromeHost};
use tauri::AppHandle;
use tokio::sync::mpsc;

/// The native views of a session's tabs, which the pane places.
pub(crate) trait PageViews: Send + Sync {
    /// Moves a tab's view over the pane's placeholder, or hides it.
    fn place(&self, placement: BrowserPlacement) -> Result<(), ChromeError>;
    /// Hides every view of the session, when its last lease ends.
    fn hide_all(&self);
    /// Gives the pages to the owner, or disables them for the agent after
    /// focus returned to the app (ADR-0064, decision 6).
    fn set_owner(&self, owner: bool) -> Result<(), ChromeError>;
    /// Moves keyboard focus into a tab's page, in takeover.
    fn focus(&self, tab: &str) -> Result<(), ChromeError>;
    /// The latest time keyboard focus left a page for the app.
    fn focus_return(&self) -> Option<BrowserFocusReturn>;
    /// Where each tab's window is, for the end-to-end tests (V13).
    #[cfg(feature = "validation")]
    fn surfaces(&self) -> mannie_plugin_sdk::BoxFuture<'static, Vec<SurfaceReport>>;
}

/// A tab's window as the end-to-end tests see it (V13).
#[cfg(feature = "validation")]
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SurfaceReport {
    /// The tab.
    pub(crate) tab: String,
    /// Whether the window shows.
    pub(crate) visible: bool,
    /// The visible agent cursor's hotspot in page-window pixels.
    pub(crate) cursor: Option<(i32, i32)>,
    /// Whether owner input reaches it.
    pub(crate) enabled: bool,
    /// Whether it has the keyboard focus.
    pub(crate) focused: bool,
    /// Its place in the main window's client area, in pixels.
    pub(crate) x: i32,
    /// As above.
    pub(crate) y: i32,
    /// Its size in pixels.
    pub(crate) width: i32,
    /// As above.
    pub(crate) height: i32,
}

/// A session's engine: the driver's host and the pane's views.
pub(crate) trait SessionHost: ChromeHost + PageViews {}

/// A session that the engine started, with the events of its tabs.
pub(crate) struct StartedSession {
    pub(crate) host: Arc<dyn SessionHost>,
    pub(crate) events: mpsc::Receiver<ChromeEvent>,
}

impl std::fmt::Debug for StartedSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StartedSession")
            .finish_non_exhaustive()
    }
}

/// Starts browser sessions in folders under one root.
#[derive(Clone)]
pub(crate) struct Engine {
    root: PathBuf,
    #[cfg(windows)]
    ui: super::webview2::UiThread,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Engine")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl Engine {
    pub(crate) fn new(app: AppHandle, root: PathBuf) -> Self {
        #[cfg(not(windows))]
        drop(app);
        Self {
            root,
            #[cfg(windows)]
            ui: super::webview2::UiThread::new(app),
        }
    }

    /// Whether this system can host sessions.
    pub(crate) fn supported() -> bool {
        #[cfg(windows)]
        {
            super::webview2::supported()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// Starts a session with its own folder, processes and job.
    #[cfg(windows)]
    pub(crate) async fn start(engine: &Self) -> Result<StartedSession, ChromeError> {
        if !Self::supported() {
            return Err(ChromeError::NotInstalled);
        }
        super::webview2::start(engine.ui.clone(), &engine.root).await
    }

    /// Reports that this platform has no browser engine.
    #[cfg(not(windows))]
    pub(crate) fn start(_engine: &Self) -> std::future::Ready<Result<StartedSession, ChromeError>> {
        std::future::ready(Err(ChromeError::Incompatible))
    }
}

/// Removes the engine's variables from the process environment once the
/// main window exists (ADR-0064, decision 3).
pub(crate) fn clear_engine_variables() {
    #[cfg(windows)]
    super::webview2::clear_engine_variables();
}
