//! `create_directory`: a folder and the missing folders above it.

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::tools::{Access, ToolFailure, ToolOutcome};
use schemars::JsonSchema;
use serde::Deserialize;

use super::change::{
    create_access, create_folders, locked, missing_folders, path_of, pin_missing, unchanged,
};
use crate::paths::{FinalLink, for_change};
use crate::texts;
use crate::tool::{Done, PermissionClass, PrepareError, ToolContext, TypedExecution, TypedTool};

/// The arguments of `create_directory`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// The folder, absolute or relative to the workspace.
    path: String,
}

/// The tool.
pub(crate) struct CreateDirectory;

impl TypedTool for CreateDirectory {
    type Input = Input;
    const NAME: &'static str = "create_directory";
    const CLASS: PermissionClass = PermissionClass::SideEffects;

    fn description() -> &'static str {
        texts::CREATE_DIRECTORY
    }

    /// One access for each folder to create. A folder that exists already
    /// needs none.
    fn prepare(context: &ToolContext, input: &Input) -> Result<Vec<Access>, PrepareError> {
        let target = for_change(context.workspace.as_deref(), &input.path, FinalLink::Follow)?;
        if target.is_folder() {
            return Ok(Vec::new());
        }
        if target.exists() {
            return Err(ToolFailure::AlreadyExists {
                path: target.path.display().to_string(),
            }
            .into());
        }
        let mut accesses = missing_folders(&target.path)?
            .iter()
            .map(|folder| create_access(folder, true))
            .collect::<Result<Vec<_>, _>>()?;
        accesses.push(create_access(&target.path, true)?);
        Ok(accesses)
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
                let Some(target) = accesses.last() else {
                    return Ok(exists(&context, &input));
                };
                let (path, identity) = path_of(target)?;
                let (folder, names) = pin_missing(&path, identity)?;
                let (folder, created) = create_folders(folder, &names)?;
                let name = names.last().ok_or(ToolFailure::PathChanged)?;
                folder.create_folder(name)?;
                let path = path.display().to_string();
                Ok(Done {
                    output: format!("Created {path} ({} folders).\n", created + 1),
                    outcome: ToolOutcome::FolderCreated {
                        path,
                        created: created + 1,
                    },
                    archives: Vec::new(),
                })
            })
            .await
        }
        .boxed()
    }
}

/// The result for a folder that existed already.
fn exists(context: &ToolContext, input: &Input) -> Done {
    let path = for_change(context.workspace.as_deref(), &input.path, FinalLink::Follow)
        .map_or_else(
            |_| input.path.clone(),
            |target| target.path.display().to_string(),
        );
    Done {
        output: format!("{path} exists already; nothing was created.\n"),
        outcome: ToolOutcome::FolderCreated { path, created: 0 },
        archives: Vec::new(),
    }
}
