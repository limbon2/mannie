//! Finding the skills of one place (ADR-0061, decisions 2 to 4;
//! docs/features/skills.md, places and the frontmatter).
//!
//! A skill is a direct subfolder of the place that holds a `SKILL.md`. The
//! scan is bounded, follows links only to check where they lead, and refuses
//! a folder that resolves where a skill does not belong: outside its project,
//! in or around one of Mannie's own folders, or around its place.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use mannie_types::labels::clean_label;
use mannie_types::skills::{
    MAX_SKILL_DESCRIPTION_CHARS, SkillPlaceFailure, SkillProblem, SkillProblemView, SkillWarning,
    is_skill_name,
};

use super::frontmatter::{FrontmatterError, flag, read_frontmatter};
use crate::digest::skill_digest;
use crate::paths::{FinalLink, Resolved, is_inside, resolve};
use crate::text::decode;

/// The most entries of a place that a scan looks at.
pub const MAX_PLACE_ENTRIES: usize = 256;

/// The most skills of a place that a scan keeps.
pub const MAX_PLACE_SKILLS: usize = 100;

/// The most bytes at the start of a `SKILL.md` that a scan reads, which must
/// hold the frontmatter.
pub const MAX_SKILL_HEAD_BYTES: usize = 64 * 1024;

/// The name of a skill's instructions.
pub(crate) const SKILL_FILE: &str = "SKILL.md";

/// A skill that a scan found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundSkill {
    /// Its name.
    pub name: String,
    /// Its description, cleaned as labels are and cut to the specification's
    /// limit.
    pub description: String,
    /// Its folder, in canonical form.
    pub folder: PathBuf,
    /// Whether its author lets agents see it, which `disable-model-invocation`
    /// takes away.
    pub listed: bool,
    /// Whether its author lets the owner mention it, which `user-invocable`
    /// takes away.
    pub mentionable: bool,
    /// What the owner may want to fix.
    pub warnings: Vec<SkillWarning>,
}

impl FoundSkill {
    /// The digest of what agents' list shows of the skill (ADR-0061, decision
    /// 5).
    pub fn digest(&self) -> String {
        skill_digest(&self.name, &self.description, self.listed)
    }
}

/// What a scan of a place found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaceScan {
    /// The place's folder in canonical form, or `None` when it does not exist
    /// or could not be read.
    pub folder: Option<PathBuf>,
    /// Its skills, by name.
    pub skills: Vec<FoundSkill>,
    /// Its folders that are not skills, by name.
    pub problems: Vec<SkillProblemView>,
    /// Why the place could not be read, or `None`. A place that does not
    /// exist has no skills and no failure.
    pub failure: Option<SkillPlaceFailure>,
    /// Whether the place holds more entries or skills than the scan reads.
    pub incomplete: bool,
}

/// Where a place lies, which decides what the scan refuses.
#[derive(Debug, Clone, Copy)]
pub struct PlaceBounds<'a> {
    /// The canonical folder of the place's project, inside which every skill
    /// of the place must resolve, or `None` for this computer's place.
    pub workspace: Option<&'a Path>,
    /// Mannie's own folders, in canonical form.
    pub protected: &'a [PathBuf],
}

/// Finds the skills of the place at `folder`. It reads the file system and
/// changes nothing, so the runtime runs it on a blocking thread.
pub fn scan_place(folder: &Path, bounds: &PlaceBounds<'_>) -> PlaceScan {
    let place = match resolve(folder, FinalLink::Follow) {
        Ok(resolved) if !resolved.exists() => return PlaceScan::default(),
        Ok(resolved) if !is_folder(&resolved) => {
            return failed(SkillPlaceFailure::NotAFolder);
        }
        Ok(resolved) => resolved.path,
        Err(_failure) => return failed(SkillPlaceFailure::ReadFailed),
    };
    let Ok(entries) = fs::read_dir(&place) else {
        return failed(SkillPlaceFailure::ReadFailed);
    };
    let mut names = Vec::new();
    let mut incomplete = false;
    for entry in entries {
        let Ok(entry) = entry else { continue };
        if names.len() == MAX_PLACE_ENTRIES {
            incomplete = true;
            break;
        }
        // A name that is not Unicode cannot name a skill or show in the
        // settings; names that start with a dot are hidden.
        if let Some(name) = entry.file_name().to_str()
            && !name.starts_with('.')
        {
            names.push(name.to_owned());
        }
    }
    names.sort();
    let mut scan = PlaceScan {
        folder: Some(place.clone()),
        incomplete,
        ..PlaceScan::default()
    };
    for entry_name in names {
        if scan.skills.len() == MAX_PLACE_SKILLS {
            scan.incomplete = true;
            break;
        }
        match read_entry(&place, &entry_name, bounds) {
            Entry::NotASkill => {}
            Entry::Problem(problem) => scan.problems.push(SkillProblemView {
                folder: entry_name,
                problem,
            }),
            Entry::Skill(skill) => {
                if scan.skills.iter().any(|found| found.name == skill.name) {
                    scan.problems.push(SkillProblemView {
                        folder: entry_name,
                        problem: SkillProblem::NameTaken,
                    });
                } else {
                    scan.skills.push(skill);
                }
            }
        }
    }
    scan.skills
        .sort_by(|left, right| left.name.cmp(&right.name));
    scan
}

/// A scan of a place that could not be read.
fn failed(failure: SkillPlaceFailure) -> PlaceScan {
    PlaceScan {
        failure: Some(failure),
        ..PlaceScan::default()
    }
}

/// Whether a resolved path is a folder. A resolution that followed its links
/// describes what they lead to.
fn is_folder(resolved: &Resolved) -> bool {
    resolved.metadata.as_ref().is_some_and(fs::Metadata::is_dir)
}

/// What one entry of a place is.
enum Entry {
    /// Not a folder, or a folder without `SKILL.md`, which the settings do not
    /// list.
    NotASkill,
    /// A folder with `SKILL.md` that is not a skill, and why.
    Problem(SkillProblem),
    /// A skill.
    Skill(FoundSkill),
}

impl From<SkillProblem> for Entry {
    fn from(problem: SkillProblem) -> Self {
        Self::Problem(problem)
    }
}

/// Reads one entry of a place.
fn read_entry(place: &Path, entry_name: &str, bounds: &PlaceBounds<'_>) -> Entry {
    let Ok(resolved) = resolve(&place.join(entry_name), FinalLink::Follow) else {
        return SkillProblem::ReadFailed.into();
    };
    if !is_folder(&resolved) {
        return Entry::NotASkill;
    }
    let folder = resolved.path;
    let Ok(skill_file) = resolve(&folder.join(SKILL_FILE), FinalLink::Follow) else {
        return SkillProblem::ReadFailed.into();
    };
    if !skill_file.exists() {
        return Entry::NotASkill;
    }
    if let Some(problem) = misplaced(place, &folder, bounds) {
        return problem.into();
    }
    if !is_inside(&skill_file.path, &folder) {
        return SkillProblem::OutsideSkill.into();
    }
    let is_file = skill_file
        .metadata
        .as_ref()
        .is_some_and(fs::Metadata::is_file);
    if !is_file {
        return SkillProblem::ReadFailed.into();
    }
    let text = match read_head(&skill_file.path) {
        Ok(text) => text,
        Err(problem) => return problem.into(),
    };
    match skill_of(&text, entry_name, folder) {
        Ok(skill) => Entry::Skill(skill),
        Err(problem) => problem.into(),
    }
}

/// Why a skill's canonical folder does not belong where it resolves, or
/// `None` when it does.
fn misplaced(place: &Path, folder: &Path, bounds: &PlaceBounds<'_>) -> Option<SkillProblem> {
    let in_or_around_protected = bounds
        .protected
        .iter()
        .any(|protected| is_inside(folder, protected) || is_inside(protected, folder));
    if in_or_around_protected {
        return Some(SkillProblem::InProtectedFolder);
    }
    if is_inside(place, folder) {
        return Some(SkillProblem::AroundPlace);
    }
    let outside_project = bounds
        .workspace
        .is_some_and(|workspace| !is_inside(folder, workspace));
    outside_project.then_some(SkillProblem::OutsideProject)
}

/// The text at the start of a skill's `SKILL.md`, at most
/// [`MAX_SKILL_HEAD_BYTES`], cut after its last whole line when the file is
/// longer.
fn read_head(path: &Path) -> Result<String, SkillProblem> {
    let file = File::open(path).map_err(|_error| SkillProblem::ReadFailed)?;
    let mut bytes = Vec::new();
    let limit = u64::try_from(MAX_SKILL_HEAD_BYTES + 1).unwrap_or(u64::MAX);
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|_error| SkillProblem::ReadFailed)?;
    if bytes.len() > MAX_SKILL_HEAD_BYTES {
        bytes.truncate(MAX_SKILL_HEAD_BYTES);
        let whole_lines = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |position| position + 1);
        bytes.truncate(whole_lines);
    }
    decode(&bytes)
        .map(|text| text.text)
        .map_err(|_not_text| SkillProblem::NotText)
}

/// The skill that a `SKILL.md`'s text describes, in the folder named
/// `entry_name`, or why it is not one.
fn skill_of(text: &str, entry_name: &str, folder: PathBuf) -> Result<FoundSkill, SkillProblem> {
    let frontmatter = read_frontmatter(text).map_err(|error| match error {
        FrontmatterError::Missing => SkillProblem::FrontmatterMissing,
        FrontmatterError::NotClosed => SkillProblem::FrontmatterNotClosed,
    })?;
    let mut warnings = Vec::new();
    let named = frontmatter.get("name").map(str::trim);
    let name = match named {
        Some(name) if is_skill_name(name) => {
            if name != entry_name {
                warnings.push(SkillWarning::NameDiffers);
            }
            name.to_owned()
        }
        _ if is_skill_name(entry_name) => {
            warnings.push(SkillWarning::NameFromFolder);
            entry_name.to_owned()
        }
        _ => return Err(SkillProblem::NameNotValid),
    };
    let full = frontmatter
        .get("description")
        .and_then(|description| clean_label(description, usize::MAX))
        .ok_or(SkillProblem::DescriptionMissing)?;
    let description = if full.chars().count() > MAX_SKILL_DESCRIPTION_CHARS {
        warnings.push(SkillWarning::DescriptionCut);
        clean_label(&full, MAX_SKILL_DESCRIPTION_CHARS).unwrap_or_default()
    } else {
        full
    };
    let hidden_from_agents =
        frontmatter.get("disable-model-invocation").and_then(flag) == Some(true);
    let hidden_from_owner = frontmatter.get("user-invocable").and_then(flag) == Some(false);
    Ok(FoundSkill {
        name,
        description,
        folder,
        listed: !hidden_from_agents,
        mentionable: !hidden_from_owner,
        warnings,
    })
}

#[cfg(test)]
mod tests;
