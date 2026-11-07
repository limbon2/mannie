//! The files of a skill that `read_skill` and the mentions of skills read
//! (ADR-0061, decision 7): `SKILL.md`, or a path inside the skill's canonical
//! folder that neither leaves it, by its text or through a link, nor names a
//! file that may hold secrets.

use std::fs;
use std::path::{Component, Path};

use mannie_types::tools::{SkillFileRefusal, ToolFailure};

use super::frontmatter::read_frontmatter;
use super::scan::SKILL_FILE;
use crate::mentions::may_hold_secrets;
use crate::paths::{FinalLink, Resolved, check_text, io_failure, is_inside, resolve};
use crate::text::{FileText, decode};
use crate::tools::read_file::{MAX_TEXT_FILE_BYTES, not_text, too_large};

/// The most characters of a name or a path that a failure repeats.
const MAX_ECHO_CHARS: usize = 200;

/// A text that the model wrote, cut for a failure that repeats it.
pub(crate) fn echo(text: &str) -> String {
    text.chars().take(MAX_ECHO_CHARS).collect()
}

/// The refusal of a path of a skill.
fn refused(path: &str, reason: SkillFileRefusal) -> ToolFailure {
    ToolFailure::SkillFileRefused {
        path: echo(path),
        reason,
    }
}

/// The file of the skill in `folder`, which is canonical: `SKILL.md` for
/// `None`, or the path relative to the folder. The file exists, is a file and
/// is not larger than the text tools read.
pub(crate) fn skill_file(folder: &Path, relative: Option<&str>) -> Result<Resolved, ToolFailure> {
    let text = relative.unwrap_or(SKILL_FILE);
    check_text(text).map_err(|reason| ToolFailure::PathRefused { reason })?;
    let path = Path::new(text);
    let stays_inside = path
        .components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir));
    if !stays_inside {
        return Err(refused(text, SkillFileRefusal::OutsideSkill));
    }
    if may_hold_secrets(&text.replace('\\', "/")) {
        return Err(refused(text, SkillFileRefusal::MayHoldSecrets));
    }
    let resolved = resolve(&folder.join(path), FinalLink::Follow)?;
    // A link inside the skill may lead out of it, or to a file whose name may
    // hold secrets, so the canonical place is checked again.
    if !is_inside(&resolved.path, folder) {
        return Err(refused(text, SkillFileRefusal::OutsideSkill));
    }
    let canonical_relative = resolved.path.strip_prefix(folder).map_or_else(
        |_differs_in_case| text.replace('\\', "/"),
        |inside| inside.to_string_lossy().replace('\\', "/"),
    );
    if may_hold_secrets(&canonical_relative) {
        return Err(refused(text, SkillFileRefusal::MayHoldSecrets));
    }
    let Some(metadata) = &resolved.metadata else {
        return Err(ToolFailure::NotFound {
            path: resolved.path.display().to_string(),
        });
    };
    if !metadata.is_file() {
        return Err(ToolFailure::NotAFile {
            path: resolved.path.display().to_string(),
        });
    }
    if metadata.len() > MAX_TEXT_FILE_BYTES {
        return Err(too_large());
    }
    Ok(resolved)
}

/// The text of a file of a skill.
pub(crate) fn file_text(resolved: &Resolved) -> Result<(Vec<u8>, FileText), ToolFailure> {
    let path = resolved.text()?;
    let bytes = fs::read(&resolved.path).map_err(|error| io_failure(&error, &resolved.path))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_TEXT_FILE_BYTES {
        return Err(too_large());
    }
    let text = decode(&bytes).map_err(|reason| not_text(reason, path))?;
    Ok((bytes, text))
}

/// A skill's instructions: the text of its `SKILL.md` after the frontmatter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInstructions {
    /// The canonical path of `SKILL.md`.
    pub path: String,
    /// The instructions, without the frontmatter.
    pub body: String,
}

/// Reads the instructions of the skill in the canonical `folder`, as a
/// mention gives them to the model. A `SKILL.md` without a frontmatter that
/// Mannie reads gives its whole text.
pub fn read_skill_instructions(folder: &Path) -> Result<SkillInstructions, ToolFailure> {
    let resolved = skill_file(folder, None)?;
    let (_bytes, text) = file_text(&resolved)?;
    let body = match read_frontmatter(&text.text) {
        Ok(frontmatter) => frontmatter.body(&text.text).to_owned(),
        Err(_missing) => text.text.clone(),
    };
    Ok(SkillInstructions {
        path: resolved.text()?,
        body,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use mannie_types::tools::{SkillFileRefusal, ToolFailure};
    use tempfile::TempDir;

    use super::{read_skill_instructions, skill_file};
    use crate::paths::{FinalLink, resolve};

    fn skill_folder(temporary: &TempDir) -> PathBuf {
        let root = resolve(temporary.path(), FinalLink::Follow).unwrap().path;
        let folder = root.join("pdf");
        fs::create_dir_all(folder.join("references")).unwrap();
        fs::write(
            folder.join("SKILL.md"),
            "---\nname: pdf\ndescription: Forms.\n---\n# Forms\nRead references/forms.md.\n",
        )
        .unwrap();
        fs::write(folder.join("references").join("forms.md"), "Fields.\n").unwrap();
        fs::write(folder.join(".env"), "TOKEN=planted").unwrap();
        fs::write(root.join("outside.md"), "Outside.\n").unwrap();
        folder
    }

    fn refusal(path: &str, reason: SkillFileRefusal) -> ToolFailure {
        ToolFailure::SkillFileRefused {
            path: path.to_owned(),
            reason,
        }
    }

    #[test]
    fn a_skill_s_files_resolve_inside_its_folder() {
        let temporary = tempfile::tempdir().unwrap();
        let folder = skill_folder(&temporary);

        let instructions = skill_file(&folder, None).unwrap();
        let reference = skill_file(&folder, Some("references/forms.md")).unwrap();
        let with_dot = skill_file(&folder, Some("./references/forms.md")).unwrap();

        assert_eq!(instructions.path, folder.join("SKILL.md"));
        assert_eq!(reference.path, folder.join("references").join("forms.md"));
        assert_eq!(with_dot.path, reference.path);
    }

    #[test]
    fn paths_that_leave_the_skill_or_may_hold_secrets_are_refused() {
        let temporary = tempfile::tempdir().unwrap();
        let folder = skill_folder(&temporary);
        let absolute = folder.join("SKILL.md").display().to_string();

        assert_eq!(
            skill_file(&folder, Some("../outside.md")).unwrap_err(),
            refusal("../outside.md", SkillFileRefusal::OutsideSkill)
        );
        assert_eq!(
            skill_file(&folder, Some("references/../SKILL.md")).unwrap_err(),
            refusal("references/../SKILL.md", SkillFileRefusal::OutsideSkill)
        );
        assert_eq!(
            skill_file(&folder, Some(&absolute)).unwrap_err(),
            refusal(&absolute, SkillFileRefusal::OutsideSkill)
        );
        assert_eq!(
            skill_file(&folder, Some(".env")).unwrap_err(),
            refusal(".env", SkillFileRefusal::MayHoldSecrets)
        );
        assert!(matches!(
            skill_file(&folder, Some("missing.md")),
            Err(ToolFailure::NotFound { .. })
        ));
        assert!(matches!(
            skill_file(&folder, Some("references")),
            Err(ToolFailure::NotAFile { .. })
        ));
    }

    #[test]
    fn a_link_inside_the_skill_that_leads_out_of_it_is_refused() {
        let temporary = tempfile::tempdir().unwrap();
        let folder = skill_folder(&temporary);
        let outside = folder.parent().unwrap().join("elsewhere");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("notes.md"), "Elsewhere.\n").unwrap();
        link_folder(&folder.join("linked"), &outside);

        assert_eq!(
            skill_file(&folder, Some("linked/notes.md")).unwrap_err(),
            refusal("linked/notes.md", SkillFileRefusal::OutsideSkill)
        );
    }

    #[test]
    fn a_skill_s_instructions_come_without_their_frontmatter() {
        let temporary = tempfile::tempdir().unwrap();
        let folder = skill_folder(&temporary);

        let instructions = read_skill_instructions(&folder).unwrap();

        assert_eq!(instructions.body, "# Forms\nRead references/forms.md.\n");
        assert!(Path::new(&instructions.path).ends_with("SKILL.md"));
    }

    /// Links a folder: a symbolic link on Unix, a junction on Windows.
    fn link_folder(link: &Path, target: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        {
            let status = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .stdout(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(status.success(), "mklink /J failed");
        }
    }
}
