//! The tray icon (docs/features/window-and-tray.md, tray; ADR-0074). Its
//! labels are visible text, so they come from the window's message catalog:
//! the tray appears once the window sends them, and follows each later change.

use mannie_types::window::TrayLabels;
use tauri::AppHandle;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

use crate::notifications::badge;

use super::close::quit_from_tray;
use super::show_main;

/// The ID of Mannie's tray icon.
pub(crate) const TRAY_ID: &str = "main";

/// The ID of the menu item that shows the window.
pub(crate) const OPEN_ITEM: &str = "open";

/// The ID of the menu item that quits.
pub(crate) const QUIT_ITEM: &str = "quit";

/// Why the tray could not be set up.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TrayError {
    /// The bundle has no icon for the tray.
    #[error("the app has no icon for the tray")]
    MissingIcon,
    /// Tauri could not create the menu or the icon, for example without a
    /// tray host on Linux.
    #[error("the tray could not be created")]
    Tauri(#[from] tauri::Error),
}

fn tray_icon_image(app: &AppHandle, badged: bool) -> Result<Image<'static>, TrayError> {
    let base = app.default_window_icon().ok_or(TrayError::MissingIcon)?;
    if badged && let Some(with_dot) = badge::with_badge_dot(base) {
        return Ok(with_dot);
    }
    Ok(base.clone().to_owned())
}

/// Creates the tray with `labels`, or gives the existing tray the new labels
/// and current badge state.
pub(crate) fn apply_labels(
    app: &AppHandle,
    labels: &TrayLabels,
    badged: bool,
    attention_tooltip: Option<&str>,
) -> Result<(), TrayError> {
    let open = MenuItem::with_id(app, OPEN_ITEM, &labels.open, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT_ITEM, &labels.quit, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let tooltip = if badged {
        attention_tooltip.unwrap_or(&labels.tooltip)
    } else {
        &labels.tooltip
    };
    let icon = tray_icon_image(app, badged)?;
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(menu))?;
        tray.set_tooltip(Some(tooltip))?;
        tray.set_icon(Some(icon))?;
        return Ok(());
    }
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip(tooltip)
        .menu(&menu)
        // On Windows a left click shows the window and a right click opens
        // the menu. Linux opens the menu on any click.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| on_menu_item(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    tracing::info!("created the tray icon");
    Ok(())
}

/// Updates the tray icon's badge and tooltip when the tray already exists.
pub(crate) fn apply_badge(
    app: &AppHandle,
    badged: bool,
    tooltip: Option<&str>,
) -> Result<(), TrayError> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };
    let icon = tray_icon_image(app, badged)?;
    tray.set_icon(Some(icon))?;
    if let Some(tooltip) = tooltip {
        tray.set_tooltip(Some(tooltip))?;
    }
    Ok(())
}

/// Runs a menu item of the tray.
pub(crate) fn on_menu_item(app: &AppHandle, item: &str) {
    match item {
        OPEN_ITEM => show_main(app),
        QUIT_ITEM => quit_from_tray(app),
        other => tracing::warn!(item = other, "the tray reported an unknown menu item"),
    }
}

/// Whether the tray icon exists.
#[cfg(feature = "validation")]
pub(crate) fn exists(app: &AppHandle) -> bool {
    app.tray_by_id(TRAY_ID).is_some()
}
