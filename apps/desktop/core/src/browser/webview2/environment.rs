//! A session's WebView2 environment: its private folder, its switches and
//! the job that holds its browser process.

use std::collections::hash_map::Entry;
use std::path::Path;
use std::sync::Arc;

use mannie_tools::browser::{ChromeError, ProcessHandle};
use tokio::sync::oneshot;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_PROCESS_KIND_BROWSER, CreateCoreWebView2EnvironmentWithOptions,
    GetAvailableCoreWebView2BrowserVersionString, ICoreWebView2Environment,
    ICoreWebView2Environment7, ICoreWebView2Environment8, ICoreWebView2EnvironmentOptions,
};
use webview2_com::{
    BrowserProcessExitedEventHandler, CoreWebView2EnvironmentOptions,
    CreateCoreWebView2EnvironmentCompletedHandler, ProcessInfosChangedEventHandler, take_pwstr,
};
use windows::core::{HSTRING, Interface as _, PCWSTR, PWSTR};

use super::{SESSIONS, SessionUi, Shared, lock};

/// The oldest runtime whose `DevTools` and event APIs this host uses.
const MINIMUM_MAJOR_VERSION: u32 = 120;

/// The switches of every session. The back-forward cache, prerendering and
/// speculative prefetch would load documents that request interception does
/// not see (ADR-0064, decision 8). Occlusion tracking would stop rendering
/// while the app is minimized or in the tray.
const ARGUMENTS: &str = "--disable-features=BackForwardCache,Prerender2,SpeculationRulesPrefetch,CalculateNativeWinOcclusion";

/// The installed runtime's version, or `None` without one.
pub(super) fn runtime_version() -> Option<String> {
    let mut version = PWSTR::null();
    // SAFETY: a null browser folder asks for the installed runtime, and
    // `version` is a writable pointer that the call fills or leaves null.
    let found =
        unsafe { GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), &raw mut version) };
    if found.is_err() || version.is_null() {
        return None;
    }
    let version = take_pwstr(version);
    let major = version.split('.').next()?.parse::<u32>().ok()?;
    (major >= MINIMUM_MAJOR_VERSION).then_some(version)
}

/// Starts creating the environment, on the main thread. `created` answers
/// once the environment exists and its browser process is in the job.
pub(super) fn create(
    shared: &Arc<Shared>,
    folder: &Path,
    created: oneshot::Sender<Result<(), ChromeError>>,
) {
    let Some(parent) = shared.ui.main_window() else {
        let _sent = created.send(Err(ChromeError::Launch));
        return;
    };
    let options = CoreWebView2EnvironmentOptions::default();
    // SAFETY: the options object is new and not shared, so nothing reads
    // the string while it is set.
    unsafe { options.set_additional_browser_arguments(ARGUMENTS.to_owned()) };
    let options: ICoreWebView2EnvironmentOptions = options.into();
    let requested = folder.to_path_buf();
    let completing = Arc::clone(shared);
    let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
        move |result, environment| {
            let outcome = result
                .map_err(|error| {
                    tracing::warn!(%error, "WebView2 could not create a browser environment");
                    ChromeError::Launch
                })
                .and_then(|()| environment.ok_or(ChromeError::Launch))
                .and_then(|environment| adopt(&completing, &requested, environment, parent));
            let _sent = created.send(outcome);
            Ok(())
        },
    ));
    let folder = HSTRING::from(folder.as_os_str());
    // SAFETY: the folder string and the options live until the call
    // returns, and the handler is a COM object that WebView2 keeps.
    let started = unsafe {
        CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), &folder, &options, &handler)
    };
    if let Err(error) = started {
        tracing::warn!(%error, "WebView2 refused to create a browser environment");
    }
}

/// Checks a new environment and puts its browser process into the job.
fn adopt(
    shared: &Arc<Shared>,
    requested: &Path,
    environment: ICoreWebView2Environment,
    parent: windows::Win32::Foundation::HWND,
) -> Result<(), ChromeError> {
    // An environment variable or a policy can replace the folder that the
    // host asked for (ADR-0064, validation, check 5).
    let environment7: ICoreWebView2Environment7 = environment
        .cast()
        .map_err(|_old| ChromeError::Incompatible)?;
    let mut folder = PWSTR::null();
    // SAFETY: `folder` is a writable pointer that the call fills.
    unsafe { environment7.UserDataFolder(&raw mut folder) }
        .map_err(|_failed| ChromeError::Incompatible)?;
    let folder = take_pwstr(folder);
    if !same_folder(Path::new(&folder), requested) {
        tracing::error!("a browser environment does not use its own folder; it is refused");
        return Err(ChromeError::Profile);
    }

    let environment8: ICoreWebView2Environment8 = environment
        .cast()
        .map_err(|_old| ChromeError::Incompatible)?;
    // The list can still be empty here; the first tab adds its browser
    // process too.
    record_processes(shared, &environment8)?;
    let watching = Arc::clone(shared);
    let changed = ProcessInfosChangedEventHandler::create(Box::new(move |sender, _| {
        if let Some(environment) = sender.and_then(|sender| sender.cast().ok()) {
            let _recorded = record_processes(&watching, &environment);
        }
        Ok(())
    }));
    let mut token = 0;
    // SAFETY: the handler is a COM object that WebView2 keeps, and `token`
    // is writable.
    unsafe { environment8.add_ProcessInfosChanged(&changed, &raw mut token) }
        .map_err(|_failed| ChromeError::Incompatible)?;

    let environment5 = environment
        .cast::<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment5>()
        .map_err(|_old| ChromeError::Incompatible)?;
    let exited = Arc::clone(shared);
    let browser_exited = BrowserProcessExitedEventHandler::create(Box::new(move |_, _| {
        exited.fail();
        Ok(())
    }));
    let mut token = 0;
    // SAFETY: as above.
    unsafe { environment5.add_BrowserProcessExited(&browser_exited, &raw mut token) }
        .map_err(|_failed| ChromeError::Incompatible)?;

    SESSIONS.with_borrow_mut(|sessions| {
        sessions.insert(
            shared.id,
            SessionUi {
                environment,
                parent,
                tabs: std::collections::HashMap::new(),
                popups: std::collections::HashMap::new(),
            },
        );
    });
    Ok(())
}

/// Records the session's processes, and puts its browser process into the
/// job. The engine's sandboxed processes leave the job; teardown ends them
/// through the browser process (ADR-0064, decision 4).
fn record_processes(
    shared: &Shared,
    environment: &ICoreWebView2Environment8,
) -> Result<(), ChromeError> {
    // SAFETY: the environment is live on this thread.
    let infos = unsafe { environment.GetProcessInfos() }.map_err(|_failed| ChromeError::Launch)?;
    let mut count = 0;
    // SAFETY: `count` is writable.
    unsafe { infos.Count(&raw mut count) }.map_err(|_failed| ChromeError::Launch)?;
    for index in 0..count {
        // SAFETY: the index is below the collection's count.
        let Ok(info) = (unsafe { infos.GetValueAtIndex(index) }) else {
            continue;
        };
        let mut process = 0;
        // SAFETY: `process` is writable.
        if unsafe { info.ProcessId(&raw mut process) }.is_err() {
            continue;
        }
        let Ok(process) = u32::try_from(process) else {
            continue;
        };
        let mut kind = COREWEBVIEW2_PROCESS_KIND_BROWSER;
        // SAFETY: `kind` is writable.
        if unsafe { info.Kind(&raw mut kind) }.is_err() {
            continue;
        }
        if kind == COREWEBVIEW2_PROCESS_KIND_BROWSER {
            adopt_browser_process(shared, process)?;
        } else if let Entry::Vacant(entry) = lock(&shared.processes).entry(process) {
            match ProcessHandle::open(process) {
                Ok(handle) => {
                    entry.insert(handle);
                }
                Err(error) => tracing::debug!(%error, "could not track a Browser subprocess"),
            }
        }
    }
    Ok(())
}

/// Puts the session's browser process into its job, once.
pub(super) fn adopt_browser_process(shared: &Shared, process: u32) -> Result<(), ChromeError> {
    let mut processes = lock(&shared.processes);
    let handle = shared.job.adopt(process).map_err(|error| {
        tracing::warn!(%error, "could not put a browser process into its job");
        ChromeError::Launch
    })?;
    processes.insert(process, handle);
    Ok(())
}

fn same_folder(reported: &Path, requested: &Path) -> bool {
    let canonical = |path: &Path| {
        without_verbatim_prefix(
            &std::fs::canonicalize(path).unwrap_or_else(|_missing| path.to_path_buf()),
        )
        .to_string_lossy()
        .to_lowercase()
    };
    canonical(reported) == canonical(requested)
}

/// Drops the `\\?\` prefix that canonicalization adds on Windows.
fn without_verbatim_prefix(path: &Path) -> std::path::PathBuf {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\")
        .map_or_else(|| path.to_path_buf(), std::path::PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::same_folder;

    #[test]
    fn folders_compare_by_their_canonical_form() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("Data");
        std::fs::create_dir(&folder).unwrap();
        let upper = root.path().join("DATA");
        assert!(same_folder(&upper, &folder));
        assert!(!same_folder(root.path(), &folder));
    }
}
