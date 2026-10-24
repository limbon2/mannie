//! `read_file`: numbered lines of a text file, with the file's revision.

use std::fs;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::tools::{Access, AccessMode, ToolFailure, ToolOutcome};
use schemars::JsonSchema;
use serde::Deserialize;

use super::{existing_file, path_access};
use crate::digest::revision;
use crate::paths::{Resolved, for_read, io_failure};
use crate::results::{Page, count, cut_line};
use crate::text::{Encoding, FileText, NotText, decode, lines};
use crate::texts;
use crate::tool::{
    Done, PermissionClass, PrepareError, ToolContext, TypedExecution, TypedTool, blocking,
};

/// The most lines of a page.
pub(crate) const MAX_LIMIT: u32 = 2_000;
/// The most characters of a line that a page shows.
const MAX_LINE_CHARS: usize = 2_000;
/// The largest file that the text tools read or change.
pub(crate) const MAX_TEXT_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// The arguments of `read_file`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// The file, absolute or relative to the workspace.
    path: String,
    /// The first line to read, counting from 1. 1 when null.
    #[serde(default)]
    offset: Option<u32>,
    /// How many lines to read: 2000 when null, at most 2000.
    #[serde(default)]
    limit: Option<u32>,
}

/// The tool.
pub(crate) struct ReadFile;

impl TypedTool for ReadFile {
    type Input = Input;
    const NAME: &'static str = "read_file";
    const CLASS: PermissionClass = PermissionClass::ReadOnly;

    fn description() -> &'static str {
        texts::READ_FILE
    }

    fn prepare(context: &ToolContext, input: &Input) -> Result<Vec<Access>, PrepareError> {
        let file = text_file(context, &input.path)?;
        Ok(vec![path_access(&file, AccessMode::Read, false)?])
    }

    fn execute(execution: TypedExecution<Input>) -> BoxFuture<'static, Result<Done, ToolFailure>> {
        let TypedExecution { context, input, .. } = execution;
        blocking(move || read(&context, &input)).boxed()
    }
}

/// The path of a file that the text tools may read: it exists, is a file and
/// is not too large.
pub(crate) fn text_file(context: &ToolContext, path: &str) -> Result<Resolved, ToolFailure> {
    let file = for_read(context.workspace.as_deref(), path)?;
    let metadata = existing_file(&file)?;
    if metadata.len() > MAX_TEXT_FILE_BYTES {
        return Err(too_large());
    }
    Ok(file)
}

/// The failure for a file over [`MAX_TEXT_FILE_BYTES`].
pub(crate) fn too_large() -> ToolFailure {
    ToolFailure::TooLarge {
        limit_bytes: u32::try_from(MAX_TEXT_FILE_BYTES).unwrap_or(u32::MAX),
    }
}

/// The failure for bytes that are not text.
pub(crate) fn not_text(reason: NotText, path: String) -> ToolFailure {
    match reason {
        NotText::Binary => ToolFailure::BinaryFile { path },
        NotText::UnsupportedEncoding => ToolFailure::UnsupportedEncoding { path },
    }
}

fn read(context: &ToolContext, input: &Input) -> Result<Done, ToolFailure> {
    let file = text_file(context, &input.path)?;
    let path = file.text()?;
    let bytes = fs::read(&file.path).map_err(|error| io_failure(&error, &file.path))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_TEXT_FILE_BYTES {
        return Err(too_large());
    }
    let revision = revision(&bytes);
    let text = decode(&bytes).map_err(|reason| not_text(reason, path.clone()))?;
    let all = lines(&text.text);
    let total = all.len();
    let first = usize::try_from(input.offset.unwrap_or(1).max(1)).unwrap_or(usize::MAX);
    let limit =
        usize::try_from(input.limit.unwrap_or(MAX_LIMIT).clamp(1, MAX_LIMIT)).unwrap_or(usize::MAX);

    let mut page = Page::new(context.result_limit, &[&path, &revision]);
    let mut last = first - 1;
    for (index, line) in all.iter().enumerate().skip(first - 1).take(limit) {
        if !page.push(&numbered_line(index, line)) {
            break;
        }
        last = index + 1;
    }

    let mut output = file_header(&path, &revision, &text);
    if total == 0 {
        output.push_str(texts::EMPTY_FILE);
        output.push('\n');
    } else {
        output.push_str(&texts::lines_shown(first, last, total));
        output.push('\n');
        output.push_str(&page.into_body());
        if last >= first && last < total {
            output.push_str(&texts::read_on(last + 1));
            output.push('\n');
        }
    }
    Ok(Done {
        outcome: ToolOutcome::Read {
            path,
            first_line: count(first),
            last_line: count(last),
            total_lines: count(total),
        },
        output,
        archives: Vec::new(),
        images: Vec::new(),
    })
}

/// A line of a page: its number, from 1 at `index` 0, and its text, cut to
/// [`MAX_LINE_CHARS`] characters.
pub(crate) fn numbered_line(index: usize, line: &str) -> String {
    let (shown, left_out) = cut_line(line, MAX_LINE_CHARS);
    let mut numbered = format!("{:>6}\t{shown}", index + 1);
    if left_out > 0 {
        numbered.push_str(&texts::line_cut(left_out));
    }
    numbered
}

/// The header of a page: the path, the revision and, when the file does not
/// store UTF-8 with LF, how it does.
pub(crate) fn file_header(path: &str, revision: &str, text: &FileText) -> String {
    let mut header = format!("File: {path}\nRevision: {revision}\n");
    if let Some(note) = encoding_note(text.encoding, text.crlf) {
        header.push_str(note);
        header.push('\n');
    }
    header
}

/// How a file stores its text, when that is not UTF-8 with LF.
fn encoding_note(encoding: Encoding, crlf: bool) -> Option<&'static str> {
    match (encoding, crlf) {
        (Encoding::Utf8, false) => None,
        (Encoding::Utf8, true) => Some(
            "Text: UTF-8 with CRLF line endings, shown as LF and kept when the file is written.",
        ),
        (Encoding::Utf8WithBom, false) => {
            Some("Text: UTF-8 with a byte order mark, which is kept when the file is written.")
        }
        (Encoding::Utf8WithBom, true) => Some(
            "Text: UTF-8 with a byte order mark and CRLF line endings, shown as LF; both are kept when the file is written.",
        ),
        (Encoding::Utf16Le | Encoding::Utf16Be, false) => {
            Some("Text: UTF-16 with a byte order mark, which is kept when the file is written.")
        }
        (Encoding::Utf16Le | Encoding::Utf16Be, true) => Some(
            "Text: UTF-16 with a byte order mark and CRLF line endings, shown as LF; both are kept when the file is written.",
        ),
    }
}
