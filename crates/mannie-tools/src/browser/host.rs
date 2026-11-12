//! The seam between the portable driver and the engine that hosts its tabs
//! (ADR-0063, decision 2).
//!
//! The desktop core implements [`ChromeHost`] with WebView2, where each tab
//! has a `DevTools` endpoint of its own. The driver's tests use a scripted
//! host. Nothing here depends on a webview.

use std::fmt;

use futures_util::future::BoxFuture;
use serde_json::Value;
use tokio::sync::oneshot;
use zeroize::Zeroizing;

use super::ChromeError;

/// How a `DevTools` command ended at the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromeHostFailure {
    /// The engine answered with an error for this command.
    Protocol,
    /// The tab or the session is gone, or the host could not deliver the
    /// command.
    Disconnected,
}

/// The answer to one command: the `result` object of the `DevTools` response.
pub type ChromeReply = oneshot::Receiver<Result<Value, ChromeHostFailure>>;

/// An engine that hosts one browser session's tabs.
///
/// Every method returns without waiting for the engine. A command's answer
/// arrives on its [`ChromeReply`]; a reply whose sender is dropped means the
/// tab or the session ended.
pub trait ChromeHost: Send + Sync {
    /// Sends a `DevTools` command to one tab. `params` is the JSON object of
    /// the command's parameters, which the host wipes after use.
    ///
    /// # Errors
    ///
    /// [`ChromeError::NoTab`] for an unknown tab and
    /// [`ChromeError::Disconnected`] when the session has ended.
    fn send(
        &self,
        tab: &str,
        method: &str,
        params: Zeroizing<String>,
    ) -> Result<ChromeReply, ChromeError>;

    /// Creates a blank tab that has not navigated yet, and returns its ID.
    fn create_tab(&self) -> BoxFuture<'static, Result<String, ChromeError>>;

    /// Hands the deferred new window `popup` the tab `tab`, which the driver
    /// set up for it. The engine then loads the opener's request in `tab`.
    ///
    /// # Errors
    ///
    /// [`ChromeError::NoTab`] when the popup or the tab is gone.
    fn release_popup(&self, popup: &str, tab: &str) -> Result<(), ChromeError>;

    /// Refuses the deferred new window `popup`.
    fn refuse_popup(&self, popup: &str);

    /// Closes a tab.
    fn close_tab(&self, tab: &str) -> BoxFuture<'static, Result<(), ChromeError>>;

    /// Ends the session: its processes first, then its folder.
    fn end(&self) -> BoxFuture<'static, ()>;
}

/// What the host reports about a session's tabs.
pub enum ChromeEvent {
    /// A `DevTools` event of one tab.
    Protocol {
        /// The tab.
        tab: String,
        /// The event's method, such as `Fetch.requestPaused`.
        method: String,
        /// The event's parameters.
        params: Value,
    },
    /// A page asked for a new window. The host holds it until the driver
    /// releases or refuses it.
    Popup {
        /// The deferred request's ID.
        popup: String,
        /// The tab that asked.
        opener: String,
    },
    /// A tab's document title changed.
    Title {
        /// The tab.
        tab: String,
        /// The new title.
        title: String,
    },
    /// The owner put keyboard focus into a tab.
    Focused {
        /// The tab.
        tab: String,
    },
    /// The engine closed a tab, for example after `window.close()`.
    TabClosed {
        /// The tab.
        tab: String,
    },
}

impl fmt::Debug for ChromeEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Self::Protocol { .. } => "Protocol",
            Self::Popup { .. } => "Popup",
            Self::Title { .. } => "Title",
            Self::Focused { .. } => "Focused",
            Self::TabClosed { .. } => "TabClosed",
        };
        formatter
            .debug_struct("ChromeEvent")
            .field("kind", &kind)
            .finish_non_exhaustive()
    }
}
