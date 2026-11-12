//! A guarded browser session over a host that holds its tabs (ADR-0064).
//!
//! The driver is portable: [`ChromeHost`] is the engine, which the desktop
//! core implements with WebView2 and the tests script.

use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use futures_util::future::BoxFuture;

pub use crate::supervisor::browser::{BrowserFolder, cleanup_browser_profiles};
#[cfg(windows)]
pub use crate::supervisor::windows::{ProcessTreeJob, process_running, terminate_process};
pub use driver::Chrome;
pub use host::{ChromeEvent, ChromeHost, ChromeHostFailure, ChromeReply};

mod driver;
mod host;
mod input;
mod observation;
mod wire;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Canonical exact origin for an address the browser can open.
#[must_use]
pub fn chrome_origin(address: &str) -> Option<String> {
    if address.len() > 8192 {
        return None;
    }
    let address = url::Url::parse(address).ok()?;
    if !matches!(address.scheme(), "http" | "https")
        || !address.username().is_empty()
        || address.password().is_some()
    {
        return None;
    }
    Some(address.origin().ascii_serialization())
}

/// A top-level request to check. The engine has not sent it.
#[derive(Clone, Debug)]
pub struct ChromeRequest {
    /// Tab receiving the new document.
    pub tab: String,
    /// Exact origin to authorize.
    pub origin: String,
    /// The page state when the request was paused.
    pub revision: String,
}

/// The desktop host's decision about the exact request before it continues.
pub trait ChromeNavigation: Send + Sync {
    /// Decides at once when that needs nobody: `Some(true)` for an origin
    /// that the current authorization or the owner's control covers,
    /// `Some(false)` for a refusal, and `None` when the owner must decide.
    fn approved(&self, request: &ChromeRequest) -> Option<bool>;

    /// Returns true once the owner authorized the request and the host
    /// journaled its continuation.
    fn allow(&self, request: ChromeRequest) -> BoxFuture<'_, bool>;
}

/// A tab and the origin of each available history destination.
#[derive(Clone, Debug)]
pub struct ChromeTab {
    /// Opaque tab identifier within the session.
    pub id: String,
    /// Sanitized address for display.
    pub url: String,
    /// Page title.
    pub title: String,
    /// Changes with navigation, snapshot invalidation and owner takeover.
    pub revision: String,
    /// Exact origin of the previous entry, when it is an eligible address.
    pub back: Option<String>,
    /// Exact origin of the next entry, when it is an eligible address.
    pub forward: Option<String>,
    /// Whether the tab's document is loading.
    pub loading: bool,
}

/// The agent's latest gesture in a tab, for the owner's activity line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChromeActivity {
    /// What the agent did: `click`, `drag`, `fill`, `select`, `press` or
    /// `scroll`.
    pub kind: &'static str,
    /// Changes for each reported gesture.
    pub sequence: u32,
}

/// A navigation request from the owner or the agent.
pub enum ChromeNavigationAction {
    /// Navigate to an explicit web address.
    Go(String),
    /// Navigate back.
    Back,
    /// Navigate forward.
    Forward,
    /// Reload the current page.
    Reload,
}

/// A rendered viewport as a base64-encoded JPEG.
pub struct ChromeFrame {
    /// The encoded JPEG.
    pub jpeg: String,
    /// Encoded image width in pixels.
    pub width: u32,
    /// Encoded image height in pixels.
    pub height: u32,
}

/// Progress from a stopped batch of agent clicks.
#[derive(Debug)]
pub struct ChromeBatchFailure {
    /// Number of clicks completed before the batch stopped.
    pub completed: usize,
    /// Zero-based index of the first click that could not be completed.
    pub failed_index: usize,
    /// The reason the batch stopped.
    pub error: ChromeError,
}

/// One item in the semantic page outline.
pub struct ChromeNode {
    /// Accessibility role.
    pub role: String,
    /// Accessible name with sensitive values omitted.
    pub name: String,
    /// A reference tied to the observation, when the item can be targeted.
    pub reference: Option<String>,
    /// Reported binary state of a checkbox, when available.
    pub checked: Option<bool>,
}

/// Whether the viewport image was available to this observer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromeScreenshotStatus {
    /// The observer did not request an image.
    NotRequested,
    /// The document has contained a protected value.
    Withheld,
    /// The engine could not return a bounded image of this viewport.
    Unavailable,
    /// The viewport image was captured.
    Returned,
}

/// An observation of one browser document.
pub struct ChromeObservation {
    /// The tab and document observed.
    pub tab: ChromeTab,
    /// Optional screenshot.
    pub frame: Option<ChromeFrame>,
    /// Whether the screenshot was requested and captured.
    pub screenshot: ChromeScreenshotStatus,
    /// Bounded semantic outline.
    pub outline: Vec<ChromeNode>,
    /// Bounded visible text.
    pub text: String,
}

/// A target from an observation.
#[derive(Debug)]
pub enum ChromeTarget {
    /// A semantic reference from the latest observation.
    Element(String),
    /// A point in the screenshot's pixels.
    Point {
        /// Horizontal position.
        x: i32,
        /// Vertical position.
        y: i32,
    },
}

/// Input the browser accepts. Its Debug output omits text.
pub enum ChromeInput {
    /// Click a semantic target or point.
    Click(ChromeTarget),
    /// Drag from one target to another.
    Drag {
        /// Where the drag starts.
        from: ChromeTarget,
        /// Where it ends.
        to: ChromeTarget,
    },
    /// Replace a field's text.
    Fill {
        /// The field.
        target: ChromeTarget,
        /// The new text.
        text: String,
        /// A protected session value that must be redacted.
        protected: bool,
    },
    /// Select an option in a native select control.
    Select {
        /// The select control.
        target: ChromeTarget,
        /// Option value.
        value: String,
    },
    /// Press and release a supported key.
    Press {
        /// Key name.
        key: String,
    },
    /// Scroll the document by these offsets.
    Scroll {
        /// Horizontal delta.
        x: i32,
        /// Vertical delta.
        y: i32,
    },
}

/// A browser failure suitable for dispatch and UI presentation.
#[derive(Clone, Copy, Debug, thiserror::Error, PartialEq, Eq)]
pub enum ChromeError {
    /// The engine's runtime was not found.
    #[error("the WebView2 runtime is missing")]
    NotInstalled,
    /// The session's private folder could not be created or used.
    #[error("the isolated browser folder could not be created")]
    Profile,
    /// The engine could not start.
    #[error("the browser could not start")]
    Launch,
    /// The engine lacks required control features.
    #[error("the browser engine is incompatible")]
    Incompatible,
    /// The browser stopped before an action was sent.
    #[error("the browser disconnected")]
    Disconnected,
    /// A protocol command failed with a definite error.
    #[error("the browser could not perform the requested operation")]
    Protocol,
    /// The request or the target was refused.
    #[error("the browser request was refused")]
    Rejected,
    /// The observation or target has changed.
    #[error("the browser page changed. Observe it again")]
    Stale,
    /// A point needs the latest observation to contain an image.
    #[error("the latest observation has no screenshot")]
    ScreenshotRequired,
    /// The live viewport geometry differs from the screenshot's.
    #[error("the browser viewport changed since the screenshot")]
    ViewportChanged,
    /// A screenshot point is outside the image or the page viewport.
    #[error("the point is outside the screenshot or page viewport")]
    PointOutOfBounds,
    /// The target is outside the authorized frame.
    #[error("the target is in a frame on another origin")]
    CrossOriginTarget,
    /// The page requested a native prompt.
    #[error("the site opened a browser prompt that this session cannot handle")]
    NativePrompt,
    /// A dispatched action may have run.
    #[error("the browser lost the action result. Observe the page before continuing")]
    Uncertain,
    /// The address is outside the browser's HTTP or HTTPS scope.
    #[error("the address must be an HTTP or HTTPS URL")]
    InvalidAddress,
    /// A bounded browser resource is full.
    #[error("the browser session reached its limit")]
    Limit,
    /// The browser tab has ended.
    #[error("the browser tab is unavailable")]
    NoTab,
}

impl ChromeError {
    /// Stable presentation code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::NotInstalled => "browser_not_installed",
            Self::Profile => "browser_profile_failed",
            Self::Launch => "browser_launch_failed",
            Self::Incompatible => "browser_incompatible",
            Self::Disconnected => "browser_disconnected",
            Self::Protocol => "browser_protocol",
            Self::Rejected => "browser_rejected",
            Self::Stale => "browser_stale",
            Self::ScreenshotRequired => "browser_screenshot_required",
            Self::ViewportChanged => "browser_viewport_changed",
            Self::PointOutOfBounds => "browser_point_out_of_bounds",
            Self::CrossOriginTarget => "browser_cross_origin_target",
            Self::NativePrompt => "browser_native_prompt",
            Self::Uncertain => "browser_uncertain",
            Self::InvalidAddress => "browser_invalid_address",
            Self::Limit => "browser_limit",
            Self::NoTab => "browser_no_tab",
        }
    }
}

impl fmt::Debug for ChromeNavigationAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Self::Go(_) => "Go",
            Self::Back => "Back",
            Self::Forward => "Forward",
            Self::Reload => "Reload",
        };
        f.write_str(kind)
    }
}

impl fmt::Debug for ChromeFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChromeFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for ChromeNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChromeNode").finish_non_exhaustive()
    }
}

impl fmt::Debug for ChromeObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChromeObservation").finish_non_exhaustive()
    }
}

impl fmt::Debug for ChromeInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChromeInput").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::chrome_origin;

    #[test]
    fn origins_distinguish_scheme_and_port_and_exclude_local_files() {
        assert_eq!(
            chrome_origin("HTTP://localhost:8123/path?secret=value").as_deref(),
            Some("http://localhost:8123")
        );
        assert_eq!(
            chrome_origin("https://EXAMPLE.com:443/").as_deref(),
            Some("https://example.com")
        );
        assert_eq!(chrome_origin("file:///etc/passwd"), None);
        assert_eq!(chrome_origin("https://user:password@example.com"), None);
    }
}
