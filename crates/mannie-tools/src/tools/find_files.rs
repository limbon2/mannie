//! `find_files`: the files below a folder whose path matches a glob pattern.

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::tools::{Access, AccessMode, ToolFailure, ToolOutcome};
use schemars::JsonSchema;
use serde::Deserialize;

use super::walk::{Pattern, files};
use super::{existing_folder, folder_or_workspace, page_bounds, path_access, stopped};
use crate::paths::for_read;
use crate::results::{Page, count};
use crate::stop::StopSignal;
use crate::texts;
use crate::tool::{
    Done, PermissionClass, PrepareError, ToolContext, TypedExecution, TypedTool, blocking,
};

/// The files of a page when the model names no limit.
const DEFAULT_LIMIT: u32 = 200;
/// The most files of a page.
const MAX_LIMIT: u32 = 500;

/// The arguments of `find_files`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// A glob pattern, such as **/*.rs or src/*.toml. A pattern without a
    /// slash matches file names at any depth.
    pattern: String,
    /// The folder to search, absolute or relative to the workspace. The
    /// workspace when null.
    #[serde(default)]
    path: Option<String>,
    /// Whether to include hidden files and files that .gitignore excludes.
    /// False when null.
    #[serde(default)]
    include_ignored: Option<bool>,
    /// How many matches to skip, to read the next page. 0 when null.
    #[serde(default)]
    offset: Option<u32>,
    /// How many matches to show: 200 when null, at most 500.
    #[serde(default)]
    limit: Option<u32>,
}

/// The tool.
pub(crate) struct FindFiles;

impl TypedTool for FindFiles {
    type Input = Input;
    const NAME: &'static str = "find_files";
    const CLASS: PermissionClass = PermissionClass::ReadOnly;

    fn description() -> &'static str {
        texts::FIND_FILES
    }

    fn prepare(context: &ToolContext, input: &Input) -> Result<Vec<Access>, PrepareError> {
        Pattern::new(&input.pattern)?;
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
        blocking(move || find(&context, &input, &stop)).boxed()
    }
}

fn find(context: &ToolContext, input: &Input, stop: &StopSignal) -> Result<Done, ToolFailure> {
    let pattern = Pattern::new(&input.pattern)?;
    let folder = for_read(
        context.workspace.as_deref(),
        folder_or_workspace(input.path.as_deref()),
    )?;
    existing_folder(&folder)?;
    let folder_text = folder.text()?;
    let (offset, limit) = page_bounds(input.offset, input.limit, DEFAULT_LIMIT, MAX_LIMIT);
    let mut page = Page::new(context.result_limit, &[&folder_text, &input.pattern]);
    let mut matched = 0_usize;
    let mut shown = 0_usize;
    let mut more = false;
    let candidates = files(&folder.path, input.include_ignored.unwrap_or(false))
        .filter(|relative| pattern.matches(relative));
    for relative in candidates {
        if stop.is_stopped() {
            return Err(stopped());
        }
        matched += 1;
        if matched <= offset {
            continue;
        }
        if shown == limit || !page.push(&relative.display().to_string()) {
            more = true;
            break;
        }
        shown += 1;
    }
    let mut output = format!("Folder: {folder_text}\nPattern: {}\n", input.pattern);
    if matched == 0 {
        output.push_str("No files match.\n");
    } else {
        output.push_str(&texts::items_shown(
            "Matches",
            offset + 1,
            offset + shown,
            None,
        ));
        output.push('\n');
        output.push_str(&page.into_body());
        if more {
            output.push_str(&texts::read_on(offset + shown));
            output.push('\n');
        }
    }
    Ok(Done {
        outcome: ToolOutcome::Found {
            path: folder_text,
            shown: count(shown),
            more,
        },
        output,
        archives: Vec::new(),
    })
}
