//! Conversation-owned browser sessions on the desktop (ADR-0064).

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::contracts::ConversationId;
use crate::points::tools::{BrowserAccess, BrowserAuthorization, BrowserControlAccess};
use crate::service::Service;
use crate::{BoxFuture, BoxStream};

/// The sensitive desktop browser service.
#[derive(Debug)]
pub enum Browser {}

impl Service for Browser {
    const NAME: &'static str = "browser";
    const MAJOR: u16 = 1;
    const SENSITIVE: bool = true;
    type Api = dyn BrowserApi;
}

/// The service bound to a plugin by the desktop.
pub trait BrowserApi: Send + Sync + fmt::Debug {
    /// Whether this system can host browser sessions. On other systems the
    /// plugin does not start (ADR-0064, decision 1).
    fn supported(&self) -> bool;
    /// Current session metadata for the owner.
    fn status(
        &self,
        conversation: ConversationId,
    ) -> BoxFuture<'static, Result<BrowserStatus, BrowserError>>;
    /// The conversation's status, first as it is and then after each
    /// change, each item complete. The stream is the lease of the session's
    /// native page views: when every stream of a conversation ends, or the
    /// app page loads again, its pages hide (ADR-0064, decision 5).
    fn watch(&self, conversation: ConversationId) -> BoxStream<'static, BrowserStatus>;
    /// Places a tab's native page view over the pane's placeholder.
    fn place(
        &self,
        conversation: ConversationId,
        placement: BrowserPlacement,
    ) -> BoxFuture<'static, Result<(), BrowserError>>;
    /// Moves keyboard focus into a tab's page, in takeover.
    fn focus(
        &self,
        conversation: ConversationId,
        tab: String,
    ) -> BoxFuture<'static, Result<(), BrowserError>>;
    /// A still image of a tab's page, which the pane shows while an overlay
    /// hides the native view. It is for the owner and never reaches a model.
    fn still(
        &self,
        conversation: ConversationId,
        tab: String,
    ) -> BoxFuture<'static, Result<Option<BrowserFrame>, BrowserError>>;
    /// Direct browser control from the owner, optionally addressed to a pane's tab.
    fn control(
        &self,
        conversation: ConversationId,
        tab: Option<String>,
        action: BrowserControl,
    ) -> BoxFuture<'static, Result<(), BrowserError>>;
    /// Change a session credential from the owner.
    fn credential(
        &self,
        conversation: ConversationId,
        action: BrowserCredential,
    ) -> BoxFuture<'static, Result<(), BrowserError>>;
    /// Compute the canonical accesses for an agent request without changing the browser.
    fn prepare(
        &self,
        conversation: ConversationId,
        request: BrowserRequest,
    ) -> BoxFuture<'static, Result<Vec<BrowserAccess>, BrowserError>>;
    /// Prepare a request to return control of the owner's live browser.
    /// Returns no access when the agent already controls the browser.
    fn prepare_control(
        &self,
        conversation: ConversationId,
    ) -> BoxFuture<'static, Result<Option<BrowserControlAccess>, BrowserError>>;
    /// Return control after the owner authorizes this exact handoff.
    fn request_control(
        &self,
        conversation: ConversationId,
        authorization: Option<BrowserAuthorization>,
    ) -> BoxFuture<'static, Result<(), BrowserError>>;
    /// Execute an agent request using authorization that the runtime issued.
    fn execute(
        &self,
        conversation: ConversationId,
        request: BrowserRequest,
        authorization: Option<BrowserAuthorization>,
    ) -> BoxFuture<'static, Result<BrowserResult, BrowserError>>;
}

/// Stable browser errors for commands and tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum BrowserError {
    /// This conversation has no browser session.
    #[error("open a browser session first")]
    SessionMissing,
    /// The WebView2 runtime was not found.
    #[error("install the Microsoft Edge WebView2 runtime to use the browser")]
    BrowserMissing,
    /// Browser startup or protocol validation failed.
    #[error("the browser engine could not start with the required capabilities")]
    BrowserIncompatible,
    /// The browser connection ended.
    #[error("the browser disconnected")]
    BrowserDisconnected,
    /// An action was sent before the browser lost its response.
    #[error("the action may have taken effect; observe the page before acting again")]
    OperationUncertain,
    /// The owner is using the browser.
    #[error(
        "the owner has control of the browser. Call browser_request_control to ask the owner to return control."
    )]
    OwnerControl,
    /// The agent is using the browser.
    #[error("take control before interacting with the browser")]
    AgentControl,
    /// The site or requested action was not authorized.
    #[error("the browser action was denied")]
    NavigationDenied,
    /// The browser requested a native prompt that Mannie cannot serve.
    #[error("this browser prompt is unsupported")]
    UnsupportedPrompt,
    /// The action's page or target changed.
    #[error(
        "the page or target changed. Call browser_observe again and use its returned revision. Request a screenshot for a point click or drag."
    )]
    StaleTarget,
    /// An input did not match the browser contract.
    #[error("the browser input is invalid")]
    InvalidBrowserInput,
    /// The conversation or tab does not exist.
    #[error("the browser conversation or tab was not found")]
    NotFound,
    /// The referenced secret cannot be used at this origin.
    #[error("this credential is unavailable at the field's origin")]
    CredentialMissing,
    /// The desktop's browser resource limit was reached.
    #[error("the browser session or tab limit was reached")]
    LimitReached,
}

impl BrowserError {
    /// The stable code carried by browser failures.
    pub fn as_code(self) -> &'static str {
        match self {
            Self::SessionMissing => "session_missing",
            Self::BrowserMissing => "browser_missing",
            Self::BrowserIncompatible => "browser_incompatible",
            Self::BrowserDisconnected => "browser_disconnected",
            Self::OperationUncertain => "operation_uncertain",
            Self::OwnerControl => "owner_control",
            Self::AgentControl => "agent_control",
            Self::NavigationDenied => "navigation_denied",
            Self::UnsupportedPrompt => "unsupported_prompt",
            Self::StaleTarget => "stale_target",
            Self::InvalidBrowserInput => "invalid_browser_input",
            Self::NotFound => "not_found",
            Self::CredentialMissing => "credential_missing",
            Self::LimitReached => "limit_reached",
        }
    }
}

impl crate::ipc::PluginError for BrowserError {
    fn code(&self) -> &'static str {
        (*self).as_code()
    }
}

/// Who can send input to a browser session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BrowserControlMode {
    /// Only the owner can interact or observe through the pane.
    Owner,
    /// Agent tools can act under the policy.
    Agent,
}

/// Tabs and control state for the browser pane.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    /// The active control mode, if a session is running.
    pub control: Option<BrowserControlMode>,
    /// The session's tabs.
    pub tabs: Vec<BrowserTab>,
    /// The selected tab's identifier.
    pub active_tab: Option<String>,
    /// The agent's latest gesture in the active tab, for the pane's
    /// activity line.
    pub activity: Option<BrowserActivity>,
    /// Set when keyboard focus left a page for the app, so the pane can put
    /// it on its own controls.
    pub focus_return: Option<BrowserFocusReturn>,
    /// Why the last session ended on its own, until a new one starts.
    pub failure: Option<BrowserError>,
    /// References to credentials provided by the owner.
    pub credentials: Vec<BrowserCredentialRef>,
}

/// The agent's latest gesture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct BrowserActivity {
    /// `click`, `fill`, `select`, `press` or `scroll`.
    pub kind: String,
    /// Changes on each reported gesture, including repeated ones.
    pub sequence: u32,
}

/// Keyboard focus that left a page for the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct BrowserFocusReturn {
    /// Changes each time focus leaves a page.
    pub sequence: u32,
    /// Whether focus moved forward, with Tab past the page's end or F6.
    pub forward: bool,
}

/// A tab in the owner view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTab {
    /// Identifier within the session.
    pub id: String,
    /// Display URL.
    pub url: String,
    /// Page title.
    pub title: String,
    /// Whether the page is loading.
    pub loading: bool,
    /// Whether the tab has a previous history entry on a web origin.
    pub can_go_back: bool,
    /// Whether the tab has a next history entry on a web origin.
    pub can_go_forward: bool,
}

/// Where the pane wants a tab's page, in the app webview's CSS pixels. The
/// core clamps it to the window's client area (ADR-0064, decision 5).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserPlacement {
    /// The tab to place.
    pub tab_id: String,
    /// The placeholder's rectangle.
    pub rect: BrowserRect,
    /// The app webview's device pixel ratio.
    pub scale: f64,
    /// Whether the page shows. An open dialog, menu, popover or select over
    /// the page hides it, and the pane shows a still in its place.
    pub visible: bool,
    /// Parts of the rectangle that an overlay such as a tooltip covers,
    /// which the native view leaves out.
    pub clip: Vec<BrowserRect>,
}

/// A rectangle in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct BrowserRect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// A credential reference. The actual value stays in the desktop host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct BrowserCredentialRef {
    /// Opaque key for use in a browser fill.
    pub reference: String,
    /// The exact origin at which it can be used.
    pub origin: String,
    /// The name the owner gave it.
    pub label: String,
}

/// A bounded JPEG of the page viewport.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BrowserFrame {
    /// Base64 JPEG bytes.
    pub jpeg: String,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
}

/// One entry of the semantic page outline.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BrowserNode {
    /// Semantic role.
    pub role: String,
    /// Accessible name or visible text.
    pub name: String,
    /// Opaque element reference for the current observation.
    pub reference: Option<String>,
    /// Whether the element can be filled.
    pub editable: bool,
}

/// A direct owner action.
#[derive(Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BrowserControl {
    /// Start an empty session in owner control.
    Start,
    /// Switch to owner control.
    TakeOver,
    /// Return to agent control.
    Resume,
    /// End the browser session.
    End,
    /// Open and select an empty tab.
    OpenTab,
    /// Select a tab.
    SelectTab {
        /// Target tab.
        tab_id: String,
    },
    /// Close a tab.
    CloseTab {
        /// Target tab.
        tab_id: String,
    },
    /// Go to a URL in the current tab.
    Navigate {
        /// Destination URL.
        url: String,
    },
    /// Open the previous history entry.
    Back,
    /// Open the next history entry.
    Forward,
    /// Reload the selected page.
    Reload,
}

impl fmt::Debug for BrowserControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrowserControl")
            .finish_non_exhaustive()
    }
}

/// Owner action on a session credential.
#[derive(Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserCredential {
    /// Give the session a credential at an exact origin.
    Set {
        /// Credential origin.
        origin: String,
        /// Name the agent sees.
        label: String,
        /// Protected value, kept in memory.
        value: String,
    },
    /// Remove a credential from the session.
    Remove {
        /// Reference to remove.
        reference: String,
    },
}

impl fmt::Debug for BrowserCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrowserCredential")
            .finish_non_exhaustive()
    }
}

/// A tool's request to the host browser.
#[derive(Debug, Serialize)]
pub enum BrowserRequest {
    /// List tab identifiers without inspecting any page.
    Tabs,
    /// Navigate or manage tabs.
    Navigate(BrowserNavigation),
    /// Observe the selected tab.
    Observe {
        /// Include a screenshot if the document allows it.
        screenshot: bool,
    },
    /// Interact with a page at the observed revision.
    Interact {
        /// Observed viewport or document revision.
        revision: String,
        /// Interaction with the selected tab.
        action: BrowserInteraction,
    },
}

/// The model's navigation and tab commands.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BrowserNavigation {
    /// Open an HTTP or HTTPS URL.
    Go {
        /// Destination URL.
        url: String,
    },
    /// Open a URL in a new tab.
    OpenTab {
        /// Destination URL.
        url: String,
    },
    /// Select a tab by ID.
    SelectTab {
        /// Tab ID.
        tab_id: String,
    },
    /// Close a tab by ID.
    CloseTab {
        /// Tab ID.
        tab_id: String,
    },
    /// Go to the previous history entry.
    Back,
    /// Go to the next history entry.
    Forward,
    /// Reload the selected tab.
    Reload,
}

/// A page interaction requested by the model.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserInteraction {
    /// Click a target.
    Click {
        /// Element or point to click.
        target: BrowserTarget,
    },
    /// Drag from one element or point to another, as a person drags with
    /// the mouse. It works with HTML drag and drop and with pointer-based
    /// sorting. Both ends come from the latest observation.
    Drag {
        /// Where the drag starts.
        from: BrowserTarget,
        /// Where it ends.
        to: BrowserTarget,
    },
    /// Click several distinct elements from one observation in order.
    ClickMany {
        /// Two to sixteen element references from the same observation.
        #[schemars(length(min = 2, max = 16))]
        references: Vec<String>,
    },
    /// Fill an editable element.
    Fill {
        /// Element to fill.
        reference: String,
        /// Text or a credential reference.
        value: BrowserFillValue,
    },
    /// Select an option in a native select element.
    Select {
        /// Element to use.
        reference: String,
        /// Option value.
        value: String,
    },
    /// Press a key in the focused control, such as Enter after a fill. It
    /// needs no new observation.
    Press {
        /// One of `Enter`, `Tab`, `Escape`, `Space`, `ArrowLeft`, `ArrowUp`,
        /// `ArrowRight`, `ArrowDown`, `Backspace`, `Delete`, `Home` or `End`.
        /// Enter text with `fill`.
        key: String,
    },
    /// Scroll the viewport.
    Scroll {
        /// Horizontal delta in the page's CSS pixels.
        x: i32,
        /// Vertical delta in the page's CSS pixels.
        y: i32,
    },
}

/// A target inside the observed viewport.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserTarget {
    /// A reference returned by the browser observation.
    Element {
        /// Opaque element reference.
        reference: String,
    },
    /// Pixels of the observed screenshot, from its top left.
    Point {
        /// Horizontal coordinate.
        x: i32,
        /// Vertical coordinate.
        y: i32,
    },
}

/// Value to enter in an editable element.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserFillValue {
    /// Ordinary, nonsensitive text.
    Text {
        /// Text to enter.
        text: String,
    },
    /// A secret supplied through the Browser pane.
    Credential {
        /// Origin-bound secret reference.
        reference: String,
    },
}

/// Output that the service returns to an agent tool.
#[derive(Debug)]
pub struct BrowserResult {
    /// Bounded browser metadata and structured observation.
    pub data: serde_json::Value,
    /// The optional viewport image for a model that supports images.
    pub frame: Option<BrowserFrame>,
    /// A batch can report completed work along with its first failure.
    pub failure: Option<BrowserError>,
}
