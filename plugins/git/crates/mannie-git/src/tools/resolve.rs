//! `git_resolve` (plugins/git/docs/conflicts-and-history.md, resolving):
//! resolves one conflicted file and stages it. The plugin writes no file
//! itself: a side's version is already a blob, other content becomes one
//! through `git hash-object` with the file's path, so its attributes apply,
//! the blob becomes the file's entry in the index, and `git checkout-index`
//! writes the work tree's file, with the line endings and filters that its
//! attributes name. A file that one side deleted is kept or removed with
//! `git add` or `git rm`, and a file that the agent edited is staged once no
//! conflict markers remain.

use std::fmt::Write as _;

use mannie_plugin_sdk::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::arguments::{pathspec, repository_path};
use crate::changes::{Acting, ChangeTool, Plan};
use crate::effects::Change;
use crate::failure::GitFailure;
use crate::operation::Operation;
use crate::parse::conflicts::{
    ConflictHunk, Segment, StageEntry, Stages, hunks, parse_markers, parse_unmerged,
};
use crate::parse::status::Conflict;
use crate::texts;
use crate::tools::conflicts::conflicted_files;
use crate::tools::{Answer, Reading};

/// The tool's name.
pub(crate) const GIT_RESOLVE: &str = "git_resolve";

/// The mode of a regular file, for content without a stage to take it from.
const REGULAR_FILE: &str = "100644";

/// A side of a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Side {
    Current,
    Incoming,
    Base,
    Both,
}

/// The resolution of one conflict of a file.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct HunkChoice {
    /// The conflict's number, as `git_conflicts` lists it.
    number: u32,
    /// The side to keep: current, incoming, base, or both, current first.
    /// Not with text.
    take: Option<Side>,
    /// The lines that replace the conflict. Not with take.
    text: Option<String>,
}

/// The arguments of `git_resolve`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResolveArguments {
    #[schemars(description = texts::REPOSITORY_ARGUMENT)]
    repository: Option<String>,
    /// The conflicted file, relative to the repository's top.
    path: String,
    /// One side for the whole file: current, incoming or base take that
    /// side's version of the file, and both keeps both sides of every
    /// conflict, current first.
    take: Option<Side>,
    /// A resolution for each remaining conflict of the file, by its number.
    hunks: Option<Vec<HunkChoice>>,
    /// The file's whole resolved content.
    content: Option<String>,
    /// For a file that one side deleted: true deletes it, false keeps the
    /// changed version.
    deleted: Option<bool>,
    /// True when you resolved the file with `edit_file`, to stage it once no
    /// conflict markers remain.
    edited: Option<bool>,
}

/// How a file is resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resolution {
    /// A side's version, which the index holds as a blob.
    Blob { mode: String, blob: String },
    /// Content that becomes a blob.
    Content { mode: String, content: String },
    /// The file removed.
    Delete,
    /// The work tree's file staged as it is.
    Stage,
}

/// `git_resolve`.
#[derive(Debug)]
pub(crate) struct GitResolve;

impl ChangeTool for GitResolve {
    type Arguments = ResolveArguments;
    type Planned = (String, Resolution);

    const NAME: &'static str = GIT_RESOLVE;
    const DESCRIPTION: &'static str = texts::RESOLVE_DESCRIPTION;

    fn repository(arguments: &ResolveArguments) -> Option<&str> {
        arguments.repository.as_deref()
    }

    fn plan<'a>(
        reading: &'a Reading,
        arguments: &'a ResolveArguments,
    ) -> BoxFuture<'a, Result<Plan<(String, Resolution)>, GitFailure>> {
        Box::pin(plan(reading, arguments))
    }

    fn act<'a>(
        acting: &'a Acting,
        _arguments: &'a ResolveArguments,
        plan: Plan<(String, Resolution)>,
    ) -> BoxFuture<'a, Result<Answer, GitFailure>> {
        Box::pin(act(acting, plan))
    }
}

fn invalid(detail: String) -> GitFailure {
    GitFailure::InvalidArgument { detail }
}

/// The stages of the conflicted file at `path`.
async fn stages_of(reading: &Reading, path: &str) -> Result<Stages, GitFailure> {
    let ran = reading
        .git
        .output(reading.run([
            "ls-files".to_owned(),
            "-u".to_owned(),
            "-z".to_owned(),
            "--".to_owned(),
            pathspec(path),
        ]))
        .await?;
    parse_unmerged(&ran.output)
        .into_iter()
        .find(|stages| stages.path == path)
        .ok_or_else(|| invalid(format!("{path} is not conflicted.")))
}

/// The work tree's file as text, read for its conflict markers.
fn work_tree_text(reading: &Reading, path: &str) -> Result<String, GitFailure> {
    let bytes = std::fs::read(reading.layout.top.join(path))
        .map_err(|_error| invalid(format!("{path} is not in the work tree.")))?;
    String::from_utf8(bytes).map_err(|_error| {
        invalid(format!(
            "{path} is not UTF-8 text. Take a whole side with take, or give its content."
        ))
    })
}

/// The lines that a side keeps of a conflict.
fn side_lines(hunk: &ConflictHunk, side: Side) -> Result<Vec<String>, GitFailure> {
    match side {
        Side::Current => Ok(hunk.current.clone()),
        Side::Incoming => Ok(hunk.incoming.clone()),
        Side::Both => Ok([hunk.current.clone(), hunk.incoming.clone()].concat()),
        Side::Base => hunk.base.clone().ok_or_else(|| {
            invalid(format!(
                "Conflict {} has no base section, so take current, incoming or both, or give its text.",
                hunk.number
            ))
        }),
    }
}

/// A text that replaces a conflict, ended by a line break in the style of
/// the conflict's own lines.
fn replacement(hunk: &ConflictHunk, text: &str) -> String {
    let crlf = hunk
        .current
        .iter()
        .chain(&hunk.incoming)
        .any(|line| line.ends_with("\r\n"));
    let needs_break = !text.is_empty() && !text.ends_with('\n');
    match (needs_break, crlf) {
        (true, true) => format!("{text}\r\n"),
        (true, false) => format!("{text}\n"),
        (false, _crlf) => text.to_owned(),
    }
}

/// The file's content with each conflict resolved as `resolve` says.
fn resolved_content(
    segments: &[Segment],
    resolve: impl Fn(&ConflictHunk) -> Result<String, GitFailure>,
) -> Result<String, GitFailure> {
    let mut content = String::new();
    for segment in segments {
        match segment {
            Segment::Text(text) => content.push_str(text),
            Segment::Conflict(hunk) => content.push_str(&resolve(hunk)?),
        }
    }
    Ok(content)
}

/// The content of the hunks' choices, each remaining conflict chosen once.
fn chosen_content(segments: &[Segment], choices: &[HunkChoice]) -> Result<String, GitFailure> {
    let numbers: Vec<u32> = hunks(segments).iter().map(|hunk| hunk.number).collect();
    for choice in choices {
        if !numbers.contains(&choice.number) {
            return Err(invalid(format!(
                "The file has no conflict {}.",
                choice.number
            )));
        }
        if choices
            .iter()
            .filter(|other| other.number == choice.number)
            .count()
            > 1
        {
            return Err(invalid(format!(
                "Conflict {} is chosen twice.",
                choice.number
            )));
        }
    }
    resolved_content(segments, |hunk| {
        let choice = choices
            .iter()
            .find(|choice| choice.number == hunk.number)
            .ok_or_else(|| {
                invalid(format!(
                    "Conflict {} has no choice; choose every conflict.",
                    hunk.number
                ))
            })?;
        match (&choice.take, &choice.text) {
            (Some(side), None) => Ok(side_lines(hunk, *side)?.concat()),
            (None, Some(text)) => Ok(replacement(hunk, text)),
            _both_or_none => Err(invalid(format!(
                "Conflict {} needs either take or text.",
                hunk.number
            ))),
        }
    })
}

/// A stage's entry, or the failure that names the missing side.
fn stage_entry(
    entry: Option<&StageEntry>,
    side: &str,
    path: &str,
) -> Result<StageEntry, GitFailure> {
    entry.cloned().ok_or_else(|| {
        invalid(format!(
            "{path} has no {side} version; keep or delete it with deleted."
        ))
    })
}

async fn plan(
    reading: &Reading,
    arguments: &ResolveArguments,
) -> Result<Plan<(String, Resolution)>, GitFailure> {
    let path = repository_path(&reading.layout.top, "path", &arguments.path)?;
    let chosen = [
        arguments.take.is_some(),
        arguments.hunks.is_some(),
        arguments.content.is_some(),
        arguments.deleted.is_some(),
        arguments.edited == Some(true),
    ];
    if chosen.iter().filter(|chosen| **chosen).count() != 1 {
        return Err(invalid(
            "Pass exactly one of take, hunks, content, deleted and edited.".to_owned(),
        ));
    }
    let stages = stages_of(reading, &path).await?;
    let conflict = stages.conflict();
    let mode = stages
        .current
        .as_ref()
        .or(stages.incoming.as_ref())
        .or(stages.base.as_ref())
        .map_or_else(|| REGULAR_FILE.to_owned(), |entry| entry.mode.clone());
    let resolution = if let Some(deleted) = arguments.deleted {
        let one_side_deleted = matches!(
            conflict,
            Conflict::DeletedCurrent | Conflict::DeletedIncoming | Conflict::BothDeleted
        );
        if !one_side_deleted {
            return Err(invalid(format!(
                "Neither side deleted {path}; resolve its content instead."
            )));
        }
        if deleted {
            Resolution::Delete
        } else {
            Resolution::Stage
        }
    } else if arguments.edited == Some(true) {
        let text = work_tree_text(reading, &path)?;
        let segments =
            parse_markers(&text).map_err(|error| invalid(format!("{path}: {error}.")))?;
        let left = hunks(&segments).len();
        if left > 0 {
            return Err(invalid(format!(
                "{path} still has {left} conflicts; resolve them first."
            )));
        }
        Resolution::Stage
    } else if let Some(content) = &arguments.content {
        if content.contains('\0') {
            return Err(invalid("content holds a NUL character.".to_owned()));
        }
        Resolution::Content {
            mode,
            content: content.clone(),
        }
    } else if let Some(choices) = &arguments.hunks {
        // A text may be empty, which drops the conflict's lines.
        if choices.iter().any(|choice| {
            choice
                .text
                .as_deref()
                .is_some_and(|text| text.contains('\0'))
        }) {
            return Err(invalid(
                "A conflict's text holds a NUL character.".to_owned(),
            ));
        }
        let text = work_tree_text(reading, &path)?;
        let segments =
            parse_markers(&text).map_err(|error| invalid(format!("{path}: {error}.")))?;
        Resolution::Content {
            mode,
            content: chosen_content(&segments, choices)?,
        }
    } else {
        side_resolution(
            reading,
            &stages,
            &path,
            arguments.take.unwrap_or(Side::Both),
            mode,
        )?
    };
    let command = first_run(&path, &resolution);
    Ok(Plan::new(Change::Resolve, command, (path, resolution)))
}

/// The resolution that takes one side for the whole file: the side's
/// version, or both sides of every conflict.
fn side_resolution(
    reading: &Reading,
    stages: &Stages,
    path: &str,
    side: Side,
    mode: String,
) -> Result<Resolution, GitFailure> {
    let entry = match side {
        Side::Current => stage_entry(stages.current.as_ref(), "current", path)?,
        Side::Incoming => stage_entry(stages.incoming.as_ref(), "incoming", path)?,
        Side::Base => stage_entry(stages.base.as_ref(), "base", path)?,
        Side::Both => {
            let text = work_tree_text(reading, path)?;
            let segments =
                parse_markers(&text).map_err(|error| invalid(format!("{path}: {error}.")))?;
            let content =
                resolved_content(&segments, |hunk| Ok(side_lines(hunk, Side::Both)?.concat()))?;
            return Ok(Resolution::Content { mode, content });
        }
    };
    Ok(Resolution::Blob {
        mode: entry.mode,
        blob: entry.blob,
    })
}

/// The first run of a resolution, which the owner decides on.
fn first_run(path: &str, resolution: &Resolution) -> Vec<String> {
    match resolution {
        Resolution::Blob { mode, blob } => vec![
            "update-index".to_owned(),
            "--cacheinfo".to_owned(),
            format!("{mode},{blob},{path}"),
        ],
        Resolution::Content { .. } => vec![
            "hash-object".to_owned(),
            "-w".to_owned(),
            "--stdin".to_owned(),
            format!("--path={path}"),
        ],
        Resolution::Delete => vec![
            "rm".to_owned(),
            "--quiet".to_owned(),
            "--".to_owned(),
            pathspec(path),
        ],
        Resolution::Stage => vec!["add".to_owned(), "--".to_owned(), pathspec(path)],
    }
}

async fn act(acting: &Acting, plan: Plan<(String, Resolution)>) -> Result<Answer, GitFailure> {
    let reading = &acting.reading;
    let git = &reading.git;
    let (path, resolution) = plan.planned;
    let how = match &resolution {
        Resolution::Blob { .. } => {
            git.output(acting.run(plan.command)).await?;
            "took one side's version".to_owned()
        }
        Resolution::Content { mode, content } => {
            let hashed = git
                .output(
                    acting
                        .run(plan.command)
                        .with_input(content.clone().into_bytes()),
                )
                .await?;
            let blob = hashed.text().trim().to_owned();
            git.output(acting.run([
                "update-index".to_owned(),
                "--cacheinfo".to_owned(),
                format!("{mode},{blob},{path}"),
            ]))
            .await?;
            "wrote the resolved content".to_owned()
        }
        Resolution::Delete => {
            git.output(acting.run(plan.command)).await?;
            "deleted it".to_owned()
        }
        Resolution::Stage => {
            git.output(acting.run(plan.command)).await?;
            "staged it".to_owned()
        }
    };
    if matches!(
        resolution,
        Resolution::Blob { .. } | Resolution::Content { .. }
    ) {
        // The work tree's file follows the index's entry.
        git.output(acting.run(["checkout-index", "-f", "--", path.as_str()]))
            .await?;
    }
    let left = conflicted_files(reading).await?;
    let mut text = format!("Resolved {path}: {how}, and staged it.\n");
    if left.is_empty() {
        if Operation::read(&reading.layout.git_dir).is_some() {
            text.push_str("No file is conflicted now; call git_operation with continue.\n");
        } else {
            text.push_str("No file is conflicted now.\n");
        }
    } else {
        let _written = writeln!(text, "{} conflicted files remain.", left.len());
    }
    Ok(Answer { text, data: None })
}

#[cfg(test)]
mod tests {
    use super::{HunkChoice, Side, chosen_content, replacement};
    use crate::parse::conflicts::{hunks, parse_markers};

    const FILE: &str = "start\n<<<<<<< HEAD\nours\n||||||| base\nold\n=======\ntheirs\n>>>>>>> feature\nmiddle\n<<<<<<< HEAD\na\r\n=======\nb\r\n>>>>>>> feature\nend\n";

    #[test]
    fn each_conflict_takes_its_choice_and_the_rest_stays() {
        let segments = parse_markers(FILE).unwrap();
        let choices = [
            HunkChoice {
                number: 1,
                take: Some(Side::Both),
                text: None,
            },
            HunkChoice {
                number: 2,
                take: None,
                text: Some("c".to_owned()),
            },
        ];

        let content = chosen_content(&segments, &choices).unwrap();

        assert_eq!(content, "start\nours\ntheirs\nmiddle\nc\r\nend\n");
    }

    #[test]
    fn every_conflict_needs_one_choice() {
        let segments = parse_markers(FILE).unwrap();
        let one = [HunkChoice {
            number: 1,
            take: Some(Side::Base),
            text: None,
        }];
        let unknown = [HunkChoice {
            number: 3,
            take: Some(Side::Current),
            text: None,
        }];

        assert!(
            chosen_content(&segments, &one)
                .unwrap_err()
                .text()
                .contains("Conflict 2 has no choice")
        );
        assert!(
            chosen_content(&segments, &unknown)
                .unwrap_err()
                .text()
                .contains("no conflict 3")
        );
        let found = hunks(&segments);
        assert_eq!(replacement(found[1], "x"), "x\r\n");
        assert_eq!(replacement(found[0], "x\n"), "x\n");
        assert_eq!(replacement(found[0], ""), "");
    }
}
