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
    let mut configured = Cow::Borrowed(config);
    if hides_window() {
        let hidden = configured.to_mut();
        hidden.visible = false;
        hidden.center = false;
        hidden.focus = false;
        // A window that cannot be activated is never brought to the front, also
        // not when it is shown again.
        hidden.focusable = false;
        hidden.skip_taskbar = true;
        hidden.additional_browser_args = Some(BROWSER_ARGUMENTS.to_owned());
    }
    #[cfg(target_os = "windows")]
    if std::env::var("TAURI_WEBVIEW_AUTOMATION").as_deref() == Ok("true") {
        // WebView2 150 and later ignore environment overrides in an elevated
        // host. EdgeDriver supplies its debugging arguments and temporary data
        // folder through those overrides, so this validation-only build passes
        // them through the API instead. This also covers visible test windows.
        // https://learn.microsoft.com/microsoft-edge/webview2/concepts/security
        let arguments = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").ok();
        let data_folder = std::env::var_os("WEBVIEW2_USER_DATA_FOLDER");
        apply_webdriver_configuration(
            configured.to_mut(),
            arguments.as_deref(),
            data_folder.as_deref(),
        );
    }
    configured
}

#[cfg(any(target_os = "windows", test))]
fn apply_webdriver_configuration(
    config: &mut WindowConfig,
    arguments: Option<&str>,
    data_folder: Option<&std::ffi::OsStr>,
) {
    if let Some(arguments) = arguments.filter(|value| !value.is_empty()) {
        let combined = config
            .additional_browser_args
            .get_or_insert_with(String::new);
        if !combined.is_empty() {
            combined.push(' ');
        }
        combined.push_str(arguments);
    }
    if let Some(data_folder) = data_folder.filter(|value| !value.is_empty()) {
        config.data_directory = Some(data_folder.into());
    }
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

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::PathBuf;

    use super::{BROWSER_ARGUMENTS, WindowConfig, apply_webdriver_configuration};

    #[test]
    fn webdriver_settings_keep_the_background_browser_arguments() {
        let mut config = WindowConfig {
            additional_browser_args: Some(BROWSER_ARGUMENTS.to_owned()),
            ..WindowConfig::default()
        };
        let arguments = "--remote-debugging-port=0 --enable-automation";
        let data_folder = OsStr::new(r"C:\Users\CI User\AppData\Local\Temp\driver profile");
        apply_webdriver_configuration(&mut config, Some(arguments), Some(data_folder));
        assert_eq!(
            config.additional_browser_args.as_deref(),
            Some(format!("{BROWSER_ARGUMENTS} {arguments}").as_str())
        );
        assert_eq!(config.data_directory, Some(PathBuf::from(data_folder)));
    }

    #[test]
    fn webdriver_settings_also_apply_without_hidden_window_arguments() {
        let mut config = WindowConfig::default();
        apply_webdriver_configuration(&mut config, Some("--remote-debugging-port=0"), None);
        assert_eq!(
            config.additional_browser_args.as_deref(),
            Some("--remote-debugging-port=0")
        );
        assert_eq!(config.data_directory, None);
    }

    #[test]
    fn missing_or_empty_webdriver_settings_preserve_the_configuration() {
        let mut config = WindowConfig {
            additional_browser_args: Some(BROWSER_ARGUMENTS.to_owned()),
            data_directory: Some(PathBuf::from("existing-profile")),
            ..WindowConfig::default()
        };
        for (arguments, data_folder) in [(None, None), (Some(""), Some(OsStr::new("")))] {
            apply_webdriver_configuration(&mut config, arguments, data_folder);
            assert_eq!(
                config.additional_browser_args.as_deref(),
                Some(BROWSER_ARGUMENTS)
            );
            assert_eq!(
                config.data_directory,
                Some(PathBuf::from("existing-profile"))
            );
        }
    }
}
