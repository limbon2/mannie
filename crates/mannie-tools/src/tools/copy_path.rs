//! `copy_path`: copies a file, or a folder with its contents, to a path that
//! does not exist yet.

use std::ffi::OsString;
use std::fs;
use std::path::Path;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::tools::{Access, AccessMode, ToolFailure, ToolOutcome};
use schemars::JsonSchema;
use serde::Deserialize;

use super::change::{create_access, locked, path_of, pin_existing, pin_missing, unchanged};
use super::path_access;
use crate::mutate::{EntryType, PinnedFolder};
use crate::paths::{FinalLink, for_change, for_info, io_failure, is_inside};
use crate::texts;
use crate::tool::{Done, PermissionClass, PrepareError, ToolContext, TypedExecution, TypedTool};

/// The most entries that one copy copies.
const MAX_ENTRIES: usize = 10_000;

/// The arguments of `copy_path`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// The file or folder to copy, absolute or relative to the workspace.
    source: String,
    /// The new path, which must not exist yet. The folder above it must exist.
    destination: String,
}

/// The tool.
pub(crate) struct CopyPath;

impl TypedTool for CopyPath {
    type Input = Input;
    const NAME: &'static str = "copy_path";
    const CLASS: PermissionClass = PermissionClass::SideEffects;

    fn description() -> &'static str {
        texts::COPY_PATH
    }

    fn prepare(context: &ToolContext, input: &Input) -> Result<Vec<Access>, PrepareError> {
        let workspace = context.workspace.as_deref();
        let source = for_info(workspace, &input.source)?;
        if !source.exists() {
            return Err(ToolFailure::NotFound {
                path: source.path.display().to_string(),
            }
            .into());
        }
        if source.is_link() {
            return Err(ToolFailure::LinkInSource {
                path: source.path.display().to_string(),
            }
            .into());
        }
        if source.is_folder() {
            check_tree(&source.path)?;
        }
        let destination = for_change(workspace, &input.destination, FinalLink::Follow)?;
        if destination.exists() {
            return Err(ToolFailure::AlreadyExists {
                path: destination.path.display().to_string(),
            }
            .into());
        }
        existing_parent(&destination.path)?;
        if is_inside(&destination.path, &source.path) {
            return Err(PrepareError::invalid(
                "the destination is inside the source",
            ));
        }
        let is_folder = source.is_folder();
        Ok(vec![
            path_access(&source, AccessMode::Read, is_folder)?,
            create_access(&destination.path, is_folder)?,
        ])
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
                let [source, destination] = accesses.as_slice() else {
                    return Err(ToolFailure::PathChanged);
                };
                copy(source, destination)
            })
            .await
        }
        .boxed()
    }
}

fn copy(source: &Access, destination: &Access) -> Result<Done, ToolFailure> {
    let (source_path, source_identity) = path_of(source)?;
    let (destination_path, destination_identity) = path_of(destination)?;
    let (source_folder, source_name) = pin_existing(&source_path, source_identity)?;
    let (destination_folder, names) = pin_missing(&destination_path, destination_identity)?;
    let [destination_name] = names.as_slice() else {
        return Err(ToolFailure::PathChanged);
    };
    let mut files = 0_u32;
    copy_entry(
        &source_folder,
        &source_name,
        &destination_folder,
        destination_name,
        &mut files,
    )?;
    let source_text = source_path.display().to_string();
    let destination_text = destination_path.display().to_string();
    Ok(Done {
        output: format!("Copied {source_text} to {destination_text}: {files} files.\n"),
        outcome: ToolOutcome::Copied {
            source: source_text,
            destination: destination_text,
            files,
        },
        archives: Vec::new(),
    })
}

/// Copies one entry, and a folder's entries below it. A link anywhere ends
/// the copy: the preparation found none, so one appeared since.
fn copy_entry(
    source: &PinnedFolder,
    name: &OsString,
    destination: &PinnedFolder,
    new_name: &OsString,
    files: &mut u32,
) -> Result<(), ToolFailure> {
    let Some((_, entry_type)) = source.entry(name)? else {
        return Err(ToolFailure::PathChanged);
    };
    match entry_type {
        EntryType::Link => Err(ToolFailure::LinkInSource {
            path: source.child_path(name).display().to_string(),
        }),
        EntryType::Other => Err(ToolFailure::NotAFile {
            path: source.child_path(name).display().to_string(),
        }),
        EntryType::File => {
            let mut reader = source.open_file(name)?;
            let mut draft = destination.draft(new_name)?;
            draft.copy_from(&mut reader)?;
            destination.commit_new(draft, new_name)?;
            *files += 1;
            Ok(())
        }
        EntryType::Folder => {
            destination.create_folder(new_name)?;
            let source_child = source.child(name)?;
            let destination_child = destination.child(new_name)?;
            let mut names: Vec<OsString> = fs::read_dir(source_child.path())
                .map_err(|error| io_failure(&error, source_child.path()))?
                .map(|entry| entry.map(|entry| entry.file_name()))
                .collect::<Result<_, _>>()
                .map_err(|error| io_failure(&error, source_child.path()))?;
            names.sort();
            for child in &names {
                copy_entry(&source_child, child, &destination_child, child, files)?;
            }
            Ok(())
        }
    }
}

/// Checks a folder to copy: no links inside, and not too many entries.
fn check_tree(folder: &Path) -> Result<(), ToolFailure> {
    let mut entries = 0_usize;
    let mut pending = vec![folder.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).map_err(|error| io_failure(&error, &current))? {
            let entry = entry.map_err(|error| io_failure(&error, &current))?;
            let file_type = entry
                .file_type()
                .map_err(|error| io_failure(&error, &entry.path()))?;
            if file_type.is_symlink() {
                return Err(ToolFailure::LinkInSource {
                    path: entry.path().display().to_string(),
                });
            }
            if file_type.is_dir() {
                pending.push(entry.path());
            }
            entries += 1;
            if entries > MAX_ENTRIES {
                return Err(ToolFailure::TooManyEntries {
                    limit: u32::try_from(MAX_ENTRIES).unwrap_or(u32::MAX),
                });
            }
        }
    }
    Ok(())
}

/// The folder above a path that a copy or move creates must exist.
pub(crate) fn existing_parent(path: &Path) -> Result<(), ToolFailure> {
    let Some(parent) = path.parent() else {
        return Err(ToolFailure::NotAFolder {
            path: path.display().to_string(),
        });
    };
    match fs::symlink_metadata(parent) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(ToolFailure::NotAFolder {
            path: parent.display().to_string(),
        }),
        Err(error) => Err(io_failure(&error, parent)),
    }
}
