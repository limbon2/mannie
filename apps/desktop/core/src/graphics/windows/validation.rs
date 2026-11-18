//! Real child placement and the two framebuffer pixels the validation driver
//! inspects. Readback exists only in validation builds.

use skia_safe::{AlphaType, ColorType, ImageInfo};
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsWindowVisible};

use super::super::GraphicsSurfaceReport;
use super::Surface;

pub(super) struct FramePixels {
    center: Option<[u8; 3]>,
    corner: Option<[u8; 3]>,
}

impl FramePixels {
    pub(super) fn read(target: &mut skia_safe::Surface) -> Self {
        let x = target.width() / 2;
        let y = target.height() / 2;
        Self {
            center: pixel(target, x, y),
            corner: pixel(target, 0, 0),
        }
    }
}

fn pixel(target: &mut skia_safe::Surface, x: i32, y: i32) -> Option<[u8; 3]> {
    let info = ImageInfo::new((1, 1), ColorType::RGBA8888, AlphaType::Opaque, None);
    let mut rgba = [0; 4];
    let read = target.read_pixels(&info, &mut rgba, 4, (x, y));
    read.then_some([rgba[0], rgba[1], rgba[2]])
}

pub(super) fn report(surface: &Surface) -> Option<GraphicsSurfaceReport> {
    let hwnd = surface.child.hwnd;
    let mut rect = RECT::default();
    // SAFETY: the child's window is live on this thread, and rect is writable.
    unsafe { GetWindowRect(hwnd, &raw mut rect) }.ok()?;

    let mut origin = POINT {
        x: rect.left,
        y: rect.top,
    };

    // SAFETY: parent is the window used when creating this child and origin is writable.
    if !unsafe { ScreenToClient(surface.parent, &raw mut origin) }.as_bool() {
        return None;
    }

    // SAFETY: hwnd is the live child being inspected on the UI thread.
    let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
    let pixels = surface.renderer.pixels.as_ref().filter(|_| visible);
    Some(GraphicsSurfaceReport {
        visible,
        x: origin.x,
        y: origin.y,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
        center: pixels.and_then(|sample| sample.center),
        corner: pixels.and_then(|sample| sample.corner),
    })
}
