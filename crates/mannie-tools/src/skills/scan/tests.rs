use std::fs;
use std::path::{Path, PathBuf};

use mannie_types::skills::{SkillPlaceFailure, SkillProblem, SkillProblemView, SkillWarning};
use tempfile::TempDir;

use super::{MAX_PLACE_SKILLS, MAX_SKILL_HEAD_BYTES, PlaceBounds, scan_place};
use crate::paths::{FinalLink, resolve};

/// A temporary folder and its canonical path.
struct Folder {
    _temporary: TempDir,
    root: PathBuf,
}

impl Folder {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = resolve(temporary.path(), FinalLink::Follow).unwrap().path;
        Self {
            _temporary: temporary,
            root,
        }
    }

    fn path(&self, relative: &str) -> PathBuf {
        let mut path = self.root.clone();
        for name in relative.split('/') {
            path.push(name);
        }
        path
    }

    fn folder(&self, relative: &str) -> PathBuf {
        let path = self.path(relative);
        fs::create_dir_all(&path).unwrap();
        path
    }

    /// A folder holding a `SKILL.md` with this text.
    fn skill(&self, relative: &str, text: &str) -> PathBuf {
        let folder = self.folder(relative);
        fs::write(folder.join("SKILL.md"), text).unwrap();
        folder
    }
}

/// Links a folder: a symbolic link on Unix, a junction on Windows, which needs
/// no privilege.
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

fn skill_text(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n")
}

/// The bounds of this computer's place, with Mannie's data in `data`.
fn computer(folder: &Folder) -> Vec<PathBuf> {
    vec![folder.folder("data/mannie")]
}

fn problem(folder: &str, problem: SkillProblem) -> SkillProblemView {
    SkillProblemView {
        folder: folder.to_owned(),
        problem,
    }
}

#[test]
fn a_place_lists_its_skills_by_name_with_what_their_authors_allow() {
    let folder = Folder::new();
    let place = folder.folder("home/.agents/skills");
    let pdf = folder.skill(
        "home/.agents/skills/pdf",
        &skill_text("pdf", "Fills PDF forms."),
    );
    folder.skill(
        "home/.agents/skills/deploy",
        "---\nname: deploy\ndescription: Deploys the site.\ndisable-model-invocation: true\nuser-invocable: False\n---\n",
    );
    folder.folder("home/.agents/skills/notes");
    fs::write(place.join("README.md"), "not a skill").unwrap();
    folder.skill(
        "home/.agents/skills/.hidden",
        &skill_text("hidden", "Hidden."),
    );
    let protected = computer(&folder);

    let scan = scan_place(
        &place,
        &PlaceBounds {
            workspace: None,
            protected: &protected,
        },
    );

    assert_eq!(scan.failure, None);
    assert!(!scan.incomplete);
    assert_eq!(scan.problems, []);
    let names: Vec<&str> = scan
        .skills
        .iter()
        .map(|skill| skill.name.as_str())
        .collect();
    assert_eq!(names, ["deploy", "pdf"]);
    let deploy = &scan.skills[0];
    assert!(!deploy.listed && !deploy.mentionable, "{deploy:?}");
    let found_pdf = &scan.skills[1];
    assert_eq!(found_pdf.description, "Fills PDF forms.");
    assert_eq!(found_pdf.folder, pdf);
    assert!(found_pdf.listed && found_pdf.mentionable);
    assert_eq!(found_pdf.warnings, []);
    assert_eq!(scan.folder.as_deref(), Some(place.as_path()));
}

#[test]
fn a_missing_place_has_no_skills_and_a_file_in_its_place_fails() {
    let folder = Folder::new();
    let protected = computer(&folder);
    let bounds = PlaceBounds {
        workspace: None,
        protected: &protected,
    };

    let missing = scan_place(&folder.path("home/.agents/skills"), &bounds);
    fs::create_dir_all(folder.path("work/.agents")).unwrap();
    fs::write(folder.path("work/.agents/skills"), "a file").unwrap();
    let file = scan_place(&folder.path("work/.agents/skills"), &bounds);

    assert_eq!(missing.skills, []);
    assert_eq!(missing.failure, None);
    assert_eq!(file.failure, Some(SkillPlaceFailure::NotAFolder));
}

#[test]
fn folders_that_are_not_skills_show_why_and_warnings_keep_a_skill() {
    let folder = Folder::new();
    let place = folder.folder("home/.agents/skills");
    folder.skill("home/.agents/skills/no-frontmatter", "# Just a title\n");
    folder.skill(
        "home/.agents/skills/not-closed",
        "---\nname: not-closed\ndescription: Never closed.\n",
    );
    folder.skill(
        "home/.agents/skills/no-description",
        "---\nname: no-description\n---\n",
    );
    folder.skill(
        "home/.agents/skills/Bad_Name",
        &skill_text("Bad_Name", "Neither name is valid."),
    );
    folder.skill(
        "home/.agents/skills/from-folder",
        &skill_text("From Folder", "The folder names it."),
    );
    folder.skill(
        "home/.agents/skills/folder-a",
        &skill_text("other-name", "Named apart from its folder."),
    );
    folder.skill(
        "home/.agents/skills/folder-b",
        &skill_text("other-name", "The same name again."),
    );
    let long = "word ".repeat(400);
    folder.skill("home/.agents/skills/long", &skill_text("long", &long));
    let binary = folder.folder("home/.agents/skills/binary");
    fs::write(binary.join("SKILL.md"), b"---\0\0\0binary\0").unwrap();
    let protected = computer(&folder);

    let scan = scan_place(
        &place,
        &PlaceBounds {
            workspace: None,
            protected: &protected,
        },
    );

    assert_eq!(
        scan.problems,
        [
            problem("Bad_Name", SkillProblem::NameNotValid),
            problem("binary", SkillProblem::NotText),
            problem("folder-b", SkillProblem::NameTaken),
            problem("no-description", SkillProblem::DescriptionMissing),
            problem("no-frontmatter", SkillProblem::FrontmatterMissing),
            problem("not-closed", SkillProblem::FrontmatterNotClosed),
        ]
    );
    let warnings: Vec<(&str, &[SkillWarning])> = scan
        .skills
        .iter()
        .map(|skill| (skill.name.as_str(), skill.warnings.as_slice()))
        .collect();
    assert_eq!(
        warnings,
        [
            ("from-folder", [SkillWarning::NameFromFolder].as_slice()),
            ("long", [SkillWarning::DescriptionCut].as_slice()),
            ("other-name", [SkillWarning::NameDiffers].as_slice()),
        ]
    );
    let cut = &scan.skills[1].description;
    assert_eq!(cut.chars().count(), 1_024);
    assert!(cut.ends_with('…'), "{cut}");
}

#[test]
fn a_project_s_skill_that_resolves_outside_the_project_is_refused() {
    let folder = Folder::new();
    let workspace = folder.folder("work");
    let place = folder.folder("work/.agents/skills");
    let outside = folder.skill("elsewhere/pdf", &skill_text("pdf", "Fills PDF forms."));
    link_folder(&place.join("pdf"), &outside);
    folder.skill(
        "work/.agents/skills/docx",
        &skill_text("docx", "Writes documents."),
    );
    let protected = computer(&folder);

    let scan = scan_place(
        &place,
        &PlaceBounds {
            workspace: Some(&workspace),
            protected: &protected,
        },
    );

    assert_eq!(
        scan.problems,
        [problem("pdf", SkillProblem::OutsideProject)]
    );
    assert_eq!(scan.skills.len(), 1);
    assert_eq!(scan.skills[0].name, "docx");
}

#[test]
fn a_skill_that_links_into_or_around_mannie_s_folders_or_its_place_is_refused() {
    let folder = Folder::new();
    let place = folder.folder("home/.agents/skills");
    let protected = computer(&folder);
    folder.skill(
        "data/mannie/pdf",
        &skill_text("pdf", "Inside Mannie's data."),
    );
    link_folder(&place.join("inside"), &folder.path("data/mannie/pdf"));
    fs::write(
        folder.path("data/SKILL.md"),
        skill_text("data", "Around Mannie's data."),
    )
    .unwrap();
    link_folder(&place.join("around"), &folder.path("data"));
    fs::write(
        folder.path("home/.agents/SKILL.md"),
        skill_text("agents", "Around the place."),
    )
    .unwrap();
    link_folder(&place.join("parent"), &folder.path("home/.agents"));
    // A skill of this computer may live anywhere else, as the owner links it.
    let elsewhere = folder.skill("tools/docx", &skill_text("docx", "Writes documents."));
    link_folder(&place.join("docx"), &elsewhere);

    let scan = scan_place(
        &place,
        &PlaceBounds {
            workspace: None,
            protected: &protected,
        },
    );

    assert_eq!(
        scan.problems,
        [
            problem("around", SkillProblem::InProtectedFolder),
            problem("inside", SkillProblem::InProtectedFolder),
            problem("parent", SkillProblem::AroundPlace),
        ]
    );
    assert_eq!(scan.skills.len(), 1);
    assert_eq!(scan.skills[0].folder, elsewhere);
}

#[cfg(unix)]
#[test]
fn a_skill_file_that_links_outside_its_folder_is_refused() {
    let folder = Folder::new();
    let place = folder.folder("home/.agents/skills");
    let skill = folder.folder("home/.agents/skills/pdf");
    fs::write(folder.path("secret.md"), skill_text("pdf", "Elsewhere.")).unwrap();
    std::os::unix::fs::symlink(folder.path("secret.md"), skill.join("SKILL.md")).unwrap();
    let protected = computer(&folder);

    let scan = scan_place(
        &place,
        &PlaceBounds {
            workspace: None,
            protected: &protected,
        },
    );

    assert_eq!(scan.problems, [problem("pdf", SkillProblem::OutsideSkill)]);
}

#[test]
fn a_place_keeps_at_most_its_limit_of_skills() {
    let folder = Folder::new();
    let place = folder.folder("home/.agents/skills");
    for number in 0..=MAX_PLACE_SKILLS {
        let name = format!("skill-{number:03}");
        folder.skill(
            &format!("home/.agents/skills/{name}"),
            &skill_text(&name, "One of many."),
        );
    }
    let protected = computer(&folder);

    let scan = scan_place(
        &place,
        &PlaceBounds {
            workspace: None,
            protected: &protected,
        },
    );

    assert_eq!(scan.skills.len(), MAX_PLACE_SKILLS);
    assert!(scan.incomplete);
}

#[test]
fn only_the_start_of_a_long_skill_file_is_read() {
    let folder = Folder::new();
    let place = folder.folder("home/.agents/skills");
    let body = "A line of the body.\n".repeat(MAX_SKILL_HEAD_BYTES / 10);
    folder.skill(
        "home/.agents/skills/long-body",
        &format!("{}{body}", skill_text("long-body", "A long body.")),
    );
    let padding = "padding: value\n".repeat(MAX_SKILL_HEAD_BYTES / 10);
    folder.skill(
        "home/.agents/skills/long-frontmatter",
        &format!("---\nname: long-frontmatter\n{padding}description: Too late.\n---\n"),
    );
    let protected = computer(&folder);

    let scan = scan_place(
        &place,
        &PlaceBounds {
            workspace: None,
            protected: &protected,
        },
    );

    assert_eq!(scan.skills.len(), 1);
    assert_eq!(scan.skills[0].name, "long-body");
    assert_eq!(
        scan.problems,
        [problem(
            "long-frontmatter",
            SkillProblem::FrontmatterNotClosed
        )]
    );
}

#[test]
fn a_skill_s_digest_follows_what_agents_list_shows_of_it() {
    let folder = Folder::new();
    let place = folder.folder("home/.agents/skills");
    folder.skill(
        "home/.agents/skills/pdf",
        &skill_text("pdf", "Fills forms."),
    );
    let protected = computer(&folder);
    let bounds = PlaceBounds {
        workspace: None,
        protected: &protected,
    };
    let before = scan_place(&place, &bounds).skills[0].clone();

    folder.skill(
        "home/.agents/skills/pdf",
        &format!("{}\nA new body line.\n", skill_text("pdf", "Fills forms.")),
    );
    let body_changed = scan_place(&place, &bounds).skills[0].clone();
    folder.skill(
        "home/.agents/skills/pdf",
        &skill_text("pdf", "Fills any form."),
    );
    let description_changed = scan_place(&place, &bounds).skills[0].clone();

    assert_eq!(before.digest(), body_changed.digest());
    assert_ne!(before.digest(), description_changed.digest());
    assert_eq!(before.digest().len(), 64);
    let hidden = super::FoundSkill {
        listed: false,
        ..before.clone()
    };
    assert_ne!(before.digest(), hidden.digest());
}
