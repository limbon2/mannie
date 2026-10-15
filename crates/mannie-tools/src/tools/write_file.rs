//! `write_file`: creates a text file, or replaces one whose revision the
//! model names.

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::tools::{Access, AccessMode, ToolFailure, ToolOutcome};
use schemars::JsonSchema;
use serde::Deserialize;

use super::change::{
    create_access, create_folders, locked, missing_folders, path_of, pin_existing, pin_missing,
    refuse_read_only, unchanged,
};
use super::read_file::{MAX_TEXT_FILE_BYTES, not_text, too_large};
use super::{existing_file, path_access};
use crate::digest::revision;
use crate::paths::{FinalLink, for_change};
use crate::results::count;
use crate::text::{FileText, decode};
use crate::texts;
use crate::tool::{Done, PermissionClass, PrepareError, ToolContext, TypedExecution, TypedTool};

/// The largest content that `write_file` writes.
const MAX_CONTENT_BYTES: usize = 1024 * 1024;

/// The arguments of `write_file`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// The file, absolute or relative to the workspace. Missing folders above
    /// a new file are created.
    path: String,
    /// The whole content of the file, at most 1 MiB.
    content: String,
    /// The revision from your latest `read_file` of this file, to replace it.
    /// Null to create a new file, which fails if the file exists.
    #[serde(default)]
    expected_revision: Option<String>,
}

/// The tool.
pub(crate) struct WriteFile;

impl TypedTool for WriteFile {
    type Input = Input;
    const NAME: &'static str = "write_file";
    const CLASS: PermissionClass = PermissionClass::SideEffects;

    fn description() -> &'static str {
        texts::WRITE_FILE
    }

    fn prepare(context: &ToolContext, input: &Input) -> Result<Vec<Access>, PrepareError> {
        if input.content.len() > MAX_CONTENT_BYTES {
            return Err(ToolFailure::TooLarge {
                limit_bytes: u32::try_from(MAX_CONTENT_BYTES).unwrap_or(u32::MAX),
            }
            .into());
        }
        let target = for_change(context.workspace.as_deref(), &input.path, FinalLink::Follow)?;
        if input.expected_revision.is_some() {
            let metadata = existing_file(&target)?;
            if metadata.len() > MAX_TEXT_FILE_BYTES {
                return Err(too_large().into());
            }
            refuse_read_only(&target.path, metadata)?;
            return Ok(vec![path_access(&target, AccessMode::Write, false)?]);
        }
        if target.exists() {
            return Err(ToolFailure::AlreadyExists {
                path: target.path.display().to_string(),
            }
            .into());
        }
        let mut accesses = missing_folders(&target.path)?
            .iter()
            .map(|folder| create_access(folder, true))
            .collect::<Result<Vec<_>, _>>()?;
        accesses.push(create_access(&target.path, false)?);
        Ok(accesses)
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
                let target = accesses.last().ok_or(ToolFailure::PathChanged)?;
                match &input.expected_revision {
                    Some(expected) => replace(target, &input.content, expected),
                    None => create(target, input.content),
                }
            })
            .await
        }
        .boxed()
    }
}

/// Creates the file and the folders above it that are missing.
fn create(target: &Access, content: String) -> Result<Done, ToolFailure> {
    let (path, identity) = path_of(target)?;
    let (folder, names) = pin_missing(&path, identity)?;
    let (folder, _) = create_folders(folder, &names)?;
    let name = names.last().ok_or(ToolFailure::PathChanged)?;
    let bytes = FileText::new_file(content).encode();
    let mut draft = folder.draft(name)?;
    draft.write(&bytes)?;
    folder.commit_new(draft, name)?;
    Ok(written(&path.display().to_string(), true, &bytes))
}

/// Replaces the file when its revision still matches, keeping its encoding,
/// byte order mark and line endings.
fn replace(target: &Access, content: &str, expected: &str) -> Result<Done, ToolFailure> {
    let (path, identity) = path_of(target)?;
    let path_text = path.display().to_string();
    let (folder, name) = pin_existing(&path, identity)?;
    let existing = folder.read(&name, MAX_TEXT_FILE_BYTES)?;
    let current = revision(&existing.bytes);
    if current != expected.trim() {
        return Err(ToolFailure::RevisionMismatch { current });
    }
    let old = decode(&existing.bytes).map_err(|reason| not_text(reason, path_text.clone()))?;
    let bytes = FileText {
        text: content.to_owned(),
        ..old
    }
    .encode();
    let mut draft = folder.draft(&name)?;
    draft.write(&bytes)?;
    folder.commit_replace(draft, &name, &existing)?;
    Ok(written(&path_text, false, &bytes))
}

/// The result of a write, with the file's new revision.
fn written(path: &str, created: bool, bytes: &[u8]) -> Done {
    let verb = if created { "Created" } else { "Replaced" };
    Done {
        output: format!(
            "{verb} {path} ({} bytes).\nRevision: {}\n",
            bytes.len(),
            revision(bytes)
        ),
        outcome: ToolOutcome::Written {
            path: path.to_owned(),
            created,
            bytes: count(bytes.len()),
        },
        archives: Vec::new(),
    }
}
