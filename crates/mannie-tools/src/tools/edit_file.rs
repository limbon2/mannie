//! `edit_file`: replaces exact text in a file, all edits or none.

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::tools::{Access, AccessMode, ToolFailure, ToolOutcome};
use schemars::JsonSchema;
use serde::Deserialize;

use super::change::{locked, path_of, pin_existing, refuse_read_only, unchanged};
use super::read_file::{MAX_TEXT_FILE_BYTES, not_text, too_large};
use super::{existing_file, path_access};
use crate::digest::revision;
use crate::paths::{FinalLink, for_change};
use crate::results::count;
use crate::text::{FileText, decode};
use crate::texts;
use crate::tool::{Done, PermissionClass, PrepareError, ToolContext, TypedExecution, TypedTool};

/// The most edits of one call.
const MAX_EDITS: usize = 50;

/// The arguments of `edit_file`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// The file, absolute or relative to the workspace.
    path: String,
    /// The revision from your latest `read_file` of this file.
    expected_revision: String,
    /// From 1 to 50 replacements, applied together.
    edits: Vec<Edit>,
}

/// One replacement.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Edit {
    /// The exact text to replace, without line numbers. It must appear exactly
    /// once in the file.
    old_text: String,
    /// The text that replaces it.
    new_text: String,
}

/// The tool.
pub(crate) struct EditFile;

impl TypedTool for EditFile {
    type Input = Input;
    const NAME: &'static str = "edit_file";
    const CLASS: PermissionClass = PermissionClass::SideEffects;

    fn description() -> &'static str {
        texts::EDIT_FILE
    }

    fn prepare(context: &ToolContext, input: &Input) -> Result<Vec<Access>, PrepareError> {
        if input.edits.is_empty() || input.edits.len() > MAX_EDITS {
            return Err(PrepareError::invalid("edits must hold from 1 to 50 edits"));
        }
        if let Some(index) = input.edits.iter().position(|edit| edit.old_text.is_empty()) {
            return Err(PrepareError::InvalidArguments {
                detail: format!("the old_text of edit {} is empty", index + 1),
            });
        }
        let target = for_change(context.workspace.as_deref(), &input.path, FinalLink::Follow)?;
        let metadata = existing_file(&target)?;
        if metadata.len() > MAX_TEXT_FILE_BYTES {
            return Err(too_large().into());
        }
        refuse_read_only(&target.path, metadata)?;
        Ok(vec![path_access(&target, AccessMode::Write, false)?])
    }

    fn execute(execution: TypedExecution<Input>) -> BoxFuture<'static, Result<Done, ToolFailure>> {
        let TypedExecution {
            context,
            input,
            accesses,
            ..
        } = execution;
        let lock = context.clone();
        async move {
            locked(&lock, move || {
                unchanged(&accesses, Self::prepare(&context, &input))?;
                let target = accesses.first().ok_or(ToolFailure::PathChanged)?;
                edit(target, &input)
            })
            .await
        }
        .boxed()
    }
}

fn edit(target: &Access, input: &Input) -> Result<Done, ToolFailure> {
    let (path, identity) = path_of(target)?;
    let path_text = path.display().to_string();
    let (folder, name) = pin_existing(&path, identity)?;
    let existing = folder.read(&name, MAX_TEXT_FILE_BYTES)?;
    let current = revision(&existing.bytes);
    if current != input.expected_revision.trim() {
        return Err(ToolFailure::RevisionMismatch { current });
    }
    let old = decode(&existing.bytes).map_err(|reason| not_text(reason, path_text.clone()))?;
    let text = apply(&old.text, &input.edits, old.crlf)?;
    let bytes = FileText { text, ..old }.encode();
    let mut draft = folder.draft(&name)?;
    draft.write(&bytes)?;
    folder.commit_replace(draft, &name, &existing)?;
    Ok(Done {
        output: format!(
            "Edited {path_text}: {} replacements.\nRevision: {}\n",
            input.edits.len(),
            revision(&bytes)
        ),
        outcome: ToolOutcome::Edited {
            path: path_text,
            replacements: count(input.edits.len()),
        },
        archives: Vec::new(),
        images: Vec::new(),
    })
}

/// Applies the edits to the text as it was: each old text must appear exactly
/// once, and no two edits may overlap. In a file whose lines end in CRLF,
/// which the model saw with LF, the edits' CRLF counts as LF.
fn apply(text: &str, edits: &[Edit], crlf: bool) -> Result<String, ToolFailure> {
    let normalize = |value: &str| {
        if crlf {
            value.replace("\r\n", "\n")
        } else {
            value.to_owned()
        }
    };
    // Each edit's range in the text, with its new text and its number.
    let mut ranges: Vec<(usize, usize, String, u32)> = Vec::new();
    for (index, edit) in edits.iter().enumerate() {
        let number = count(index + 1);
        let old_text = normalize(&edit.old_text);
        let positions = occurrences(text, &old_text);
        match positions.as_slice() {
            [] => return Err(ToolFailure::EditNotFound { edit: number }),
            [start] => ranges.push((
                *start,
                start + old_text.len(),
                normalize(&edit.new_text),
                number,
            )),
            more => {
                return Err(ToolFailure::EditAmbiguous {
                    edit: number,
                    matches: count(more.len()),
                });
            }
        }
    }
    ranges.sort_by_key(|range| range.0);
    for pair in ranges.windows(2) {
        let (before, after) = (&pair[0], &pair[1]);
        if after.0 < before.1 {
            return Err(ToolFailure::EditsOverlap {
                first: before.3.min(after.3),
                second: before.3.max(after.3),
            });
        }
    }
    let mut result = String::with_capacity(text.len());
    let mut position = 0;
    for (start, end, new_text, _) in &ranges {
        result.push_str(&text[position..*start]);
        result.push_str(new_text);
        position = *end;
    }
    result.push_str(&text[position..]);
    Ok(result)
}

/// Where a pattern starts in a text, overlapping occurrences included, so a
/// pattern that matches twice in `aaa` counts as ambiguous.
fn occurrences(text: &str, pattern: &str) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut from = 0;
    while let Some(found) = text[from..].find(pattern) {
        let start = from + found;
        positions.push(start);
        let step = text[start..].chars().next().map_or(1, char::len_utf8);
        from = start + step;
        if from > text.len() {
            break;
        }
    }
    positions
}

#[cfg(test)]
mod tests {
    use mannie_types::tools::ToolFailure;

    use super::{Edit, apply, occurrences};

    fn edit(old_text: &str, new_text: &str) -> Edit {
        Edit {
            old_text: old_text.to_owned(),
            new_text: new_text.to_owned(),
        }
    }

    #[test]
    fn edits_apply_together() {
        let text = "fn one() {}\nfn two() {}\nfn three() {}\n";

        let edited = apply(
            text,
            &[edit("fn three", "fn third"), edit("fn one", "fn first")],
            false,
        )
        .unwrap();

        assert_eq!(edited, "fn first() {}\nfn two() {}\nfn third() {}\n");
    }

    #[test]
    fn missing_ambiguous_and_overlapping_edits_fail() {
        let text = "alpha beta alpha\n";

        assert_eq!(
            apply(text, &[edit("gamma", "")], false),
            Err(ToolFailure::EditNotFound { edit: 1 })
        );
        assert_eq!(
            apply(text, &[edit("beta", "b"), edit("alpha", "a")], false),
            Err(ToolFailure::EditAmbiguous {
                edit: 2,
                matches: 2
            })
        );
        assert_eq!(
            apply(
                text,
                &[edit("alpha beta", "x"), edit("beta alpha", "y")],
                false
            ),
            Err(ToolFailure::EditsOverlap {
                first: 1,
                second: 2
            })
        );
    }

    #[test]
    fn overlapping_occurrences_count() {
        assert_eq!(occurrences("aaa", "aa"), vec![0, 1]);
        assert_eq!(occurrences("äöä", "ä"), vec![0, 4]);
    }

    #[test]
    fn crlf_in_edits_counts_as_lf_in_crlf_files() {
        let edited = apply("one\ntwo\n", &[edit("one\r\ntwo", "1\r\n2")], true).unwrap();

        assert_eq!(edited, "1\n2\n");
    }
}
