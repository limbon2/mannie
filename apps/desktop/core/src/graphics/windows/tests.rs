//! Exercises WGL presentation on the machine's actual display driver. The
//! parent stays off screen and never takes focus.

use std::sync::{Arc, mpsc};

use mannie_plugin_sdk::services::graphics::{
    GraphicsColor, GraphicsError, GraphicsPlacement, GraphicsRect, GraphicsRectangle, GraphicsScene,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{InvalidateRect, UpdateWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DispatchMessageW, GWL_EXSTYLE, GWL_STYLE, GetWindowLongW, IsWindow, MSG,
    PM_REMOVE, PeekMessageW, SW_HIDE, SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE, ShowWindow,
    WINDOW_EX_STYLE, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP, WS_VISIBLE,
};
use windows::core::w;

use super::{Child, Failure, SURFACES, close, open_child, render, reports};

fn parent() -> Child {
    // SAFETY: STATIC is a system class. The test thread owns and later
    // destroys this top-level window and all children created under it.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            w!("STATIC"),
            w!(""),
            WS_POPUP | WS_CLIPCHILDREN,
            -10_000,
            -10_000,
            300,
            200,
            None,
            None,
            None,
            None,
        )
    }
    .expect("the test's stand-in parent window opens");
    // SAFETY: hwnd is this thread's live window. Showing it without activation
    // permits actual swaps without bringing the test over the owner's UI.
    let _shown = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
    Child {
        hwnd,
        destroying: false,
    }
}

fn placement() -> GraphicsPlacement {
    let bounds = GraphicsRect::new(10.0, 10.0, 100.0, 80.0);
    GraphicsPlacement::new(bounds, bounds, 1.5, true)
}

fn scene() -> GraphicsScene {
    let background = GraphicsColor::new(241, 231, 245);
    let rectangle = GraphicsRectangle::new(0.25, 0.25, 0.5, 0.5, GraphicsColor::new(12, 56, 217));
    GraphicsScene::new(background, rectangle)
}

fn child_handle(lease: u64) -> HWND {
    SURFACES.with_borrow(|surfaces| surfaces[&lease].child.hwnd)
}

#[test]
#[ignore = "requires a Windows OpenGL display driver"]
fn real_wgl_clips_the_shared_webview_across_concurrent_leases() {
    let parent = parent();
    // SAFETY: STATIC is a system class. This test thread owns its parent and
    // destroys the stand-in WebView2 host along with that parent.
    let host = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN,
            0,
            0,
            300,
            200,
            Some(parent.hwnd),
            None,
            None,
            None,
        )
    }
    .unwrap();

    // SAFETY: both style reads name a live HWND owned by this thread.
    let style = || unsafe { GetWindowLongW(host, GWL_STYLE) }.cast_unsigned();
    let original = style();

    for lease in [4, 5] {
        super::parent::clip_siblings(host).unwrap();
        open_child(parent.hwnd, lease, Arc::new(|_| {})).unwrap();

        // SAFETY: the lease keeps this native graphics window alive.
        let ex_style = unsafe { GetWindowLongW(child_handle(lease), GWL_EXSTYLE) }.cast_unsigned();
        assert_eq!(ex_style & WS_EX_TRANSPARENT.0, 0);
    }

    for lease in [4, 5] {
        close(lease);
        assert_eq!(style(), original | WS_CLIPSIBLINGS.0);
    }
}

#[test]
#[ignore = "requires a Windows OpenGL display driver"]
fn real_wgl_tracks_scale_and_restores_after_the_parent_hides_or_minimizes() {
    let parent = parent();
    let (failures, result) = mpsc::sync_channel(1);
    open_child(
        parent.hwnd,
        3,
        Arc::new(move |error| {
            let _sent = failures.try_send(error);
        }),
    )
    .unwrap();

    let child = child_handle(3);
    let mut placement = placement();
    let mut scene = scene();

    for (scale, width) in [(1.0, 100), (2.25, 225), (1.5, 150)] {
        placement.scale_factor = scale;
        render(3, &placement, &scene).unwrap();

        let surface = &reports()[0];
        assert_eq!(surface.width, width);
        assert_eq!(surface.center, Some([12, 56, 217]));
    }

    for mode in [SW_HIDE, SW_SHOWMINNOACTIVE] {
        // SAFETY: parent is this thread's offscreen test window.
        let _hidden = unsafe { ShowWindow(parent.hwnd, mode) };
        SURFACES.with_borrow_mut(|surfaces| surfaces.get_mut(&3).unwrap().renderer.pixels = None);
        scene.rectangle.color.green += 5;
        render(3, &placement, &scene).unwrap();
        assert!(SURFACES.with_borrow(|surfaces| surfaces[&3].renderer.pixels.is_none()));
        // SAFETY: restore this thread's window without activating it.
        let _shown = unsafe { ShowWindow(parent.hwnd, SW_SHOWNOACTIVATE) };
        let mut message = MSG::default();
        let reflow = super::parent::REFLOW;
        // SAFETY: pump only the layout message for the test's live child.
        let posted =
            unsafe { PeekMessageW(&raw mut message, Some(child), reflow, reflow, PM_REMOVE) };
        assert!(posted.as_bool());
        // SAFETY: PeekMessageW produced this message for this UI thread.
        unsafe { DispatchMessageW(&raw const message) };
        // SAFETY: deliver any invalidation caused by the restore to this live child.
        let _updated = unsafe { UpdateWindow(child) };
        let surface = &reports()[0];
        assert!(surface.visible);
        assert_eq!(surface.center, Some([12, scene.rectangle.color.green, 217]));
    }

    close(3);
    assert_eq!(
        result.try_recv().unwrap_err(),
        mpsc::TryRecvError::Disconnected
    );
}

#[test]
#[ignore = "requires a Windows OpenGL display driver"]
fn real_wgl_presents_repaints_and_releases_the_native_child() {
    let parent = parent();
    let (failures, result) = mpsc::sync_channel(1);
    let report_failure: Failure = Arc::new(move |error| {
        let _sent = failures.try_send(error);
    });
    open_child(parent.hwnd, 1, Arc::clone(&report_failure)).unwrap();
    assert!(!reports()[0].visible);
    let child = child_handle(1);
    let mut placement = placement();
    let scene = scene();
    render(1, &placement, &scene).unwrap();

    let report = &reports()[0];
    assert!(report.visible);
    assert_eq!(
        (report.x, report.y, report.width, report.height),
        (15, 15, 150, 120)
    );
    assert_eq!(report.center, Some([12, 56, 217]));
    assert_eq!(report.corner, Some([241, 231, 245]));

    // Clear just the validation samples. New samples prove that Windows
    // requested a fresh draw from the retained scene.
    SURFACES.with_borrow_mut(|surfaces| surfaces.get_mut(&1).unwrap().renderer.pixels = None);
    // SAFETY: this live child belongs to the current thread.
    assert!(unsafe { InvalidateRect(Some(child), None, false) }.as_bool());
    // SAFETY: the test thread dispatches the child's pending paint synchronously.
    assert!(unsafe { UpdateWindow(child) }.as_bool());
    assert_eq!(reports()[0].center, Some([12, 56, 217]));

    placement.visible = false;
    render(1, &placement, &scene).unwrap();
    assert!(!reports()[0].visible);
    placement.visible = true;
    placement.clip.width = 10.0;
    render(1, &placement, &scene).unwrap();
    assert_eq!(reports()[0].width, 15);
    assert_eq!(reports()[0].center, Some([241, 231, 245]));

    close(1);
    // SAFETY: IsWindow accepts a handle to a previously destroyed window.
    assert!(!unsafe { IsWindow(Some(child)) }.as_bool());
    assert!(reports().is_empty());
    assert_eq!(result.try_recv().unwrap_err(), mpsc::TryRecvError::Empty);

    // The main window can close before a plugin has submitted its close.
    open_child(parent.hwnd, 2, report_failure).unwrap();
    drop(parent);
    assert_eq!(result.try_recv().unwrap(), GraphicsError::RenderFailed);
    assert!(reports().is_empty());
    close(2);
}
