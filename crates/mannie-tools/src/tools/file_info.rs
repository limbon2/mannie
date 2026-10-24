//! `file_info`: what a path is, without following a final link.

use std::fs;
use std::io;
use std::time::SystemTime;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::tools::{Access, AccessMode, EntryKind, ToolFailure, ToolOutcome};
use schemars::JsonSchema;
use serde::Deserialize;

use super::{entry_kind, kind_name, path_access};
use crate::paths::{Resolved, for_info, io_failure};
use crate::results::format_time;
use crate::texts;
use crate::tool::{
    Done, PermissionClass, PrepareError, ToolContext, TypedExecution, TypedTool, blocking,
};

/// The arguments of `file_info`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// The file, folder or link, absolute or relative to the workspace.
    path: String,
}

/// The tool.
pub(crate) struct FileInfo;

impl TypedTool for FileInfo {
    type Input = Input;
    const NAME: &'static str = "file_info";
    const CLASS: PermissionClass = PermissionClass::ReadOnly;

    fn description() -> &'static str {
        texts::FILE_INFO
    }

    fn prepare(context: &ToolContext, input: &Input) -> Result<Vec<Access>, PrepareError> {
        let resolved = existing(context, input)?;
        Ok(vec![path_access(
            &resolved,
            AccessMode::Read,
            resolved.is_folder(),
        )?])
    }

    fn execute(execution: TypedExecution<Input>) -> BoxFuture<'static, Result<Done, ToolFailure>> {
        let TypedExecution { context, input, .. } = execution;
        blocking(move || describe(&context, &input)).boxed()
    }
}

/// The path, which must exist.
fn existing(context: &ToolContext, input: &Input) -> Result<Resolved, ToolFailure> {
    let resolved = for_info(context.workspace.as_deref(), &input.path)?;
    if !resolved.exists() {
        return Err(ToolFailure::NotFound {
            path: resolved.path.display().to_string(),
        });
    }
    Ok(resolved)
}

fn describe(context: &ToolContext, input: &Input) -> Result<Done, ToolFailure> {
    let resolved = existing(context, input)?;
    let path = resolved.text()?;
    let metadata =
        fs::symlink_metadata(&resolved.path).map_err(|error| io_failure(&error, &resolved.path))?;
    let kind = entry_kind(&metadata);
    let mut lines = vec![
        format!("Path: {path}"),
        format!("Kind: {}", kind_name(kind)),
    ];
    match kind {
        EntryKind::Link => {
            let target = fs::read_link(&resolved.path)
                .map_err(|error| io_failure(&error, &resolved.path))?;
            lines.push(format!("Points to: {} (not followed)", target.display()));
        }
        EntryKind::File => lines.push(format!("Size: {} bytes", metadata.len())),
        EntryKind::Folder | EntryKind::Other => {}
    }
    lines.push(format!("Changed: {}", time(metadata.modified())));
    lines.push(format!("Created: {}", time(metadata.created())));
    lines.push(format!("Accessed: {}", time(metadata.accessed())));
    let read_only = if metadata.permissions().readonly() {
        "yes"
    } else {
        "no"
    };
    lines.push(format!("Read-only: {read_only}"));
    let mut output = lines.join("\n");
    output.push('\n');
    Ok(Done {
        outcome: ToolOutcome::Described { path, entry: kind },
        output,
        archives: Vec::new(),
        images: Vec::new(),
    })
}

/// A time of the file system, or `unknown` where the system does not record
/// it.
fn time(time: io::Result<SystemTime>) -> String {
    time.map_or_else(|_unsupported| "unknown".to_owned(), format_time)
}
