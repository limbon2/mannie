//! A tab: one WebView2 controller in a child window of the main window, with
//! the settings and native handlers that keep a page inside the session's
//! rules (ADR-0063, decisions 2 and 8, and ADR-0064, decision 4).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use mannie_tools::browser::{ChromeError, ChromeEvent};
use tokio::sync::oneshot;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN, COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN,
    COREWEBVIEW2_MOVE_FOCUS_REASON_NEXT, COREWEBVIEW2_MOVE_FOCUS_REASON_PREVIOUS,
    COREWEBVIEW2_PERMISSION_STATE_DENY, COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL,
    ICoreWebView2, ICoreWebView2_4, ICoreWebView2_5, ICoreWebView2_10, ICoreWebView2_11,
    ICoreWebView2_13, ICoreWebView2_14, ICoreWebView2_18, ICoreWebView2_25,
    ICoreWebView2AcceleratorKeyPressedEventArgs2, ICoreWebView2ContextMenuItemCollection,
    ICoreWebView2Controller, ICoreWebView2Controller4, ICoreWebView2Profile6,
    ICoreWebView2Settings3,
};
use webview2_com::{
    AcceleratorKeyPressedEventHandler, BasicAuthenticationRequestedEventHandler,
    ClientCertificateRequestedEventHandler, ContextMenuRequestedEventHandler,
    CreateCoreWebView2ControllerCompletedHandler, DocumentTitleChangedEventHandler,
    DownloadStartingEventHandler, FocusChangedEventHandler, LaunchingExternalUriSchemeEventHandler,
    MoveFocusRequestedEventHandler, NavigationStartingEventHandler, NewWindowRequestedEventHandler,
    PermissionRequestedEventHandler, SaveAsUIShowingEventHandler,
    ServerCertificateErrorDetectedEventHandler, WindowCloseRequestedEventHandler, take_pwstr,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_C, VK_CONTROL, VK_F6, VK_F12, VK_I, VK_J, VK_P, VK_S, VK_SHIFT, VK_U,
};
use windows::core::{Interface as _, PWSTR};

use super::{PendingPopup, SESSIONS, Shared, TabUi, cdp, cursor, lock, surface, tab_ui};

/// The context menu items that stay in takeover: editing, navigation and
/// links. Save, print, inspection and sharing leave the page or the session.
const MENU_ITEMS: [&str; 16] = [
    "back",
    "forward",
    "reload",
    "undo",
    "redo",
    "cut",
    "copy",
    "paste",
    "pasteAndMatchStyle",
    "selectAll",
    "emoji",
    "openLinkInNewWindow",
    "copyLinkLocation",
    "copyLinkToHighlight",
    "copyImage",
    "copyImageLocation",
];

type Created = oneshot::Sender<Result<(), ChromeError>>;

/// Creates a tab's window and controller, on the main thread.
pub(super) fn create(shared: &Arc<Shared>, id: String, created: Created) {
    let Some((environment, parent)) = SESSIONS.with_borrow(|sessions| {
        sessions
            .get(&shared.id)
            .map(|ui| (ui.environment.clone(), ui.parent))
    }) else {
        let _sent = created.send(Err(ChromeError::Disconnected));
        return;
    };
    let container = match surface::create_container(parent, shared.owner.load(Ordering::Acquire)) {
        Ok(container) => container,
        Err(error) => {
            tracing::warn!(%error, "could not create a browser page window");
            let _sent = created.send(Err(ChromeError::Launch));
            return;
        }
    };
    let completing = Arc::clone(shared);
    let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
        move |result, controller| {
            let outcome = match (result, controller) {
                (Ok(()), Some(controller)) => adopt(&completing, &id, container, controller),
                _ => Err(ChromeError::Launch),
            };
            if outcome.is_err() {
                surface::destroy_container(container);
            }
            let _sent = created.send(outcome);
            Ok(())
        },
    ));
    // SAFETY: the container is a live window of this thread, and the
    // handler is a COM object that WebView2 keeps.
    if let Err(error) = unsafe { environment.CreateCoreWebView2Controller(container, &handler) } {
        tracing::warn!(%error, "WebView2 refused to create a browser tab");
    }
}

fn adopt(
    shared: &Arc<Shared>,
    id: &str,
    container: windows::Win32::Foundation::HWND,
    controller: ICoreWebView2Controller,
) -> Result<(), ChromeError> {
    let incompatible = |error: windows::core::Error| {
        tracing::warn!(%error, "a browser tab could not be configured");
        ChromeError::Incompatible
    };
    // SAFETY: the controller is live on this thread.
    let webview = unsafe { controller.CoreWebView2() }.map_err(incompatible)?;
    let mut browser = 0;
    // SAFETY: `browser` is writable.
    unsafe { webview.BrowserProcessId(&raw mut browser) }.map_err(incompatible)?;
    super::environment::adopt_browser_process(shared, browser)?;
    configure(&controller, &webview).map_err(incompatible)?;
    guard(shared, id, &webview).map_err(incompatible)?;
    keyboard(shared, id, &controller).map_err(incompatible)?;
    cdp::subscribe(shared, id, &webview).map_err(incompatible)?;
    surface::start_hidden(container, &controller).map_err(incompatible)?;
    let cursor = cursor::create(container);
    SESSIONS.with_borrow_mut(|sessions| {
        if let Some(ui) = sessions.get_mut(&shared.id) {
            ui.tabs.insert(
                id.to_owned(),
                TabUi {
                    container,
                    controller,
                    webview,
                    cursor,
                },
            );
        }
    });
    lock(&shared.tabs).insert(id.to_owned());
    Ok(())
}

/// Turns off what a page could use to reach past its tab: web messages, host
/// objects, the `DevTools` window, autofill, saved passwords and the status
/// bar. Drops stay on: `WebView2` has one drop target for a page, so turning
/// off drops from other programs also breaks the page's own drag and drop
/// (ADR-0063, decision 6).
fn configure(
    controller: &ICoreWebView2Controller,
    webview: &ICoreWebView2,
) -> windows::core::Result<()> {
    // SAFETY: the webview is live on this thread.
    let settings = unsafe { webview.Settings() }?;
    // SAFETY: each setter below takes a plain value on a live settings
    // object of this thread.
    unsafe { settings.SetIsWebMessageEnabled(false) }?;
    // SAFETY: as above.
    unsafe { settings.SetAreHostObjectsAllowed(false) }?;
    // SAFETY: as above. The DevTools endpoint that the driver uses works
    // without the DevTools window.
    unsafe { settings.SetAreDevToolsEnabled(false) }?;
    // SAFETY: as above.
    unsafe { settings.SetIsStatusBarEnabled(false) }?;
    // SAFETY: as above.
    unsafe { settings.SetAreDefaultContextMenusEnabled(true) }?;
    // SAFETY: as above.
    unsafe { settings.SetAreDefaultScriptDialogsEnabled(true) }?;
    let settings3: ICoreWebView2Settings3 = settings.cast()?;
    // SAFETY: as above. The accelerator handler filters the keys.
    unsafe { settings3.SetAreBrowserAcceleratorKeysEnabled(true) }?;

    let profile_api: ICoreWebView2_13 = webview.cast()?;
    // SAFETY: the webview is live on this thread.
    let profile = unsafe { profile_api.Profile() }?;
    let profile6: ICoreWebView2Profile6 = profile.cast()?;
    // SAFETY: plain values on a live profile of this thread.
    unsafe { profile6.SetIsGeneralAutofillEnabled(false) }?;
    // SAFETY: as above.
    unsafe { profile6.SetIsPasswordAutosaveEnabled(false) }?;

    let controller4: ICoreWebView2Controller4 = controller.cast()?;
    // SAFETY: a plain value on a live controller of this thread.
    unsafe { controller4.SetAllowExternalDrop(true) }?;
    Ok(())
}

/// The native handlers that replace a browser's own policy: web schemes
/// only, no permissions, downloads, external programs, authentication
/// prompts, client certificates or certificate errors, no saving, and a
/// context menu without save, print and `DevTools`.
#[expect(
    clippy::too_many_lines,
    reason = "one handler per native event, each a few lines, read best in one place"
)]
fn guard(shared: &Arc<Shared>, id: &str, webview: &ICoreWebView2) -> windows::core::Result<()> {
    let navigating = NavigationStartingEventHandler::create(Box::new(|_, arguments| {
        if let Some(arguments) = arguments {
            let mut uri = PWSTR::null();
            // SAFETY: `uri` is writable, and `take_pwstr` frees it.
            unsafe { arguments.Uri(&raw mut uri) }?;
            if !web_address(&take_pwstr(uri)) {
                // SAFETY: a plain value on live event arguments.
                unsafe { arguments.SetCancel(true) }?;
            }
        }
        Ok(())
    }));
    let mut token = 0;
    // SAFETY: the handler is a COM object that WebView2 keeps, and `token`
    // is writable. The same holds for every registration below.
    unsafe { webview.add_NavigationStarting(&navigating, &raw mut token) }?;

    let popups = Arc::clone(shared);
    let opener = id.to_owned();
    let new_window = NewWindowRequestedEventHandler::create(Box::new(move |_, arguments| {
        let Some(arguments) = arguments else {
            return Ok(());
        };
        // SAFETY: the arguments are live; the deferral keeps them so until
        // the driver releases or refuses the popup.
        let deferral = unsafe { arguments.GetDeferral() }?;
        let popup = format!(
            "popup-{}",
            popups.next_popup.fetch_add(1, Ordering::Relaxed)
        );
        let stored = SESSIONS.with_borrow_mut(|sessions| {
            sessions.get_mut(&popups.id).is_some_and(|ui| {
                ui.popups.insert(
                    popup.clone(),
                    PendingPopup {
                        args: arguments.clone(),
                        deferral: deferral.clone(),
                    },
                );
                true
            })
        });
        if stored {
            popups.emit(ChromeEvent::Popup {
                popup,
                opener: opener.clone(),
            });
        } else {
            // SAFETY: plain values on live arguments and deferral.
            unsafe { arguments.SetHandled(true) }?;
            // SAFETY: as above.
            unsafe { deferral.Complete() }?;
        }
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { webview.add_NewWindowRequested(&new_window, &raw mut token) }?;

    let permissions = PermissionRequestedEventHandler::create(Box::new(|_, arguments| {
        if let Some(arguments) = arguments {
            // SAFETY: a plain value on live event arguments.
            unsafe { arguments.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY) }?;
        }
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { webview.add_PermissionRequested(&permissions, &raw mut token) }?;

    let closing = Arc::clone(shared);
    let closed_tab = id.to_owned();
    let close_requested = WindowCloseRequestedEventHandler::create(Box::new(move |_, _| {
        lock(&closing.tabs).remove(&closed_tab);
        closing.emit(ChromeEvent::TabClosed {
            tab: closed_tab.clone(),
        });
        let session = closing.id;
        let tab = closed_tab.clone();
        // Closing the controller inside its own event is not allowed.
        if closing.ui.post(move || close(session, &tab)).is_err() {
            tracing::debug!("the event loop ended before a closed tab was removed");
        }
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { webview.add_WindowCloseRequested(&close_requested, &raw mut token) }?;

    let titles = Arc::clone(shared);
    let titled_tab = id.to_owned();
    let title_changed = DocumentTitleChangedEventHandler::create(Box::new(move |sender, _| {
        if let Some(sender) = sender {
            let mut title = PWSTR::null();
            // SAFETY: `title` is writable, and `take_pwstr` frees it.
            unsafe { sender.DocumentTitle(&raw mut title) }?;
            titles.emit(ChromeEvent::Title {
                tab: titled_tab.clone(),
                title: take_pwstr(title),
            });
        }
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { webview.add_DocumentTitleChanged(&title_changed, &raw mut token) }?;

    let downloads_api: ICoreWebView2_4 = webview.cast()?;
    let downloads = DownloadStartingEventHandler::create(Box::new(|_, arguments| {
        if let Some(arguments) = arguments {
            // SAFETY: a plain value on live event arguments.
            unsafe { arguments.SetCancel(true) }?;
        }
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { downloads_api.add_DownloadStarting(&downloads, &raw mut token) }?;

    let certificates_api: ICoreWebView2_5 = webview.cast()?;
    let certificates = ClientCertificateRequestedEventHandler::create(Box::new(|_, arguments| {
        if let Some(arguments) = arguments {
            // SAFETY: a plain value on live event arguments.
            unsafe { arguments.SetCancel(true) }?;
        }
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { certificates_api.add_ClientCertificateRequested(&certificates, &raw mut token) }?;

    let authentication_api: ICoreWebView2_10 = webview.cast()?;
    let authentication =
        BasicAuthenticationRequestedEventHandler::create(Box::new(|_, arguments| {
            if let Some(arguments) = arguments {
                // SAFETY: a plain value on live event arguments.
                unsafe { arguments.SetCancel(true) }?;
            }
            Ok(())
        }));
    // SAFETY: as above.
    unsafe {
        authentication_api.add_BasicAuthenticationRequested(&authentication, &raw mut token)
    }?;

    let menu_api: ICoreWebView2_11 = webview.cast()?;
    let menu = ContextMenuRequestedEventHandler::create(Box::new(|_, arguments| {
        if let Some(arguments) = arguments {
            // SAFETY: the arguments are live on this thread.
            let items = unsafe { arguments.MenuItems() }?;
            filter_menu(&items)?;
        }
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { menu_api.add_ContextMenuRequested(&menu, &raw mut token) }?;

    let certificate_errors_api: ICoreWebView2_14 = webview.cast()?;
    let certificate_errors =
        ServerCertificateErrorDetectedEventHandler::create(Box::new(|_, arguments| {
            if let Some(arguments) = arguments {
                // SAFETY: a plain value on live event arguments.
                unsafe {
                    arguments.SetAction(COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL)
                }?;
            }
            Ok(())
        }));
    // SAFETY: as above.
    unsafe {
        certificate_errors_api
            .add_ServerCertificateErrorDetected(&certificate_errors, &raw mut token)
    }?;

    let external_api: ICoreWebView2_18 = webview.cast()?;
    let external = LaunchingExternalUriSchemeEventHandler::create(Box::new(|_, arguments| {
        if let Some(arguments) = arguments {
            // SAFETY: a plain value on live event arguments.
            unsafe { arguments.SetCancel(true) }?;
        }
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { external_api.add_LaunchingExternalUriScheme(&external, &raw mut token) }?;

    // Older runtimes have no Save As event; their downloads are refused.
    if let Ok(save_as_api) = webview.cast::<ICoreWebView2_25>() {
        let save_as = SaveAsUIShowingEventHandler::create(Box::new(|_, arguments| {
            if let Some(arguments) = arguments {
                // SAFETY: a plain value on live event arguments.
                unsafe { arguments.SetCancel(true) }?;
            }
            Ok(())
        }));
        // SAFETY: as above.
        unsafe { save_as_api.add_SaveAsUIShowing(&save_as, &raw mut token) }?;
    }
    Ok(())
}

/// Removes the items that leave the page from a context menu.
fn filter_menu(items: &ICoreWebView2ContextMenuItemCollection) -> windows::core::Result<()> {
    let mut count = 0;
    // SAFETY: `count` is writable.
    unsafe { items.Count(&raw mut count) }?;
    for index in (0..count).rev() {
        // SAFETY: the index is below the collection's count.
        let item = unsafe { items.GetValueAtIndex(index) }?;
        let mut name = PWSTR::null();
        // SAFETY: `name` is writable, and `take_pwstr` frees it.
        unsafe { item.Name(&raw mut name) }?;
        let name = take_pwstr(name);
        let kept = name.is_empty() || MENU_ITEMS.contains(&name.as_str());
        if !kept {
            // SAFETY: the index is still below the count, which only the
            // removals at higher indexes changed.
            unsafe { items.RemoveValueAtIndex(index) }?;
        }
    }
    Ok(())
}

/// Keys: F6 and Tab past either end of the page return focus to the app,
/// and the browser keys for saving, printing, downloads, the page source and
/// `DevTools` do nothing (ADR-0063, decision 6, and ADR-0064, decision 4).
fn keyboard(
    shared: &Arc<Shared>,
    id: &str,
    controller: &ICoreWebView2Controller,
) -> windows::core::Result<()> {
    let accelerators = Arc::clone(shared);
    let pressed = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, arguments| {
        let Some(arguments) = arguments else {
            return Ok(());
        };
        let mut kind = COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN;
        // SAFETY: `kind` is writable.
        unsafe { arguments.KeyEventKind(&raw mut kind) }?;
        if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
            && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
        {
            return Ok(());
        }
        let mut key = 0;
        // SAFETY: `key` is writable.
        unsafe { arguments.VirtualKey(&raw mut key) }?;
        let control = pressed_key(VK_CONTROL.0);
        let shift = pressed_key(VK_SHIFT.0);
        if key == u32::from(VK_F6.0) {
            // SAFETY: a plain value on live event arguments.
            unsafe { arguments.SetHandled(true) }?;
            accelerators.focus_left(!shift);
        } else if refused_key(key, control, shift) {
            let arguments2: ICoreWebView2AcceleratorKeyPressedEventArgs2 = arguments.cast()?;
            // SAFETY: a plain value on live event arguments. The page still
            // receives the key; only the browser's own action is off.
            unsafe { arguments2.SetIsBrowserAcceleratorKeyEnabled(false) }?;
        }
        Ok(())
    }));
    let mut token = 0;
    // SAFETY: the handler is a COM object that WebView2 keeps, and `token`
    // is writable. The same holds below.
    unsafe { controller.add_AcceleratorKeyPressed(&pressed, &raw mut token) }?;

    let leaving = Arc::clone(shared);
    let move_focus = MoveFocusRequestedEventHandler::create(Box::new(move |_, arguments| {
        let Some(arguments) = arguments else {
            return Ok(());
        };
        let mut reason = COREWEBVIEW2_MOVE_FOCUS_REASON_NEXT;
        // SAFETY: `reason` is writable.
        unsafe { arguments.Reason(&raw mut reason) }?;
        if reason == COREWEBVIEW2_MOVE_FOCUS_REASON_NEXT
            || reason == COREWEBVIEW2_MOVE_FOCUS_REASON_PREVIOUS
        {
            // SAFETY: a plain value on live event arguments.
            unsafe { arguments.SetHandled(true) }?;
            leaving.focus_left(reason == COREWEBVIEW2_MOVE_FOCUS_REASON_NEXT);
        }
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { controller.add_MoveFocusRequested(&move_focus, &raw mut token) }?;

    let focusing = Arc::clone(shared);
    let focused_tab = id.to_owned();
    let got_focus = FocusChangedEventHandler::create(Box::new(move |_, _| {
        focusing.emit(ChromeEvent::Focused {
            tab: focused_tab.clone(),
        });
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { controller.add_GotFocus(&got_focus, &raw mut token) }?;
    Ok(())
}

fn pressed_key(key: u16) -> bool {
    // SAFETY: GetKeyState takes a plain key code and reads this thread's
    // keyboard state.
    let state = unsafe { GetKeyState(i32::from(key)) };
    state < 0
}

fn refused_key(key: u32, control: bool, shift: bool) -> bool {
    let is = |virtual_key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| {
        key == u32::from(virtual_key.0)
    };
    is(VK_F12)
        || control && !shift && (is(VK_S) || is(VK_P) || is(VK_J) || is(VK_U))
        || control && shift && (is(VK_I) || is(VK_J) || is(VK_C) || is(VK_S))
}

/// Top-level navigation goes only to web pages and the blank page.
fn web_address(address: &str) -> bool {
    address == "about:blank"
        || url::Url::parse(address).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
}

/// Hands a deferred new window the tab that the driver set up, or refuses
/// it, on the main thread.
pub(super) fn release_popup(session: u64, popup: &str, tab: Option<&str>) {
    let pending = SESSIONS.with_borrow_mut(|sessions| {
        sessions
            .get_mut(&session)
            .and_then(|ui| ui.popups.remove(popup))
    });
    let Some(pending) = pending else {
        return;
    };
    if let Some(webview) = tab
        .and_then(|tab| tab_ui(session, tab))
        .map(|tab| tab.webview)
    {
        // SAFETY: the arguments and the webview are live on this thread, and
        // the webview has not navigated yet.
        if let Err(error) = unsafe { pending.args.SetNewWindow(&webview) } {
            tracing::warn!(%error, "WebView2 refused a popup's tab");
        }
    }
    // SAFETY: plain values on live arguments and deferral. Handled without
    // a new window refuses the popup.
    if let Err(error) = unsafe { pending.args.SetHandled(true) } {
        tracing::warn!(%error, "could not mark a popup handled");
    }
    // SAFETY: as above.
    if let Err(error) = unsafe { pending.deferral.Complete() } {
        tracing::warn!(%error, "could not complete a popup");
    }
}

/// Closes a tab's controller and window, on the main thread.
pub(super) fn close(session: u64, tab: &str) {
    let removed = SESSIONS.with_borrow_mut(|sessions| {
        sessions
            .get_mut(&session)
            .and_then(|ui| ui.tabs.remove(tab))
    });
    if let Some(tab) = removed {
        close_tab(&tab);
    }
}

fn close_tab(tab: &TabUi) {
    // SAFETY: the controller is live on this thread. After the session's
    // processes ended, Close can fail, which changes nothing.
    if let Err(error) = unsafe { tab.controller.Close() } {
        tracing::debug!(%error, "a browser tab's controller did not close");
    }
    surface::destroy_container(tab.container);
}

/// Closes every view of a session and drops its environment, on the main
/// thread.
pub(super) fn close_session(session: u64) {
    let removed = SESSIONS.with_borrow_mut(|sessions| sessions.remove(&session));
    let Some(ui) = removed else {
        return;
    };
    for pending in ui.popups.into_values() {
        // SAFETY: plain values on live arguments and deferral.
        drop(unsafe { pending.args.SetHandled(true) });
        // SAFETY: as above.
        drop(unsafe { pending.deferral.Complete() });
    }
    for tab in ui.tabs.values() {
        close_tab(tab);
    }
}

#[cfg(test)]
mod tests {
    use super::{refused_key, web_address};
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_A, VK_I, VK_P, VK_S};

    #[test]
    fn only_web_pages_and_the_blank_page_load() {
        assert!(web_address("https://example.com/"));
        assert!(web_address("http://127.0.0.1:8080/"));
        assert!(web_address("about:blank"));
        assert!(!web_address("file:///C:/Windows/win.ini"));
        assert!(!web_address("data:text/html,hello"));
        assert!(!web_address("mailto:owner@example.com"));
    }

    #[test]
    fn save_print_and_devtools_keys_are_refused() {
        let key = |virtual_key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| {
            u32::from(virtual_key.0)
        };
        assert!(refused_key(key(VK_S), true, false));
        assert!(refused_key(key(VK_P), true, false));
        assert!(refused_key(key(VK_I), true, true));
        assert!(!refused_key(key(VK_A), true, false));
        assert!(!refused_key(key(VK_S), false, false));
    }
}
