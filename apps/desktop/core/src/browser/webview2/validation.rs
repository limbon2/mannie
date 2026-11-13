//! What the end-to-end tests read outside the app webview (V13): the pages'
//! windows. Only validation builds compile this.

use std::sync::Arc;

use tokio::sync::oneshot;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, IsWindowEnabled};
use windows::Win32::UI::WindowsAndMessaging::{GetParent, GetWindowRect, IsChild, IsWindowVisible};

use super::super::engine::SurfaceReport;
use super::{Shared, TabUi, cursor, session_tabs};

/// Reports where each tab's window is, on the UI thread.
pub(super) fn surfaces(shared: &Arc<Shared>, reply: oneshot::Sender<Vec<SurfaceReport>>) {
    // SAFETY: GetFocus reads this thread's focus window.
    let focus = unsafe { GetFocus() };
    let reports = session_tabs(shared.id)
        .into_iter()
        .map(|(tab, ui)| report(tab, &ui, focus))
        .collect();
    let _sent = reply.send(reports);
}

fn report(tab: String, ui: &TabUi, focus: HWND) -> SurfaceReport {
    let container = ui.container;
    let mut rect = RECT::default();
    // SAFETY: the container is a live window, and `rect` is writable.
    let _measured = unsafe { GetWindowRect(container, &raw mut rect) };
    let mut origin = POINT {
        x: rect.left,
        y: rect.top,
    };
    // SAFETY: the container is a live window of this thread.
    if let Ok(parent) = unsafe { GetParent(container) } {
        // SAFETY: the parent is a live window, and `origin` is writable.
        let _converted = unsafe { ScreenToClient(parent, &raw mut origin) };
    }
    // SAFETY: the container is a live window.
    let visible = unsafe { IsWindowVisible(container) }.as_bool();
    // SAFETY: as above.
    let enabled = unsafe { IsWindowEnabled(container) }.as_bool();
    // SAFETY: both handles are plain values; a null focus is no child.
    let focused = !focus.is_invalid() && unsafe { IsChild(container, focus) }.as_bool();
    SurfaceReport {
        tab,
        visible,
        cursor: cursor::position(ui),
        cursor_accent: cursor::accent(ui),
        enabled,
        focused,
        x: origin.x,
        y: origin.y,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
    }
}
