//! The main window. It is created from its configuration in
//! `tauri.conf.json`, with the navigation rules of ADR-0004 added: the webview
//! never leaves the app's origin and never opens another window.

use tauri::webview::NewWindowResponse;
use tauri::{App, AppHandle, Manager, Url, WebviewWindowBuilder};

/// The label of the main window, the only window that receives grants.
pub(crate) const MAIN_WINDOW: &str = "main";

/// Why the main window could not be created.
#[derive(Debug, thiserror::Error)]
pub(crate) enum WindowError {
    /// `tauri.conf.json` has no window with the main window's label.
    #[error("tauri.conf.json has no window labelled {MAIN_WINDOW:?}")]
    MissingConfiguration,
    /// Tauri could not create the window.
    #[error("could not create the main window")]
    Create(#[from] tauri::Error),
}

/// Creates the main window.
pub(crate) fn create_main(app: &App) -> Result<(), WindowError> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == MAIN_WINDOW)
        .ok_or(WindowError::MissingConfiguration)?;
    let dev_url = app.config().build.dev_url.clone();
    WebviewWindowBuilder::from_config(app, config)?
        .on_navigation(move |url| {
            let allowed = is_app_origin(url, dev_url.as_ref());
            if !allowed {
                tracing::warn!(
                    scheme = url.scheme(),
                    host = url.host_str(),
                    "blocked a navigation away from the app"
                );
            }
            allowed
        })
        .on_new_window(|url, _features| {
            tracing::warn!(
                scheme = url.scheme(),
                host = url.host_str(),
                "blocked a request to open a window"
            );
            NewWindowResponse::Deny
        })
        .build()?;
    Ok(())
}

/// Shows the main window and brings it to the front, for example when Mannie
/// is started a second time.
pub(crate) fn show_main(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        tracing::warn!("a second start found no main window to show");
        return;
    };
    let shown = window
        .unminimize()
        .and_then(|()| window.show())
        .and_then(|()| window.set_focus());
    if let Err(error) = shown {
        tracing::warn!(%error, "could not bring the main window to the front");
    }
}

/// Whether a URL belongs to the app: the custom protocol that serves the
/// bundled UI, or the development server in development builds.
fn is_app_origin(url: &Url, dev_url: Option<&Url>) -> bool {
    // Linux serves the app from tauri://localhost, and Windows from
    // https://tauri.localhost because `useHttpsScheme` is set.
    let is_custom_protocol = url.scheme() == "tauri" && url.host_str() == Some("localhost");
    let is_windows_protocol = url.scheme() == "https" && url.host_str() == Some("tauri.localhost");
    let is_dev_server = cfg!(dev) && dev_url.is_some_and(|dev| dev.origin() == url.origin());
    is_custom_protocol || is_windows_protocol || is_dev_server
}

#[cfg(test)]
mod tests {
    use tauri::Url;

    use super::is_app_origin;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn the_app_protocols_are_allowed() {
        assert!(is_app_origin(&url("tauri://localhost/settings"), None));
        assert!(is_app_origin(
            &url("https://tauri.localhost/index.html"),
            None
        ));
    }

    #[test]
    fn other_origins_are_refused() {
        for other in [
            "https://example.com/",
            "http://tauri.localhost/",
            "https://tauri.localhost.example.com/",
            "tauri://example/",
            "file:///C:/Windows/System32/",
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
        ] {
            assert!(!is_app_origin(&url(other), None), "{other}");
        }
    }

    #[test]
    fn the_dev_server_is_allowed_only_in_development_builds() {
        let dev = url("http://127.0.0.1:5173");

        assert_eq!(
            is_app_origin(&url("http://127.0.0.1:5173/settings"), Some(&dev)),
            cfg!(dev)
        );
        assert!(!is_app_origin(&url("http://127.0.0.1:5174/"), Some(&dev)));
    }
}
