//! Paints the agent's pointer on a native child of the tab container.
//!
//! The window follows the tab's native clipping and is outside the page,
//! its `DevTools` capture and the app's webview. Its shape is a Win32 region
//! because a layered child would require a process-wide manifest change.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller3;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CombineRgn, CreateEllipticRgn, CreatePolygonRgn, DC_BRUSH, DeleteObject, EndPaint,
    FillRect, GetStockObject, HGDIOBJ, HRGN, InvalidateRect, NULL_PEN, PAINTSTRUCT, Polygon,
    RGN_DIFF, RGN_ERROR, RGN_OR, SelectObject, SetDCBrushColor, SetWindowRgn, WINDING,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, HTTRANSPARENT, HWND_TOP, IsWindowVisible,
    KillTimer, PostMessageW, RegisterClassW, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetTimer,
    SetWindowPos, ShowWindow, WM_APP, WM_NCDESTROY, WM_NCHITTEST, WM_PAINT, WM_TIMER, WNDCLASSW,
    WS_CHILD, WS_CLIPSIBLINGS, WS_DISABLED, WS_EX_NOACTIVATE,
};
use windows::core::{Error, Interface as _, Result, w};

use super::TabUi;

const CLASS: windows::core::PCWSTR = w!("MannieBrowserAgentCursor");
const TIMER: usize = 1;
const HIDE_AFTER_NAVIGATION: u32 = WM_APP + 1;
const FRAME_MS: u32 = 16;
const GLIDE: Duration = Duration::from_millis(112);
const PULSE: Duration = Duration::from_millis(280);
const IDLE: Duration = Duration::from_secs(3);
const PAD: f64 = 22.0;
const SIZE: f64 = 46.0;

// COLORREF stores its channels in BGR order.
const ACCENT: COLORREF = COLORREF(0x00f7_782e);
const OUTLINE: COLORREF = COLORREF(0x00ff_ffff);
const BODY: COLORREF = COLORREF(0x0029_2421);
const FACET: COLORREF = COLORREF(0x004b_4541);

#[derive(Clone, Copy)]
struct Motion {
    from: POINT,
    target: POINT,
    started: Instant,
    click: Option<Instant>,
    expires: Instant,
    scale: f64,
}

impl Motion {
    fn point(self, now: Instant) -> POINT {
        let progress = progress(now, self.started, GLIDE);
        let eased = 1.0 - (1.0 - progress).powi(3);
        let tween = |from: i32, target: i32| {
            pixels(
                f64::from(target - from).mul_add(eased, f64::from(from)),
                1.0,
            )
        };
        POINT {
            x: tween(self.from.x, self.target.x),
            y: tween(self.from.y, self.target.y),
        }
    }

    fn pulse(self, now: Instant) -> Option<f64> {
        let start = self.click?;
        let elapsed = now.checked_duration_since(start)?;
        (elapsed < PULSE).then(|| 8.0 + 12.0 * elapsed.as_secs_f64() / PULSE.as_secs_f64())
    }

    fn animating(self, now: Instant) -> bool {
        (self.from != self.target && now.saturating_duration_since(self.started) < GLIDE)
            || self.click.is_some_and(|start| now < start + PULSE)
    }
}

thread_local! {
    // Only the tab's UI thread creates or paints these windows. No borrow
    // is held across a Win32 call, which can reenter the window procedure.
    static CURSORS: RefCell<HashMap<*mut c_void, Motion>> = RefCell::new(HashMap::new());
}

extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCHITTEST => return LRESULT(HTTRANSPARENT as isize),
        WM_PAINT => {
            paint(window);
            return LRESULT(0);
        }
        WM_TIMER if wparam.0 == TIMER => {
            advance(window, Instant::now());
            return LRESULT(0);
        }
        HIDE_AFTER_NAVIGATION => {
            if !CURSORS.with_borrow(|cursors| cursors.contains_key(&window.0)) {
                hide_window(window);
            }
            return LRESULT(0);
        }
        WM_NCDESTROY => {
            CURSORS.with_borrow_mut(|cursors| cursors.remove(&window.0));
        }
        _ => {}
    }
    // SAFETY: the arguments are the ones the system passed to this procedure.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn register_class() -> Result<()> {
    static REGISTERED: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    let registered = REGISTERED.get_or_init(|| {
        // SAFETY: a null module name asks for the executable's module.
        let instance = unsafe { GetModuleHandleW(None) }.map_err(|error| error.to_string())?;
        let class = WNDCLASSW {
            hInstance: instance.into(),
            lpfnWndProc: Some(procedure),
            lpszClassName: CLASS,
            ..WNDCLASSW::default()
        };
        // SAFETY: the class and its static name live through this call.
        if unsafe { RegisterClassW(&raw const class) } == 0 {
            return Err(Error::from_thread().to_string());
        }
        Ok(())
    });
    registered
        .clone()
        .map_err(|message| Error::new(windows::core::HRESULT(-1), message))
}

pub(super) fn create(container: HWND) -> Option<HWND> {
    let create = || {
        register_class()?;
        // SAFETY: the class is registered and its parent is live on this thread.
        unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE,
                CLASS,
                w!(""),
                WS_CHILD | WS_CLIPSIBLINGS | WS_DISABLED,
                0,
                0,
                0,
                0,
                Some(container),
                None,
                None,
                None,
            )
        }
    };
    match create() {
        Ok(window) => Some(window),
        Err(error) => {
            tracing::debug!(%error, "could not create a browser cursor");
            None
        }
    }
}

fn progress(now: Instant, start: Instant, length: Duration) -> f64 {
    (now.saturating_duration_since(start).as_secs_f64() / length.as_secs_f64()).min(1.0)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "native positions are rounded to pixels"
)]
fn pixels(value: f64, scale: f64) -> i32 {
    (value * scale).round() as i32
}

/// Returns monitor scale and the scale of a page CSS pixel, respectively.
fn scales(tab: &TabUi) -> Option<(f64, f64)> {
    let controller3 = tab.controller.cast::<ICoreWebView2Controller3>().ok()?;
    let mut rasterization = 0.0;
    let mut zoom = 0.0;
    // SAFETY: the controller is live on this thread and the outputs are writable.
    unsafe { controller3.RasterizationScale(&raw mut rasterization) }.ok()?;
    // SAFETY: as above.
    unsafe { tab.controller.ZoomFactor(&raw mut zoom) }.ok()?;
    let page = rasterization * zoom;
    (rasterization.is_finite() && rasterization > 0.0 && page.is_finite() && page > 0.0)
        .then_some((rasterization, page))
}

/// Shows the cursor at a `DevTools` viewport coordinate in CSS pixels.
pub(super) fn show(tab: &TabUi, x: f64, y: f64, pulse: bool) {
    let Some(window) = tab.cursor else {
        return;
    };
    // SAFETY: the tab's native container is live on this thread.
    if !unsafe { IsWindowVisible(tab.container) }.as_bool() {
        return;
    }
    // SAFETY: as above. An enabled page is under owner control.
    if unsafe { IsWindowEnabled(tab.container) }.as_bool() {
        return;
    }
    let Some((scale, page_scale)) = scales(tab) else {
        return;
    };
    let target = POINT {
        x: pixels(x, page_scale),
        y: pixels(y, page_scale),
    };
    let mut bounds = RECT::default();
    // SAFETY: the container is live and the bounds are writable.
    if unsafe { GetClientRect(tab.container, &raw mut bounds) }.is_err()
        || !(0..bounds.right).contains(&target.x)
        || !(0..bounds.bottom).contains(&target.y)
    {
        return;
    }
    let now = Instant::now();
    CURSORS.with_borrow_mut(|cursors| {
        let state = cursors.entry(window.0).or_insert(Motion {
            from: target,
            target,
            started: now,
            click: None,
            expires: now + IDLE,
            scale,
        });
        if state.target != target {
            state.from = state.point(now);
            state.target = target;
            state.started = now;
            state.click = None;
        }
        if pulse {
            state.click = Some(if state.from == target {
                now
            } else {
                now.max(state.started + GLIDE)
            });
        }
        state.scale = scale;
        state.expires = now + IDLE;
    });
    advance(window, now);
}

pub(super) fn hide(tab: &TabUi) {
    if let Some(window) = tab.cursor {
        hide_window(window);
    }
}

/// Invalidates a cursor within WebView2's navigation callback. Native
/// window changes are deferred because the browser waits for the callback.
pub(super) fn hide_after_navigation(tab: &TabUi) {
    let Some(window) = tab.cursor else {
        return;
    };
    if CURSORS
        .with_borrow_mut(|cursors| cursors.remove(&window.0))
        .is_none()
    {
        return;
    }
    // SAFETY: the target is a live cursor window. Posting does not call it inline.
    let posted = unsafe { PostMessageW(Some(window), HIDE_AFTER_NAVIGATION, WPARAM(0), LPARAM(0)) };
    if let Err(error) = posted {
        tracing::debug!(%error, "could not post a browser cursor hide");
    }
}

fn hide_window(window: HWND) {
    CURSORS.with_borrow_mut(|cursors| cursors.remove(&window.0));
    // SAFETY: the cursor belongs to this thread; it may already have no timer.
    drop(unsafe { KillTimer(Some(window), TIMER) });
    // SAFETY: the cursor belongs to this thread and hiding it does not activate it.
    let _was_visible = unsafe { ShowWindow(window, SW_HIDE) };
}

fn advance(window: HWND, now: Instant) {
    let Some(state) = CURSORS.with_borrow(|cursors| cursors.get(&window.0).copied()) else {
        hide_window(window);
        return;
    };
    if now >= state.expires {
        hide_window(window);
        return;
    }
    if let Err(error) = render(window, state, now) {
        tracing::debug!(%error, "could not draw a browser cursor");
        hide_window(window);
        return;
    }
    let wait = if state.animating(now) {
        FRAME_MS
    } else {
        u32::try_from(state.expires.saturating_duration_since(now).as_millis()).unwrap_or(FRAME_MS)
    };
    // SAFETY: a plain timer ID and interval on a live window of this thread.
    if unsafe { SetTimer(Some(window), TIMER, wait, None) } == 0 {
        hide_window(window);
    }
}

fn render(window: HWND, state: Motion, now: Instant) -> Result<()> {
    let padding = pixels(PAD, state.scale);
    let size = pixels(SIZE, state.scale);
    let point = state.point(now);
    let shape = glyph(state.scale, &[(0, 0), (21, 10), (11, 13), (5, 23)]);
    // SAFETY: the polygon is an array of four valid points.
    let region = unsafe { CreatePolygonRgn(&shape, WINDING) };
    if region.is_invalid() {
        return Err(Error::from_thread());
    }
    if let Some(radius) = state.pulse(now) {
        add_ring(
            region,
            padding,
            pixels(radius, state.scale),
            pixels(2.0, state.scale).max(1),
        );
    }
    // SAFETY: the window and region are live. On success the system owns the region.
    if unsafe { SetWindowRgn(window, Some(region), true) } == 0 {
        delete_region(region);
        return Err(Error::from_thread());
    }
    // SAFETY: this is a live child window and the bounds are relative to its parent.
    unsafe {
        SetWindowPos(
            window,
            Some(HWND_TOP),
            point.x - padding,
            point.y - padding,
            size,
            size,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    }?;
    // SAFETY: the cursor is a live window. This refreshes the glyph after a pulse.
    let _invalidated = unsafe { InvalidateRect(Some(window), None, false) };
    Ok(())
}

fn add_ring(region: HRGN, center: i32, radius: i32, stroke: i32) {
    let circle = |radius| {
        // SAFETY: these are small relative coordinates.
        unsafe {
            CreateEllipticRgn(
                center - radius,
                center - radius,
                center + radius + 1,
                center + radius + 1,
            )
        }
    };
    let outer = circle(radius);
    let inner = circle(radius.saturating_sub(stroke).max(0));
    if !outer.is_invalid() && !inner.is_invalid() {
        // SAFETY: both regions are live; CombineRgn supports writing into its first source.
        let ring = unsafe { CombineRgn(Some(outer), Some(outer), Some(inner), RGN_DIFF) };
        if ring != RGN_ERROR {
            // SAFETY: the two regions are live; the result replaces the cursor region.
            unsafe { CombineRgn(Some(region), Some(region), Some(outer), RGN_OR) };
        }
    }
    delete_region(inner);
    delete_region(outer);
}

fn delete_region(region: HRGN) {
    if !region.is_invalid() {
        // SAFETY: the caller owns the region and has not transferred it to Win32.
        let _deleted = unsafe { DeleteObject(HGDIOBJ(region.0)) };
    }
}

fn glyph<const N: usize>(scale: f64, points: &[(i32, i32); N]) -> [POINT; N] {
    let coordinate = |value: i32| pixels(PAD + f64::from(value), scale);
    points.map(|(x, y)| POINT {
        x: coordinate(x),
        y: coordinate(y),
    })
}

fn paint(window: HWND) {
    let state = CURSORS.with_borrow(|cursors| cursors.get(&window.0).copied());
    let mut paint = PAINTSTRUCT::default();
    // SAFETY: the system sent WM_PAINT for this window and paint is writable.
    let dc = unsafe { BeginPaint(window, &raw mut paint) };
    if let Some(state) = state {
        // SAFETY: the stock pen and brush are process-wide GDI objects.
        let pen = unsafe { GetStockObject(NULL_PEN) };
        // SAFETY: as above.
        let brush = unsafe { GetStockObject(DC_BRUSH) };
        // SAFETY: dc is the paint context and the stock objects are live.
        unsafe { SelectObject(dc, pen) };
        // SAFETY: as above.
        unsafe { SelectObject(dc, brush) };
        // SAFETY: the paint context and brush are live and the paint bounds are readable.
        unsafe { SetDCBrushColor(dc, ACCENT) };
        // SAFETY: as above. The region clips the fill to the arrowhead and ring.
        unsafe {
            FillRect(
                dc,
                &raw const paint.rcPaint,
                windows::Win32::Graphics::Gdi::HBRUSH(brush.0),
            )
        };
        let draw = |color, points: &[_]| {
            // SAFETY: the stock brush is selected into this paint context.
            unsafe { SetDCBrushColor(dc, color) };
            // SAFETY: the context is live and the polygon points are readable.
            let _drawn = unsafe { Polygon(dc, points) };
        };
        draw(
            OUTLINE,
            &glyph(state.scale, &[(0, 0), (21, 10), (11, 13), (5, 23)]),
        );
        draw(
            BODY,
            &glyph(state.scale, &[(2, 3), (17, 10), (9, 12), (6, 19)]),
        );
        draw(FACET, &glyph(state.scale, &[(2, 3), (9, 12), (6, 19)]));
    }
    // SAFETY: this closes the BeginPaint above for the same window and paint structure.
    let _ended = unsafe { EndPaint(window, &raw const paint) };
}

#[cfg(feature = "validation")]
pub(super) fn position(tab: &TabUi) -> Option<(i32, i32)> {
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let window = tab.cursor?;
    // SAFETY: the cursor is live on this thread.
    if !unsafe { IsWindowVisible(window) }.as_bool() {
        return None;
    }
    let state = CURSORS.with_borrow(|cursors| cursors.get(&window.0).copied())?;
    let mut bounds = RECT::default();
    // SAFETY: the cursor is live and the bounds are writable.
    unsafe { GetWindowRect(window, &raw mut bounds) }.ok()?;
    let padding = pixels(PAD, state.scale);
    let mut point = POINT {
        x: bounds.left + padding,
        y: bounds.top + padding,
    };
    // SAFETY: the container is live and the point is writable.
    unsafe { ScreenToClient(tab.container, &raw mut point) }
        .as_bool()
        .then_some((point.x, point.y))
}
