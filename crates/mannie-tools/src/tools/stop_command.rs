//! `stop_command`: ends a command that runs in the background (ADR-0041,
//! docs/features/system-tools/background-commands.md, stopping a command).
//! It is an operation without accesses, which the policy allows, since the
//! command belongs to the same conversation and its start was authorized;
//! its call is journaled as every side effect is.

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::tools::{Access, BackgroundSource, ToolFailure, ToolOutcome};
use schemars::JsonSchema;
use serde::Deserialize;

use super::read_command_output::{header_command, place};
use crate::texts;
use crate::tool::{Done, PermissionClass, PrepareError, ToolContext, TypedExecution, TypedTool};

/// The arguments of `stop_command`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// The command's number, as `start_command` returned it.
    command: u32,
}

/// The tool.
pub(crate) struct StopCommand;

impl TypedTool for StopCommand {
    type Input = Input;
    const NAME: &'static str = "stop_command";
    const CLASS: PermissionClass = PermissionClass::SideEffects;

    fn description() -> &'static str {
        texts::STOP_COMMAND
    }

    /// The command belongs to the conversation, so ending it needs no access
    /// of its own: its start was authorized.
    fn prepare(_context: &ToolContext, _input: &Input) -> Result<Vec<Access>, PrepareError> {
        Ok(Vec::new())
    }

    fn execute(execution: TypedExecution<Input>) -> BoxFuture<'static, Result<Done, ToolFailure>> {
        let TypedExecution { context, input, .. } = execution;
        async move {
            let stopped = context
                .background
                .stop(context.conversation_id.clone(), input.command)
                .await?;
            let view = &stopped.view;
            let remote = matches!(view.source, BackgroundSource::Remote { .. });
            let state = texts::command_state(view, stopped.ran_for, remote);
            let what = if stopped.had_ended {
                "had ended already"
            } else {
                "is stopped"
            };
            let output = format!(
                "Command {} ({}, {}) {what}. {state} {} of its lines are unread.\n",
                input.command,
                header_command(&view.command),
                place(&view.source),
                stopped.unread
            );
            let ending = view.ending.ok_or_else(|| ToolFailure::IoError {
                kind: "the command still runs after it was stopped".to_owned(),
            })?;
            Ok(Done {
                outcome: ToolOutcome::CommandStopped {
                    number: input.command,
                    ending,
                    exit_code: view.exit_code,
                },
                output,
                archives: Vec::new(),
            })
        }
        .boxed()
    }
}
