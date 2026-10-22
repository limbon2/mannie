//! `run_command`: runs a command in the platform's shell, contained, so every
//! process it starts ends with it (docs/features/system-tools/commands.md).

use std::path::PathBuf;
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::ids::ArchiveId;
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
pub(crate) const MAX_COMMAND_BYTES: usize = 100_000;
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
            background: false,
        }])
    }

    fn execute(execution: TypedExecution<Input>) -> BoxFuture<'static, Result<Done, ToolFailure>> {
        let TypedExecution {
            context,
            input,
            accesses,
            stop,
            mut process,
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
            };
            let timeout = Duration::from_secs(u64::from(timeout_seconds));
            let finished = supervisor::run(&spec, timeout, &stop, |record| {
                if let Some(sender) = process.take()
                    && sender.send(record).is_err()
                {
                    tracing::debug!("the journal no longer waits for the command's process");
                }
            })
            .await
            .map_err(|error| super::spawn_failure(&error, &shell))?;
            Ok(render(&context, &input.command, timeout_seconds, &finished))
        }
        .boxed()
    }
}

/// The shell's path, which must exist.
pub(crate) fn shell_path(context: &ToolContext) -> Result<PathBuf, ToolFailure> {
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

/// The result: how the command ended, how long it ran, and its output within
/// the result limit. A stream that does not fit is archived as it was
/// captured, and the rendering shows its start and its end.
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
/// its start and its end, with the stream archived as it was captured.
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
        let cut = Cut::of(&captured.text, room);
        archives.push(ArchivedOutput {
            id: id.clone(),
            name: name.to_owned(),
            content: captured.text.clone(),
        });
        // A start that ends inside a line still puts the note on a line of
        // its own.
        let line_break = if cut.head.is_empty() || cut.head.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        format!(
            "{}{line_break}{}\n{}",
            cut.head,
            left_out_note(&cut, captured.left_out, &id),
            cut.tail
        )
    };
    let line_break = if body.ends_with('\n') { "" } else { "\n" };
    format!("{header}{body}{line_break}")
}

/// The note between the start and the end of a stream: what the rendering
/// leaves out, and where the archive is. The archive lacks the middle of a
/// stream that was longer than the capture keeps, and the note says so.
fn left_out_note(cut: &Cut<'_>, not_captured: u64, id: &ArchiveId) -> String {
    let archived = if not_captured == 0 {
        "the whole output is archived".to_owned()
    } else {
        format!(
            "the output is archived without the {not_captured} bytes in its middle that Mannie did not keep"
        )
    };
    format!(
        "[{}: {archived}; read it with read_tool_output, output_id {id}, offset {}]",
        cut.left_out(),
        cut.first
    )
}

/// The start and the end of a text that does not fit its room, and what the
/// rendering leaves out between them.
#[derive(Debug)]
struct Cut<'a> {
    head: &'a str,
    tail: &'a str,
    /// The first line left out, in whole or in part, counting from 1.
    first: usize,
    /// The last line left out, in whole or in part.
    last: usize,
    /// The bytes between the start and the end.
    bytes: usize,
    /// Whether the start or the end stops inside a line, which then shows in
    /// part.
    splits_a_line: bool,
}

impl<'a> Cut<'a> {
    /// The start and the end of `text`, which is longer than `room` bytes,
    /// cut at line breaks. A first or a last line that alone is too long for
    /// its share is cut at a character, so the start shows its beginning and
    /// the end shows its end.
    fn of(text: &'a str, room: usize) -> Self {
        let head_room = room / HEAD_SHARE;
        let tail_room = room - head_room;
        let mut head_end = 0;
        for line in text.split_inclusive('\n') {
            if head_end + line.len() > head_room {
                break;
            }
            head_end += line.len();
        }
        if head_end == 0 {
            head_end = text.floor_char_boundary(head_room);
        }
        let mut tail_start = text.len();
        for line in text[head_end..].split_inclusive('\n').rev() {
            if text.len() - tail_start + line.len() > tail_room {
                break;
            }
            tail_start -= line.len();
        }
        if tail_start == text.len() {
            tail_start =
                text.ceil_char_boundary(text.len().saturating_sub(tail_room).max(head_end));
        }
        let head = &text[..head_end];
        let tail = &text[tail_start..];
        let before_tail = &text[..tail_start];
        let tail_starts_a_line = before_tail.ends_with('\n');
        let head_splits = !head.is_empty() && !head.ends_with('\n');
        let tail_splits = !tail.is_empty() && !tail_starts_a_line;
        let first = head.matches('\n').count() + 1;
        // The last line left out holds the byte just before the end: a line
        // break there ends the line before the end's first line.
        let last = before_tail.matches('\n').count() + usize::from(!tail_starts_a_line);
        Self {
            head,
            tail,
            first,
            last: last.max(first),
            bytes: tail_start.saturating_sub(head_end),
            splits_a_line: head_splits || tail_splits,
        }
    }

    /// What the note says is left out: whole lines, or the bytes of the lines
    /// that the start or the end cuts.
    fn left_out(&self) -> String {
        match (self.splits_a_line, self.first == self.last) {
            (false, true) => format!("line {} left out", self.first),
            (false, false) => format!("lines {} to {} left out", self.first, self.last),
            (true, true) => format!("{} bytes of line {} left out", self.bytes, self.first),
            (true, false) => format!(
                "{} bytes left out, from line {} to line {}",
                self.bytes, self.first, self.last
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use mannie_types::ids::ArchiveId;

    use super::{Cut, left_out_note, share};

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

        let cut = Cut::of(&text, 200);

        // Each line has 9 bytes: 50 bytes hold 5 lines, 150 bytes 16.
        assert_eq!(
            cut.head,
            "line 001\nline 002\nline 003\nline 004\nline 005\n"
        );
        assert!(
            cut.tail.starts_with("line 085\n") && cut.tail.ends_with("line 100\n"),
            "{}",
            cut.tail
        );
        assert_eq!((cut.first, cut.last), (6, 84));
        assert_eq!(cut.left_out(), "lines 6 to 84 left out");
    }

    #[test]
    fn a_single_long_line_keeps_its_beginning_and_its_end() {
        let text = "x".repeat(1000);

        let cut = Cut::of(&text, 100);

        assert_eq!((cut.head.len(), cut.tail.len()), (25, 75));
        assert_eq!((cut.first, cut.last, cut.bytes), (1, 1, 900));
        assert_eq!(cut.left_out(), "900 bytes of line 1 left out");
    }

    #[test]
    fn a_first_line_too_long_for_the_start_shows_its_beginning() {
        let text = format!("{}\nshort\n", "a".repeat(500));

        let cut = Cut::of(&text, 100);

        assert_eq!(cut.head, "a".repeat(25));
        assert_eq!(cut.tail, "short\n");
        assert_eq!(cut.left_out(), "476 bytes of line 1 left out");
    }

    #[test]
    fn a_cut_from_inside_one_line_into_another_names_both() {
        let text = format!("{}\nmiddle\n{}", "a".repeat(200), "z".repeat(200));

        let cut = Cut::of(&text, 100);

        assert_eq!(cut.head, "a".repeat(25));
        assert_eq!(cut.tail, "z".repeat(75));
        assert_eq!(cut.left_out(), "308 bytes left out, from line 1 to line 3");
    }

    #[test]
    fn a_cut_through_characters_keeps_them_whole() {
        // Each character has 2 bytes, and 102 bytes of room give the start
        // 25 and the end 77, so both cuts fall inside a character.
        let text = "é".repeat(500);

        let cut = Cut::of(&text, 102);

        assert_eq!((cut.head.len(), cut.tail.len()), (24, 76));
        assert!(cut.head.chars().all(|character| character == 'é'));
        assert!(cut.tail.chars().all(|character| character == 'é'));
    }

    #[test]
    fn the_note_calls_the_archive_whole_only_when_the_capture_kept_everything() {
        let id = ArchiveId::parse("0199e0f2-0000-7000-a000-000000000001").unwrap();
        let text = "x".repeat(1000);
        let cut = Cut::of(&text, 100);

        let kept = left_out_note(&cut, 0, &id);
        let partial = left_out_note(&cut, 5_000, &id);

        assert_eq!(
            kept,
            format!(
                "[900 bytes of line 1 left out: the whole output is archived; read it with read_tool_output, output_id {id}, offset 1]"
            )
        );
        assert!(
            partial.contains(
                "the output is archived without the 5000 bytes in its middle that Mannie did not keep"
            ),
            "{partial}"
        );
        assert!(!partial.contains("whole"), "{partial}");
    }
}
