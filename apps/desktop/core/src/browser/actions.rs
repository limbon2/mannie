//! Canonical browser accesses and the translation to the driver.

use std::collections::HashSet;

use mannie_plugin_sdk::contracts::ConversationId;
use mannie_plugin_sdk::points::tools::BrowserAction;
use mannie_plugin_sdk::services::browser::{
    BrowserControl, BrowserError, BrowserFillValue, BrowserFrame, BrowserInteraction,
    BrowserNavigation, BrowserNode, BrowserRequest, BrowserResult, BrowserTab, BrowserTarget,
};
use mannie_tools::browser::{
    Chrome, ChromeError, ChromeFrame, ChromeHost, ChromeInput, ChromeNavigationAction,
    ChromeObservation, ChromeScreenshotStatus, ChromeTab, ChromeTarget,
};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use url::Url;
use zeroize::Zeroizing;

use super::credentials::SiteCredentials;
use super::{Slot, lock};

pub(super) struct AgentAccess<'a> {
    pub(super) conversation: &'a ConversationId,
    pub(super) origin: &'a str,
    pub(super) catalog: Option<&'a SiteCredentials>,
}

pub(super) fn error(error: ChromeError) -> BrowserError {
    match error {
        ChromeError::NotInstalled => BrowserError::BrowserMissing,
        ChromeError::Profile | ChromeError::Launch | ChromeError::Incompatible => {
            BrowserError::BrowserIncompatible
        }
        ChromeError::Disconnected | ChromeError::Protocol => BrowserError::BrowserDisconnected,
        ChromeError::Rejected | ChromeError::CrossOriginTarget => BrowserError::NavigationDenied,
        ChromeError::CertificateRejected => BrowserError::CertificateRejected,
        ChromeError::Stale
        | ChromeError::ScreenshotRequired
        | ChromeError::ViewportChanged
        | ChromeError::PointOutOfBounds => BrowserError::StaleTarget,
        ChromeError::NativePrompt => BrowserError::UnsupportedPrompt,
        ChromeError::Uncertain => BrowserError::OperationUncertain,
        ChromeError::InvalidAddress => BrowserError::InvalidBrowserInput,
        ChromeError::Limit => BrowserError::LimitReached,
        ChromeError::NoTab => BrowserError::NotFound,
    }
}

fn web_url(address: &str) -> Result<Url, BrowserError> {
    if address.len() > 8192 || address.chars().any(char::is_control) {
        return Err(BrowserError::InvalidBrowserInput);
    }
    let url = Url::parse(address).map_err(|_parse| BrowserError::InvalidBrowserInput)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(BrowserError::InvalidBrowserInput);
    }
    Ok(url)
}

pub(super) fn origin(address: &str) -> Result<String, BrowserError> {
    web_url(address).map(|url| url.origin().ascii_serialization())
}

pub(super) fn credential_origin(address: &str) -> Result<String, BrowserError> {
    let url = web_url(address)?;
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err(BrowserError::InvalidBrowserInput);
    }
    Ok(url.origin().ascii_serialization())
}

pub(super) fn stable_revision(value: &impl Serialize) -> String {
    // Every caller hashes a concrete browser DTO or strings, with no custom
    // serializer or floating-point values that can make serialization fail.
    let bytes = serde_json::to_vec(value).expect("browser access metadata serializes");
    format!("{:x}", Sha256::digest(bytes))
}

fn lookup(chrome: &Chrome, id: &str) -> Result<ChromeTab, BrowserError> {
    chrome
        .tabs()
        .into_iter()
        .find(|tab| tab.id == id)
        .ok_or(BrowserError::NotFound)
}

pub(super) fn target_tab(
    slot: &Slot,
    request: &BrowserRequest,
) -> Result<Option<ChromeTab>, BrowserError> {
    let Ok(chrome) = slot.chrome() else {
        return Ok(None);
    };
    let tab = match request {
        BrowserRequest::Navigate(BrowserNavigation::OpenTab { .. }) => return Ok(None),
        BrowserRequest::Navigate(
            BrowserNavigation::SelectTab { tab_id } | BrowserNavigation::CloseTab { tab_id },
        ) => Some(tab_id.clone()),
        _ => slot.active(&chrome),
    };
    tab.map(|id| lookup(&chrome, &id)).transpose()
}

pub(super) fn site_and_action(
    request: &BrowserRequest,
    tab: Option<&ChromeTab>,
) -> Result<(String, BrowserAction), BrowserError> {
    let tab_origin = || {
        let tab = tab.ok_or(BrowserError::SessionMissing)?;
        origin(&tab.url)
    };
    match request {
        BrowserRequest::Navigate(
            BrowserNavigation::Go { url } | BrowserNavigation::OpenTab { url },
        ) => Ok((origin(url)?, BrowserAction::Navigate)),
        BrowserRequest::Navigate(BrowserNavigation::Back) => {
            let site = tab
                .and_then(|tab| tab.back.as_ref())
                .ok_or(BrowserError::NotFound)?;
            Ok((origin(site)?, BrowserAction::Navigate))
        }
        BrowserRequest::Navigate(BrowserNavigation::Forward) => {
            let site = tab
                .and_then(|tab| tab.forward.as_ref())
                .ok_or(BrowserError::NotFound)?;
            Ok((origin(site)?, BrowserAction::Navigate))
        }
        BrowserRequest::Navigate(BrowserNavigation::Reload) => {
            Ok((tab_origin()?, BrowserAction::Navigate))
        }
        BrowserRequest::Navigate(_) => Ok((tab_origin()?, BrowserAction::Tabs)),
        BrowserRequest::Observe { .. } => Ok((tab_origin()?, BrowserAction::Observe)),
        BrowserRequest::Interact { revision, action } => {
            if revision.is_empty() || revision.len() > 256 {
                return Err(BrowserError::StaleTarget);
            }
            if let BrowserInteraction::ClickMany { references } = action {
                validate_batch(references)?;
            }
            Ok((tab_origin()?, BrowserAction::Interact))
        }
        BrowserRequest::Tabs => Err(BrowserError::InvalidBrowserInput),
    }
}

pub(super) fn browser_tab(tab: ChromeTab) -> BrowserTab {
    BrowserTab {
        can_go_back: tab.back.is_some(),
        can_go_forward: tab.forward.is_some(),
        id: tab.id,
        url: tab.url,
        title: tab.title,
        loading: tab.loading,
        navigation_error: tab.navigation_error.map(error),
        navigation_error_sequence: tab.navigation_error_sequence,
    }
}

fn browser_frame(frame: ChromeFrame) -> BrowserFrame {
    BrowserFrame {
        jpeg: frame.jpeg,
        width: frame.width,
        height: frame.height,
    }
}

/// The outline, revision, image and text of an observation.
struct ObservedPage {
    revision: String,
    frame: Option<BrowserFrame>,
    outline: Vec<BrowserNode>,
    checked: Vec<Option<bool>>,
    text: String,
}

fn observed_page(observation: ChromeObservation) -> ObservedPage {
    let checked = observation
        .outline
        .iter()
        .map(|node| node.checked)
        .collect();
    let outline = observation
        .outline
        .into_iter()
        .map(|node| {
            let editable = matches!(node.role.as_str(), "textbox" | "searchbox" | "spinbutton");
            BrowserNode {
                role: node.role,
                name: node.name,
                reference: node.reference,
                editable,
            }
        })
        .collect();
    ObservedPage {
        revision: observation.tab.revision,
        frame: observation.frame.map(browser_frame),
        outline,
        checked,
        text: observation.text,
    }
}

/// A still of a tab's page for the owner's pane, while an overlay hides the
/// native view. It never reaches a model.
pub(super) async fn still(
    host: &dyn ChromeHost,
    tab: &str,
) -> Result<Option<BrowserFrame>, BrowserError> {
    let send = |method: &str, params: &str| {
        host.send(tab, method, Zeroizing::new(params.to_owned()))
            .map_err(error)
    };
    let metrics = send("Page.getLayoutMetrics", "{}")?;
    let capture = send(
        "Page.captureScreenshot",
        r#"{"format":"jpeg","quality":70,"fromSurface":true,"captureBeyondViewport":false}"#,
    )?;
    let (Ok(Ok(metrics)), Ok(Ok(capture))) = (metrics.await, capture.await) else {
        return Ok(None);
    };
    let Some(jpeg) = capture["data"].as_str() else {
        return Ok(None);
    };
    let pixels = |key: &str| {
        metrics["visualViewport"][key]
            .as_f64()
            .filter(|value| value.is_finite() && *value > 0.0 && *value < 100_000.0)
            .map(|value| {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "the value is checked to be positive and small above"
                )]
                let pixels = value.round() as u32;
                pixels
            })
    };
    Ok(pixels("clientWidth")
        .zip(pixels("clientHeight"))
        .map(|(width, height)| BrowserFrame {
            jpeg: jpeg.to_owned(),
            width,
            height,
        }))
}

fn current_tab(slot: &Slot, chrome: &Chrome) -> Result<String, BrowserError> {
    slot.active(chrome).ok_or(BrowserError::NotFound)
}

async fn navigate(
    chrome: &Chrome,
    tab: &str,
    action: ChromeNavigationAction,
) -> Result<(), BrowserError> {
    chrome.navigate(tab, action).await.map_err(error)
}

pub(super) fn select(slot: &Slot, chrome: &Chrome, tab: String) -> Result<(), BrowserError> {
    lookup(chrome, &tab)?;
    lock(&slot.state).active = Some(tab);
    Ok(())
}

async fn open(slot: &Slot, chrome: &Chrome) -> Result<String, BrowserError> {
    let tab = chrome.open_tab().await.map_err(error)?;
    lock(&slot.state).active = Some(tab.clone());
    Ok(tab)
}

async fn close(slot: &Slot, chrome: &Chrome, tab: &str) -> Result<(), BrowserError> {
    chrome.close_tab(tab).await.map_err(error)?;
    let mut state = lock(&slot.state);
    if state.active.as_deref() == Some(tab) {
        state.active = None;
    }
    Ok(())
}

pub(super) async fn owner_input(slot: &Slot, action: BrowserControl) -> Result<(), BrowserError> {
    let chrome = slot.chrome()?;
    match action {
        BrowserControl::OpenTab => {
            open(slot, &chrome).await?;
        }
        BrowserControl::SelectTab { tab_id } => select(slot, &chrome, tab_id)?,
        BrowserControl::CloseTab { tab_id } => close(slot, &chrome, &tab_id).await?,
        BrowserControl::Navigate { url } => {
            web_url(&url)?;
            let tab = if let Some(tab) = slot.active(&chrome) {
                tab
            } else {
                open(slot, &chrome).await?
            };
            navigate(&chrome, &tab, ChromeNavigationAction::Go(url)).await?;
        }
        BrowserControl::Back | BrowserControl::Forward | BrowserControl::Reload => {
            let tab = current_tab(slot, &chrome)?;
            let navigation = match action {
                BrowserControl::Back => ChromeNavigationAction::Back,
                BrowserControl::Forward => ChromeNavigationAction::Forward,
                _ => ChromeNavigationAction::Reload,
            };
            navigate(&chrome, &tab, navigation).await?;
        }
        // The session's own controls go through the service, not here.
        BrowserControl::Start
        | BrowserControl::TakeOver
        | BrowserControl::Resume
        | BrowserControl::End => return Err(BrowserError::InvalidBrowserInput),
    }
    Ok(())
}

/// The keys that `press` sends; text goes into a field with `fill`.
const PRESS_KEYS: [&str; 12] = [
    "Enter",
    "Tab",
    "Escape",
    "Space",
    "ArrowLeft",
    "ArrowUp",
    "ArrowRight",
    "ArrowDown",
    "Backspace",
    "Delete",
    "Home",
    "End",
];

fn validate_batch(references: &[String]) -> Result<(), BrowserError> {
    let mut distinct = HashSet::new();
    if !(2..=16).contains(&references.len())
        || references.iter().any(|reference| {
            reference.is_empty() || reference.len() > 128 || !distinct.insert(reference)
        })
    {
        return Err(BrowserError::InvalidBrowserInput);
    }
    Ok(())
}

fn chrome_target(target: BrowserTarget) -> Result<ChromeTarget, BrowserError> {
    match target {
        BrowserTarget::Element { reference } => element(reference),
        BrowserTarget::Point { x, y } => Ok(ChromeTarget::Point { x, y }),
    }
}

fn element(reference: String) -> Result<ChromeTarget, BrowserError> {
    if reference.is_empty() || reference.len() > 128 {
        return Err(BrowserError::StaleTarget);
    }
    Ok(ChromeTarget::Element(reference))
}

fn agent_gesture(action: BrowserInteraction) -> Result<ChromeInput, BrowserError> {
    match action {
        BrowserInteraction::Click { target } => Ok(ChromeInput::Click(chrome_target(target)?)),
        BrowserInteraction::Drag { from, to } => Ok(ChromeInput::Drag {
            from: chrome_target(from)?,
            to: chrome_target(to)?,
        }),
        BrowserInteraction::ClickMany { .. } => Err(BrowserError::InvalidBrowserInput),
        BrowserInteraction::Fill {
            reference,
            value: BrowserFillValue::Text { text },
        } => {
            if text.len() > 8192 {
                return Err(BrowserError::InvalidBrowserInput);
            }
            Ok(ChromeInput::Fill {
                target: element(reference)?,
                text,
                protected: false,
            })
        }
        BrowserInteraction::Fill { .. } => Err(BrowserError::CredentialMissing),
        BrowserInteraction::Select { reference, value } => {
            if value.len() > 8192 {
                return Err(BrowserError::InvalidBrowserInput);
            }
            Ok(ChromeInput::Select {
                target: element(reference)?,
                value,
            })
        }
        BrowserInteraction::Press { key } => {
            if !PRESS_KEYS.contains(&key.as_str()) {
                return Err(BrowserError::InvalidBrowserInput);
            }
            Ok(ChromeInput::Press { key })
        }
        BrowserInteraction::Scroll { x, y } => {
            if x.unsigned_abs() > 4000 || y.unsigned_abs() > 4000 {
                return Err(BrowserError::InvalidBrowserInput);
            }
            Ok(ChromeInput::Scroll { x, y })
        }
    }
}

fn screenshot_details(observation: &ChromeObservation) -> serde_json::Value {
    match observation.screenshot {
        ChromeScreenshotStatus::Returned => observation.frame.as_ref().map_or_else(
            || json!({ "status": "unavailable" }),
            |frame| json!({ "status": "returned", "width": frame.width, "height": frame.height }),
        ),
        ChromeScreenshotStatus::Withheld => json!({ "status": "withheld" }),
        ChromeScreenshotStatus::Unavailable => json!({ "status": "unavailable" }),
        ChromeScreenshotStatus::NotRequested => json!({ "status": "not_requested" }),
    }
}

#[derive(Serialize)]
struct AgentBrowserNode {
    #[serde(flatten)]
    node: BrowserNode,
    #[serde(skip_serializing_if = "Option::is_none")]
    checked: Option<bool>,
}

async fn observe(
    slot: &Slot,
    chrome: &Chrome,
    screenshot: bool,
    access: &AgentAccess<'_>,
) -> Result<BrowserResult, BrowserError> {
    let tab = current_tab(slot, chrome)?;
    let observation = chrome.observe(&tab, screenshot).await.map_err(error)?;
    let site = origin(&observation.tab.url)?;
    if site != access.origin {
        return Err(BrowserError::StaleTarget);
    }
    let screenshot = screenshot_details(&observation);
    let screenshot_status = if observation.screenshot == ChromeScreenshotStatus::Returned
        && observation.frame.is_none()
    {
        ChromeScreenshotStatus::Unavailable
    } else {
        observation.screenshot
    };
    let mut credentials = match access.catalog {
        Some(catalog) => catalog.for_origin(Some(site.clone())).await?,
        None => Vec::new(),
    };
    for credential in &mut credentials {
        credential.label = chrome.redact_protected(&credential.label);
    }
    let page = observed_page(observation);
    let outline: Vec<_> = page
        .outline
        .into_iter()
        .zip(page.checked)
        .map(|(node, checked)| AgentBrowserNode { node, checked })
        .collect();
    let data = json!({
        "tab": tab,
        "origin": site,
        "revision": page.revision,
        "text": page.text,
        "outline": outline,
        "credentials": credentials,
        "screenshot": screenshot,
    });
    let mut state = lock(&slot.state);
    state.last_observation = Some(TargetObservation {
        tab,
        revision: page.revision,
        screenshot: screenshot_status,
        consumed: false,
    });
    state.needs_observation = false;
    Ok(BrowserResult {
        data,
        frame: page.frame,
        failure: None,
    })
}

pub(super) async fn agent_input(
    slot: &Slot,
    chrome: &Chrome,
    request: BrowserRequest,
    access: AgentAccess<'_>,
) -> Result<BrowserResult, BrowserError> {
    match request {
        BrowserRequest::Tabs => return Err(BrowserError::InvalidBrowserInput),
        BrowserRequest::Observe { screenshot } => {
            return observe(slot, chrome, screenshot, &access).await;
        }
        BrowserRequest::Navigate(action) => agent_navigation(slot, chrome, action).await?,
        BrowserRequest::Interact { revision, action } => {
            return interact(slot, chrome, &revision, action, &access).await;
        }
    }
    Ok(tab_result(slot, chrome))
}

fn tab_result(slot: &Slot, chrome: &Chrome) -> BrowserResult {
    BrowserResult {
        data: json!({ "tab": slot.active(chrome) }),
        frame: None,
        failure: None,
    }
}

pub(super) struct TargetObservation {
    tab: String,
    revision: String,
    screenshot: ChromeScreenshotStatus,
    consumed: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum StaleInteraction {
    RevisionChanged,
    ObservationRequired,
    ObservationConsumed,
    ScreenshotRequired,
    ScreenshotWithheld,
    ViewportChanged,
    PointOutOfBounds,
}

impl StaleInteraction {
    fn result(self, point: bool) -> BrowserResult {
        let (reason, detail) = match self {
            Self::RevisionChanged => (
                "revision_changed",
                "The supplied revision is no longer current.",
            ),
            Self::ObservationRequired => (
                "observation_required",
                "This action needs a current observation.",
            ),
            Self::ObservationConsumed => (
                "observation_consumed",
                "A targeted action already consumed this observation.",
            ),
            Self::ScreenshotRequired => (
                "screenshot_required",
                "The latest observation has no screenshot for point input.",
            ),
            Self::ScreenshotWithheld => (
                "screenshot_withheld",
                "Screenshots are withheld for this document.",
            ),
            Self::ViewportChanged => (
                "viewport_changed",
                "The viewport moved or changed size or scale after the screenshot.",
            ),
            Self::PointOutOfBounds => (
                "point_out_of_bounds",
                "A point is outside the screenshot or the page content area.",
            ),
        };
        let recovery = if self == Self::ScreenshotWithheld {
            "Use an element reference from the current observation if one is available."
        } else if point {
            r#"Call browser_observe with {"screenshot":true}. Choose points in the page content from the returned image and use its revision."#
        } else {
            "Call browser_observe again and use its returned revision and any element references it returns."
        };
        let failure = BrowserError::StaleTarget;
        BrowserResult {
            data: json!({
                "code": failure.as_code(),
                "reason": reason,
                "message": format!("{detail} {recovery}"),
            }),
            frame: None,
            failure: Some(failure),
        }
    }
}

fn validate_observation(
    observation: Option<&TargetObservation>,
    tab: &str,
    revision: &str,
    point: bool,
) -> Result<(), StaleInteraction> {
    let observation = observation
        .filter(|observation| observation.tab == tab && observation.revision == revision)
        .ok_or(StaleInteraction::ObservationRequired)?;
    if observation.consumed {
        return Err(StaleInteraction::ObservationConsumed);
    }
    match (point, observation.screenshot) {
        (false, _) | (true, ChromeScreenshotStatus::Returned) => Ok(()),
        (true, ChromeScreenshotStatus::Withheld) => Err(StaleInteraction::ScreenshotWithheld),
        (true, ChromeScreenshotStatus::Unavailable | ChromeScreenshotStatus::NotRequested) => {
            Err(StaleInteraction::ScreenshotRequired)
        }
    }
}

fn consume_observation(slot: &Slot) {
    if let Some(observation) = lock(&slot.state).last_observation.as_mut() {
        observation.consumed = true;
    }
}

async fn interact(
    slot: &Slot,
    chrome: &Chrome,
    revision: &str,
    action: BrowserInteraction,
    access: &AgentAccess<'_>,
) -> Result<BrowserResult, BrowserError> {
    let tab = lookup(chrome, &current_tab(slot, chrome)?)?;
    // An element reference or a point comes from the latest observation, so
    // an action that targets one needs that observation. A key press and a
    // scroll act on the page as it is, such as Enter after a fill.
    let targeted = !matches!(
        action,
        BrowserInteraction::Press { .. } | BrowserInteraction::Scroll { .. }
    );
    let point = matches!(
        action,
        BrowserInteraction::Click {
            target: BrowserTarget::Point { .. }
        } | BrowserInteraction::Drag {
            from: BrowserTarget::Point { .. },
            ..
        } | BrowserInteraction::Drag {
            to: BrowserTarget::Point { .. },
            ..
        }
    );
    if revision != tab.revision {
        return Ok(StaleInteraction::RevisionChanged.result(point));
    }
    if targeted
        && let Err(reason) = validate_observation(
            lock(&slot.state).last_observation.as_ref(),
            &tab.id,
            revision,
            point,
        )
    {
        return Ok(reason.result(point));
    }
    if let BrowserInteraction::ClickMany { references } = action {
        validate_batch(&references)?;
        consume_observation(slot);
        let outcome = chrome.click_many(&tab.id, revision, &references).await;
        let (progress, failure) = match outcome {
            Ok(()) => (json!({ "completed": references.len() }), None),
            Err(stopped) => {
                let failure = error(stopped.error);
                (
                    json!({
                        "completed": stopped.completed,
                        "failedIndex": stopped.failed_index,
                        "reason": failure.as_code(),
                        "message": failure.to_string(),
                    }),
                    Some(failure),
                )
            }
        };
        return Ok(BrowserResult {
            data: json!({ "tab": tab.id, "batch": progress }),
            frame: None,
            failure,
        });
    }
    // A refused input leaves the page as it was, and its observation usable.
    let outcome = dispatch_interaction(slot, chrome, &tab, action, access, targeted).await?;
    match outcome {
        Ok(()) => Ok(tab_result(slot, chrome)),
        Err(ChromeError::ScreenshotRequired) => {
            Ok(StaleInteraction::ScreenshotRequired.result(point))
        }
        Err(ChromeError::ViewportChanged) => Ok(StaleInteraction::ViewportChanged.result(point)),
        Err(ChromeError::PointOutOfBounds) => Ok(StaleInteraction::PointOutOfBounds.result(point)),
        Err(failure) => Err(error(failure)),
    }
}

async fn dispatch_interaction(
    slot: &Slot,
    chrome: &Chrome,
    tab: &ChromeTab,
    action: BrowserInteraction,
    access: &AgentAccess<'_>,
    targeted: bool,
) -> Result<Result<(), ChromeError>, BrowserError> {
    let outcome = match action {
        BrowserInteraction::Fill {
            reference: field,
            value: BrowserFillValue::Credential { reference },
        } => {
            let target = element(field)?;
            if origin(&tab.url)? != access.origin {
                return Err(BrowserError::StaleTarget);
            }
            let catalog = access.catalog.ok_or(BrowserError::CredentialMissing)?;
            let credential = catalog
                .fill(access.conversation, access.origin, &reference)
                .await?;
            consume_observation(slot);
            let dispatched = chrome
                .dispatch_credential(
                    &tab.id,
                    &tab.revision,
                    access.origin,
                    target,
                    credential.value.expose().to_owned(),
                )
                .await;
            drop(credential);
            match dispatched {
                Ok(settlement) => settlement.await,
                Err(error) => Err(error),
            }
        }
        action => {
            let input = agent_gesture(action)?;
            if targeted {
                consume_observation(slot);
            }
            chrome.interact(&tab.id, &tab.revision, input).await
        }
    };
    Ok(outcome)
}

async fn agent_navigation(
    slot: &Slot,
    chrome: &Chrome,
    action: BrowserNavigation,
) -> Result<(), BrowserError> {
    lock(&slot.state).last_observation = None;
    match action {
        BrowserNavigation::Go { url } => {
            let tab = if let Some(tab) = slot.active(chrome) {
                tab
            } else {
                open(slot, chrome).await?
            };
            navigate(chrome, &tab, ChromeNavigationAction::Go(url)).await
        }
        BrowserNavigation::OpenTab { url } => {
            let tab = open(slot, chrome).await?;
            navigate(chrome, &tab, ChromeNavigationAction::Go(url)).await
        }
        BrowserNavigation::SelectTab { tab_id } => select(slot, chrome, tab_id),
        BrowserNavigation::CloseTab { tab_id } => close(slot, chrome, &tab_id).await,
        action => {
            let tab = current_tab(slot, chrome)?;
            let navigation = match action {
                BrowserNavigation::Back => ChromeNavigationAction::Back,
                BrowserNavigation::Forward => ChromeNavigationAction::Forward,
                _ => ChromeNavigationAction::Reload,
            };
            navigate(chrome, &tab, navigation).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_input_requires_an_unused_observation_with_a_screenshot() {
        let slot = Slot::new();
        lock(&slot.state).last_observation = Some(TargetObservation {
            tab: "tab-1".to_owned(),
            revision: "3".to_owned(),
            screenshot: ChromeScreenshotStatus::NotRequested,
            consumed: false,
        });
        let check = |tab, revision, point| {
            validate_observation(
                lock(&slot.state).last_observation.as_ref(),
                tab,
                revision,
                point,
            )
        };
        assert_eq!(check("tab-1", "3", false), Ok(()));
        assert_eq!(
            check("tab-1", "3", true),
            Err(StaleInteraction::ScreenshotRequired)
        );
        assert_eq!(
            check("tab-2", "3", true),
            Err(StaleInteraction::ObservationRequired)
        );

        lock(&slot.state)
            .last_observation
            .as_mut()
            .unwrap()
            .screenshot = ChromeScreenshotStatus::Withheld;
        assert_eq!(
            check("tab-1", "3", true),
            Err(StaleInteraction::ScreenshotWithheld)
        );
        lock(&slot.state)
            .last_observation
            .as_mut()
            .unwrap()
            .screenshot = ChromeScreenshotStatus::Returned;
        assert_eq!(check("tab-1", "3", true), Ok(()));
        consume_observation(&slot);
        assert_eq!(
            check("tab-1", "3", true),
            Err(StaleInteraction::ObservationConsumed)
        );
    }

    #[test]
    fn stale_point_results_keep_the_failure_code_and_explain_recovery() {
        for (failure, reason) in [
            (StaleInteraction::ScreenshotRequired, "screenshot_required"),
            (
                StaleInteraction::ObservationConsumed,
                "observation_consumed",
            ),
            (StaleInteraction::ViewportChanged, "viewport_changed"),
            (StaleInteraction::PointOutOfBounds, "point_out_of_bounds"),
        ] {
            let result = failure.result(true);
            assert_eq!(result.failure, Some(BrowserError::StaleTarget));
            assert_eq!(result.data["code"], "stale_target");
            assert_eq!(result.data["reason"], reason);
            let message = result.data["message"].as_str().unwrap();
            assert!(message.contains(r#"browser_observe with {"screenshot":true}"#));
            assert!(message.contains("use its revision"));
        }

        let withheld = StaleInteraction::ScreenshotWithheld.result(true);
        assert_eq!(withheld.data["reason"], "screenshot_withheld");
        assert!(
            withheld.data["message"]
                .as_str()
                .unwrap()
                .contains("Use an element reference")
        );
    }

    #[test]
    fn press_takes_named_keys_and_refuses_text() {
        let press = |key: &str| {
            agent_gesture(BrowserInteraction::Press {
                key: key.to_owned(),
            })
        };
        assert!(matches!(press("Enter"), Ok(ChromeInput::Press { .. })));
        assert!(matches!(press("ArrowDown"), Ok(ChromeInput::Press { .. })));
        assert!(matches!(press("a"), Err(BrowserError::InvalidBrowserInput)));
        assert!(matches!(
            press("Control+S"),
            Err(BrowserError::InvalidBrowserInput)
        ));
    }

    #[test]
    fn canonicalizes_origins_including_local_ports() {
        assert_eq!(
            origin("https://Example.COM:443/path").unwrap(),
            "https://example.com"
        );
        assert_eq!(
            origin("http://127.0.0.1:8080").unwrap(),
            "http://127.0.0.1:8080"
        );
        assert_eq!(origin("http://[::1]:8000").unwrap(), "http://[::1]:8000");
        assert_eq!(
            origin("file:///tmp/file"),
            Err(BrowserError::InvalidBrowserInput)
        );
        assert_eq!(
            origin("https://name:password@example.com"),
            Err(BrowserError::InvalidBrowserInput)
        );
        assert_eq!(
            credential_origin("https://example.com/path"),
            Err(BrowserError::InvalidBrowserInput)
        );
    }
}
