//! The main webview must clip higher siblings. The main window also notifies
//! the native child when a restore or resize can change its visible area.

use std::sync::mpsc;

use mannie_plugin_sdk::services::graphics::GraphicsError;
use tauri::WebviewWindow;
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetWindowLongW, PostMessageW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SetWindowLongW, SetWindowPos, WM_APP, WM_WINDOWPOSCHANGED,
    WS_CLIPSIBLINGS,
};

use super::on_surface;

pub(super) const REFLOW: u32 = WM_APP + 1;

pub(super) fn clip_webview(window: &WebviewWindow) -> Result<(), GraphicsError> {
    let (send, receive) = mpsc::sync_channel(1);
    // The caller is already on the UI thread, so Wry calls this closure inline.
    window
        .with_webview(move |webview| {
            let _sent = send.send(clip_controller_host(&webview.controller()));
        })
        .map_err(|error| {
            tracing::warn!(%error, "could not access the main webview controller");
            GraphicsError::GraphicsUnavailable
        })?;

    receive
        .try_recv()
        .map_err(|error| {
            tracing::warn!(%error, "the webview controller did not run on the window thread");
            GraphicsError::GraphicsUnavailable
        })?
        .map_err(|error| {
            tracing::warn!(%error, "could not clip the webview around native graphics");
            GraphicsError::GraphicsUnavailable
        })
}

fn clip_controller_host(controller: &ICoreWebView2Controller) -> windows::core::Result<()> {
    let mut host = HWND::default();
    // SAFETY: this is the live main webview's controller on its owning thread.
    unsafe { controller.ParentWindow(&raw mut host) }?;
    clip_siblings(host)
}

pub(super) fn clip_siblings(host: HWND) -> windows::core::Result<()> {
    // The WebView2 host can otherwise paint over the graphics HWND even when
    // the graphics HWND is higher in the Z order. Keep this style for the
    // host's lifetime: each higher visible sibling clips only its own bounds.
    // SAFETY: host is the webview's live child window on this thread.
    let style = unsafe { GetWindowLongW(host, GWL_STYLE) };
    let clip = WS_CLIPSIBLINGS.0.cast_signed();

    if style & clip != 0 {
        return Ok(());
    }

    // A child window's previous style always contains WS_CHILD, so zero
    // indicates failure here. Preserve every other bit of the current style.
    // SAFETY: GWL_STYLE holds this window's 32-bit style, not a pointer.
    if unsafe { SetWindowLongW(host, GWL_STYLE, style | clip) } == 0 {
        return Err(windows::core::Error::from_thread());
    }

    // SAFETY: apply the new style without moving, raising or activating the host.
    unsafe {
        SetWindowPos(
            host,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    }
}

pub(super) struct ParentObserver {
    parent: HWND,
    child: HWND,
}

impl ParentObserver {
    pub(super) fn new(parent: HWND, child: HWND) -> Result<Self, GraphicsError> {
        // SAFETY: both windows belong to this thread. The callback and its ID
        // remain valid until Drop removes them, before the child is destroyed.
        let installed = unsafe { SetWindowSubclass(parent, Some(parent_proc), child.0.addr(), 0) };

        if !installed.as_bool() {
            return Err(GraphicsError::GraphicsUnavailable);
        }

        Ok(Self { parent, child })
    }
}

impl Drop for ParentObserver {
    fn drop(&mut self) {
        // SAFETY: these are the window, function and ID of this registration.
        let _removed =
            unsafe { RemoveWindowSubclass(self.parent, Some(parent_proc), self.child.0.addr()) };
    }
}

extern "system" fn parent_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    child: usize,
    _: usize,
) -> LRESULT {
    if message == WM_WINDOWPOSCHANGED {
        let child = HWND(std::ptr::without_provenance_mut(child));
        on_surface(child, |surface| {
            if !surface.layout_pending {
                // SAFETY: the registry lookup established the live child and the
                // message contains no pointers. Reflow after the parent's stack unwinds.
                unsafe { PostMessageW(Some(child), REFLOW, WPARAM(0), LPARAM(0)) }.map_err(
                    |error| {
                        tracing::warn!(%error, "could not schedule a graphics layout");
                        GraphicsError::RenderFailed
                    },
                )?;
                surface.layout_pending = true;
            }

            Ok(())
        });
    }

    // SAFETY: forwards the original window message down the subclass chain.
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}
