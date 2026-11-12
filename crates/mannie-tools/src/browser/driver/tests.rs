//! The driver against a scripted host. The real engine's tests live with the
//! WebView2 host in `mannie-desktop-core`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use zeroize::Zeroizing;

use crate::browser::{
    Chrome, ChromeError, ChromeEvent, ChromeHost, ChromeHostFailure, ChromeInput, ChromeNavigation,
    ChromeNavigationAction, ChromeReply, ChromeRequest, ChromeScreenshotStatus, ChromeTarget,
};

type Responder = dyn Fn(&str, &str, &Value) -> Result<Value, ChromeHostFailure> + Send + Sync;

/// A host that answers every command at once and records what it was asked.
struct Script {
    calls: Mutex<Vec<(String, String, Value)>>,
    respond: Box<Responder>,
    next_tab: AtomicU32,
    released: Mutex<Vec<(String, String)>>,
    refused: Mutex<Vec<String>>,
    ended: AtomicBool,
}

impl Script {
    fn new(
        respond: impl Fn(&str, &str, &Value) -> Result<Value, ChromeHostFailure> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::default(),
            respond: Box::new(respond),
            next_tab: AtomicU32::new(0),
            released: Mutex::default(),
            refused: Mutex::default(),
            ended: AtomicBool::new(false),
        })
    }

    fn calls(&self, method: &str) -> Vec<(String, Value)> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, called, _)| called == method)
            .map(|(tab, _, params)| (tab.clone(), params.clone()))
            .collect()
    }

    fn methods(&self, tab: &str) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(called, _, _)| called == tab)
            .map(|(_, method, _)| method.clone())
            .collect()
    }
}

impl ChromeHost for Script {
    fn send(
        &self,
        tab: &str,
        method: &str,
        params: Zeroizing<String>,
    ) -> Result<ChromeReply, ChromeError> {
        let params: Value = serde_json::from_str(&params).unwrap();
        let answer = (self.respond)(tab, method, &params);
        self.calls
            .lock()
            .unwrap()
            .push((tab.to_owned(), method.to_owned(), params));
        let (send, reply) = oneshot::channel();
        send.send(answer).unwrap();
        Ok(reply)
    }

    fn create_tab(&self) -> BoxFuture<'static, Result<String, ChromeError>> {
        let id = self.next_tab.fetch_add(1, Ordering::Relaxed) + 1;
        Box::pin(std::future::ready(Ok(format!("tab-{id}"))))
    }

    fn release_popup(&self, popup: &str, tab: &str) -> Result<(), ChromeError> {
        self.released
            .lock()
            .unwrap()
            .push((popup.to_owned(), tab.to_owned()));
        Ok(())
    }

    fn refuse_popup(&self, popup: &str) {
        self.refused.lock().unwrap().push(popup.to_owned());
    }

    fn close_tab(&self, _tab: &str) -> BoxFuture<'static, Result<(), ChromeError>> {
        Box::pin(std::future::ready(Ok(())))
    }

    fn end(&self) -> BoxFuture<'static, ()> {
        self.ended.store(true, Ordering::Release);
        Box::pin(std::future::ready(()))
    }
}

/// Answers the commands that a page's setup, observations and navigations
/// need. The viewport is 800 by 600 CSS pixels at a scale of 1.5.
fn page_responses(method: &str) -> Value {
    match method {
        "Page.getFrameTree" => json!({ "frameTree": { "frame": { "id": "main" } } }),
        "Page.navigate" => json!({ "frameId": "main", "loaderId": "navigation" }),
        "Page.getNavigationHistory" => json!({ "currentIndex": 0, "entries": [] }),
        "Accessibility.getFullAXTree" => json!({ "nodes": [
            { "nodeId": "1", "role": { "value": "RootWebArea" }, "name": { "value": "Fixture" } },
            { "nodeId": "2", "role": { "value": "StaticText" }, "name": { "value": "Hello" } },
            { "nodeId": "3", "role": { "value": "button" }, "name": { "value": "Target" }, "backendDOMNodeId": 10 },
        ] }),
        "Page.getLayoutMetrics" => json!({
            "cssLayoutViewport": { "pageX": 0, "pageY": 0 },
            "cssVisualViewport": {
                "clientWidth": 800.0, "clientHeight": 600.0, "scale": 1.0,
                "pageX": 0.0, "pageY": 0.0, "offsetX": 0.0, "offsetY": 0.0,
            },
            "visualViewport": { "clientWidth": 1200.0, "clientHeight": 900.0 },
        }),
        // A JPEG SOF header declaring a 1200 by 900 image.
        "Page.captureScreenshot" => json!({ "data": "/9j/wAALCAOEBLABAREA/9k=" }),
        "DOM.getNodeForLocation" => json!({ "frameId": "main", "backendNodeId": 10 }),
        "DOM.describeNode" => json!({ "node": { "nodeName": "CANVAS" } }),
        _ => json!({}),
    }
}

/// The owner's decisions, scripted by origin. An origin without a quick
/// decision goes to `asks`, where the test answers it.
struct Gate {
    quick: Mutex<HashMap<String, bool>>,
    asks: mpsc::Sender<(ChromeRequest, oneshot::Sender<bool>)>,
}

impl ChromeNavigation for Gate {
    fn approved(&self, request: &ChromeRequest) -> Option<bool> {
        self.quick.lock().unwrap().get(&request.origin).copied()
    }

    fn allow(&self, request: ChromeRequest) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            let (decide, answer) = oneshot::channel();
            if self.asks.send((request, decide)).await.is_err() {
                return false;
            }
            answer.await.unwrap_or(false)
        })
    }
}

struct Fixture {
    chrome: Chrome,
    host: Arc<Script>,
    events: mpsc::Sender<ChromeEvent>,
    gate: Arc<Gate>,
    asks: mpsc::Receiver<(ChromeRequest, oneshot::Sender<bool>)>,
}

fn fixture() -> Fixture {
    fixture_with(page_responses)
}

fn fixture_with(respond: impl Fn(&str) -> Value + Send + Sync + 'static) -> Fixture {
    let host = Script::new(move |_tab, method, _params| Ok(respond(method)));
    let (events, receive) = mpsc::channel(64);
    let (asks_sender, asks) = mpsc::channel(16);
    let gate = Arc::new(Gate {
        quick: Mutex::default(),
        asks: asks_sender,
    });
    let navigation: Arc<dyn ChromeNavigation> = Arc::<Gate>::clone(&gate);
    let chrome = Chrome::start(Arc::<Script>::clone(&host), receive, navigation);
    Fixture {
        chrome,
        host,
        events,
        gate,
        asks,
    }
}

impl Fixture {
    fn approve(&self, origin: &str) {
        self.gate
            .quick
            .lock()
            .unwrap()
            .insert(origin.to_owned(), true);
    }

    async fn protocol(&self, tab: &str, method: &str, params: Value) {
        self.events
            .send(ChromeEvent::Protocol {
                tab: tab.to_owned(),
                method: method.to_owned(),
                params,
            })
            .await
            .unwrap();
    }

    async fn pause(&self, tab: &str, request: &str, url: &str, frame: &str, extra: Value) {
        let mut request_value =
            json!({ "url": url, "method": "GET", "headers": { "Accept": "text/html" } });
        if let (Some(target), Some(source)) = (request_value.as_object_mut(), extra.as_object()) {
            for (key, value) in source {
                target.insert(key.clone(), value.clone());
            }
        }
        self.protocol(
            tab,
            "Fetch.requestPaused",
            json!({ "requestId": request, "frameId": frame, "resourceType": "Document", "request": request_value }),
        )
        .await;
    }

    async fn commit(&self, tab: &str, url: &str, loader: &str) {
        self.protocol(
            tab,
            "Page.frameNavigated",
            json!({ "frame": { "id": "main", "url": url, "loaderId": loader } }),
        )
        .await;
    }
}

async fn eventually(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the condition holds in time");
}

#[test]
fn protects_novel_hostnames_without_matching_known_hosts_or_url_syntax() {
    let mut state = super::State::default();
    state
        .protected_values
        .push(Zeroizing::new("secret".to_owned()));
    state.committed_hosts.insert("secret.example".to_owned());
    assert!(!state.new_host_discloses_credential("https://secret.example"));
    assert!(state.new_host_discloses_credential("https://secret.other.example"));
    assert!(!state.new_host_discloses_credential("https://example.com:443"));
}

#[tokio::test]
async fn a_new_tab_enables_interception_before_anything_else() {
    let fixture = fixture();
    let tab = fixture.chrome.open_tab().await.unwrap();
    let methods = fixture.host.methods(&tab);
    assert_eq!(methods.first().map(String::as_str), Some("Fetch.enable"));
    assert!(methods.iter().any(|method| method == "Page.enable"));
    let enabled = &fixture.host.calls("Fetch.enable")[0].1;
    assert_eq!(enabled["patterns"][0]["requestStage"], "Request");
    assert_eq!(fixture.chrome.tabs().len(), 1);
}

#[tokio::test]
async fn an_approved_origin_continues_and_commits() {
    let fixture = fixture();
    let tab = fixture.chrome.open_tab().await.unwrap();
    fixture.approve("http://approved.test");
    fixture
        .pause(&tab, "r1", "http://approved.test/page", "main", json!({}))
        .await;
    eventually(|| !fixture.host.calls("Fetch.continueRequest").is_empty()).await;
    assert_eq!(fixture.host.calls("Fetch.failRequest").len(), 0);
    fixture
        .commit(&tab, "http://approved.test/page", "doc-1")
        .await;
    eventually(|| fixture.chrome.tabs()[0].url == "http://approved.test/page").await;
    assert!(!fixture.chrome.ended());
}

#[tokio::test]
async fn an_unapproved_request_fails_at_once_and_is_replayed_after_approval() {
    let mut fixture = fixture();
    let tab = fixture.chrome.open_tab().await.unwrap();
    fixture
        .pause(
            &tab,
            "r1",
            "http://other.test/submit",
            "main",
            json!({ "method": "POST", "postData": "name=value", "headers": { "Content-Type": "application/x-www-form-urlencoded" } }),
        )
        .await;
    let (request, decide) = tokio::time::timeout(Duration::from_secs(5), fixture.asks.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.origin, "http://other.test");
    // The engine no longer holds the request while the owner decides.
    let failed = fixture.host.calls("Fetch.failRequest");
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].1["requestId"], "r1");
    assert_eq!(fixture.host.calls("Fetch.continueRequest").len(), 0);

    decide.send(true).unwrap();
    eventually(|| !fixture.host.calls("Page.navigate").is_empty()).await;
    assert_eq!(
        fixture.host.calls("Page.navigate")[0].1["url"],
        "http://other.test/submit"
    );
    fixture
        .pause(&tab, "r2", "http://other.test/submit", "main", json!({}))
        .await;
    eventually(|| !fixture.host.calls("Fetch.continueRequest").is_empty()).await;
    let replay = &fixture.host.calls("Fetch.continueRequest")[0].1;
    assert_eq!(replay["requestId"], "r2");
    assert_eq!(replay["method"], "POST");
    assert_eq!(replay["postData"], "bmFtZT12YWx1ZQ==");
    assert!(
        replay["headers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|header| header["name"] == "Content-Type")
    );
    fixture
        .commit(&tab, "http://other.test/submit", "doc-2")
        .await;
    eventually(|| fixture.chrome.tabs()[0].url == "http://other.test/submit").await;
}

#[tokio::test]
async fn a_denied_request_is_never_replayed() {
    let mut fixture = fixture();
    let tab = fixture.chrome.open_tab().await.unwrap();
    fixture
        .pause(&tab, "r1", "http://other.test/", "main", json!({}))
        .await;
    let (_, decide) = fixture.asks.recv().await.unwrap();
    decide.send(false).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fixture.host.calls("Page.navigate").len(), 0);
    assert_eq!(fixture.host.calls("Fetch.failRequest").len(), 1);
}

#[tokio::test]
async fn cancelling_ends_an_approval_without_a_replay() {
    let mut fixture = fixture();
    let tab = fixture.chrome.open_tab().await.unwrap();
    fixture
        .pause(&tab, "r1", "http://other.test/", "main", json!({}))
        .await;
    let (_, decide) = fixture.asks.recv().await.unwrap();
    fixture.chrome.cancel_pending();
    let _answered = decide.send(true);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fixture.host.calls("Page.navigate").len(), 0);
}

#[tokio::test]
async fn a_refused_origin_fails_without_asking() {
    let mut fixture = fixture();
    let tab = fixture.chrome.open_tab().await.unwrap();
    fixture
        .gate
        .quick
        .lock()
        .unwrap()
        .insert("http://refused.test".to_owned(), false);
    fixture
        .pause(&tab, "r1", "http://refused.test/", "main", json!({}))
        .await;
    eventually(|| !fixture.host.calls("Fetch.failRequest").is_empty()).await;
    fixture.asks.try_recv().unwrap_err();
}

#[tokio::test]
async fn frames_load_under_the_engines_rules() {
    let mut fixture = fixture();
    let tab = fixture.chrome.open_tab().await.unwrap();
    fixture
        .pause(&tab, "r1", "http://frame.test/", "child", json!({}))
        .await;
    eventually(|| !fixture.host.calls("Fetch.continueRequest").is_empty()).await;
    fixture.asks.try_recv().unwrap_err();
}

#[tokio::test]
async fn a_commit_without_an_approved_request_ends_the_session() {
    let fixture = fixture();
    let tab = fixture.chrome.open_tab().await.unwrap();
    fixture.commit(&tab, "http://cached.test/", "doc-1").await;
    eventually(|| fixture.host.ended.load(Ordering::Acquire)).await;
    assert!(fixture.chrome.ended());
}

#[tokio::test]
async fn a_popup_gets_interception_before_it_is_released() {
    let fixture = fixture();
    let opener = fixture.chrome.open_tab().await.unwrap();
    fixture
        .events
        .send(ChromeEvent::Popup {
            popup: "popup-1".to_owned(),
            opener: opener.clone(),
        })
        .await
        .unwrap();
    eventually(|| !fixture.host.released.lock().unwrap().is_empty()).await;
    let (popup, tab) = fixture.host.released.lock().unwrap()[0].clone();
    assert_eq!(popup, "popup-1");
    assert_ne!(tab, opener);
    assert_eq!(
        fixture.host.methods(&tab).first().map(String::as_str),
        Some("Fetch.enable")
    );
    // The popup's first document request is top-level whatever its frame.
    fixture.approve("http://popup.test");
    fixture
        .pause(
            &tab,
            "p1",
            "http://popup.test/",
            "replaced-frame",
            json!({}),
        )
        .await;
    eventually(|| {
        fixture
            .host
            .calls("Fetch.continueRequest")
            .iter()
            .any(|(called, _)| *called == tab)
    })
    .await;
    fixture
        .protocol(
            &tab,
            "Page.frameNavigated",
            json!({ "frame": { "id": "replaced-frame", "url": "http://popup.test/", "loaderId": "doc-p" } }),
        )
        .await;
    eventually(|| {
        fixture
            .chrome
            .tabs()
            .iter()
            .any(|open| open.id == tab && open.url == "http://popup.test/")
    })
    .await;
}

#[tokio::test]
async fn a_popup_from_an_unknown_tab_is_refused() {
    let fixture = fixture();
    fixture
        .events
        .send(ChromeEvent::Popup {
            popup: "popup-1".to_owned(),
            opener: "missing".to_owned(),
        })
        .await
        .unwrap();
    eventually(|| !fixture.host.refused.lock().unwrap().is_empty()).await;
    assert_eq!(fixture.host.released.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn screenshots_report_image_pixels() {
    let fixture = fixture_with(|method| match method {
        // The JPEG includes a gutter beyond the 1200-pixel content viewport.
        "Page.captureScreenshot" => json!({ "data": "/9j/wAALCAOEBNIBAREA/9k=" }),
        "Page.getLayoutMetrics" => {
            let mut response = page_responses(method);
            response["cssVisualViewport"]["clientWidth"] = json!(800.2);
            response["visualViewport"]["clientWidth"] = json!(1200.3);
            response
        }
        _ => page_responses(method),
    });
    let tab = fixture.chrome.open_tab().await.unwrap();
    let observation = fixture.chrome.observe(&tab, true).await.unwrap();
    assert_eq!(observation.screenshot, ChromeScreenshotStatus::Returned);
    let frame = observation.frame.unwrap();
    assert_eq!((frame.width, frame.height), (1234, 900));
    assert_eq!(observation.text, "Hello");
    // The capture has no clip.
    let capture = &fixture.host.calls("Page.captureScreenshot")[0].1;
    assert!(capture.get("clip").is_none());
    assert_eq!(
        fixture
            .chrome
            .interact(&tab, &observation.tab.revision, click_point(1201, 60))
            .await,
        Err(ChromeError::PointOutOfBounds),
    );
    assert_eq!(fixture.host.calls("Input.dispatchMouseEvent"), []);
}

#[tokio::test]
async fn a_capture_without_jpeg_dimensions_leaves_a_semantic_observation() {
    let fixture = fixture_with(|method| match method {
        "Page.captureScreenshot" => json!({ "data": "aW1hZ2U=" }),
        _ => page_responses(method),
    });
    let tab = fixture.chrome.open_tab().await.unwrap();
    let observation = fixture.chrome.observe(&tab, true).await.unwrap();
    assert_eq!(observation.screenshot, ChromeScreenshotStatus::Unavailable);
    assert!(observation.frame.is_none());
    assert_eq!(observation.text, "Hello");
}

fn click_point(x: i32, y: i32) -> ChromeInput {
    ChromeInput::Click(ChromeTarget::Point { x, y })
}

#[tokio::test]
async fn point_input_uses_live_hit_testing_without_recapturing_the_image() {
    let captures = AtomicU32::new(0);
    let fixture = fixture_with(move |method| match method {
        "Page.getLayoutMetrics" => {
            let mut layout = page_responses(method);
            layout["cssVisualViewport"]["pageX"] = json!(20.0);
            layout["cssVisualViewport"]["pageY"] = json!(30.0);
            layout["cssVisualViewport"]["offsetX"] = json!(3.0);
            layout["cssVisualViewport"]["offsetY"] = json!(4.0);
            layout
        }
        "Page.captureScreenshot" if captures.fetch_add(1, Ordering::Relaxed) != 0 => {
            json!({ "data": "/9j/wAALCAOEBNIBAREA/9k=" })
        }
        _ => page_responses(method),
    });
    let tab = fixture.chrome.open_tab().await.unwrap();
    let observation = fixture.chrome.observe(&tab, true).await.unwrap();
    fixture
        .chrome
        .interact(&tab, &observation.tab.revision, click_point(152, 80))
        .await
        .unwrap();

    let hits = fixture.host.calls("DOM.getNodeForLocation");
    assert_eq!(hits, vec![(tab, json!({ "x": 124, "y": 87 }))]);
    let input = fixture.host.calls("Input.dispatchMouseEvent");
    assert_eq!(input.len(), 2);
    assert_eq!(input[0].1["type"], "mousePressed");
    assert_eq!(input[1].1["type"], "mouseReleased");
    for (_, params) in input {
        assert_eq!(
            (params["x"].as_i64(), params["y"].as_i64()),
            (Some(101), Some(53))
        );
    }
    assert_eq!(fixture.host.calls("Page.captureScreenshot").len(), 1);
}

#[tokio::test]
async fn point_drag_hit_tests_both_endpoints_before_input_without_recapturing() {
    let fixture = fixture();
    let tab = fixture.chrome.open_tab().await.unwrap();
    let observation = fixture.chrome.observe(&tab, true).await.unwrap();
    let drag = ChromeInput::Drag {
        from: ChromeTarget::Point { x: 30, y: 60 },
        to: ChromeTarget::Point { x: 90, y: 120 },
    };
    fixture
        .chrome
        .interact(&tab, &observation.tab.revision, drag)
        .await
        .unwrap();

    let hits = fixture.host.calls("DOM.getNodeForLocation");
    assert!(hits.contains(&(tab.clone(), json!({ "x": 20, "y": 40 }))));
    assert!(hits.contains(&(tab.clone(), json!({ "x": 60, "y": 80 }))));
    let methods = fixture.host.methods(&tab);
    let final_hit = methods
        .iter()
        .rposition(|method| method == "DOM.getNodeForLocation")
        .unwrap();
    let first_mouse = methods
        .iter()
        .position(|method| method == "Input.dispatchMouseEvent")
        .unwrap();
    assert!(final_hit < first_mouse);
    assert_eq!(fixture.host.calls("Page.captureScreenshot").len(), 1);
}

#[tokio::test]
async fn point_input_refuses_changed_viewport_geometry() {
    let initial = page_responses("Page.getLayoutMetrics");
    let mut resized = initial.clone();
    resized["cssVisualViewport"]["clientWidth"] = json!(802.0);
    resized["visualViewport"]["clientWidth"] = json!(1203.0);
    let mut scaled = initial.clone();
    scaled["visualViewport"]["clientWidth"] = json!(1600.0);
    let mut scrolled = initial.clone();
    scrolled["cssVisualViewport"]["pageY"] = json!(50.0);

    for (change, geometry) in [("resize", resized), ("scale", scaled), ("scroll", scrolled)] {
        let layout = Arc::new(Mutex::new(initial.clone()));
        let live = Arc::clone(&layout);
        let fixture = fixture_with(move |method| match method {
            "Page.getLayoutMetrics" => live.lock().unwrap().clone(),
            _ => page_responses(method),
        });
        let tab = fixture.chrome.open_tab().await.unwrap();
        let observation = fixture.chrome.observe(&tab, true).await.unwrap();
        assert_eq!(observation.screenshot, ChromeScreenshotStatus::Returned);
        *layout.lock().unwrap() = geometry;

        assert_eq!(
            fixture
                .chrome
                .interact(&tab, &observation.tab.revision, click_point(60, 60))
                .await,
            Err(ChromeError::ViewportChanged),
            "the point is stale after {change}",
        );
        assert_eq!(fixture.host.calls("Input.dispatchMouseEvent"), []);
    }
}

#[tokio::test]
async fn viewport_motion_during_capture_leaves_the_semantic_observation_available() {
    let captured = AtomicBool::new(false);
    let fixture = fixture_with(move |method| {
        let mut response = page_responses(method);
        if method == "Page.captureScreenshot" {
            captured.store(true, Ordering::Relaxed);
        }
        if method == "Page.getLayoutMetrics" && captured.load(Ordering::Relaxed) {
            response["cssVisualViewport"]["pageY"] = json!(50.0);
        }
        response
    });
    let tab = fixture.chrome.open_tab().await.unwrap();
    let observation = fixture.chrome.observe(&tab, true).await.unwrap();
    assert_eq!(observation.screenshot, ChromeScreenshotStatus::Unavailable);
    assert!(observation.frame.is_none());
    assert_eq!(observation.text, "Hello");
    assert!(
        observation
            .outline
            .iter()
            .any(|node| node.reference.is_some())
    );
    assert_eq!(
        fixture
            .chrome
            .interact(&tab, &observation.tab.revision, click_point(60, 60))
            .await,
        Err(ChromeError::ScreenshotRequired),
    );
    assert_eq!(fixture.host.calls("Input.dispatchMouseEvent"), []);
}

#[tokio::test]
async fn point_input_requires_the_latest_observation_to_have_an_image() {
    let fixture = fixture();
    let tab = fixture.chrome.open_tab().await.unwrap();
    let image = fixture.chrome.observe(&tab, true).await.unwrap();
    let text = fixture.chrome.observe(&tab, false).await.unwrap();
    assert_eq!(text.screenshot, ChromeScreenshotStatus::NotRequested);

    for (observation, error) in [
        (image, ChromeError::Stale),
        (text, ChromeError::ScreenshotRequired),
    ] {
        assert_eq!(
            fixture
                .chrome
                .interact(&tab, &observation.tab.revision, click_point(60, 60))
                .await,
            Err(error),
        );
    }
    assert_eq!(fixture.host.calls("Input.dispatchMouseEvent"), []);
}

#[tokio::test]
async fn point_input_checks_image_bounds_before_scaling() {
    let fixture = fixture_with(|method| {
        let mut response = page_responses(method);
        if method == "Page.getLayoutMetrics" {
            response["cssVisualViewport"]["clientWidth"] = json!(800.2);
            response["visualViewport"]["clientWidth"] = json!(1200.3);
        }
        response
    });
    let tab = fixture.chrome.open_tab().await.unwrap();
    let observation = fixture.chrome.observe(&tab, true).await.unwrap();
    assert_eq!(observation.frame.as_ref().unwrap().width, 1200);
    // The right edge can still round down to an in-bounds CSS coordinate.
    for (x, y) in [(-1, 60), (1200, 60), (60, 900)] {
        assert_eq!(
            fixture
                .chrome
                .interact(&tab, &observation.tab.revision, click_point(x, y))
                .await,
            Err(ChromeError::PointOutOfBounds),
        );
    }
    assert_eq!(fixture.host.calls("Input.dispatchMouseEvent"), []);
}

#[tokio::test]
async fn point_input_refuses_a_child_frame_or_frame_element() {
    for (frame, node) in [("child", "BUTTON"), ("main", "IFRAME")] {
        let fixture = fixture_with(move |method| match method {
            "DOM.getNodeForLocation" => json!({ "frameId": frame, "backendNodeId": 10 }),
            "DOM.describeNode" => json!({ "node": { "nodeName": node } }),
            _ => page_responses(method),
        });
        let tab = fixture.chrome.open_tab().await.unwrap();
        let observation = fixture.chrome.observe(&tab, true).await.unwrap();
        assert_eq!(
            fixture
                .chrome
                .interact(&tab, &observation.tab.revision, click_point(60, 60))
                .await,
            Err(ChromeError::CrossOriginTarget),
        );
        assert_eq!(fixture.host.calls("Input.dispatchMouseEvent"), []);
    }
}

#[tokio::test]
async fn scrolling_an_element_into_view_makes_the_other_drag_point_stale() {
    let scrolled = AtomicBool::new(false);
    let fixture = fixture_with(move |method| match method {
        "DOM.scrollIntoViewIfNeeded" => {
            scrolled.store(true, Ordering::Relaxed);
            json!({})
        }
        "Page.getLayoutMetrics" if scrolled.load(Ordering::Relaxed) => {
            let mut layout = page_responses(method);
            layout["cssLayoutViewport"]["pageY"] = json!(50);
            layout["cssVisualViewport"]["pageY"] = json!(50.0);
            layout
        }
        "DOM.getContentQuads" => json!({ "quads": [[40, 40, 80, 40, 80, 80, 40, 80]] }),
        _ => page_responses(method),
    });
    let tab = fixture.chrome.open_tab().await.unwrap();
    let observation = fixture.chrome.observe(&tab, true).await.unwrap();
    let reference = observation
        .outline
        .into_iter()
        .find_map(|node| node.reference)
        .unwrap();
    // An invalid start never scrolls the element. A valid start can become
    // stale as soon as the element is scrolled into view.
    for (x, scrolls, error) in [
        (-1, 0, ChromeError::PointOutOfBounds),
        (30, 1, ChromeError::ViewportChanged),
    ] {
        let drag = ChromeInput::Drag {
            from: ChromeTarget::Point { x, y: 30 },
            to: ChromeTarget::Element(reference.clone()),
        };
        assert_eq!(
            fixture
                .chrome
                .interact(&tab, &observation.tab.revision, drag)
                .await,
            Err(error),
        );
        assert_eq!(
            fixture.host.calls("DOM.scrollIntoViewIfNeeded").len(),
            scrolls
        );
    }
    assert_eq!(fixture.host.calls("Input.dispatchMouseEvent"), []);
    assert_eq!(fixture.host.calls("Input.dispatchDragEvent"), []);
}

#[tokio::test]
async fn a_document_the_owner_used_is_withheld_after_resume() {
    let fixture = fixture();
    let tab = fixture.chrome.open_tab().await.unwrap();
    fixture.chrome.set_owner_control(true);
    fixture
        .events
        .send(ChromeEvent::Focused { tab: tab.clone() })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    fixture.chrome.set_owner_control(false);
    let observation = fixture.chrome.observe(&tab, true).await.unwrap();
    assert_eq!(observation.screenshot, ChromeScreenshotStatus::Withheld);
    assert!(observation.frame.is_none());
    assert_eq!(
        fixture
            .chrome
            .interact(&tab, &observation.tab.revision, click_point(60, 60))
            .await,
        Err(ChromeError::ScreenshotRequired),
    );
    assert_eq!(fixture.host.calls("Input.dispatchMouseEvent"), []);

    // A new document of the same tab can be seen again.
    fixture.approve("http://next.test");
    fixture
        .pause(&tab, "r1", "http://next.test/", "main", json!({}))
        .await;
    fixture.commit(&tab, "http://next.test/", "doc-next").await;
    eventually(|| fixture.chrome.tabs()[0].url == "http://next.test/").await;
    let observation = fixture.chrome.observe(&tab, true).await.unwrap();
    assert_eq!(observation.screenshot, ChromeScreenshotStatus::Returned);
}

#[tokio::test]
async fn focus_while_the_agent_controls_withholds_nothing() {
    let fixture = fixture();
    let tab = fixture.chrome.open_tab().await.unwrap();
    fixture
        .events
        .send(ChromeEvent::Focused { tab: tab.clone() })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    fixture.chrome.set_owner_control(false);
    let observation = fixture.chrome.observe(&tab, true).await.unwrap();
    assert_eq!(observation.screenshot, ChromeScreenshotStatus::Returned);
}

#[tokio::test]
async fn an_agent_navigation_to_an_authorized_origin_succeeds() {
    let fixture = fixture();
    let tab = fixture.chrome.open_tab().await.unwrap();
    fixture.approve("http://approved.test");
    let events = fixture.events.clone();
    let navigating_tab = tab.clone();
    let commit = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        events
            .send(ChromeEvent::Protocol {
                tab: navigating_tab.clone(),
                method: "Fetch.requestPaused".to_owned(),
                params: json!({ "requestId": "r1", "frameId": "main", "request": { "url": "http://approved.test/", "method": "GET", "headers": {} } }),
            })
            .await
            .unwrap();
        events
            .send(ChromeEvent::Protocol {
                tab: navigating_tab,
                method: "Page.frameNavigated".to_owned(),
                params: json!({ "frame": { "id": "main", "url": "http://approved.test/", "loaderId": "doc-1" } }),
            })
            .await
            .unwrap();
    });
    fixture
        .chrome
        .navigate(
            &tab,
            ChromeNavigationAction::Go("http://approved.test/".to_owned()),
        )
        .await
        .unwrap();
    commit.await.unwrap();
    assert_eq!(fixture.chrome.tabs()[0].url, "http://approved.test/");
}

#[tokio::test]
async fn closing_the_event_stream_ends_the_session() {
    let fixture = fixture();
    let host = Arc::clone(&fixture.host);
    drop(fixture.events);
    eventually(|| host.ended.load(Ordering::Acquire)).await;
    assert!(fixture.chrome.ended());
    assert_eq!(
        fixture.chrome.open_tab().await.unwrap_err(),
        ChromeError::Disconnected
    );
}
