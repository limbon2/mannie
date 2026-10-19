//! The main window of validation builds, which only the end-to-end tests run.
//! It stays out of the owner's way: off every screen, without a taskbar
//! button, and never taking the focus, so that a test run does not take over
//! the owner's screen and keyboard. WebDriver reaches the page without the
//! window being seen or focused. `MANNIE_E2E_SHOW_WINDOW=1` keeps the window
//! as `tauri.conf.json` configures it, to watch a test.

use std::borrow::Cow;

use tauri::utils::config::WindowConfig;
use tauri::{LogicalPosition, WebviewWindow};

/// Far beyond any monitor arrangement, and within the coordinates that Windows
/// accepts at every scale factor.
const OFF_SCREEN: f64 = -10_000.0;

/// What WebView2 is started with. A window that no monitor shows counts as
/// occluded, and an occluded page gets no animation frames and slow timers.
/// The first three features are the ones Tauri disables when it is given no
/// arguments.
const BROWSER_ARGUMENTS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,CalculateNativeWinOcclusion";

/// Whether the main window stays out of the owner's way.
pub(crate) fn hides_window() -> bool {
    std::env::var_os("MANNIE_E2E_SHOW_WINDOW").is_none()
}

/// The main window's configuration. A window cannot be created off every
/// screen, so it is created hidden and moved before it is shown.
pub(crate) fn window_config(config: &WindowConfig) -> Cow<'_, WindowConfig> {
    if !hides_window() {
        return Cow::Borrowed(config);
    }
    let mut hidden = config.clone();
    hidden.visible = false;
    hidden.center = false;
    hidden.focus = false;
    // A window that cannot be activated is never brought to the front, also
    // not when it is shown again.
    hidden.focusable = false;
    hidden.skip_taskbar = true;
    hidden.additional_browser_args = Some(BROWSER_ARGUMENTS.to_owned());
    Cow::Owned(hidden)
}

/// Moves the new main window off every screen and shows it there.
pub(crate) fn place_window(window: &WebviewWindow) {
    if !hides_window() {
        return;
    }
    let placed = window
        .set_position(LogicalPosition::new(OFF_SCREEN, OFF_SCREEN))
        .and_then(|()| window.show());
    if let Err(error) = placed {
        tracing::warn!(%error, "could not place the validation window off the screens");
    }
}
