//! The text that the model reads from the tools: their descriptions and the
//! wording of their results (docs/features/system-tools/tools-and-results.md,
//! the descriptions the model sees). Reviewed like code: a change here changes
//! how models use the tools.

use mannie_types::tools::{PathRefusal, ToolFailure};

/// The description of the argument that every call carries for the owner
/// (docs/features/system-tools/tool-calls.md, the activity of a call).
pub(crate) const ACTIVITY: &str = "A few words for the owner that say what this call does, as an ongoing action, such as \"Reading package.json\" or \"Counting the failed tests\".";

/// The description of `system_info`.
pub(crate) const SYSTEM_INFO: &str = "Shows the operating system with its version and architecture, the shell that run_command uses, and the conversation's workspace.";

/// The description of `list_directory`.
pub(crate) const LIST_DIRECTORY: &str = "Lists the entries of a folder with their kind, size and time of change, sorted by name, in pages.";

/// The description of `file_info`.
pub(crate) const FILE_INFO: &str = "Shows the kind, size and times of a file or folder, whether it is read-only, and where a link points, without following it.";

/// The description of `find_files`.
pub(crate) const FIND_FILES: &str = "Finds files below a folder whose path matches a glob pattern, such as **/*.rs or src/*.toml. A pattern without a slash matches file names at any depth. Skips hidden files and files that .gitignore excludes unless include_ignored is true. Returns paths relative to the folder, in pages.";

/// The description of `search_text`.
pub(crate) const SEARCH_TEXT: &str = "Searches a file, or every file below a folder, for text or a regular expression, and returns the matching lines with their path and line number, in pages. Skips binary files, files over 16 MiB, and hidden or ignored files unless include_ignored is true.";

/// The description of `read_file`.
pub(crate) const READ_FILE: &str = "Reads lines of a text file, numbered from 1, with the file's revision. The numbers are not part of the file. Pass the revision to write_file or edit_file.";

/// The description of `write_file`.
pub(crate) const WRITE_FILE: &str = "Writes a whole text file. Without expected_revision it creates a new file and fails if one exists; with it, it replaces the file only when the revision still matches.";

/// The description of `edit_file`.
pub(crate) const EDIT_FILE: &str = "Replaces exact text in a file. Each old_text must appear exactly once, copied without line numbers, and edits must not overlap. Either every edit applies or none. expected_revision is the revision of your latest read.";

/// The description of `create_directory`.
pub(crate) const CREATE_DIRECTORY: &str = "Creates a folder and any missing folders above it.";

/// The description of `copy_path`.
pub(crate) const COPY_PATH: &str =
    "Copies a file, or a folder with its contents, to a path that does not exist yet.";

/// The description of `move_path`.
pub(crate) const MOVE_PATH: &str =
    "Moves or renames a file or folder on the same volume, to a path that does not exist yet.";

/// The description of `delete_path`.
pub(crate) const DELETE_PATH: &str =
    "Deletes a file or folder for good. A folder that is not empty needs recursive set to true.";

/// The description of `run_command`.
#[cfg(any(windows, target_os = "linux"))]
pub(crate) const RUN_COMMAND: &str = "Runs a command in the shell that system_info names, without a terminal or input, in working_directory or the workspace. It ends after timeout_seconds (60 by default, at most 600), and every process it started ends with it. Returns the exit code and the output.";

/// The description of `read_tool_output`.
pub(crate) const READ_TOOL_OUTPUT: &str =
    "Reads lines of an archived tool output by its reference.";

/// The description of `move_to_project`.
pub(crate) const MOVE_TO_PROJECT: &str = "Moves this conversation into one of the owner's projects, so that the tools work in the project's folder from the next call on. Call it first when the owner's request is about a project that the environment lists. A conversation moves once and never leaves its project.";

/// The result of a move into a project.
pub(crate) fn moved_to_project(name: &str, folder: &str) -> String {
    format!(
        "This conversation now belongs to the project {name}. Its workspace is {folder}, from the next tool call on."
    )
}

/// The rendering of a failure: its code, what happened and what the model can
/// do about it.
pub(crate) fn failure(failure: &ToolFailure) -> String {
    let code = failure.code().trim_start_matches("tool.");
    let explanation = match failure {
        ToolFailure::NoWorkspace => "The conversation has no workspace, so relative paths and commands without a working directory cannot be used. Use an absolute path. When the owner's request is about one of the owner's projects, move the conversation into it with move_to_project first.".to_owned(),
        ToolFailure::PathRefused { reason } => refusal(*reason).to_owned(),
        ToolFailure::PathThroughLink { resolved } => format!("The path goes through a link, and changes never follow links. The link resolves to {resolved}; use that path if it is the one you mean."),
        ToolFailure::NotFound { path } => format!("{path} does not exist."),
        ToolFailure::NotAFile { path } => format!("{path} is not a file."),
        ToolFailure::NotAFolder { path } => format!("{path} is not a folder."),
        ToolFailure::AlreadyExists { path } => format!("{path} already exists. Choose another path, or read the file and pass its revision to replace it."),
        ToolFailure::PermissionDenied { path } => format!("The operating system refused access to {path}."),
        ToolFailure::InUse { path } => format!("{path} is in use by another program. Try again once the program has released it."),
        ToolFailure::RevisionMismatch { current } => format!("The file changed since the revision you passed. Its revision is now {current}. Read it again before you change it."),
        ToolFailure::EditNotFound { edit } => format!("The old_text of edit {edit} does not appear in the file. Copy it exactly from a fresh read, without line numbers."),
        ToolFailure::EditAmbiguous { edit, matches } => format!("The old_text of edit {edit} appears {matches} times. Include more of the surrounding lines so that it appears once."),
        ToolFailure::EditsOverlap { first, second } => format!("Edits {first} and {second} change overlapping text. Combine them into one edit."),
        ToolFailure::TooLarge { limit_bytes } => format!("This is larger than the limit of {limit_bytes} bytes."),
        ToolFailure::BinaryFile { path } => format!("{path} is a binary file, which this tool does not read or change."),
        ToolFailure::UnsupportedEncoding { path } => format!("{path} is not UTF-8, or UTF-16 with a byte order mark, which this tool does not read or change."),
        ToolFailure::LinkInSource { path } => format!("{path} is a link. copy_path does not copy links."),
        ToolFailure::DifferentVolume => "The source and the destination are on different volumes, and move_path works within one volume. Copy the path, then delete the source.".to_owned(),
        ToolFailure::FolderNotEmpty { path } => format!("{path} is not empty. Set recursive to true to delete it with its contents."),
        ToolFailure::TooManyEntries { limit } => format!("This involves more than {limit} entries, more than the tool handles at once."),
        ToolFailure::InvalidPattern { reason } => format!("The pattern is not valid: {reason}"),
        ToolFailure::OutputNotFound => "No archived output has this reference in this conversation.".to_owned(),
        ToolFailure::ProjectNotFound { project } => format!("No single project is {project}. Name a project by its folder, as the environment lists it."),
        ToolFailure::AlreadyInProject { name } => format!("This conversation already belongs to the project {name}, and a conversation never leaves its project."),
        ToolFailure::PathChanged => "The path changed after the operation was authorized, for example because another program replaced a file, so nothing was changed. Check the path and call the tool again.".to_owned(),
        ToolFailure::ShellMissing { path } => format!("The shell {path} is missing, so no command can run."),
        ToolFailure::SpawnFailed { reason } => format!("The command could not be started: {reason}"),
        ToolFailure::IoError { kind } => format!("The operating system reported an error: {kind}"),
    };
    format!("Error {code}: {explanation}")
}

/// Why a path was refused, for the model.
fn refusal(reason: PathRefusal) -> &'static str {
    match reason {
        PathRefusal::Empty => "The path is empty or contains a control character.",
        PathRefusal::NetworkPath => {
            "Network paths are not allowed. Work with files on this computer."
        }
        PathRefusal::DevicePath => {
            "Device paths and paths that start with \\\\?\\ or \\\\.\\ are not allowed. Use an ordinary path."
        }
        PathRefusal::AlternateStream => {
            "Alternate data streams are not allowed: a name must not contain a colon."
        }
        PathRefusal::ReservedName => {
            "The path contains a reserved device name, such as CON or COM1, which Windows does not allow as a file name."
        }
        PathRefusal::TrailingDotOrSpace => {
            "A name in the path ends in a dot or a space, which Windows removes."
        }
        PathRefusal::DriveRelative => {
            "The path depends on the current folder of a drive. Use a path that starts with a drive, such as C:\\, or a path relative to the workspace."
        }
        PathRefusal::TooLong => "The path is longer than 32,767 characters.",
        PathRefusal::NotAbsolute => "The path must be absolute.",
    }
}

/// The note after a page that more follows.
pub(crate) fn read_on(offset: usize) -> String {
    format!("More follows: call again with offset {offset}.")
}

/// The note on a line that was cut.
pub(crate) fn line_cut(left_out: usize) -> String {
    format!(" [cut: {left_out} more characters]")
}

/// The header of a page of lines.
pub(crate) fn lines_shown(first: usize, last: usize, total: usize) -> String {
    if first > last {
        format!("Lines shown: none of {total}.")
    } else {
        format!("Lines {first} to {last} of {total}.")
    }
}

/// The header of a page of entries or matches.
pub(crate) fn items_shown(noun: &str, first: usize, last: usize, total: Option<usize>) -> String {
    match (first > last, total) {
        (true, Some(total)) => format!("{noun} shown: none of {total}."),
        (true, None) => format!("{noun} shown: none."),
        (false, Some(total)) => format!("{noun} {first} to {last} of {total}."),
        (false, None) => format!("{noun} {first} to {last}."),
    }
}
