//! The system tools (docs/features/system-tools/tools-and-results.md, the
//! tools), and the helpers they share.

mod change;
pub(crate) mod copy_path;
pub(crate) mod create_directory;
pub(crate) mod delete_path;
pub(crate) mod edit_file;
pub(crate) mod file_info;
pub(crate) mod find_files;
pub(crate) mod list_directory;
pub(crate) mod move_path;
pub(crate) mod move_to_project;
pub(crate) mod read_file;
pub(crate) mod read_tool_output;
#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
pub(crate) mod run_command;
pub(crate) mod search_text;
pub(crate) mod system_info;
mod walk;
pub(crate) mod write_file;

use std::fs::Metadata;
use std::sync::Arc;

use mannie_types::tools::{Access, AccessMode, EntryKind, ToolFailure};

use crate::identity::identity;
use crate::paths::{Resolved, is_link};
use crate::tool::{Registered, Tool};

/// The system tools of this platform, in the order the model sees them.
/// `run_command` exists only where the process supervisor contains commands,
/// and `move_to_project` is offered only where it can move a conversation.
pub(crate) fn system_tools() -> Vec<Arc<dyn Tool>> {
    let mut tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(Registered::<system_info::SystemInfo>::new()),
        Arc::new(Registered::<list_directory::ListDirectory>::new()),
        Arc::new(Registered::<file_info::FileInfo>::new()),
        Arc::new(Registered::<find_files::FindFiles>::new()),
        Arc::new(Registered::<search_text::SearchText>::new()),
        Arc::new(Registered::<read_file::ReadFile>::new()),
        Arc::new(Registered::<write_file::WriteFile>::new()),
        Arc::new(Registered::<edit_file::EditFile>::new()),
        Arc::new(Registered::<create_directory::CreateDirectory>::new()),
        Arc::new(Registered::<copy_path::CopyPath>::new()),
        Arc::new(Registered::<move_path::MovePath>::new()),
        Arc::new(Registered::<delete_path::DeletePath>::new()),
    ];
    #[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
    tools.push(Arc::new(Registered::<run_command::RunCommand>::new()));
    tools.push(Arc::new(
        Registered::<read_tool_output::ReadToolOutput>::new(),
    ));
    tools.push(Arc::new(Registered::<move_to_project::MoveToProject>::new()));
    tools
}

/// The access of a resolved path.
pub(crate) fn path_access(
    resolved: &Resolved,
    mode: AccessMode,
    folder: bool,
) -> Result<Access, ToolFailure> {
    Ok(Access::Path {
        path: resolved.text()?,
        mode,
        folder,
        exists: resolved.exists(),
        identity: identity(&resolved.path)?,
        entries: None,
    })
}

/// The folder argument of a tool: the path the model named, or the
/// workspace.
pub(crate) fn folder_or_workspace(path: Option<&str>) -> &str {
    path.unwrap_or(".")
}

/// A resolved path that must be an existing folder.
pub(crate) fn existing_folder(resolved: &Resolved) -> Result<(), ToolFailure> {
    match &resolved.metadata {
        None => Err(ToolFailure::NotFound {
            path: resolved.path.display().to_string(),
        }),
        Some(metadata) if !metadata.is_dir() => Err(ToolFailure::NotAFolder {
            path: resolved.path.display().to_string(),
        }),
        Some(_) => Ok(()),
    }
}

/// A resolved path that must be an existing file.
pub(crate) fn existing_file(resolved: &Resolved) -> Result<&Metadata, ToolFailure> {
    match &resolved.metadata {
        None => Err(ToolFailure::NotFound {
            path: resolved.path.display().to_string(),
        }),
        Some(metadata) if !metadata.is_file() => Err(ToolFailure::NotAFile {
            path: resolved.path.display().to_string(),
        }),
        Some(metadata) => Ok(metadata),
    }
}

/// The kind of an entry, from metadata that does not follow links.
pub(crate) fn entry_kind(metadata: &Metadata) -> EntryKind {
    if is_link(metadata) {
        EntryKind::Link
    } else if metadata.is_dir() {
        EntryKind::Folder
    } else if metadata.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

/// The name of an entry kind in a rendering.
pub(crate) fn kind_name(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::File => "file",
        EntryKind::Folder => "folder",
        EntryKind::Link => "link",
        EntryKind::Other => "other",
    }
}

/// The page arguments of a tool: the items to skip, and how many to show
/// within the default and the maximum.
pub(crate) fn page_bounds(
    offset: Option<u32>,
    limit: Option<u32>,
    default: u32,
    maximum: u32,
) -> (usize, usize) {
    let offset = usize::try_from(offset.unwrap_or(0)).unwrap_or(usize::MAX);
    let limit = limit.unwrap_or(default).clamp(1, maximum);
    (offset, usize::try_from(limit).unwrap_or(usize::MAX))
}

/// The failure of work that the owner stopped. The runtime records the call
/// as canceled, so the model never reads it.
pub(crate) fn stopped() -> ToolFailure {
    ToolFailure::IoError {
        kind: "the work was stopped".to_owned(),
    }
}
