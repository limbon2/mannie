//! The Windows child surface and its thread-affine WGL resources (ADR-0073).
#![expect(
    unsafe_code,
    reason = "Win32 window management and WGL context creation require native handles (ADR-0073)"
)]

mod geometry;
mod parent;
mod renderer;
#[cfg(all(test, feature = "validation"))]
mod tests;
#[cfg(feature = "validation")]
mod validation;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::num::NonZeroIsize;
use std::sync::{Arc, OnceLock};

use mannie_plugin_sdk::services::graphics::{GraphicsError, GraphicsPlacement, GraphicsScene};
use raw_window_handle::Win32WindowHandle;
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CS_OWNDC, CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, HTTRANSPARENT,
    HWND_TOP, IsIconic, IsWindowVisible, RegisterClassW, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SetWindowPos, ShowWindow, WM_DESTROY, WM_ERASEBKGND, WM_NCHITTEST, WM_PAINT, WNDCLASSW,
    WS_CHILD, WS_CLIPSIBLINGS, WS_DISABLED, WS_EX_NOACTIVATE,
};
use windows::core::w;

use crate::window::MAIN_WINDOW;

use self::geometry::DrawingArea;
use self::parent::ParentObserver;
use self::renderer::Renderer;

type Failure = Arc<dyn Fn(GraphicsError) + Send + Sync>;

thread_local! {
    // Tauri dispatch and the Win32 child procedure both run on the window thread.
    static SURFACES: RefCell<BTreeMap<u64, Surface>> = const { RefCell::new(BTreeMap::new()) };
}

struct Child {
    hwnd: HWND,
    destroying: bool,
}

impl Drop for Child {
    fn drop(&mut self) {
        if self.destroying {
            return;
        }

        // SAFETY: this thread created and owns this child, and all WGL resources
        // using it have already been dropped.
        if let Err(error) = unsafe { DestroyWindow(self.hwnd) } {
            tracing::debug!(%error, "graphics child window was already destroyed");
        }
    }
}

struct Surface {
    // Rust drops fields in declaration order. The renderer needs the live HWND.
    _observer: ParentObserver,
    renderer: Renderer,
    child: Child,
    parent: HWND,
    on_failure: Failure,
    request: Option<(GraphicsPlacement, GraphicsScene)>,
    area: Option<DrawingArea>,
    layout_pending: bool,
}

impl Surface {
    fn hide(&mut self) {
        self.area = None;

        // SAFETY: this is the child owned by this surface on its window thread.
        let _was_visible = unsafe { ShowWindow(self.child.hwnd, SW_HIDE) };
    }

    fn redraw(&mut self) -> Result<(), GraphicsError> {
        let (Some(area), Some((_, scene))) = (&self.area, &self.request) else {
            return Ok(());
        };

        // SAFETY: both windows are live on this thread. An offscreen validation
        // window is visible by this test, while a minimized or tray window is not.
        let visible = unsafe { IsWindowVisible(self.child.hwnd) }.as_bool();
        // SAFETY: the parent is the live main window.
        let minimized = unsafe { IsIconic(self.parent) }.as_bool();

        if !visible || minimized {
            return Ok(());
        }

        self.renderer.draw(area, scene)
    }

    fn place(
        &mut self,
        placement: &GraphicsPlacement,
        scene: &GraphicsScene,
    ) -> Result<(), GraphicsError> {
        if !placement.valid() || !scene.valid() {
            return Err(GraphicsError::InvalidPlacement);
        }

        self.request = Some((*placement, *scene));
        self.layout()
    }

    fn layout(&mut self) -> Result<(), GraphicsError> {
        let Some((placement, _)) = self.request.as_ref() else {
            return Ok(());
        };

        let mut client = RECT::default();
        // SAFETY: the parent is a live window on this thread and client is writable.
        unsafe { GetClientRect(self.parent, &raw mut client) }.map_err(|error| {
            tracing::warn!(%error, "could not measure the graphics host");
            GraphicsError::RenderFailed
        })?;

        let Some(area) = DrawingArea::from_placement(placement, client) else {
            self.hide();
            return Ok(());
        };

        // SAFETY: this child belongs to the parent whose client area bounded
        // this rectangle. NOACTIVATE preserves keyboard focus in the webview.
        unsafe {
            SetWindowPos(
                self.child.hwnd,
                Some(HWND_TOP),
                area.left,
                area.top,
                area.width,
                area.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        }
        .map_err(|error| {
            tracing::warn!(%error, "could not place the graphics surface");
            GraphicsError::RenderFailed
        })?;

        self.area = Some(area);
        self.redraw()
    }
}

extern "system" fn child_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_NCHITTEST => LRESULT(isize::try_from(HTTRANSPARENT).expect("Win32 hit-test code fits")),
        WM_ERASEBKGND => LRESULT(1),
        parent::REFLOW => {
            on_surface(hwnd, |surface| {
                surface.layout_pending = false;
                surface.layout()
            });
            LRESULT(0)
        }
        WM_DESTROY => {
            // A parent can destroy its children independently of the plugin's
            // lease. Release GL while the HWND and its private DC still exist.
            let removed = SURFACES.try_with(|surfaces| {
                let mut surfaces = surfaces.try_borrow_mut().ok()?;
                let lease = *surfaces
                    .iter()
                    .find(|(_, surface)| surface.child.hwnd == hwnd)?
                    .0;
                surfaces.remove(&lease)
            });

            if let Ok(Some(mut surface)) = removed {
                surface.child.destroying = true;

                let report = Arc::clone(&surface.on_failure);
                drop(surface);
                report(GraphicsError::RenderFailed);
            }

            LRESULT(0)
        }
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            // SAFETY: WM_PAINT names this live window and paint is writable.
            let _dc = unsafe { BeginPaint(hwnd, &raw mut paint) };
            on_surface(hwnd, Surface::redraw);

            // SAFETY: this pairs the BeginPaint above with its original paint structure.
            let _ended = unsafe { EndPaint(hwnd, &raw const paint) };

            LRESULT(0)
        }
        // SAFETY: the OS supplied these arguments to this window procedure.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn on_surface(hwnd: HWND, action: impl FnOnce(&mut Surface) -> Result<(), GraphicsError>) {
    let failed = SURFACES.try_with(|surfaces| {
        // SetWindowPos and destruction can dispatch messages synchronously.
        // A placement already in progress does its own redraw.
        let mut surfaces = surfaces.try_borrow_mut().ok()?;
        let surface = surfaces
            .values_mut()
            .find(|surface| surface.child.hwnd == hwnd)?;

        let error = action(surface).err()?;
        surface.hide();
        Some((Arc::clone(&surface.on_failure), error))
    });

    if let Ok(Some((report, error))) = failed {
        report(error);
    }
}

fn child_window(parent: HWND) -> windows::core::Result<(Child, Win32WindowHandle)> {
    static REGISTERED: OnceLock<windows::core::Result<()>> = OnceLock::new();
    let class = w!("MannieNativeGraphics");
    // SAFETY: None names the live process's executable.
    let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }?.into();
    REGISTERED
        .get_or_init(|| {
            let descriptor = WNDCLASSW {
                style: CS_OWNDC,
                lpfnWndProc: Some(child_proc),
                hInstance: instance,
                lpszClassName: class,
                ..Default::default()
            };

            // SAFETY: descriptor lives through the call; class and procedure are static.
            if unsafe { RegisterClassW(&raw const descriptor) } == 0 {
                return Err(windows::core::Error::from_thread());
            }

            Ok(())
        })
        .clone()?;

    // SAFETY: parent is Tauri's main HWND on this thread. The private class's
    // DC lives as long as its child. The drawable must be opaque for sibling
    // clipping to work. WS_DISABLED and WM_NCHITTEST keep input in the webview.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE,
            class,
            w!(""),
            WS_CHILD | WS_CLIPSIBLINGS | WS_DISABLED,
            0,
            0,
            1,
            1,
            Some(parent),
            None,
            Some(instance),
            None,
        )
    }?;

    let child = Child {
        hwnd,
        destroying: false,
    };
    let mut raw = Win32WindowHandle::new(
        NonZeroIsize::new(hwnd.0 as isize).ok_or_else(windows::core::Error::from_thread)?,
    );
    raw.hinstance = NonZeroIsize::new(instance.0 as isize);
    Ok((child, raw))
}

/// Initializes an invisible native child on the desktop UI thread.
pub(super) fn open(app: &AppHandle, lease: u64, on_failure: Failure) -> Result<(), GraphicsError> {
    let window = app
        .get_webview_window(MAIN_WINDOW)
        .ok_or(GraphicsError::GraphicsUnavailable)?;

    let parent = window.hwnd().map_err(|error| {
        tracing::warn!(%error, "could not access the graphics host window");
        GraphicsError::GraphicsUnavailable
    })?;

    parent::clip_webview(&window)?;
    open_child(HWND(parent.0), lease, on_failure)
}

fn open_child(parent: HWND, lease: u64, on_failure: Failure) -> Result<(), GraphicsError> {
    let (child, raw) = child_window(parent).map_err(|error| {
        tracing::warn!(%error, "could not open the graphics child window");
        GraphicsError::GraphicsUnavailable
    })?;

    let renderer = Renderer::new(raw).map_err(|error| {
        tracing::warn!(%error, "could not initialize the OpenGL renderer");
        GraphicsError::GraphicsUnavailable
    })?;

    let observer = ParentObserver::new(parent, child.hwnd)?;
    let surface = Surface {
        _observer: observer,
        renderer,
        child,
        parent,
        on_failure,
        request: None,
        area: None,
        layout_pending: false,
    };

    SURFACES.with_borrow_mut(|surfaces| surfaces.insert(lease, surface));
    Ok(())
}

/// Repositions the child and presents the requested frame on the UI thread.
pub(super) fn render(
    lease: u64,
    placement: &GraphicsPlacement,
    scene: &GraphicsScene,
) -> Result<(), GraphicsError> {
    SURFACES.with_borrow_mut(|surfaces| {
        let surface = surfaces.get_mut(&lease).ok_or(GraphicsError::StaleLease)?;
        surface.place(placement, scene)
    })
}

/// Releases the WGL objects and then destroys the child on the UI thread.
pub(super) fn close(lease: u64) {
    let surface = SURFACES.with_borrow_mut(|surfaces| surfaces.remove(&lease));
    drop(surface);
}

#[cfg(feature = "validation")]
pub(super) fn reports() -> Vec<super::GraphicsSurfaceReport> {
    SURFACES.with_borrow(|surfaces| surfaces.values().filter_map(validation::report).collect())
}
