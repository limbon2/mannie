//! `list_directory`: the entries of a folder, sorted by name, in pages.

use std::ffi::OsString;
use std::fs;
use std::path::Path;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::tools::{Access, AccessMode, EntryKind, ToolFailure, ToolOutcome};
use schemars::JsonSchema;
use serde::Deserialize;

use super::{
    entry_kind, existing_folder, folder_or_workspace, kind_name, page_bounds, path_access, stopped,
};
use crate::paths::{for_read, io_failure};
use crate::results::{Page, count, format_time};
use crate::stop::StopSignal;
use crate::texts;
use crate::tool::{
    Done, PermissionClass, PrepareError, ToolContext, TypedExecution, TypedTool, blocking,
};

/// The entries of a page when the model names no limit.
const DEFAULT_LIMIT: u32 = 200;
/// The most entries of a page.
const MAX_LIMIT: u32 = 500;

/// The arguments of `list_directory`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// The folder, absolute or relative to the workspace. The workspace when
    /// null.
    #[serde(default)]
    path: Option<String>,
    /// How many entries to skip, to read the next page. 0 when null.
    #[serde(default)]
    offset: Option<u32>,
    /// How many entries to show: 200 when null, at most 500.
    #[serde(default)]
    limit: Option<u32>,
}

/// The tool.
pub(crate) struct ListDirectory;

impl TypedTool for ListDirectory {
    type Input = Input;
    const NAME: &'static str = "list_directory";
    const CLASS: PermissionClass = PermissionClass::ReadOnly;

    fn description() -> &'static str {
        texts::LIST_DIRECTORY
    }

    fn prepare(context: &ToolContext, input: &Input) -> Result<Vec<Access>, PrepareError> {
        let folder = for_read(
            context.workspace.as_deref(),
            folder_or_workspace(input.path.as_deref()),
        )?;
        existing_folder(&folder)?;
        Ok(vec![path_access(&folder, AccessMode::Read, true)?])
    }

    fn execute(execution: TypedExecution<Input>) -> BoxFuture<'static, Result<Done, ToolFailure>> {
        let TypedExecution {
            context,
            input,
            stop,
            ..
        } = execution;
        blocking(move || list(&context, &input, &stop)).boxed()
    }
}

/// One entry of the folder.
struct Entry {
    name: OsString,
    kind: EntryKind,
}

fn list(context: &ToolContext, input: &Input, stop: &StopSignal) -> Result<Done, ToolFailure> {
    let folder = for_read(
        context.workspace.as_deref(),
        folder_or_workspace(input.path.as_deref()),
    )?;
    existing_folder(&folder)?;
    let folder_text = folder.text()?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(&folder.path).map_err(|error| io_failure(&error, &folder.path))? {
        if stop.is_stopped() {
            return Err(stopped());
        }
        let entry = entry.map_err(|error| io_failure(&error, &folder.path))?;
        // Entries that are neither links nor folders get their exact kind
        // from their metadata once they are shown.
        let kind = match entry.file_type() {
            Ok(file_type) if file_type.is_symlink() => EntryKind::Link,
            Ok(file_type) if file_type.is_dir() => EntryKind::Folder,
            _ => EntryKind::Other,
        };
        entries.push(Entry {
            name: entry.file_name(),
            kind,
        });
    }
    // Sorted by name without regard to case, the way people read listings.
    entries.sort_by_cached_key(|entry| {
        let name = entry.name.to_string_lossy();
        (name.to_lowercase(), entry.name.clone())
    });

    let (offset, limit) = page_bounds(input.offset, input.limit, DEFAULT_LIMIT, MAX_LIMIT);
    let mut page = Page::new(context.result_limit, &[&folder_text]);
    let mut shown = 0_usize;
    for entry in entries.iter().skip(offset).take(limit) {
        if !page.push(&describe(&folder.path, entry)) {
            break;
        }
        shown += 1;
    }
    let total = entries.len();
    let next = offset.saturating_add(shown);
    let mut output = format!(
        "Folder: {folder_text}\n{}\n",
        texts::items_shown("Entries", offset + 1, offset + shown, Some(total))
    );
    output.push_str(&page.into_body());
    if next < total && shown > 0 {
        output.push_str(&texts::read_on(next));
        output.push('\n');
    }
    Ok(Done {
        outcome: ToolOutcome::Listed {
            path: folder_text,
            shown: count(shown),
            total: count(total),
        },
        output,
        archives: Vec::new(),
    })
}

/// One line of the listing: the kind and name, and for files the size and the
/// time of change; for links, where they point.
fn describe(folder: &Path, entry: &Entry) -> String {
    let name = entry.name.to_string_lossy();
    let path = folder.join(&entry.name);
    match entry.kind {
        EntryKind::Link => match fs::read_link(&path) {
            Ok(target) => format!("[link] {name} -> {}", target.display()),
            Err(_unreadable) => format!("[link] {name}"),
        },
        kind => {
            let metadata = fs::symlink_metadata(&path).ok();
            let changed = metadata
                .as_ref()
                .and_then(|metadata| metadata.modified().ok())
                .map(format_time);
            let size = metadata
                .as_ref()
                .filter(|metadata| metadata.is_file())
                .map(|metadata| format!("{} bytes", metadata.len()));
            let details: Vec<String> = size
                .into_iter()
                .chain(changed.map(|time| format!("changed {time}")))
                .collect();
            let kind = kind_name(entry_kind_or(kind, metadata.as_ref()));
            if details.is_empty() {
                format!("[{kind}] {name}")
            } else {
                format!("[{kind}] {name} ({})", details.join(", "))
            }
        }
    }
}

/// The kind from the directory entry, or from metadata when the entry did not
/// say.
fn entry_kind_or(kind: EntryKind, metadata: Option<&fs::Metadata>) -> EntryKind {
    match (kind, metadata) {
        (EntryKind::Other, Some(metadata)) => entry_kind(metadata),
        _ => kind,
    }
}
