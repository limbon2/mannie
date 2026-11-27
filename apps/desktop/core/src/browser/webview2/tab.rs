//! A tab: one WebView2 controller in a child window of the main window, with
//! the settings and native handlers that keep a page inside the session's
//! rules (ADR-0064, decisions 2 and 8, and ADR-0065, decision 4).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use mannie_tools::browser::{ChromeError, ChromeEvent};
use tokio::sync::oneshot;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN, COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN,
    COREWEBVIEW2_MOVE_FOCUS_REASON_NEXT, COREWEBVIEW2_MOVE_FOCUS_REASON_PREVIOUS,
    COREWEBVIEW2_PERMISSION_STATE_DENY, COREWEBVIEW2_PROCESS_FAILED_KIND,
    COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_PPAPI_BROKER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_PPAPI_PLUGIN_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
    COREWEBVIEW2_PROCESS_FAILED_KIND_SANDBOX_HELPER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_UNKNOWN_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_UTILITY_PROCESS_EXITED, COREWEBVIEW2_PROCESS_FAILED_REASON,
    COREWEBVIEW2_PROCESS_FAILED_REASON_CRASHED, COREWEBVIEW2_PROCESS_FAILED_REASON_LAUNCH_FAILED,
    COREWEBVIEW2_PROCESS_FAILED_REASON_OUT_OF_MEMORY,
    COREWEBVIEW2_PROCESS_FAILED_REASON_PROFILE_DELETED,
    COREWEBVIEW2_PROCESS_FAILED_REASON_TERMINATED, COREWEBVIEW2_PROCESS_FAILED_REASON_UNEXPECTED,
    COREWEBVIEW2_PROCESS_FAILED_REASON_UNRESPONSIVE, COREWEBVIEW2_WEB_ERROR_STATUS,
    COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT,
    COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT,
    COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED,
    COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID,
    COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED,
    COREWEBVIEW2_WEB_ERROR_STATUS_CLIENT_CERTIFICATE_CONTAINS_ERRORS,
    COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED,
    COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET, COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED,
    COREWEBVIEW2_WEB_ERROR_STATUS_ERROR_HTTP_INVALID_SERVER_RESPONSE,
    COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED,
    COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED,
    COREWEBVIEW2_WEB_ERROR_STATUS_REDIRECT_FAILED,
    COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE, COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT,
    COREWEBVIEW2_WEB_ERROR_STATUS_UNEXPECTED_ERROR, COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
    COREWEBVIEW2_WEB_ERROR_STATUS_VALID_AUTHENTICATION_CREDENTIALS_REQUIRED,
    COREWEBVIEW2_WEB_ERROR_STATUS_VALID_PROXY_AUTHENTICATION_REQUIRED, ICoreWebView2,
    ICoreWebView2_4, ICoreWebView2_5, ICoreWebView2_10, ICoreWebView2_11, ICoreWebView2_13,
    ICoreWebView2_18, ICoreWebView2_25, ICoreWebView2AcceleratorKeyPressedEventArgs2,
    ICoreWebView2ContextMenuItemCollection, ICoreWebView2Controller, ICoreWebView2Controller4,
    ICoreWebView2ProcessFailedEventArgs2, ICoreWebView2Profile6, ICoreWebView2Settings3,
};
use webview2_com::{
    AcceleratorKeyPressedEventHandler, BasicAuthenticationRequestedEventHandler,
    ClientCertificateRequestedEventHandler, ContextMenuRequestedEventHandler,
    CreateCoreWebView2ControllerCompletedHandler, DocumentTitleChangedEventHandler,
    DownloadStartingEventHandler, FocusChangedEventHandler, LaunchingExternalUriSchemeEventHandler,
    MoveFocusRequestedEventHandler, NavigationCompletedEventHandler,
    NavigationStartingEventHandler, NewWindowRequestedEventHandler,
    PermissionRequestedEventHandler, ProcessFailedEventHandler, SaveAsUIShowingEventHandler,
    WindowCloseRequestedEventHandler, take_pwstr,
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
    if shared.stopping() {
        let _sent = created.send(Err(ChromeError::Disconnected));
        return;
    }
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
    if shared.stopping() {
        // SAFETY: the returned controller is live on this thread and was never published.
        drop(unsafe { controller.Close() });
        return Err(ChromeError::Disconnected);
    }
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
/// (ADR-0064, decision 6).
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
/// prompts or client certificates, controlled server certificate exceptions, no saving, and a
/// context menu without save, print and `DevTools`.
#[expect(
    clippy::too_many_lines,
    reason = "one handler per native event, each a few lines, read best in one place"
)]
fn guard(shared: &Arc<Shared>, id: &str, webview: &ICoreWebView2) -> windows::core::Result<()> {
    let navigation = Rc::new(RefCell::new(None::<(u64, String)>));
    let starting_navigation = Rc::clone(&navigation);
    let starting = Arc::clone(shared);
    let start_tab = id.to_owned();
    let navigating = NavigationStartingEventHandler::create(Box::new(move |_, arguments| {
        if let Some(arguments) = arguments {
            let mut uri = PWSTR::null();
            // SAFETY: `uri` is writable, and `take_pwstr` frees it.
            unsafe { arguments.Uri(&raw mut uri) }?;
            let uri = take_pwstr(uri);
            let allowed = web_address(&uri);
            let stopping = starting.stopping();
            if !allowed || stopping {
                if !allowed && !stopping {
                    tracing::warn!(
                        session = starting.id,
                        tab = %start_tab,
                        scheme = address_scheme(&uri),
                        "cancelled a browser navigation to a non-web address"
                    );
                }
                // SAFETY: a plain value on live event arguments.
                unsafe { arguments.SetCancel(true) }?;
            } else {
                let mut id = 0;
                // SAFETY: the event arguments are live and the output is writable.
                unsafe { arguments.NavigationId(&raw mut id) }?;
                *starting_navigation.borrow_mut() = Some((id, uri));
                starting.emit(ChromeEvent::NavigationStarted {
                    tab: start_tab.clone(),
                    id,
                });
            }
        }
        Ok(())
    }));
    let mut token = 0;
    // SAFETY: the handler is a COM object that WebView2 keeps, and `token`
    // is writable. The same holds for every registration below.
    unsafe { webview.add_NavigationStarting(&navigating, &raw mut token) }?;

    let completing = Arc::clone(shared);
    let complete_tab = id.to_owned();
    let completed_navigation = Rc::clone(&navigation);
    let completed = NavigationCompletedEventHandler::create(Box::new(move |_, arguments| {
        if let Some(arguments) = arguments {
            let mut id = 0;
            let mut success = windows::core::BOOL::default();
            let mut status = COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN;
            // SAFETY: the event arguments are live and the outputs are writable.
            unsafe { arguments.NavigationId(&raw mut id) }?;
            // SAFETY: as above.
            unsafe { arguments.IsSuccess(&raw mut success) }?;
            if success.as_bool()
                && completed_navigation
                    .borrow()
                    .as_ref()
                    .is_some_and(|(current, _)| *current == id)
            {
                completed_navigation.borrow_mut().take();
            }
            if !success.as_bool() {
                // SAFETY: as above.
                unsafe { arguments.WebErrorStatus(&raw mut status) }?;
            }
            let certificate_error = matches!(
                status,
                COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT
                    | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED
                    | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID
                    | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED
            );
            if !success.as_bool() && !completing.stopping() {
                tracing::warn!(
                    session = completing.id,
                    tab = %complete_tab,
                    navigation_id = id,
                    web_error_status = web_error_status_name(status),
                    certificate_error,
                    "a browser top-level navigation did not succeed"
                );
            }
            completing.emit(ChromeEvent::NavigationCompleted {
                tab: complete_tab.clone(),
                id,
                certificate_error,
                rejected_certificate: None,
            });
        }
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { webview.add_NavigationCompleted(&completed, &raw mut token) }?;

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

    let process_failed_session = Arc::clone(shared);
    let process_failed_tab = id.to_owned();
    let process_failed = ProcessFailedEventHandler::create(Box::new(move |_, arguments| {
        if process_failed_session.stopping() {
            return Ok(());
        }
        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND_UNKNOWN_PROCESS_EXITED;
        let mut reason = None;
        let mut exit_code = None;
        let mut description = String::new();
        if let Some(arguments) = arguments {
            // SAFETY: the event arguments are live on this thread and the output is writable.
            drop(unsafe { arguments.ProcessFailedKind(&raw mut kind) });
            if let Ok(arguments2) = arguments.cast::<ICoreWebView2ProcessFailedEventArgs2>() {
                let mut failed_reason = COREWEBVIEW2_PROCESS_FAILED_REASON_UNEXPECTED;
                // SAFETY: the event arguments are live on this thread and the output is writable.
                if unsafe { arguments2.Reason(&raw mut failed_reason) }.is_ok() {
                    reason = Some(process_failed_reason_name(failed_reason));
                }
                let mut code = 0;
                // SAFETY: as above.
                if unsafe { arguments2.ExitCode(&raw mut code) }.is_ok() {
                    exit_code = Some(code);
                }
                let mut text = PWSTR::null();
                // SAFETY: `text` is writable, and `take_pwstr` frees it.
                if unsafe { arguments2.ProcessDescription(&raw mut text) }.is_ok() {
                    description = sanitize_process_description(&take_pwstr(text));
                }
            }
        }
        let exit_code_hex = exit_code.map(|code| format!("0x{:08x}", code.cast_unsigned()));
        tracing::warn!(
            session = process_failed_session.id,
            tab = %process_failed_tab,
            kind = process_failed_kind_name(kind),
            reason = reason.unwrap_or("unknown"),
            exit_code,
            exit_code_hex = exit_code_hex.as_deref().unwrap_or(""),
            process_description = %description,
            "a WebView2 process failed"
        );
        Ok(())
    }));
    // SAFETY: as above.
    unsafe { webview.add_ProcessFailed(&process_failed, &raw mut token) }?;

    let certificates_api: ICoreWebView2_5 = webview.cast()?;
    let client_cert_session = Arc::clone(shared);
    let client_cert_tab = id.to_owned();
    let certificates =
        ClientCertificateRequestedEventHandler::create(Box::new(move |_, arguments| {
            if let Some(arguments) = arguments {
                if !client_cert_session.stopping() {
                    tracing::warn!(
                        session = client_cert_session.id,
                        tab = %client_cert_tab,
                        "cancelled a browser client certificate request"
                    );
                }
                // SAFETY: a plain value on live event arguments.
                unsafe { arguments.SetCancel(true) }?;
            }
            Ok(())
        }));
    // SAFETY: as above.
    unsafe { certificates_api.add_ClientCertificateRequested(&certificates, &raw mut token) }?;

    let authentication_api: ICoreWebView2_10 = webview.cast()?;
    let auth_session = Arc::clone(shared);
    let auth_tab = id.to_owned();
    let authentication =
        BasicAuthenticationRequestedEventHandler::create(Box::new(move |_, arguments| {
            if let Some(arguments) = arguments {
                if !auth_session.stopping() {
                    tracing::warn!(
                        session = auth_session.id,
                        tab = %auth_tab,
                        "cancelled a browser basic authentication prompt"
                    );
                }
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

    let certificate_session = Arc::clone(shared);
    let certificate_tab = id.to_owned();
    super::certificates::guard(shared, webview, move |uri, rejected_certificate| {
        // WebView2 can report a cancelled navigation after rejecting TLS.
        // Use its certificate event only for the matching top-level request so
        // subresource rejections stay silent (ADR-0082).
        if let Some((navigation_id, address)) = navigation.borrow().as_ref()
            && same_navigation_target(address, uri)
        {
            certificate_session.emit(ChromeEvent::NavigationCompleted {
                tab: certificate_tab.clone(),
                id: *navigation_id,
                certificate_error: true,
                rejected_certificate,
            });
        }
    })?;

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
/// `DevTools` do nothing (ADR-0064, decision 6, and ADR-0065, decision 4).
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

fn address_scheme(address: &str) -> String {
    url::Url::parse(address).map_or_else(|_| "<invalid>".to_owned(), |url| url.scheme().to_owned())
}

fn sanitize_process_description(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_control())
        .take(128)
        .collect()
}

fn web_error_status_name(status: COREWEBVIEW2_WEB_ERROR_STATUS) -> &'static str {
    match status {
        COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN => "unknown",
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT => {
            "certificate_common_name_is_incorrect"
        }
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED => "certificate_expired",
        COREWEBVIEW2_WEB_ERROR_STATUS_CLIENT_CERTIFICATE_CONTAINS_ERRORS => {
            "client_certificate_contains_errors"
        }
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED => "certificate_revoked",
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID => "certificate_is_invalid",
        COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE => "server_unreachable",
        COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT => "timeout",
        COREWEBVIEW2_WEB_ERROR_STATUS_ERROR_HTTP_INVALID_SERVER_RESPONSE => {
            "error_http_invalid_server_response"
        }
        COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED => "connection_aborted",
        COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET => "connection_reset",
        COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED => "disconnected",
        COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT => "cannot_connect",
        COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED => "host_name_not_resolved",
        COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED => "operation_canceled",
        COREWEBVIEW2_WEB_ERROR_STATUS_REDIRECT_FAILED => "redirect_failed",
        COREWEBVIEW2_WEB_ERROR_STATUS_UNEXPECTED_ERROR => "unexpected_error",
        COREWEBVIEW2_WEB_ERROR_STATUS_VALID_AUTHENTICATION_CREDENTIALS_REQUIRED => {
            "valid_authentication_credentials_required"
        }
        COREWEBVIEW2_WEB_ERROR_STATUS_VALID_PROXY_AUTHENTICATION_REQUIRED => {
            "valid_proxy_authentication_required"
        }
        _ => "other",
    }
}

fn process_failed_kind_name(kind: COREWEBVIEW2_PROCESS_FAILED_KIND) -> &'static str {
    match kind {
        COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => "browser_process_exited",
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => "render_process_exited",
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => {
            "render_process_unresponsive"
        }
        COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED => {
            "frame_render_process_exited"
        }
        COREWEBVIEW2_PROCESS_FAILED_KIND_UTILITY_PROCESS_EXITED => "utility_process_exited",
        COREWEBVIEW2_PROCESS_FAILED_KIND_SANDBOX_HELPER_PROCESS_EXITED => {
            "sandbox_helper_process_exited"
        }
        COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED => "gpu_process_exited",
        COREWEBVIEW2_PROCESS_FAILED_KIND_PPAPI_PLUGIN_PROCESS_EXITED => {
            "ppapi_plugin_process_exited"
        }
        COREWEBVIEW2_PROCESS_FAILED_KIND_PPAPI_BROKER_PROCESS_EXITED => {
            "ppapi_broker_process_exited"
        }
        COREWEBVIEW2_PROCESS_FAILED_KIND_UNKNOWN_PROCESS_EXITED => "unknown_process_exited",
        _ => "other",
    }
}

fn process_failed_reason_name(reason: COREWEBVIEW2_PROCESS_FAILED_REASON) -> &'static str {
    match reason {
        COREWEBVIEW2_PROCESS_FAILED_REASON_UNEXPECTED => "unexpected",
        COREWEBVIEW2_PROCESS_FAILED_REASON_UNRESPONSIVE => "unresponsive",
        COREWEBVIEW2_PROCESS_FAILED_REASON_TERMINATED => "terminated",
        COREWEBVIEW2_PROCESS_FAILED_REASON_CRASHED => "crashed",
        COREWEBVIEW2_PROCESS_FAILED_REASON_LAUNCH_FAILED => "launch_failed",
        COREWEBVIEW2_PROCESS_FAILED_REASON_OUT_OF_MEMORY => "out_of_memory",
        COREWEBVIEW2_PROCESS_FAILED_REASON_PROFILE_DELETED => "profile_deleted",
        _ => "other",
    }
}

/// Compares a top-level `NavigationStarting` URL against a `ServerCertificateErrorDetected`
/// request URI after normalizing scheme/host casing and trailing slashes.
fn same_navigation_target(navigation_address: &str, request_uri: &str) -> bool {
    if navigation_address == request_uri {
        return true;
    }
    match (
        url::Url::parse(navigation_address),
        url::Url::parse(request_uri),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
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
