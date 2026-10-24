//! `move_to_project`: moves a general conversation into one of the owner's
//! projects, whose folder becomes its workspace (docs/features/chat/
//! projects.md, moving a conversation into a project). The policy decides on
//! the move like on every operation: without a question only when the
//! owner's own request names the project (ADR-0023, decision 3).

use std::path::Path;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::projects::ProjectView;
use mannie_types::tools::{Access, ToolFailure, ToolOutcome};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::paths::is_same_path;
use crate::projects::{Availability, MoveFailure};
use crate::texts;
use crate::tool::{Done, PermissionClass, PrepareError, ToolContext, TypedExecution, TypedTool};

/// The most characters of the call's project that a failure repeats.
const MAX_ECHO_CHARS: usize = 200;

/// The arguments of `move_to_project`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// The project's folder as the environment lists it, or its name when no
    /// other project has the same name.
    project: String,
}

/// The tool.
pub(crate) struct MoveToProject;

impl TypedTool for MoveToProject {
    type Input = Input;
    const NAME: &'static str = "move_to_project";
    const CLASS: PermissionClass = PermissionClass::SideEffects;

    fn description() -> &'static str {
        texts::MOVE_TO_PROJECT
    }

    /// Only a general conversation moves, and only while the owner has
    /// projects.
    fn offered(availability: &Availability<'_>) -> bool {
        availability.project.is_none() && !availability.projects.is_empty()
    }

    fn prepare(context: &ToolContext, input: &Input) -> Result<Vec<Access>, PrepareError> {
        if let Some(current) = &context.projects.current {
            return Err(ToolFailure::AlreadyInProject {
                name: current.name.clone(),
            }
            .into());
        }
        let named = input.project.trim();
        let project =
            find(&context.projects.all, named).ok_or_else(|| ToolFailure::ProjectNotFound {
                project: named.chars().take(MAX_ECHO_CHARS).collect(),
            })?;
        Ok(vec![Access::Project {
            id: project.id.clone(),
            name: project.name.clone(),
            folder: project.folder.clone(),
        }])
    }

    fn execute(execution: TypedExecution<Input>) -> BoxFuture<'static, Result<Done, ToolFailure>> {
        let TypedExecution {
            context, accesses, ..
        } = execution;
        async move {
            // The authorized access names the project, so the conversation
            // moves where the policy or the owner allowed it to.
            let Some(Access::Project { id, name, .. }) = accesses.into_iter().next() else {
                return Err(ToolFailure::ProjectNotFound {
                    project: String::new(),
                });
            };
            let moved = context
                .projects
                .moves
                .move_into(context.conversation_id.clone(), id)
                .await
                .map_err(|failure| match failure {
                    MoveFailure::ProjectNotFound => ToolFailure::ProjectNotFound {
                        project: name.chars().take(MAX_ECHO_CHARS).collect(),
                    },
                    MoveFailure::AlreadyInProject { name } => {
                        ToolFailure::AlreadyInProject { name }
                    }
                    MoveFailure::Storage => ToolFailure::IoError {
                        kind: "the move could not be saved".to_owned(),
                    },
                })?;
            Ok(Done {
                output: texts::moved_to_project(&moved.name, &moved.folder),
                outcome: ToolOutcome::MovedToProject {
                    name: moved.name,
                    folder: moved.folder,
                },
                archives: Vec::new(),
                images: Vec::new(),
            })
        }
        .boxed()
    }
}

/// The project that a call names: the one with the folder, or else the only
/// one with the name, without regard to case.
fn find<'a>(projects: &'a [ProjectView], named: &str) -> Option<&'a ProjectView> {
    let by_folder = projects
        .iter()
        .find(|project| is_same_path(Path::new(&project.folder), Path::new(named)));
    if by_folder.is_some() {
        return by_folder;
    }
    let lowered = named.to_lowercase();
    let mut by_name = projects
        .iter()
        .filter(|project| project.name.to_lowercase() == lowered);
    let first = by_name.next()?;
    by_name.next().is_none().then_some(first)
}

#[cfg(test)]
mod tests {
    use mannie_types::clock::UtcMillis;
    use mannie_types::ids::ProjectId;
    use mannie_types::projects::ProjectView;

    use super::find;

    fn project(number: u32, name: &str, folder: &str) -> ProjectView {
        ProjectView {
            id: ProjectId::parse(&format!("0199e0f2-0000-7000-a000-{number:012}")).unwrap(),
            name: name.to_owned(),
            folder: folder.to_owned(),
            created_at: UtcMillis::EPOCH,
        }
    }

    #[test]
    fn a_call_names_a_project_by_its_folder_or_its_unique_name() {
        let (first, second) = if cfg!(windows) {
            (r"C:\work\app", r"C:\other\app")
        } else {
            ("/work/app", "/other/app")
        };
        let projects = [
            project(1, "app", first),
            project(2, "app", second),
            project(3, "Mannie", "/work/mannie"),
        ];

        assert_eq!(
            find(&projects, second).map(|found| &found.id),
            Some(&projects[1].id)
        );
        assert_eq!(
            find(&projects, "mannie").map(|found| &found.id),
            Some(&projects[2].id)
        );
        assert_eq!(find(&projects, "app"), None);
        assert_eq!(find(&projects, "elsewhere"), None);
    }
}
