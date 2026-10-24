//! `search_text`: the lines of files that contain text or match a regular
//! expression.

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::tools::{Access, AccessMode, ToolFailure, ToolOutcome};
use regex::{Regex, RegexBuilder};
use schemars::JsonSchema;
use serde::Deserialize;

use super::walk::{Pattern, files};
use super::{folder_or_workspace, page_bounds, path_access, stopped};
use crate::paths::for_read;
use crate::results::{Page, count, cut_line};
use crate::stop::StopSignal;
use crate::text::{NotText, decode, lines};
use crate::texts;
use crate::tool::{
    Done, PermissionClass, PrepareError, ToolContext, TypedExecution, TypedTool, blocking,
};

/// The matches of a page when the model names no limit.
const DEFAULT_LIMIT: u32 = 100;
/// The most matches of a page.
const MAX_LIMIT: u32 = 200;
/// Files larger than this are skipped.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// The most characters of a matching line that a result shows.
const MAX_LINE_CHARS: usize = 500;
/// The characters before a match that a cut line keeps.
const CONTEXT_CHARS: usize = 100;
/// The largest compiled regular expression, so a pattern cannot take the
/// memory of the machine.
const MAX_REGEX_BYTES: usize = 10 * 1024 * 1024;

/// The arguments of `search_text`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// The text to find, or a regular expression when regex is true.
    query: String,
    /// Whether the query is a regular expression in Rust's syntax. False when
    /// null.
    #[serde(default)]
    regex: Option<bool>,
    /// The file or folder to search, absolute or relative to the workspace.
    /// The workspace when null.
    #[serde(default)]
    path: Option<String>,
    /// A glob pattern that limits which files below the folder are searched,
    /// such as *.rs. A pattern without a slash matches file names at any
    /// depth. Every file when null.
    #[serde(default)]
    glob: Option<String>,
    /// Whether upper and lower case must match. False when null.
    #[serde(default)]
    case_sensitive: Option<bool>,
    /// Whether to search hidden files and files that .gitignore excludes.
    /// False when null.
    #[serde(default)]
    include_ignored: Option<bool>,
    /// How many matches to skip, to read the next page. 0 when null.
    #[serde(default)]
    offset: Option<u32>,
    /// How many matches to show: 100 when null, at most 200.
    #[serde(default)]
    limit: Option<u32>,
}

impl Input {
    /// The compiled query.
    fn compile(&self) -> Result<Regex, ToolFailure> {
        if self.query.is_empty() {
            return Err(ToolFailure::InvalidPattern {
                reason: "the query is empty".to_owned(),
            });
        }
        let pattern: Cow<'_, str> = if self.regex.unwrap_or(false) {
            Cow::Borrowed(&self.query)
        } else {
            Cow::Owned(regex::escape(&self.query))
        };
        RegexBuilder::new(&pattern)
            .case_insensitive(!self.case_sensitive.unwrap_or(false))
            .size_limit(MAX_REGEX_BYTES)
            .build()
            .map_err(|error| ToolFailure::InvalidPattern {
                reason: error.to_string(),
            })
    }

    /// The compiled glob, if any.
    fn glob(&self) -> Result<Option<Pattern>, ToolFailure> {
        self.glob.as_deref().map(Pattern::new).transpose()
    }
}

/// The tool.
pub(crate) struct SearchText;

impl TypedTool for SearchText {
    type Input = Input;
    const NAME: &'static str = "search_text";
    const CLASS: PermissionClass = PermissionClass::ReadOnly;

    fn description() -> &'static str {
        texts::SEARCH_TEXT
    }

    fn prepare(context: &ToolContext, input: &Input) -> Result<Vec<Access>, PrepareError> {
        input.compile()?;
        input.glob()?;
        let target = for_read(
            context.workspace.as_deref(),
            folder_or_workspace(input.path.as_deref()),
        )?;
        if !target.exists() {
            return Err(ToolFailure::NotFound {
                path: target.path.display().to_string(),
            }
            .into());
        }
        Ok(vec![path_access(
            &target,
            AccessMode::Read,
            target.is_folder(),
        )?])
    }

    fn execute(execution: TypedExecution<Input>) -> BoxFuture<'static, Result<Done, ToolFailure>> {
        let TypedExecution {
            context,
            input,
            stop,
            ..
        } = execution;
        blocking(move || search(&context, &input, &stop)).boxed()
    }
}

/// Files that a search skipped, which the result names.
#[derive(Debug, Default)]
struct Skipped {
    large: usize,
    binary: usize,
    unreadable: usize,
}

fn search(context: &ToolContext, input: &Input, stop: &StopSignal) -> Result<Done, ToolFailure> {
    let query = input.compile()?;
    let glob = input.glob()?;
    let target = for_read(
        context.workspace.as_deref(),
        folder_or_workspace(input.path.as_deref()),
    )?;
    if !target.exists() {
        return Err(ToolFailure::NotFound {
            path: target.path.display().to_string(),
        });
    }
    let target_text = target.text()?;
    // The files and how a match names them: below the folder, or by name.
    let (folder, candidates): (PathBuf, Box<dyn Iterator<Item = PathBuf>>) = if target.is_folder() {
        let found = files(&target.path, input.include_ignored.unwrap_or(false))
            .filter(move |relative| glob.as_ref().is_none_or(|glob| glob.matches(relative)));
        (target.path.clone(), Box::new(found))
    } else {
        let parent = target
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let name = target
            .path
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_default();
        (parent, Box::new(std::iter::once(name)))
    };

    let (offset, limit) = page_bounds(input.offset, input.limit, DEFAULT_LIMIT, MAX_LIMIT);
    let mut page = Page::new(context.result_limit, &[&target_text, &input.query]);
    let mut skipped = Skipped::default();
    let mut matched = 0_usize;
    let mut shown = 0_usize;
    let mut more = false;
    'files: for relative in candidates {
        if stop.is_stopped() {
            return Err(stopped());
        }
        let Some(text) = file_text(&folder.join(&relative), &mut skipped) else {
            continue;
        };
        for (index, line) in lines(&text).into_iter().enumerate() {
            let Some(found) = query.find(line) else {
                continue;
            };
            matched += 1;
            if matched <= offset {
                continue;
            }
            let rendered = format!(
                "{}:{}: {}",
                relative.display(),
                index + 1,
                excerpt(line, found.start())
            );
            if shown == limit || !page.push(&rendered) {
                more = true;
                break 'files;
            }
            shown += 1;
        }
    }

    let mut output = format!("Searched: {target_text}\n");
    if matched == 0 {
        output.push_str("No lines match.\n");
    } else {
        output.push_str(&texts::items_shown(
            "Matches",
            offset + 1,
            offset + shown,
            None,
        ));
        output.push('\n');
        output.push_str(&page.into_body());
        if more {
            output.push_str(&texts::read_on(offset + shown));
            output.push('\n');
        }
    }
    if let Some(note) = skipped_note(&skipped) {
        output.push_str(&note);
        output.push('\n');
    }
    Ok(Done {
        outcome: ToolOutcome::Searched {
            path: target_text,
            shown: count(shown),
            more,
        },
        output,
        archives: Vec::new(),
        images: Vec::new(),
    })
}

/// The text of a file to search, or `None` when it is skipped. A file in an
/// encoding other than UTF-8 or UTF-16 is searched as UTF-8 with its invalid
/// bytes replaced, so the ASCII text of files in other encodings is found.
fn file_text(path: &Path, skipped: &mut Skipped) -> Option<String> {
    let Ok(metadata) = fs::metadata(path) else {
        skipped.unreadable += 1;
        return None;
    };
    if metadata.len() > MAX_FILE_BYTES {
        skipped.large += 1;
        return None;
    }
    let Ok(bytes) = fs::read(path) else {
        skipped.unreadable += 1;
        return None;
    };
    match decode(&bytes) {
        Ok(text) => Some(text.text),
        Err(NotText::Binary) => {
            skipped.binary += 1;
            None
        }
        Err(NotText::UnsupportedEncoding) => Some(String::from_utf8_lossy(&bytes).into_owned()),
    }
}

/// A matching line, cut to its part around the match when it is long.
fn excerpt(line: &str, match_start: usize) -> String {
    let line = line.trim_end_matches('\r');
    let match_char = line[..match_start.min(line.len())].chars().count();
    let skip = match_char.saturating_sub(CONTEXT_CHARS);
    let shown_from = line
        .char_indices()
        .nth(skip)
        .map_or(line.len(), |(index, _)| index);
    let (shown, left_out) = cut_line(&line[shown_from..], MAX_LINE_CHARS);
    let before = if skip > 0 {
        format!("[cut: {skip} characters before] ")
    } else {
        String::new()
    };
    let after = if left_out > 0 {
        texts::line_cut(left_out)
    } else {
        String::new()
    };
    format!("{before}{shown}{after}")
}

/// The note on skipped files, if any were.
fn skipped_note(skipped: &Skipped) -> Option<String> {
    let mut parts = Vec::new();
    if skipped.large > 0 {
        parts.push(format!("{} over 16 MiB", skipped.large));
    }
    if skipped.binary > 0 {
        parts.push(format!("{} binary", skipped.binary));
    }
    if skipped.unreadable > 0 {
        parts.push(format!("{} unreadable", skipped.unreadable));
    }
    (!parts.is_empty()).then(|| format!("Files skipped: {}.", parts.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::excerpt;

    #[test]
    fn long_lines_show_the_part_around_the_match() {
        let line = format!("{}needle{}", "a".repeat(300), "b".repeat(1000));

        let shown = excerpt(&line, 300);

        assert!(
            shown.starts_with("[cut: 200 characters before] "),
            "{shown}"
        );
        assert!(shown.contains("needle"));
        assert!(shown.ends_with(" [cut: 606 more characters]"), "{shown}");
    }

    #[test]
    fn short_lines_are_shown_whole() {
        assert_eq!(excerpt("let x = needle;\r", 8), "let x = needle;");
    }
}
