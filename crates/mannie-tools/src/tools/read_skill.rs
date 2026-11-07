//! `read_skill`: a skill's instructions or another of its files, in numbered
//! lines (ADR-0061, decision 7). The skill comes from the call's shelf, and
//! the read is a path access that the policy decides: inside a skill that is
//! on it runs without asking, as it does inside the workspace.

use std::fs;
use std::path::{Path, PathBuf};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::tools::{Access, AccessMode, ToolFailure, ToolOutcome};
use schemars::JsonSchema;
use serde::Deserialize;

use super::path_access;
use super::read_file::{MAX_LIMIT, file_header, numbered_line};
use crate::digest::revision;
use crate::mentions::may_hold_secrets;
use crate::paths::{Resolved, is_link, path_text};
use crate::projects::Availability;
use crate::results::{Page, count};
use crate::skills::{echo, file_text, read_frontmatter, skill_file};
use crate::text::lines;
use crate::texts;
use crate::tool::{
    Done, PermissionClass, PrepareError, ToolContext, TypedExecution, TypedTool, blocking,
};

/// The name of a skill's instructions, as the outcome names them.
const INSTRUCTIONS: &str = "SKILL.md";

/// The most other files of a skill that the last page of its instructions
/// lists.
const MAX_LISTED_FILES: usize = 100;

/// How many folders deep the list of other files goes.
const MAX_LISTED_DEPTH: usize = 3;

/// The most entries that listing the other files looks at, so a skill with a
/// huge tree costs a bounded walk.
const MAX_WALKED_ENTRIES: usize = 2_000;

/// The arguments of `read_skill`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// The skill's name, as your instructions list it or the owner's mention
    /// gives it.
    name: String,
    /// A file in the skill's folder, relative to it, such as
    /// references/forms.md, or null for the skill's instructions.
    #[serde(default)]
    path: Option<String>,
    /// The first line to read, counting from 1. 1 when null.
    #[serde(default)]
    offset: Option<u32>,
    /// How many lines to read: 2000 when null, at most 2000.
    #[serde(default)]
    limit: Option<u32>,
}

/// The tool.
pub(crate) struct ReadSkill;

impl TypedTool for ReadSkill {
    type Input = Input;
    const NAME: &'static str = "read_skill";
    const CLASS: PermissionClass = PermissionClass::ReadOnly;

    fn description() -> &'static str {
        texts::READ_SKILL
    }

    fn offered(availability: &Availability<'_>) -> bool {
        availability.skills
    }

    fn prepare(context: &ToolContext, input: &Input) -> Result<Vec<Access>, PrepareError> {
        let file = skill_file_of(context, input)?;
        Ok(vec![path_access(&file.resolved, AccessMode::Read, false)?])
    }

    fn execute(execution: TypedExecution<Input>) -> BoxFuture<'static, Result<Done, ToolFailure>> {
        let TypedExecution { context, input, .. } = execution;
        blocking(move || read(&context, &input)).boxed()
    }
}

/// The file of a skill that a call names.
struct SkillFile {
    /// The skill's name.
    name: String,
    /// The skill's canonical folder.
    folder: PathBuf,
    /// The file, resolved.
    resolved: Resolved,
    /// The file's path in the skill's folder, with `/` between names.
    relative: String,
}

impl SkillFile {
    /// Whether the file is the skill's instructions.
    fn is_instructions(&self) -> bool {
        self.relative == INSTRUCTIONS
    }
}

fn skill_file_of(context: &ToolContext, input: &Input) -> Result<SkillFile, ToolFailure> {
    let skill = context
        .skills
        .find(&input.name)
        .ok_or_else(|| ToolFailure::SkillNotFound {
            name: echo(&input.name),
        })?;
    let resolved = skill_file(&skill.folder, input.path.as_deref())?;
    let relative = input.path.as_deref().map_or_else(
        || INSTRUCTIONS.to_owned(),
        |path| path.replace('\\', "/").trim_start_matches("./").to_owned(),
    );
    Ok(SkillFile {
        name: skill.name.clone(),
        folder: skill.folder.clone(),
        resolved,
        relative,
    })
}

fn read(context: &ToolContext, input: &Input) -> Result<Done, ToolFailure> {
    let file = skill_file_of(context, input)?;
    let path = file.resolved.text()?;
    let folder = path_text(&file.folder)?;
    let (bytes, text) = file_text(&file.resolved)?;
    let revision = revision(&bytes);
    // The instructions come without their frontmatter, whose fields agents'
    // list already gave.
    let shown = if file.is_instructions() {
        read_frontmatter(&text.text).map_or(text.text.as_str(), |frontmatter| {
            frontmatter.body(&text.text)
        })
    } else {
        text.text.as_str()
    };
    let all = lines(shown);
    let total = all.len();
    let first = usize::try_from(input.offset.unwrap_or(1).max(1)).unwrap_or(usize::MAX);
    let limit =
        usize::try_from(input.limit.unwrap_or(MAX_LIMIT).clamp(1, MAX_LIMIT)).unwrap_or(usize::MAX);

    let heading = texts::skill_heading(&file.name);
    let folder_line = texts::skill_folder(&folder);
    let mut page = Page::new(
        context.result_limit,
        &[&heading, &folder_line, &path, &revision],
    );
    let mut last = first - 1;
    for (index, line) in all.iter().enumerate().skip(first - 1).take(limit) {
        if !page.push(&numbered_line(index, line)) {
            break;
        }
        last = index + 1;
    }
    let reached_end = last >= total;
    if file.is_instructions() && reached_end {
        list_other_files(&mut page, &file.folder);
    }

    let mut output = format!("{heading}\n{folder_line}\n");
    output.push_str(&file_header(&path, &revision, &text));
    if total == 0 {
        output.push_str(texts::EMPTY_FILE);
        output.push('\n');
    } else {
        output.push_str(&texts::lines_shown(first, last, total));
        output.push('\n');
    }
    output.push_str(&page.into_body());
    if last >= first && last < total {
        output.push_str(&texts::read_on(last + 1));
        output.push('\n');
    }
    let shown_any = last >= first;
    Ok(Done {
        outcome: ToolOutcome::SkillRead {
            skill: file.name,
            path: file.relative,
            first_line: if shown_any { count(first) } else { 0 },
            last_line: if shown_any { count(last) } else { 0 },
            total_lines: count(total),
        },
        output,
        archives: Vec::new(),
        images: Vec::new(),
        cited: Vec::new(),
    })
}

/// Adds the skill's other files to the last page of its instructions, as far
/// as they fit: files three folders deep at most, without hidden names, names
/// that may hold secrets, links or the instructions themselves.
fn list_other_files(page: &mut Page, folder: &Path) {
    let files = other_files(folder);
    if !page.push("") {
        return;
    }
    if files.is_empty() {
        page.push(texts::SKILL_NO_FILES);
        return;
    }
    if !page.push(texts::SKILL_FILES) {
        return;
    }
    let listed = files.len().min(MAX_LISTED_FILES);
    for (position, file) in files.iter().take(listed).enumerate() {
        if !page.push(&format!("- {file}")) {
            page.push(&texts::skill_files_left_out(files.len() - position));
            return;
        }
    }
    if files.len() > listed {
        page.push(&texts::skill_files_left_out(files.len() - listed));
    }
}

/// The files in a skill's folder, by their paths in it, sorted.
fn other_files(folder: &Path) -> Vec<String> {
    let mut files = Vec::new();
    let mut walked = 0_usize;
    let mut pending = vec![(folder.to_path_buf(), String::new(), 0_usize)];
    while let Some((current, prefix, depth)) = pending.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            walked += 1;
            if walked > MAX_WALKED_ENTRIES {
                files.sort();
                return files;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let relative = format!("{prefix}{name}");
            let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
                continue;
            };
            let hidden = name.starts_with('.') || may_hold_secrets(&relative);
            if hidden || is_link(&metadata) || relative == INSTRUCTIONS {
                continue;
            }
            if metadata.is_dir() {
                if depth + 1 < MAX_LISTED_DEPTH {
                    pending.push((entry.path(), format!("{relative}/"), depth + 1));
                }
            } else if metadata.is_file() {
                files.push(relative);
            }
        }
    }
    files.sort();
    files
}
