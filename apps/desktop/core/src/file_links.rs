//! Local navigation from replies. Only an owner's action opens a location
//! (ADR-0069); neither resolution nor model output grants tool permissions.

use std::cell::Cell;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use mannie_store::Store;
use mannie_types::file_links::{
    FileLinkApplication, FileLinkError, FileLinkKind, FileLinkRef, FileLinkTarget, ResolvedFileLink,
};
use mannie_types::links::MAX_URL_BYTES;

use crate::state::Core;

/// Resolve using the containing conversation's persisted workspace, including
/// a child conversation's inherited workspace. Never use the window's selection.
pub(crate) async fn resolve(
    store: Option<&Store>,
    input: FileLinkRef,
) -> Result<ResolvedFileLink, FileLinkError> {
    check_text(&input.path)?;
    let mut conversation = input.conversation_id;
    let mut visited = HashSet::new();
    let workspace = loop {
        let Some(id) = conversation else {
            break None;
        };
        if !visited.insert(id.clone()) {
            return Err(FileLinkError::Unavailable);
        }
        let summary = store
            .ok_or(FileLinkError::Unavailable)?
            .conversation_summary(&id)
            .await
            .map_err(|_error| FileLinkError::Unavailable)?
            .ok_or(FileLinkError::ConversationMissing)?;
        if let Some(task) = summary.task {
            if task.worker.is_some() || task.executor.is_some() {
                return Err(FileLinkError::NotLocal);
            }
            // Child rows have no project of their own. Their tools inherit
            // the parent's project, so references use the same stored chain.
            conversation = Some(task.parent_id);
        } else {
            break summary.workspace;
        }
    };
    tauri::async_runtime::spawn_blocking(move || {
        resolve_path(&input.path, workspace.as_deref().map(Path::new))
    })
    .await
    .map_err(|_error| FileLinkError::Unavailable)?
}

fn check_text(value: &str) -> Result<(), FileLinkError> {
    if value.is_empty() || value.len() > MAX_URL_BYTES || value.chars().any(char::is_control) {
        return Err(FileLinkError::InvalidPath);
    }
    Ok(())
}

fn windows_drive(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// Refuse direct remote paths and device names before filesystem access.
fn check_local(value: &str) -> Result<(), FileLinkError> {
    check_text(value)?;
    if value.starts_with("\\\\") || value.starts_with("//") {
        return Err(FileLinkError::NotLocal);
    }
    if value.starts_with('\\') || value.starts_with('~') {
        return Err(FileLinkError::InvalidPath);
    }
    if windows_drive(value) {
        if !cfg!(windows) {
            return Err(FileLinkError::NotLocal);
        }
        if !matches!(value.as_bytes().get(2), Some(b'/' | b'\\')) {
            return Err(FileLinkError::InvalidPath);
        }
    }
    #[cfg(windows)]
    {
        // Root-relative paths depend on the process's current drive.
        if value.starts_with('/') {
            return Err(FileLinkError::InvalidPath);
        }
        for component in value.split(['/', '\\']) {
            let name = component
                .split(['.', ':'])
                .next()
                .unwrap_or_default()
                .trim_end();
            let upper = name.to_ascii_uppercase();
            let port_number = upper
                .strip_prefix("COM")
                .or_else(|| upper.strip_prefix("LPT"))
                .unwrap_or_default();
            let mut digits = port_number.chars();
            // Windows also treats superscript digits one to three as port numbers.
            let numbered_device = digits.next().is_some_and(|digit| {
                digit.is_ascii_digit() || matches!(digit, '\u{b9}' | '\u{b2}' | '\u{b3}')
            }) && digits.next().is_none();
            if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
                || numbered_device
                || upper.starts_with("CONIN$")
                || upper.starts_with("CONOUT$")
            {
                return Err(FileLinkError::InvalidPath);
            }
        }
    }
    Ok(())
}

/// Remove a positive source location only after trying the literal filename.
fn without_location(value: &str) -> Option<&str> {
    fn digits(value: &str) -> bool {
        !value.is_empty()
            && value.bytes().all(|byte| byte.is_ascii_digit())
            && value.bytes().any(|byte| byte != b'0')
    }
    if let Some((path, line)) = value.rsplit_once("#L")
        && !path.is_empty()
        && digits(line)
    {
        return Some(path);
    }
    let (path, last) = value.rsplit_once(':')?;
    if path.len() <= 1 || !digits(last) {
        return None;
    }
    if let Some((file, line)) = path.rsplit_once(':')
        && file.len() > 1
        && digits(line)
    {
        return Some(file);
    }
    Some(path)
}

fn native_path(value: &str) -> Result<PathBuf, FileLinkError> {
    check_text(value)?;
    if value
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file:"))
    {
        let authority_and_path = value[5..]
            .strip_prefix("//")
            .ok_or(FileLinkError::InvalidPath)?;
        if authority_and_path
            .as_bytes()
            .get(..2)
            .is_some_and(|prefix| prefix.iter().all(|byte| matches!(byte, b'/' | b'\\')))
        {
            return Err(FileLinkError::NotLocal);
        }
        // The URL parser can remove a remote host when a drive letter follows it.
        let discarded_host = Cell::new(false);
        let note_violation = |violation| {
            if violation == url::SyntaxViolation::FileWithHostAndWindowsDrive {
                discarded_host.set(true);
            }
        };
        let url = url::Url::options()
            .syntax_violation_callback(Some(&note_violation))
            .parse(value)
            .map_err(|_error| FileLinkError::InvalidPath)?;
        if discarded_host.get() || url.host_str().is_some_and(|host| host != "localhost") {
            return Err(FileLinkError::NotLocal);
        }
        if url.query().is_some() || url.cannot_be_a_base() {
            return Err(FileLinkError::InvalidPath);
        }
        // A drive-only URL without a trailing slash creates a drive-relative
        // path and triggers an assertion in to_file_path on Windows.
        #[cfg(windows)]
        if !url.path().trim_start_matches('/').contains('/') {
            return Err(FileLinkError::InvalidPath);
        }
        let mut path = url
            .to_file_path()
            .map_err(|()| FileLinkError::InvalidPath)?;
        if let Some(fragment) = url.fragment() {
            if !fragment.starts_with('L') || !fragment[1..].bytes().all(|b| b.is_ascii_digit()) {
                return Err(FileLinkError::InvalidPath);
            }
            // Try the literal filename, including the fragment, first.
            let mut name = path.into_os_string();
            name.push(format!("#{fragment}"));
            path = PathBuf::from(name);
        }
        check_local(path.to_str().ok_or(FileLinkError::InvalidPath)?)?;
        return Ok(path);
    }
    check_local(value)?;
    // Colons in the first component indicate a URI scheme, except for a
    // native drive or a source location. Never hand schemes to the OS.
    let unlocated = without_location(value).unwrap_or(value);
    let first = value.split(['/', '\\']).next().unwrap_or_default();
    if !windows_drive(value) && first.contains(':') && !unlocated.contains('.') {
        return Err(FileLinkError::InvalidPath);
    }
    if !windows_drive(unlocated)
        && unlocated
            .split(['/', '\\'])
            .next()
            .is_some_and(|part| part.contains(':'))
    {
        return Err(FileLinkError::InvalidPath);
    }
    Ok(PathBuf::from(value))
}

fn io_error(error: &io::Error) -> FileLinkError {
    match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::InvalidInput => FileLinkError::NotFound,
        _ => FileLinkError::Unavailable,
    }
}

fn inspect(path: &Path) -> Result<ResolvedFileLink, FileLinkError> {
    check_local(path.to_str().ok_or(FileLinkError::InvalidPath)?)?;
    // Reject alternate data streams on Windows, including source suffixes;
    // the caller retries their location-free path after this missing result.
    #[cfg(windows)]
    if path
        .to_str()
        .is_some_and(|path| path.get(2..).is_some_and(|rest| rest.contains(':')))
    {
        return Err(FileLinkError::NotFound);
    }
    let canonical = std::fs::canonicalize(path).map_err(|error| io_error(&error))?;
    let text = canonical.to_str().ok_or(FileLinkError::InvalidPath)?;
    // Windows canonicalization adds a trusted extended-length prefix.
    #[cfg(windows)]
    let text = text.strip_prefix("\\\\?\\").unwrap_or(text);
    #[cfg(windows)]
    if text.starts_with("UNC\\") {
        return Err(FileLinkError::NotLocal);
    }
    check_local(text)?;
    let metadata = std::fs::metadata(&canonical).map_err(|error| io_error(&error))?;
    let kind = if metadata.is_dir() {
        FileLinkKind::Directory
    } else if metadata.is_file() {
        FileLinkKind::File
    } else {
        return Err(FileLinkError::UnsupportedKind);
    };
    Ok(ResolvedFileLink {
        path: text.to_owned(),
        kind,
        target: FileLinkTarget::FileManager,
        application: if cfg!(windows) {
            FileLinkApplication::FileExplorer
        } else {
            FileLinkApplication::FileManager
        },
    })
}

pub(crate) fn resolve_path(
    value: &str,
    workspace: Option<&Path>,
) -> Result<ResolvedFileLink, FileLinkError> {
    let path = native_path(value)?;
    let path = if path.is_absolute() {
        path
    } else {
        let workspace = workspace.ok_or(FileLinkError::NoProject)?;
        check_local(workspace.to_str().ok_or(FileLinkError::InvalidPath)?)?;
        if !workspace.is_absolute() {
            return Err(FileLinkError::NoProject);
        }
        workspace.join(path)
    };
    match inspect(&path) {
        Err(FileLinkError::NotFound) => {
            let text = path.to_str().ok_or(FileLinkError::InvalidPath)?;
            let Some(file) = without_location(text) else {
                return Err(FileLinkError::NotFound);
            };
            inspect(Path::new(file))
        }
        other => other,
    }
}

/// Opening checks the original reference again, on a blocking worker. A
/// validation build records the same checked action without launching it.
pub(crate) async fn open(core: &Core, input: FileLinkRef) -> Result<(), FileLinkError> {
    let resolved = resolve(core.store(), input).await?;
    #[cfg(feature = "validation")]
    core.record_opened_file_link(resolved);
    #[cfg(not(feature = "validation"))]
    tauri::async_runtime::spawn_blocking(move || launch(&resolved))
        .await
        .map_err(|_error| FileLinkError::OpenFailed)??;
    tracing::info!("opened a local location in the file manager");
    Ok(())
}

#[cfg(not(feature = "validation"))]
fn launch(link: &ResolvedFileLink) -> Result<(), FileLinkError> {
    use tauri_plugin_opener::{open_path, reveal_item_in_dir};

    // Recheck immediately before crossing into the OS. Only directories are
    // passed to open_path; a file is never opened with its associated program.
    let checked = inspect(Path::new(&link.path))?;
    if checked.kind == FileLinkKind::Directory {
        return open_path(&checked.path, None::<&str>).map_err(|_error| FileLinkError::OpenFailed);
    }
    match reveal_item_in_dir(&checked.path) {
        Ok(()) => Ok(()),
        Err(_) => {
            #[cfg(target_os = "linux")]
            {
                let parent = Path::new(&checked.path)
                    .parent()
                    .ok_or(FileLinkError::OpenFailed)?;
                let directory = inspect(parent)?;
                if directory.kind != FileLinkKind::Directory {
                    return Err(FileLinkError::OpenFailed);
                }
                open_path(&directory.path, None::<&str>).map_err(|_error| FileLinkError::OpenFailed)
            }
            #[cfg(not(target_os = "linux"))]
            Err(FileLinkError::OpenFailed)
        }
    }
}

#[cfg(test)]
mod tests;
