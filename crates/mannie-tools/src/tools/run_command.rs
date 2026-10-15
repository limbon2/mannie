//! `run_command`: runs a command in the platform's shell, contained, so every
//! process it starts ends with it (docs/features/system-tools/commands.md).

use std::path::PathBuf;
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::tools::{Access, CommandEnding, ToolFailure, ToolOutcome};
use schemars::JsonSchema;
use serde::Deserialize;

use super::change::unchanged;
use super::{existing_folder, folder_or_workspace};
use crate::paths::{for_read, path_text};
use crate::supervisor::{self, Captured, CommandSpec, Finished};
use crate::texts;
use crate::tool::{
    ArchivedOutput, Done, PermissionClass, PrepareError, ToolContext, TypedExecution, TypedTool,
};

/// The longest command text: well within the longest argument that Linux
/// passes to a program.
const MAX_COMMAND_BYTES: usize = 100_000;
/// The timeout when the model names none.
const DEFAULT_TIMEOUT_SECONDS: u32 = 60;
/// The longest timeout.
const MAX_TIMEOUT_SECONDS: u32 = 600;
/// The share of a stream's room that its start gets when it is cut; the end
/// gets the rest, because errors and summaries come last.
const HEAD_SHARE: usize = 4;
/// The bytes of a rendering kept free for its header and its notes.
const FRAME_BYTES: usize = 1024;

/// The arguments of `run_command`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// The command, in the shell that `system_info` names. It runs without a
    /// terminal and without input.
    command: String,
    /// The folder the command runs in, absolute or relative to the workspace.
    /// The workspace when null.
    #[serde(default)]
    working_directory: Option<String>,
    /// How many seconds the command may run: 60 when null, at most 600.
    #[serde(default)]
    timeout_seconds: Option<u32>,
}

/// The tool.
pub(crate) struct RunCommand;

impl TypedTool for RunCommand {
    type Input = Input;
    const NAME: &'static str = "run_command";
    const CLASS: PermissionClass = PermissionClass::SideEffects;

    fn description() -> &'static str {
        texts::RUN_COMMAND
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
        }])
    }

    fn execute(execution: TypedExecution<Input>) -> BoxFuture<'static, Result<Done, ToolFailure>> {
        let TypedExecution {
            context,
            input,
            accesses,
            stop,
            mut process,
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
            let timeout_seconds = input
                .timeout_seconds
                .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
                .clamp(1, MAX_TIMEOUT_SECONDS);
            let shell = PathBuf::from(shell);
            let working_directory = PathBuf::from(working_directory);
            let spec = CommandSpec {
                shell: &shell,
                text: &input.command,
                working_directory: &working_directory,
                environment: context.environment.variables(),
                timeout: Duration::from_secs(u64::from(timeout_seconds)),
            };
            let finished = supervisor::run(&spec, &stop, |record| {
                if let Some(sender) = process.take()
                    && sender.send(record).is_err()
                {
                    tracing::debug!("the journal no longer waits for the command's process");
                }
            })
            .await
            .map_err(|error| spawn_failure(&error, &shell))?;
            Ok(render(&context, &input.command, timeout_seconds, &finished))
        }
        .boxed()
    }
}

/// The shell's path, which must exist.
fn shell_path(context: &ToolContext) -> Result<PathBuf, ToolFailure> {
    let Some(shell) = &context.system.shell else {
        return Err(ToolFailure::ShellMissing {
            path: String::new(),
        });
    };
    if !shell.path.is_file() {
        return Err(ToolFailure::ShellMissing {
            path: shell.path.display().to_string(),
        });
    }
    Ok(shell.path.clone())
}

/// The failure of a command that could not start.
fn spawn_failure(error: &std::io::Error, shell: &std::path::Path) -> ToolFailure {
    if error.kind() == std::io::ErrorKind::NotFound {
        ToolFailure::ShellMissing {
            path: shell.display().to_string(),
        }
    } else {
        ToolFailure::SpawnFailed {
            reason: error.to_string(),
        }
    }
}

/// The result: how the command ended, how long it ran, and its output within
/// the result limit. A stream that does not fit is archived whole, and the
/// rendering shows its start and its end.
fn render(context: &ToolContext, command: &str, timeout_seconds: u32, finished: &Finished) -> Done {
    let ending = match (finished.ending, finished.exit_code) {
        (CommandEnding::Exited, Some(code)) => format!("Exit code: {code}\n"),
        (CommandEnding::Exited, None) => "The shell ended without an exit code.\n".to_owned(),
        (CommandEnding::TimedOut, _) => format!(
            "Timed out after {timeout_seconds} s: Mannie ended the command and every process it started.\n"
        ),
        (CommandEnding::Stopped, _) => {
            "Stopped by the owner: Mannie ended the command and every process it started.\n"
                .to_owned()
        }
        (CommandEnding::Interrupted, _) => {
            "Interrupted because Mannie quit: Mannie ended the command and every process it started.\n"
                .to_owned()
        }
    };
    let duration = format!("Ran for {:.1} s.\n", finished.duration.as_secs_f64());
    let budget = context
        .result_limit
        .saturating_sub(ending.len() + duration.len() + FRAME_BYTES);
    let (stdout_room, stderr_room) = share(
        budget,
        finished.stdout.text.len(),
        finished.stderr.text.len(),
    );
    let mut archives = Vec::new();
    let stdout = stream(
        context,
        ("stdout", "Standard output"),
        &finished.stdout,
        stdout_room,
        &mut archives,
    );
    let stderr = stream(
        context,
        ("stderr", "Standard error"),
        &finished.stderr,
        stderr_room,
        &mut archives,
    );
    let replaced = if finished.stdout.replaced || finished.stderr.replaced {
        "Bytes that are not valid UTF-8 were replaced with U+FFFD.\n"
    } else {
        ""
    };
    Done {
        outcome: ToolOutcome::CommandRan {
            command: command.to_owned(),
            ending: finished.ending,
            exit_code: finished.exit_code,
            duration_ms: u32::try_from(finished.duration.as_millis()).unwrap_or(u32::MAX),
        },
        output: format!("{ending}{duration}{stdout}{stderr}{replaced}"),
        archives,
    }
}

/// How the room of a rendering is shared between the two streams: a stream
/// that needs less than half keeps all it needs, and the other gets the rest.
fn share(budget: usize, stdout: usize, stderr: usize) -> (usize, usize) {
    if stdout + stderr <= budget {
        return (stdout, stderr);
    }
    let half = budget / 2;
    match (stdout <= half, stderr <= half) {
        (true, _) => (stdout, budget - stdout),
        (_, true) => (budget - stderr, stderr),
        _ => (half, budget - half),
    }
}

/// One stream's part of the rendering: the whole stream when it fits, else
/// its start and its end, with the whole stream archived.
fn stream(
    context: &ToolContext,
    (name, title): (&str, &str),
    captured: &Captured,
    room: usize,
    archives: &mut Vec<ArchivedOutput>,
) -> String {
    if captured.text.is_empty() {
        return format!("{title}: none.\n");
    }
    let header = format!("{title} ({} bytes):\n", captured.total);
    let body = if captured.text.len() <= room {
        captured.text.clone()
    } else {
        let id = (context.archive_ids)();
        let (head, tail, first_left_out, last_left_out) = head_and_tail(&captured.text, room);
        archives.push(ArchivedOutput {
            id: id.clone(),
            name: name.to_owned(),
            content: captured.text.clone(),
        });
        format!(
            "{head}[lines {first_left_out} to {last_left_out} left out: the whole output is archived; read it with read_tool_output, output_id {id}, offset {first_left_out}]\n{tail}"
        )
    };
    let line_break = if body.ends_with('\n') { "" } else { "\n" };
    format!("{header}{body}{line_break}")
}

/// The start and the end of a text within `room` bytes, cut at line breaks,
/// with the numbers of the first and the last line left out, counting from 1.
/// A first or last line that alone is too long is cut at a character.
fn head_and_tail(text: &str, room: usize) -> (&str, &str, usize, usize) {
    let head_room = room / HEAD_SHARE;
    let tail_room = room - head_room;
    let mut head_end = 0;
    let mut head_lines = 0;
    for line in text.split_inclusive('\n') {
        if head_end + line.len() > head_room {
            break;
        }
        head_end += line.len();
        head_lines += 1;
    }
    let mut tail_start = text.len();
    let mut tail_lines = 0;
    for line in text[head_end..].split_inclusive('\n').rev() {
        if text.len() - tail_start + line.len() > tail_room {
            break;
        }
        tail_start -= line.len();
        tail_lines += 1;
    }
    if tail_lines == 0 {
        // The last line alone is too long: keep its end.
        tail_start = floor_boundary(text, text.len().saturating_sub(tail_room).max(head_end));
    }
    let total_lines = text.split_inclusive('\n').count();
    let first_left_out = head_lines + 1;
    let last_left_out = total_lines.saturating_sub(tail_lines).max(first_left_out);
    (
        &text[..head_end],
        &text[tail_start..],
        first_left_out,
        last_left_out,
    )
}

/// The nearest character boundary at or after `index`.
fn floor_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::{head_and_tail, share};

    #[test]
    fn streams_share_the_room() {
        assert_eq!(share(100, 30, 40), (30, 40));
        assert_eq!(share(100, 10, 400), (10, 90));
        assert_eq!(share(100, 400, 10), (90, 10));
        assert_eq!(share(100, 400, 400), (50, 50));
    }

    #[test]
    fn the_start_and_the_end_are_kept_at_line_breaks() {
        let lines: Vec<String> = (1..=100).map(|line| format!("line {line:03}")).collect();
        let text = format!("{}\n", lines.join("\n"));

        let (head, tail, first, last) = head_and_tail(&text, 200);

        // Each line has 9 bytes: 50 bytes hold 5 lines, 150 bytes 16.
        assert_eq!(head, "line 001\nline 002\nline 003\nline 004\nline 005\n");
        assert!(
            tail.starts_with("line 085\n") && tail.ends_with("line 100\n"),
            "{tail}"
        );
        assert_eq!((first, last), (6, 84));
    }

    #[test]
    fn a_single_long_line_keeps_its_end() {
        let text = "x".repeat(1000);

        let (head, tail, first, last) = head_and_tail(&text, 100);

        assert_eq!(head, "");
        assert_eq!(tail.len(), 75);
        assert_eq!((first, last), (1, 1));
    }
}
