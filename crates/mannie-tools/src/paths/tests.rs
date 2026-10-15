use std::fs;
use std::path::{Path, PathBuf};

use mannie_types::tools::{PathRefusal, ToolFailure};
use tempfile::TempDir;

use super::{
    FinalLink, FolderError, for_change, for_info, for_read, is_inside, locate, resolve,
    workspace_folder,
};

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

    fn file(&self, relative: &str) -> PathBuf {
        let path = self.path(relative);
        fs::write(&path, "content").unwrap();
        path
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

#[test]
fn relative_paths_need_a_workspace() {
    assert_eq!(locate(None, "notes.txt"), Err(ToolFailure::NoWorkspace));
}

#[test]
fn dot_and_dot_dot_are_removed_before_anything_is_resolved() {
    let folder = Folder::new();
    let workspace = folder.path("work");

    assert_eq!(
        locate(Some(&workspace), "src/./../notes.txt").unwrap(),
        folder.path("work/notes.txt")
    );
    let above_everything = "../".repeat(64);
    let top = locate(Some(&workspace), &above_everything).unwrap();
    assert_eq!(top.parent(), None, "{top:?} is not a root");
}

#[test]
fn refused_paths_fail_before_the_workspace_is_used() {
    let refusal = |reason| Err(ToolFailure::PathRefused { reason });

    assert_eq!(locate(None, ""), refusal(PathRefusal::Empty));
    assert_eq!(locate(None, "notes\n.txt"), refusal(PathRefusal::Empty));
    assert_eq!(
        locate(None, &"a".repeat(40_000)),
        refusal(PathRefusal::TooLong)
    );
    #[cfg(windows)]
    {
        assert_eq!(
            locate(None, r"\\attacker\share\file"),
            refusal(PathRefusal::NetworkPath)
        );
        assert_eq!(
            locate(None, "C:notes.txt"),
            refusal(PathRefusal::DriveRelative)
        );
    }
}

#[test]
fn reads_follow_links_and_say_so() {
    let folder = Folder::new();
    let real = folder.folder("real");
    folder.file("real/notes.txt");
    link_folder(&folder.path("link"), &real);

    let resolved = for_read(Some(&folder.root), "link/notes.txt").unwrap();

    assert_eq!(resolved.path, folder.path("real/notes.txt"));
    assert!(resolved.through_link);
    assert!(resolved.exists());
    assert!(!resolved.is_folder());
}

#[test]
fn changes_through_a_link_are_refused_with_the_resolved_path() {
    let folder = Folder::new();
    let real = folder.folder("real");
    link_folder(&folder.path("link"), &real);

    let failure = for_change(Some(&folder.root), "link/new.txt", FinalLink::Follow).unwrap_err();

    assert_eq!(
        failure,
        ToolFailure::PathThroughLink {
            resolved: folder.path("real/new.txt").to_str().unwrap().to_owned(),
        }
    );
    let at_the_link = for_change(Some(&folder.root), "link", FinalLink::Follow).unwrap_err();
    assert!(
        matches!(at_the_link, ToolFailure::PathThroughLink { .. }),
        "{at_the_link:?}"
    );
}

#[test]
fn a_final_link_is_kept_when_the_tool_acts_on_the_link() {
    let folder = Folder::new();
    let real = folder.folder("real");
    link_folder(&folder.path("link"), &real);

    let kept = for_change(Some(&folder.root), "link", FinalLink::Keep).unwrap();
    let described = for_info(Some(&folder.root), "link").unwrap();

    assert_eq!(kept.path, folder.path("link"));
    assert!(kept.is_link());
    assert!(!kept.through_link);
    assert_eq!(described.path, folder.path("link"));
    assert!(described.is_link());
}

#[test]
fn missing_paths_keep_their_names() {
    let folder = Folder::new();
    folder.folder("work");

    let resolved = for_read(Some(&folder.root), "work/missing/deeper.txt").unwrap();

    assert_eq!(resolved.path, folder.path("work/missing/deeper.txt"));
    assert!(!resolved.exists());
    assert!(!resolved.through_link);
}

#[test]
fn a_path_below_a_file_does_not_exist() {
    let folder = Folder::new();
    folder.file("notes.txt");

    let resolved = for_read(Some(&folder.root), "notes.txt/inner").unwrap();

    assert!(!resolved.exists());
}

#[cfg(unix)]
#[test]
fn link_loops_end_with_an_error() {
    let folder = Folder::new();
    std::os::unix::fs::symlink(folder.path("second"), folder.path("first")).unwrap();
    std::os::unix::fs::symlink(folder.path("first"), folder.path("second")).unwrap();

    let failure = for_read(Some(&folder.root), "first/file").unwrap_err();

    assert_eq!(
        failure,
        ToolFailure::IoError {
            kind: "the path goes through too many links".to_owned(),
        }
    );
}

#[cfg(unix)]
#[test]
fn relative_link_targets_resolve_from_the_link_folder() {
    let folder = Folder::new();
    folder.folder("a/b");
    folder.file("a/target.txt");
    std::os::unix::fs::symlink("../target.txt", folder.path("a/b/link.txt")).unwrap();

    let resolved = for_read(Some(&folder.root), "a/b/link.txt").unwrap();

    assert_eq!(resolved.path, folder.path("a/target.txt"));
}

#[cfg(windows)]
#[test]
fn existing_names_come_out_in_the_case_the_file_system_stores() {
    let folder = Folder::new();
    folder.folder("MixedCase");

    let resolved = for_read(Some(&folder.root), "mixedcase/new.txt").unwrap();

    assert_eq!(resolved.path, folder.path("MixedCase/new.txt"));
}

#[test]
fn inside_compares_whole_names() {
    let folder = Folder::new();
    let work = folder.path("work");

    assert!(is_inside(&work, &work));
    assert!(is_inside(&folder.path("work/src/main.rs"), &work));
    assert!(!is_inside(&folder.path("workshop/main.rs"), &work));
    assert!(!is_inside(&folder.root, &work));
    #[cfg(windows)]
    assert!(is_inside(
        Path::new(r"c:\USERS\owner\work"),
        Path::new(r"C:\Users\Owner")
    ));
}

#[test]
fn workspaces_are_existing_folders_in_canonical_form() {
    let folder = Folder::new();
    let real = folder.folder("real");
    folder.file("notes.txt");
    link_folder(&folder.path("link"), &real);
    let text = |relative: &str| folder.path(relative).to_str().unwrap().to_owned();

    assert_eq!(workspace_folder(&text("link")), Ok(text("real")));
    assert_eq!(
        workspace_folder("relative/folder"),
        Err(FolderError::Refused {
            reason: PathRefusal::NotAbsolute
        })
    );
    assert_eq!(
        workspace_folder(&text("missing")),
        Err(FolderError::NotFound)
    );
    assert_eq!(
        workspace_folder(&text("notes.txt")),
        Err(FolderError::NotAFolder)
    );
}
