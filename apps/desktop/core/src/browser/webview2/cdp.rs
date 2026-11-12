//! Each tab's own `DevTools` endpoint: commands and the events the driver
//! needs (ADR-0064, decision 3).

use std::sync::Arc;

use mannie_tools::browser::{ChromeEvent, ChromeHostFailure};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::oneshot;
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2;
use webview2_com::{
    CallDevToolsProtocolMethodCompletedHandler, DevToolsProtocolEventReceivedEventHandler,
    take_pwstr,
};
use windows::core::{HSTRING, PCWSTR, PWSTR};
use zeroize::{Zeroize as _, Zeroizing};

use super::{Shared, cursor, tab_ui};

/// The events that the driver reads. Request interception and the commit
/// check depend on them, and an agent's drag reads the page's drag data.
pub(super) const EVENTS: [&str; 9] = [
    "Input.dragIntercepted",
    "Fetch.requestPaused",
    "Fetch.authRequired",
    "Page.frameNavigated",
    "Page.navigatedWithinDocument",
    "Page.frameStartedLoading",
    "Page.frameStoppedLoading",
    "Page.javascriptDialogOpening",
    "Page.fileChooserOpened",
];

type Reply = oneshot::Sender<Result<Value, ChromeHostFailure>>;

#[derive(Deserialize)]
struct PointerParameters {
    #[serde(rename = "type")]
    kind: PointerKind,
    x: f64,
    y: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum PointerKind {
    MousePressed,
    MouseMoved,
    MouseWheel,
    DragOver,
    Drop,
}

/// Reads just the main-frame viewport coordinates of an agent gesture.
/// Drag payloads may contain page data and are skipped by deserialization.
fn pointer_activity(method: &str, params: &str) -> Option<(f64, f64, bool)> {
    if !matches!(
        method,
        "Input.dispatchMouseEvent" | "Input.dispatchDragEvent"
    ) {
        return None;
    }
    let PointerParameters { kind, x, y } = serde_json::from_str(params).ok()?;
    let pulse = match (method, kind) {
        ("Input.dispatchMouseEvent", PointerKind::MousePressed | PointerKind::MouseWheel)
        | ("Input.dispatchDragEvent", PointerKind::Drop) => true,
        ("Input.dispatchMouseEvent", PointerKind::MouseMoved)
        | ("Input.dispatchDragEvent", PointerKind::DragOver) => false,
        _ => return None,
    };
    (x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0).then_some((x, y, pulse))
}

/// Sends one command to a tab, on the main thread. A missing tab drops the
/// reply, which the driver reads as a closed tab.
pub(super) fn call(session: u64, tab: &str, method: &str, params: Zeroizing<String>, reply: Reply) {
    let Some(tab) = tab_ui(session, tab) else {
        return;
    };
    let activity = pointer_activity(method, &params);
    let method = HSTRING::from(method);
    // The parameters can hold a value that the owner protected. Pass them
    // from a buffer that is wiped after the call copied it.
    let mut wide: Vec<u16> = params.encode_utf16().chain(std::iter::once(0)).collect();
    drop(params);
    let handler =
        CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |result, json| {
            let answer = match result {
                Ok(()) => serde_json::from_str(&json).map_err(|_json| ChromeHostFailure::Protocol),
                Err(_failed) => Err(ChromeHostFailure::Protocol),
            };
            let _delivered = reply.send(answer);
            Ok(())
        }));
    // SAFETY: the method and the NUL-terminated parameter buffer live until
    // the call returns, and the handler is a COM object that WebView2 keeps.
    let sent = unsafe {
        tab.webview
            .CallDevToolsProtocolMethod(&method, PCWSTR(wide.as_ptr()), &handler)
    };
    wide.zeroize();
    if let Err(error) = sent {
        tracing::warn!(%error, "WebView2 refused a DevTools command");
    } else if let Some((x, y, pulse)) = activity {
        cursor::show(&tab, x, y, pulse);
    }
}

/// Subscribes the driver to a tab's events.
pub(super) fn subscribe(
    shared: &Arc<Shared>,
    tab: &str,
    webview: &ICoreWebView2,
) -> windows::core::Result<()> {
    for event in EVENTS {
        let emitting = Arc::clone(shared);
        let id = tab.to_owned();
        let handler =
            DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, arguments| {
                let Some(arguments) = arguments else {
                    return Ok(());
                };
                let mut json = PWSTR::null();
                // SAFETY: `json` is a writable pointer that the call fills
                // with a string that `take_pwstr` frees.
                unsafe { arguments.ParameterObjectAsJson(&raw mut json) }?;
                let json = take_pwstr(json);
                match serde_json::from_str::<Value>(&json) {
                    Ok(params) => {
                        let frame = &params["frame"];
                        if event == "Page.frameNavigated"
                            && frame["id"].is_string()
                            && frame["parentId"].is_null()
                            && let Some(tab) = tab_ui(emitting.id, &id)
                        {
                            cursor::hide_after_navigation(&tab);
                        }
                        emitting.emit(ChromeEvent::Protocol {
                            tab: id.clone(),
                            method: event.to_owned(),
                            params,
                        });
                    }
                    // An event the driver cannot read could be a request it
                    // must govern.
                    Err(_json) => emitting.fail(),
                }
                Ok(())
            }));
        // SAFETY: the event name lives until the call returns.
        let receiver = unsafe { webview.GetDevToolsProtocolEventReceiver(&HSTRING::from(event)) }?;
        let mut token = 0;
        // SAFETY: the handler is a COM object that WebView2 keeps, and
        // `token` is writable.
        unsafe { receiver.add_DevToolsProtocolEventReceived(&handler, &raw mut token) }?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::pointer_activity;

    #[test]
    fn visualizes_mouse_presses_wheel_input_and_drag_steps() {
        for (method, kind, pulse) in [
            ("Input.dispatchMouseEvent", "mousePressed", true),
            ("Input.dispatchMouseEvent", "mouseWheel", true),
            ("Input.dispatchMouseEvent", "mouseMoved", false),
            ("Input.dispatchDragEvent", "dragOver", false),
            ("Input.dispatchDragEvent", "drop", true),
        ] {
            let params = serde_json::json!({
                "type": kind, "x": 12.5, "y": 38, "data": { "items": [] },
            });
            assert_eq!(
                pointer_activity(method, &params.to_string()),
                Some((12.5, 38.0, pulse)),
            );
        }
    }

    #[test]
    fn skips_releases_drag_setup_and_invalid_coordinates() {
        for (method, params) in [
            (
                "Input.dispatchMouseEvent",
                r#"{"type":"mouseReleased","x":2,"y":3}"#,
            ),
            (
                "Input.dispatchDragEvent",
                r#"{"type":"dragEnter","x":2,"y":3}"#,
            ),
            ("Input.dispatchMouseEvent", r#"{"type":"drop","x":2,"y":3}"#),
            (
                "Input.dispatchMouseEvent",
                r#"{"type":"mouseMoved","x":-1,"y":3}"#,
            ),
            ("Input.dispatchMouseEvent", r#"{"type":"mouseMoved","x":2}"#),
            (
                "Input.dispatchMouseEvent",
                r#"{"type":"mouseMoved","x":"2","y":3}"#,
            ),
            (
                "Input.dispatchKeyEvent",
                r#"{"type":"mousePressed","x":2,"y":3}"#,
            ),
        ] {
            assert_eq!(pointer_activity(method, params), None);
        }
    }
}
