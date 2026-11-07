//! Skills (ADR-0061; docs/features/skills.md): folders with a `SKILL.md` that
//! the `skills` CLI installs in this computer's `~/.agents/skills` or that a
//! project holds in its `.agents/skills`.
//!
//! [`scan_place`] finds the skills of one place, with the refusals that keep
//! a skill's folder where it belongs, and reads each frontmatter with a
//! tolerant reader. The runtime keeps the owner's switches and gives each
//! call a [`SkillShelf`]: the skills that the call can read, whose folders
//! that are on the policy allows reads in (decision 8). `read_skill` reads
//! from it. Mannie never writes into a skill's folder and never runs anything
//! that it holds.

mod files;
mod frontmatter;
mod scan;

use std::path::{Path, PathBuf};

pub use files::{SkillInstructions, read_skill_instructions};
pub(crate) use files::{echo, file_text, skill_file};
pub(crate) use frontmatter::read_frontmatter;
pub use scan::{
    FoundSkill, MAX_PLACE_ENTRIES, MAX_PLACE_SKILLS, MAX_SKILL_HEAD_BYTES, PlaceBounds, PlaceScan,
    scan_place,
};

/// The folder of the skills of a place, below its base: `~/.agents/skills`
/// for this computer, and `<project>/.agents/skills` for a project. The
/// `skills` CLI installs there with `--agent universal`.
pub fn skills_folder_in(base: &Path) -> PathBuf {
    base.join(".agents").join("skills")
}

/// A skill that a call can read: its name and its canonical folder, and
/// whether it is on, so the policy allows reads in its folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShelvedSkill {
    /// Its name, as `read_skill` takes it.
    pub name: String,
    /// Its folder, in canonical form.
    pub folder: PathBuf,
    /// Whether it is on for the call's places.
    pub is_on: bool,
}

/// The skills that one call can read (ADR-0061, decision 7), which the
/// runtime gives each call's surroundings. The runtime puts the skills that
/// are on first, a project's before this computer's, and then the project's
/// skills that are off, so a name finds the skill that agents' list shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillShelf {
    skills: Vec<ShelvedSkill>,
}

impl SkillShelf {
    /// A shelf of these skills. The first skill of a name wins.
    pub fn new(skills: Vec<ShelvedSkill>) -> Self {
        Self { skills }
    }

    /// The skill of this name, or `None`.
    pub fn find(&self, name: &str) -> Option<&ShelvedSkill> {
        self.skills.iter().find(|skill| skill.name == name)
    }

    /// The folders of the skills that are on, which the policy allows reads
    /// in.
    pub fn on_folders(&self) -> Vec<PathBuf> {
        self.skills
            .iter()
            .filter(|skill| skill.is_on)
            .map(|skill| skill.folder.clone())
            .collect()
    }

    /// Whether some skill on the shelf is on, which offers `read_skill`.
    pub fn has_skills_on(&self) -> bool {
        self.skills.iter().any(|skill| skill.is_on)
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{ShelvedSkill, SkillShelf, skills_folder_in};

    fn shelved(name: &str, folder: &str, is_on: bool) -> ShelvedSkill {
        ShelvedSkill {
            name: name.to_owned(),
            folder: PathBuf::from(folder),
            is_on,
        }
    }

    #[test]
    fn a_shelf_finds_the_first_skill_of_a_name_and_lists_the_folders_that_are_on() {
        let shelf = SkillShelf::new(vec![
            shelved("pdf", "/home/.agents/skills/pdf", true),
            shelved("docx", "/home/.agents/skills/docx", true),
            shelved("pdf", "/work/.agents/skills/pdf", false),
        ]);

        assert_eq!(
            shelf.find("pdf").map(|skill| skill.folder.as_path()),
            Some(Path::new("/home/.agents/skills/pdf"))
        );
        assert_eq!(shelf.find("xlsx"), None);
        assert_eq!(
            shelf.on_folders(),
            [
                PathBuf::from("/home/.agents/skills/pdf"),
                PathBuf::from("/home/.agents/skills/docx")
            ]
        );
        assert!(shelf.has_skills_on());
        assert!(!SkillShelf::default().has_skills_on());
    }

    #[test]
    fn the_folder_of_a_place_is_the_cli_s() {
        assert_eq!(
            skills_folder_in(Path::new("/home/owner")),
            Path::new("/home/owner").join(".agents").join("skills")
        );
    }
}
