//! Skills (ADR-0061; docs/features/skills.md): folders with a `SKILL.md` that
//! the `skills` CLI installs in this computer's `~/.agents/skills` or that a
//! project holds in its `.agents/skills`. This module holds what the settings
//! section shows of them, the argument of `set_skill_on`, the check of a
//! skill's name and the errors of the feature's commands.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::ProjectId;

/// The most characters of a skill's name, as the Agent Skills specification
/// allows.
pub const MAX_SKILL_NAME_CHARS: usize = 64;

/// The most characters of a skill's description, as the Agent Skills
/// specification allows. A longer one is cut.
pub const MAX_SKILL_DESCRIPTION_CHARS: usize = 1_024;

/// Where a skill lives.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SkillPlace {
    /// `~/.agents/skills`, the folder of this computer, whose skills are on
    /// until the owner turns one off.
    Computer,
    /// The `.agents/skills` folder of a project, whose skills are off until
    /// the owner turns each one on.
    Project {
        /// The project.
        project_id: ProjectId,
    },
}

/// Something about a skill that the owner may want to fix, which does not
/// keep it from working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SkillWarning {
    /// The frontmatter's name is valid and differs from the folder's name.
    NameDiffers,
    /// The frontmatter has no valid name, so the folder's name stands in.
    NameFromFolder,
    /// The description was longer than the specification allows and is cut.
    DescriptionCut,
}

/// Why a folder of a place is not a skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SkillProblem {
    /// `SKILL.md` does not start with a line `---`.
    FrontmatterMissing,
    /// The frontmatter has no closing line within the part that Mannie reads.
    FrontmatterNotClosed,
    /// Neither the frontmatter nor the folder gives a valid name.
    NameNotValid,
    /// The frontmatter has no description.
    DescriptionMissing,
    /// Another folder of the place has a skill of the same name.
    NameTaken,
    /// The folder resolves outside its project.
    OutsideProject,
    /// The folder resolves into one of Mannie's own folders, or to a folder
    /// that holds one of them.
    InProtectedFolder,
    /// The folder resolves to a folder that holds its place.
    AroundPlace,
    /// `SKILL.md` resolves outside its skill's folder.
    OutsideSkill,
    /// `SKILL.md` is not text that Mannie reads.
    NotText,
    /// The folder or its `SKILL.md` could not be read.
    ReadFailed,
}

/// Why a place could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SkillPlaceFailure {
    /// The place's path is not a folder.
    NotAFolder,
    /// The place's folder could not be read.
    ReadFailed,
}

/// A skill as the settings section shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is a fact that the settings show on its own, in a contract that grows only by addition"
)]
pub struct SkillView {
    /// Its name.
    pub name: String,
    /// Its description, cleaned as labels are.
    pub description: String,
    /// Its folder, with its links resolved.
    pub folder: String,
    /// Whether agents see it: its switch, or for a project's skill the
    /// switch as long as its listed text has not changed.
    pub is_on: bool,
    /// Whether a project's skill that the owner turned on changed its name,
    /// its description or whether agents may see it since, which turns it off.
    pub changed: bool,
    /// Whether its author lets agents see it, which `disable-model-invocation`
    /// takes away.
    pub listed: bool,
    /// Whether its author lets the owner mention it, which `user-invocable`
    /// takes away.
    pub mentionable: bool,
    /// What the owner may want to fix.
    pub warnings: Vec<SkillWarning>,
}

/// A folder of a place that is not a skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillProblemView {
    /// The folder's name in its place.
    pub folder: String,
    /// Why it is not a skill.
    pub problem: SkillProblem,
}

/// A place with its skills, as the settings section shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillPlaceView {
    /// The place.
    pub place: SkillPlace,
    /// The project's name, or `None` for this computer.
    pub label: Option<String>,
    /// The place's folder.
    pub folder: String,
    /// Its skills, by name.
    pub skills: Vec<SkillView>,
    /// Its folders that are not skills, by name.
    pub problems: Vec<SkillProblemView>,
    /// Why the place could not be read, or `None`.
    pub failure: Option<SkillPlaceFailure>,
    /// Whether the place holds more entries or skills than Mannie reads.
    pub incomplete: bool,
    /// How many of its skills that are on agents' list leaves out, because
    /// they do not fit.
    pub left_out: u32,
}

/// What `list_skills` and `set_skill_on` return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillsOverview {
    /// This computer's place, or `None` when Mannie has no folder for it.
    pub computer: Option<SkillPlaceView>,
    /// The places of the projects that have skills or problems, in the order
    /// of the projects.
    pub projects: Vec<SkillPlaceView>,
}

/// The argument of `set_skill_on`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SetSkillOn {
    /// The skill's place.
    pub place: SkillPlace,
    /// The skill's name.
    pub name: String,
    /// Whether agents see it from now on.
    pub on: bool,
}

/// Whether `name` can name a skill, as the Agent Skills specification says:
/// 1 to 64 lower-case letters, digits and hyphens, without a hyphen at either
/// end or two hyphens in a row.
pub fn is_skill_name(name: &str) -> bool {
    let within_limit = !name.is_empty() && name.len() <= MAX_SKILL_NAME_CHARS;
    let characters_are_valid = name.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    });
    within_limit
        && characters_are_valid
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
}

/// What is wrong with an argument of `set_skill_on`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SkillField {
    /// The name is not a skill's name.
    Name,
}

/// Why a command of skills failed. The UI shows the message for the code from
/// its catalog (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SkillError {
    /// The place holds no skill of this name.
    #[error("the skill does not exist")]
    NotFound,
    /// The project does not exist.
    #[error("the project does not exist")]
    ProjectNotFound,
    /// An argument is not valid.
    #[error("an argument of the skill is not valid")]
    FieldNotValid {
        /// The argument.
        field: SkillField,
    },
    /// The store failed.
    #[error("the skills' switches could not be read or saved")]
    StorageFailed,
    /// The store is not available.
    #[error("the store is not available")]
    StoreUnavailable,
}

impl SkillError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "skills.not_found",
            Self::ProjectNotFound => "skills.project_not_found",
            Self::FieldNotValid { .. } => "skills.field_not_valid",
            Self::StorageFailed => "skills.storage_failed",
            Self::StoreUnavailable => "skills.store_unavailable",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skill_name_follows_the_specification() {
        for name in [
            "pdf",
            "pdf-forms",
            "a",
            "web-2",
            &"a".repeat(MAX_SKILL_NAME_CHARS),
        ] {
            assert!(is_skill_name(name), "{name}");
        }
        let too_long = "a".repeat(MAX_SKILL_NAME_CHARS + 1);
        for name in [
            "",
            "PDF",
            "pdf_forms",
            "-pdf",
            "pdf-",
            "pdf--forms",
            "pdf forms",
            "pdf/forms",
            "pdé",
            too_long.as_str(),
        ] {
            assert!(!is_skill_name(name), "{name}");
        }
    }

    #[test]
    fn a_place_serializes_with_its_kind() {
        let project = ProjectId::parse("0199e0f2-0000-7000-8000-000000000002").unwrap();

        assert_eq!(
            serde_json::to_value(SkillPlace::Computer).unwrap(),
            serde_json::json!({ "kind": "computer" })
        );
        assert_eq!(
            serde_json::to_value(SkillPlace::Project {
                project_id: project.clone()
            })
            .unwrap(),
            serde_json::json!({ "kind": "project", "projectId": project.to_string() })
        );
    }

    #[test]
    fn every_error_has_a_code_of_the_feature() {
        for error in [
            SkillError::NotFound,
            SkillError::ProjectNotFound,
            SkillError::FieldNotValid {
                field: SkillField::Name,
            },
            SkillError::StorageFailed,
            SkillError::StoreUnavailable,
        ] {
            assert!(error.code().starts_with("skills."), "{error:?}");
        }
    }
}
