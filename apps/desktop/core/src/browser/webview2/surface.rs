//! The page's native window over the pane: its place, its clip, its
//! visibility, and whether owner input reaches it (ADR-0064, decisions 5
//! and 6).
//!
//! The controller stays visible and only its container hides: WebView2
//! stops rendering, and capturing, a controller that is set invisible
//! (ADR-0064, validation, check 1). A tab that no pane has placed keeps a
//! default size, so an agent can work with it while the pane is closed.

use std::sync::Arc;
use std::sync::OnceLock;

use mannie_plugin_sdk::services::browser::{BrowserPlacement, BrowserRect};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC, ICoreWebView2Controller,
};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, HGDIOBJ, RGN_DIFF, SetWindowRgn,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, HWND_TOP, RegisterClassW,
    SW_HIDE, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
    SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS,
};
use windows::core::{Error, Result, w};

use super::{Shared, cursor, session_tabs, tab_ui};

const CLASS: windows::core::PCWSTR = w!("MannieBrowserPage");

/// The size in pixels of a tab that no pane has placed yet.
const DEFAULT_WIDTH: i32 = 1280;
const DEFAULT_HEIGHT: i32 = 800;

extern "system" fn window_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: the arguments are the ones the system passed to this window
    // procedure, forwarded unchanged.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn register_class() -> Result<()> {
    static REGISTERED: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    let registered = REGISTERED.get_or_init(|| {
        // SAFETY: a null module name asks for this executable's module.
        let instance = unsafe { GetModuleHandleW(None) }.map_err(|error| error.to_string())?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_procedure),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            ..WNDCLASSW::default()
        };
        // SAFETY: the class structure and its static name live until the
        // call returns.
        let atom = unsafe { RegisterClassW(&raw const class) };
        if atom == 0 {
            return Err(Error::from_thread().to_string());
        }
        Ok(())
    });
    registered
        .clone()
        .map_err(|message| Error::new(windows::core::HRESULT(-1), message))
}

/// Registers the page window class for a test's stand-in main window.
#[cfg(test)]
pub(super) fn register_class_for_tests() {
    register_class().expect("the page window class registers");
}

/// Creates a tab's hidden container, above the app webview.
pub(super) fn create_container(parent: HWND, enabled: bool) -> Result<HWND> {
    register_class()?;
    // SAFETY: the class is registered, the parent is the live main window
    // of this thread, and the other arguments are plain values.
    let container = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS,
            w!(""),
            WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            0,
            0,
            DEFAULT_WIDTH,
            DEFAULT_HEIGHT,
            Some(parent),
            None,
            None,
            None,
        )
    }?;
    set_enabled(container, enabled);
    Ok(container)
}

pub(super) fn destroy_container(container: HWND) {
    // SAFETY: the container is a window of this thread that the session
    // created; destroying it twice fails harmlessly.
    if let Err(error) = unsafe { DestroyWindow(container) } {
        tracing::debug!(%error, "a browser page window was already gone");
    }
}

/// Makes a new tab's controller visible inside its hidden container.
pub(super) fn start_hidden(container: HWND, controller: &ICoreWebView2Controller) -> Result<()> {
    let bounds = RECT {
        left: 0,
        top: 0,
        right: DEFAULT_WIDTH,
        bottom: DEFAULT_HEIGHT,
    };
    // SAFETY: plain values on a live controller of this thread.
    unsafe { controller.SetBounds(bounds) }?;
    // SAFETY: as above.
    unsafe { controller.SetIsVisible(true) }?;
    hide(container);
    Ok(())
}

fn hide(container: HWND) {
    // SAFETY: the container is a live window of this thread.
    let _was_visible = unsafe { ShowWindow(container, SW_HIDE) };
}

fn set_enabled(container: HWND, enabled: bool) {
    // SAFETY: the container is a live window of this thread.
    let _was_disabled = unsafe { EnableWindow(container, enabled) };
}

/// The rectangle in the parent's client pixels, clamped to its client area.
/// The app webview is untrusted, so nothing it reports can put a page
/// outside the window (ADR-0004).
#[expect(
    clippy::cast_possible_truncation,
    reason = "the values are rounded and clamped to a window's client area first"
)]
fn physical(rect: &BrowserRect, scale: f64, client: RECT) -> RECT {
    let clamp_x = |value: f64| {
        (value * scale)
            .round()
            .clamp(f64::from(client.left), f64::from(client.right)) as i32
    };
    let clamp_y = |value: f64| {
        (value * scale)
            .round()
            .clamp(f64::from(client.top), f64::from(client.bottom)) as i32
    };
    RECT {
        left: clamp_x(rect.x),
        top: clamp_y(rect.y),
        right: clamp_x(rect.x + rect.width),
        bottom: clamp_y(rect.y + rect.height),
    }
}

/// Moves a tab's container over the pane's placeholder, on the main thread.
pub(super) fn place(session: u64, placement: &BrowserPlacement) {
    let Some(tab) = tab_ui(session, &placement.tab_id) else {
        return;
    };
    // SAFETY: the container is a live window of this thread.
    let Ok(parent) = (unsafe { windows::Win32::UI::WindowsAndMessaging::GetParent(tab.container) })
    else {
        return;
    };
    let mut client = RECT::default();
    // SAFETY: the parent is a live window, and `client` is writable.
    if unsafe { GetClientRect(parent, &raw mut client) }.is_err() {
        return;
    }
    let area = physical(&placement.rect, placement.scale, client);
    let width = area.right - area.left;
    let height = area.bottom - area.top;
    if !placement.visible || width <= 0 || height <= 0 {
        cursor::hide(&tab);
        hide(tab.container);
        return;
    }
    let clips: Vec<RECT> = placement
        .clip
        .iter()
        .map(|clip| physical(clip, placement.scale, client))
        .collect();
    if let Err(error) = clip(tab.container, area, &clips) {
        tracing::warn!(%error, "could not clip a browser page");
    }
    // SAFETY: the container is a live window of this thread, placed above
    // its siblings, including the app webview.
    let moved = unsafe {
        SetWindowPos(
            tab.container,
            Some(HWND_TOP),
            area.left,
            area.top,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    };
    if let Err(error) = moved {
        tracing::warn!(%error, "could not place a browser page");
        return;
    }
    let bounds = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    // SAFETY: plain values on a live controller of this thread.
    if let Err(error) = unsafe { tab.controller.SetBounds(bounds) } {
        tracing::warn!(%error, "could not size a browser page");
    }
}

/// Leaves the parts of the page that an overlay covers out of its window's
/// region (ADR-0064, validation, check 6).
fn clip(container: HWND, area: RECT, clips: &[RECT]) -> Result<()> {
    if clips.is_empty() {
        // SAFETY: the container is a live window; a null region removes it.
        unsafe { SetWindowRgn(container, None, false) };
        return Ok(());
    }
    // SAFETY: plain values; the result is checked.
    let region = unsafe { CreateRectRgn(0, 0, area.right - area.left, area.bottom - area.top) };
    if region.is_invalid() {
        return Err(Error::from_thread());
    }
    for covered in clips {
        // SAFETY: plain values; the result is checked.
        let hole = unsafe {
            CreateRectRgn(
                covered.left - area.left,
                covered.top - area.top,
                covered.right - area.left,
                covered.bottom - area.top,
            )
        };
        if hole.is_invalid() {
            continue;
        }
        // SAFETY: both regions are live, and the result replaces `region`.
        unsafe { CombineRgn(Some(region), Some(region), Some(hole), RGN_DIFF) };
        // SAFETY: the hole is a region this function created and no longer
        // uses.
        let _deleted = unsafe { DeleteObject(HGDIOBJ(hole.0)) };
    }
    // SAFETY: the container is a live window, and the system owns the region
    // once the call succeeds.
    unsafe { SetWindowRgn(container, Some(region), false) };
    Ok(())
}

/// Hides every page of a session, on the main thread.
pub(super) fn hide_all(session: u64) {
    for (_, tab) in session_tabs(session) {
        cursor::hide(&tab);
        // SAFETY: the container is a live window of this thread. It keeps
        // its place and size, so the page keeps its layout.
        let _hidden = unsafe {
            SetWindowPos(
                tab.container,
                None,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_HIDEWINDOW,
            )
        };
    }
}

/// Gives the pages to the owner, or takes them back for the agent. Focus
/// returns to the app before a page is disabled, because a page that keeps
/// focus still receives keys (ADR-0064, validation, check 2).
pub(super) fn set_owner(shared: &Arc<Shared>, owner: bool) {
    let tabs = session_tabs(shared.id);
    if !owner {
        focus_main_window(&shared.ui);
    }
    for (_, tab) in tabs {
        if owner {
            cursor::hide(&tab);
        }
        set_enabled(tab.container, owner);
    }
}

/// Takes the system's keyboard focus from a page to the main window, whose
/// webview takes it from there, on the main thread. Never inside a WebView2
/// handler: the browser process waits for the handler, and this sends a
/// message to its window.
pub(super) fn focus_main_window(ui: &super::UiThread) {
    if let Some(parent) = ui.main_window() {
        // SAFETY: the main window is a live window of this thread.
        let _previous = unsafe { SetFocus(Some(parent)) };
    }
}

/// Moves keyboard focus into a tab's page, on the main thread.
pub(super) fn focus(session: u64, tab: &str) {
    let Some(tab) = tab_ui(session, tab) else {
        return;
    };
    // SAFETY: a plain value on a live controller of this thread.
    if let Err(error) = unsafe {
        tab.controller
            .MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC)
    } {
        tracing::warn!(%error, "could not focus a browser page");
    }
}

#[cfg(test)]
mod tests {
    use super::{BrowserRect, RECT, physical};

    #[test]
    fn placement_is_scaled_and_clamped_to_the_client_area() {
        let client = RECT {
            left: 0,
            top: 0,
            right: 1000,
            bottom: 800,
        };
        let inside = physical(
            &BrowserRect {
                x: 100.0,
                y: 50.0,
                width: 200.0,
                height: 100.0,
            },
            1.5,
            client,
        );
        assert_eq!(
            (inside.left, inside.top, inside.right, inside.bottom),
            (150, 75, 450, 225)
        );
        let outside = physical(
            &BrowserRect {
                x: -500.0,
                y: 700.0,
                width: 5000.0,
                height: 5000.0,
            },
            1.0,
            client,
        );
        assert_eq!(
            (outside.left, outside.top, outside.right, outside.bottom),
            (0, 700, 1000, 800)
        );
    }
}
