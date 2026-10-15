//! `read_tool_output`: lines of an archived output of the same conversation.

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::ids::ArchiveId;
use mannie_types::tools::{Access, MAX_OUTPUT_PAGE_LINES, ToolFailure, ToolOutcome};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::output::output_page;
use crate::results::{Page, count};
use crate::texts;
use crate::tool::{Done, PermissionClass, PrepareError, ToolContext, TypedExecution, TypedTool};

/// The arguments of `read_tool_output`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// The reference of the archived output, as a tool result named it.
    output_id: String,
    /// The first line to read, counting from 1. 1 when null.
    #[serde(default)]
    offset: Option<u32>,
    /// How many lines to read: 2000 when null, at most 2000.
    #[serde(default)]
    limit: Option<u32>,
}

/// The tool.
pub(crate) struct ReadToolOutput;

impl TypedTool for ReadToolOutput {
    type Input = Input;
    const NAME: &'static str = "read_tool_output";
    const CLASS: PermissionClass = PermissionClass::ReadOnly;

    fn description() -> &'static str {
        texts::READ_TOOL_OUTPUT
    }

    /// An archived output belongs to the conversation, so reading it touches
    /// nothing that needs authorization.
    fn prepare(_context: &ToolContext, _input: &Input) -> Result<Vec<Access>, PrepareError> {
        Ok(Vec::new())
    }

    fn execute(execution: TypedExecution<Input>) -> BoxFuture<'static, Result<Done, ToolFailure>> {
        let TypedExecution { context, input, .. } = execution;
        async move {
            let archive_id = ArchiveId::parse(input.output_id.trim())
                .map_err(|_invalid| ToolFailure::OutputNotFound)?;
            let archive = context
                .store
                .archive(&context.conversation_id, &archive_id)
                .await
                .map_err(|error| ToolFailure::IoError {
                    kind: format!("the archive could not be read: {error}"),
                })?
                .ok_or(ToolFailure::OutputNotFound)?;
            let first = input.offset.unwrap_or(1);
            let limit = input.limit.unwrap_or(MAX_OUTPUT_PAGE_LINES);
            let page_lines = output_page(&archive, first, limit);
            let total = usize::try_from(page_lines.total_lines).unwrap_or(usize::MAX);
            let first = usize::try_from(page_lines.first_line).unwrap_or(usize::MAX);

            let mut page = Page::new(context.result_limit, &[&archive.name]);
            let mut last = first - 1;
            for line in &page_lines.lines {
                if !page.push(line) {
                    break;
                }
                last += 1;
            }
            let mut output = format!(
                "Output: {} ({})\n{}\n",
                archive.name,
                archive_id,
                texts::lines_shown(first, last, total)
            );
            output.push_str(&page.into_body());
            if last >= first && last < total {
                output.push_str(&texts::read_on(last + 1));
                output.push('\n');
            }
            Ok(Done {
                outcome: ToolOutcome::OutputRead {
                    first_line: count(first),
                    last_line: count(last),
                    total_lines: count(total),
                },
                output,
                archives: Vec::new(),
            })
        }
        .boxed()
    }
}
