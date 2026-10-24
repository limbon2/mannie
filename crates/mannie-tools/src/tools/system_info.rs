//! `system_info`: the operating system, the shell and the workspace.

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::tools::{Access, ToolFailure, ToolOutcome};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::texts;
use crate::tool::{Done, PermissionClass, PrepareError, ToolContext, TypedExecution, TypedTool};

/// `system_info` takes no arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "models send an empty object, which a unit struct does not accept"
)]
pub(crate) struct Input {}

/// The tool.
pub(crate) struct SystemInfo;

impl TypedTool for SystemInfo {
    type Input = Input;
    const NAME: &'static str = "system_info";
    const CLASS: PermissionClass = PermissionClass::ReadOnly;

    fn description() -> &'static str {
        texts::SYSTEM_INFO
    }

    fn prepare(_context: &ToolContext, _input: &Input) -> Result<Vec<Access>, PrepareError> {
        Ok(Vec::new())
    }

    fn execute(execution: TypedExecution<Input>) -> BoxFuture<'static, Result<Done, ToolFailure>> {
        async move {
            let context = execution.context;
            let facts = &context.system;
            let shell = match &facts.shell {
                Some(shell) => format!("{} ({})", shell.name, shell.path.display()),
                None => "none: run_command is not available on this system".to_owned(),
            };
            let workspace = match &context.workspace {
                Some(workspace) => workspace.display().to_string(),
                None => "none: this is a general conversation, so use absolute paths".to_owned(),
            };
            let output = format!(
                "Operating system: {}\nShell for run_command: {shell}\nWorkspace: {workspace}\n",
                facts.system
            );
            Ok(Done {
                outcome: ToolOutcome::SystemInfo {
                    system: facts.system.clone(),
                    shell: facts
                        .shell
                        .as_ref()
                        .map(|shell| shell.name.clone())
                        .unwrap_or_default(),
                },
                output,
                archives: Vec::new(),
                images: Vec::new(),
            })
        }
        .boxed()
    }
}
