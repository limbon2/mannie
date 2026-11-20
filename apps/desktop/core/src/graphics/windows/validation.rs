//! Real child placement and framebuffer samples for the validation driver.

use mannie_plugin_sdk::services::graphics::{GraphicsScene, GraphicsShape};
use skia_safe::{AlphaType, ColorType, ImageInfo};
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsWindowVisible};

use super::super::GraphicsSurfaceReport;
use super::Surface;
use super::geometry::DrawingArea;

pub(super) struct FramePixels {
    center: Option<[u8; 3]>,
    corner: Option<[u8; 3]>,
    outline_edge: Option<[[u8; 3]; 4]>,
}

impl FramePixels {
    pub(super) fn read(
        target: &mut skia_safe::Surface,
        area: &DrawingArea,
        scene: &GraphicsScene,
    ) -> Self {
        let x = target.width() / 2;
        let y = target.height() / 2;
        Self {
            center: pixel(target, x, y),
            corner: pixel(target, 0, 0),
            outline_edge: outline_edge(target, area, scene),
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the SDK bounds physical coordinates within i32"
)]
fn outline_edge(
    target: &mut skia_safe::Surface,
    area: &DrawingArea,
    scene: &GraphicsScene,
) -> Option<[[u8; 3]; 4]> {
    let GraphicsShape::Rectangle(first) = scene.shapes.first()?;
    let rect = area.rectangle(*first, scene.view);
    let x = rect.left.round() as i32 - 2;
    let y = rect.center_y().floor() as i32;

    if x < 0 || x + 4 > target.width() || !(0..target.height()).contains(&y) {
        return None;
    }

    let info = ImageInfo::new((4, 1), ColorType::RGBA8888, AlphaType::Opaque, None);
    let mut rgba = [0; 16];
    let read = target.read_pixels(&info, &mut rgba, 16, (x, y));
    read.then(|| std::array::from_fn(|i| [rgba[4 * i], rgba[4 * i + 1], rgba[4 * i + 2]]))
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
        outline_edge: pixels.and_then(|sample| sample.outline_edge),
    })
}
