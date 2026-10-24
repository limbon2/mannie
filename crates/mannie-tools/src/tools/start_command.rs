//! `start_command`: starts a command in the platform's shell in the
//! background (ADR-0041, docs/features/system-tools/background-commands.md).
//! It prepares as `run_command` does, with the access marked as running in
//! the background, and hands the command to the machine's registry, which
//! keeps it after the call until it ends or is stopped.

use std::path::PathBuf;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::tools::{Access, MAX_VIEW_COMMAND_CHARS, ToolFailure, ToolOutcome};
use schemars::JsonSchema;
use serde::Deserialize;

use super::change::unchanged;
use super::run_command::{MAX_COMMAND_BYTES, shell_path};
use super::{existing_folder, folder_or_workspace};
use crate::background::StartRequest;
use crate::paths::{for_read, path_text};
use crate::supervisor::BackgroundSpec;
use crate::texts;
use crate::tool::{Done, PermissionClass, PrepareError, ToolContext, TypedExecution, TypedTool};

/// The arguments of `start_command`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// The command, in the shell that `system_info` names, as it runs in the
    /// foreground. It runs without a terminal and without input.
    command: String,
    /// The folder the command runs in, absolute or relative to the workspace.
    /// The workspace when null.
    #[serde(default)]
    working_directory: Option<String>,
}

/// The tool.
pub(crate) struct StartCommand;

impl TypedTool for StartCommand {
    type Input = Input;
    const NAME: &'static str = "start_command";
    const CLASS: PermissionClass = PermissionClass::SideEffects;

    fn description() -> &'static str {
        texts::START_COMMAND
    }

    fn prepare(context: &ToolContext, input: &Input) -> Result<Vec<Access>, PrepareError> {
        if input.command.trim().is_empty() {
            return Err(PrepareError::invalid("the command is empty"));
        }
        if input.command.len() > MAX_COMMAND_BYTES {
            return Err(ToolFailure::TooLarge {
                limit_bytes: u32::try_from(MAX_COMMAND_BYTES).unwrap_or(u32::MAX),
            }
            .into());
        }
        let shell = shell_path(context)?;
        let directory = for_read(
            context.workspace.as_deref(),
            folder_or_workspace(input.working_directory.as_deref()),
        )?;
        existing_folder(&directory)?;
        Ok(vec![Access::Command {
            command: input.command.clone(),
            shell: path_text(&shell)?,
            working_directory: directory.text()?,
            background: true,
        }])
    }

    fn execute(execution: TypedExecution<Input>) -> BoxFuture<'static, Result<Done, ToolFailure>> {
        let TypedExecution {
            context,
            input,
            accesses,
            process,
            part_id,
            activity,
            ..
        } = execution;
        async move {
            // The working directory and the shell are checked again right
            // before the command starts.
            let checked = context.clone();
            let check_input = input.clone();
            let fresh = tokio::task::spawn_blocking(move || Self::prepare(&checked, &check_input))
                .await
                .unwrap_or_else(|error| {
                    Err(PrepareError::Failed(ToolFailure::IoError {
                        kind: format!("the check stopped unexpectedly: {error}"),
                    }))
                });
            unchanged(&accesses, fresh)?;
            let [
                Access::Command {
                    shell,
                    working_directory,
                    ..
                },
            ] = accesses.as_slice()
            else {
                return Err(ToolFailure::PathChanged);
            };
            let Some(part_id) = part_id else {
                return Err(ToolFailure::IoError {
                    kind: "a command in the background needs the part of its call".to_owned(),
                });
            };
            let place = place(&context, working_directory);
            let request = StartRequest {
                conversation: context.conversation_id.clone(),
                part_id,
                activity,
                spec: BackgroundSpec {
                    shell: PathBuf::from(shell),
                    text: input.command.clone(),
                    working_directory: PathBuf::from(working_directory),
                    environment: context.environment.variables().to_vec(),
                },
                process,
            };
            let started = context.background.start(request).await?;
            Ok(Done {
                outcome: ToolOutcome::CommandStarted {
                    number: started.number,
                    command: input.command.chars().take(MAX_VIEW_COMMAND_CHARS).collect(),
                },
                output: texts::command_started(started.number, &place),
                archives: Vec::new(),
                images: Vec::new(),
            })
        }
        .boxed()
    }
}

/// Where a command runs, as its results name it: its folder and its shell.
fn place(context: &ToolContext, working_directory: &str) -> String {
    match &context.system.shell {
        Some(shell) => format!("in {working_directory} with {}", shell.name),
        None => format!("in {working_directory}"),
    }
}
