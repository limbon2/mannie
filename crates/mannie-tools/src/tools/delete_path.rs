//! `delete_path`: deletes a file, a link or a folder.

use std::fs;
use std::path::Path;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::tools::{Access, AccessMode, EntryCount, ToolFailure, ToolOutcome};
use schemars::JsonSchema;
use serde::Deserialize;

use super::change::{locked, path_of, pin_existing, unchanged};
use super::path_access;
use crate::paths::{FinalLink, for_change, io_failure};
use crate::texts;
use crate::tool::{Done, PermissionClass, PrepareError, ToolContext, TypedExecution, TypedTool};

/// The most entries that one delete removes.
const MAX_ENTRIES: u32 = 100_000;

/// The arguments of `delete_path`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// The file, link or folder, absolute or relative to the workspace. A
    /// link is deleted, never what it points to.
    path: String,
    /// Whether to delete a folder that is not empty with everything inside
    /// it. False when null.
    #[serde(default)]
    recursive: Option<bool>,
}

/// The tool.
pub(crate) struct DeletePath;

impl TypedTool for DeletePath {
    type Input = Input;
    const NAME: &'static str = "delete_path";
    const CLASS: PermissionClass = PermissionClass::SideEffects;

    fn description() -> &'static str {
        texts::DELETE_PATH
    }

    /// The access names, for a folder, the files and folders inside it, so a
    /// folder whose content changes needs a new authorization.
    fn prepare(context: &ToolContext, input: &Input) -> Result<Vec<Access>, PrepareError> {
        let target = for_change(context.workspace.as_deref(), &input.path, FinalLink::Keep)?;
        if !target.exists() {
            return Err(ToolFailure::NotFound {
                path: target.path.display().to_string(),
            }
            .into());
        }
        let mut access = path_access(&target, AccessMode::Delete, target.is_folder())?;
        if target.is_folder() {
            let entries = count_entries(&target.path)?;
            let empty = entries.files == 0 && entries.folders == 0;
            if !empty && !input.recursive.unwrap_or(false) {
                return Err(ToolFailure::FolderNotEmpty {
                    path: target.path.display().to_string(),
                }
                .into());
            }
            if let Access::Path {
                entries: counted, ..
            } = &mut access
            {
                *counted = Some(entries);
            }
        }
        Ok(vec![access])
    }

    fn execute(execution: TypedExecution<Input>) -> BoxFuture<'static, Result<Done, ToolFailure>> {
        let TypedExecution {
            context,
            input,
            accesses,
            ..
        } = execution;
        let lock = context.clone();
        async move {
            locked(&lock, move || {
                unchanged(&accesses, Self::prepare(&context, &input))?;
                let target = accesses.first().ok_or(ToolFailure::PathChanged)?;
                delete(target)
            })
            .await
        }
        .boxed()
    }
}

fn delete(target: &Access) -> Result<Done, ToolFailure> {
    let (path, identity) = path_of(target)?;
    let Access::Path {
        folder: is_folder,
        entries,
        ..
    } = target
    else {
        return Err(ToolFailure::PathChanged);
    };
    let (folder, name) = pin_existing(&path, identity)?;
    let (files, folders) = match entries {
        Some(entries) if *is_folder => {
            if entries.files == 0 && entries.folders == 0 {
                folder.remove_empty_folder(&name)?;
            } else {
                folder.remove_tree(&name)?;
            }
            (entries.files, entries.folders + 1)
        }
        _ => {
            folder.remove_entry(&name)?;
            (1, 0)
        }
    };
    let path = path.display().to_string();
    Ok(Done {
        output: format!("Deleted {path}: {files} files and {folders} folders.\n"),
        outcome: ToolOutcome::Deleted {
            path,
            files,
            folders,
        },
        archives: Vec::new(),
    })
}

/// The files and folders inside a folder, without following links, which
/// count as files. More than the limit fails.
fn count_entries(folder: &Path) -> Result<EntryCount, ToolFailure> {
    let mut count = EntryCount {
        files: 0,
        folders: 0,
    };
    let mut pending = vec![folder.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).map_err(|error| io_failure(&error, &current))? {
            let entry = entry.map_err(|error| io_failure(&error, &current))?;
            let file_type = entry
                .file_type()
                .map_err(|error| io_failure(&error, &entry.path()))?;
            if file_type.is_dir() && !file_type.is_symlink() {
                count.folders += 1;
                pending.push(entry.path());
            } else {
                count.files += 1;
            }
            if count.files + count.folders > MAX_ENTRIES {
                return Err(ToolFailure::TooManyEntries { limit: MAX_ENTRIES });
            }
        }
    }
    Ok(count)
}
