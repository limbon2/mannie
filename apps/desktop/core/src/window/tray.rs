//! The tray icon (docs/features/window-and-tray.md, tray). Its labels are
//! visible text, so they come from the window's message catalog: the tray
//! appears once the window sends them, and follows each later change.

use mannie_types::window::TrayLabels;
use tauri::AppHandle;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

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

/// Creates the tray with `labels`, or gives the existing tray the new labels.
pub(crate) fn apply_labels(app: &AppHandle, labels: &TrayLabels) -> Result<(), TrayError> {
    let open = MenuItem::with_id(app, OPEN_ITEM, &labels.open, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT_ITEM, &labels.quit, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(menu))?;
        tray.set_tooltip(Some(&labels.tooltip))?;
        return Ok(());
    }
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or(TrayError::MissingIcon)?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip(&labels.tooltip)
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
