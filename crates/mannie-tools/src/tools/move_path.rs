//! `move_path`: moves or renames a file, folder or link on the same volume.

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::tools::{Access, AccessMode, ToolFailure, ToolOutcome};
use schemars::JsonSchema;
use serde::Deserialize;

use super::change::{create_access, locked, path_of, pin_existing, pin_missing, unchanged};
use super::copy_path::existing_parent;
use super::path_access;
use crate::paths::{FinalLink, for_change, is_inside};
use crate::texts;
use crate::tool::{Done, PermissionClass, PrepareError, ToolContext, TypedExecution, TypedTool};

/// The arguments of `move_path`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// The file, folder or link to move, absolute or relative to the
    /// workspace. A link is moved, never what it points to.
    source: String,
    /// The new path, which must not exist yet. The folder above it must exist.
    destination: String,
}

/// The tool.
pub(crate) struct MovePath;

impl TypedTool for MovePath {
    type Input = Input;
    const NAME: &'static str = "move_path";
    const CLASS: PermissionClass = PermissionClass::SideEffects;

    fn description() -> &'static str {
        texts::MOVE_PATH
    }

    fn prepare(context: &ToolContext, input: &Input) -> Result<Vec<Access>, PrepareError> {
        let workspace = context.workspace.as_deref();
        let source = for_change(workspace, &input.source, FinalLink::Keep)?;
        if !source.exists() {
            return Err(ToolFailure::NotFound {
                path: source.path.display().to_string(),
            }
            .into());
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
        let delete = path_access(&source, AccessMode::Delete, is_folder)?;
        let create = create_access(&destination.path, is_folder)?;
        if volume(&delete) != volume(&create) {
            return Err(ToolFailure::DifferentVolume.into());
        }
        Ok(vec![delete, create])
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
                let (source_path, source_identity) = path_of(source)?;
                let (destination_path, destination_identity) = path_of(destination)?;
                let (source_folder, source_name) = pin_existing(&source_path, source_identity)?;
                let (destination_folder, names) =
                    pin_missing(&destination_path, destination_identity)?;
                let [destination_name] = names.as_slice() else {
                    return Err(ToolFailure::PathChanged);
                };
                source_folder.move_entry(&source_name, &destination_folder, destination_name)?;
                let source_text = source_path.display().to_string();
                let destination_text = destination_path.display().to_string();
                Ok(Done {
                    output: format!("Moved {source_text} to {destination_text}.\n"),
                    outcome: ToolOutcome::Moved {
                        source: source_text,
                        destination: destination_text,
                    },
                    archives: Vec::new(),
                })
            })
            .await
        }
        .boxed()
    }
}

/// The volume or device part of an access's identity.
fn volume(access: &Access) -> Option<&str> {
    match access {
        Access::Path { identity, .. } => identity.split(':').next(),
        Access::Command { .. } | Access::Project { .. } => None,
    }
}
