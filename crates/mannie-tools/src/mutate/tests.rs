use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use mannie_types::tools::ToolFailure;

use super::{EntryType, PinnedFolder};
use crate::paths::{FinalLink, resolve};

/// A temporary folder in canonical form.
fn folder() -> (tempfile::TempDir, PathBuf) {
    let temporary = tempfile::tempdir().unwrap();
    let root = resolve(temporary.path(), FinalLink::Follow).unwrap().path;
    (temporary, root)
}

fn name(text: &str) -> &OsStr {
    OsStr::new(text)
}

/// The names in a folder, sorted.
fn names(path: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
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

#[test]
fn a_committed_draft_becomes_the_file() {
    let (_temporary, root) = folder();
    let pinned = PinnedFolder::open(&root).unwrap();

    let mut draft = pinned.draft(name("notes.txt")).unwrap();
    draft.write(b"hello").unwrap();
    pinned.commit_new(draft, name("notes.txt")).unwrap();

    assert_eq!(fs::read(root.join("notes.txt")).unwrap(), b"hello");
    assert_eq!(names(&root), ["notes.txt"]);
}

#[test]
fn creating_never_replaces_an_existing_file() {
    let (_temporary, root) = folder();
    fs::write(root.join("notes.txt"), "theirs").unwrap();
    let pinned = PinnedFolder::open(&root).unwrap();

    let mut draft = pinned.draft(name("notes.txt")).unwrap();
    draft.write(b"mine").unwrap();
    let failure = pinned.commit_new(draft, name("notes.txt")).unwrap_err();

    assert!(
        matches!(failure, ToolFailure::AlreadyExists { .. }),
        "{failure:?}"
    );
    assert_eq!(
        fs::read_to_string(root.join("notes.txt")).unwrap(),
        "theirs"
    );
    // The draft's temporary file is gone.
    assert_eq!(names(&root), ["notes.txt"]);
}

#[test]
fn a_draft_that_is_dropped_leaves_nothing() {
    let (_temporary, root) = folder();
    let pinned = PinnedFolder::open(&root).unwrap();

    let mut draft = pinned.draft(name("notes.txt")).unwrap();
    draft.write(b"unused").unwrap();
    drop(draft);

    assert_eq!(names(&root), Vec::<String>::new());
}

#[test]
fn a_replacement_keeps_the_permissions_of_the_file() {
    let (_temporary, root) = folder();
    let path = root.join("build.sh");
    fs::write(&path, "echo old").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
    }
    let pinned = PinnedFolder::open(&root).unwrap();
    let existing = pinned.read(name("build.sh"), 1024).unwrap();

    let mut draft = pinned.draft(name("build.sh")).unwrap();
    draft.write(b"echo new").unwrap();
    pinned
        .commit_replace(draft, name("build.sh"), &existing)
        .unwrap();

    assert_eq!(existing.bytes, b"echo old");
    assert_eq!(fs::read_to_string(&path).unwrap(), "echo new");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o750
        );
    }
}

#[test]
fn reads_refuse_large_files_and_folders() {
    let (_temporary, root) = folder();
    fs::write(root.join("large.txt"), vec![b'a'; 100]).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    let pinned = PinnedFolder::open(&root).unwrap();

    assert_eq!(
        pinned.read(name("large.txt"), 99).unwrap_err(),
        ToolFailure::TooLarge { limit_bytes: 99 }
    );
    assert!(matches!(
        pinned.read(name("src"), 1024).unwrap_err(),
        ToolFailure::NotAFile { .. }
    ));
}

#[test]
fn pinning_refuses_a_link_on_the_path() {
    let (_temporary, root) = folder();
    let real = root.join("real");
    fs::create_dir(&real).unwrap();
    link_folder(&root.join("link"), &real);

    assert_eq!(
        PinnedFolder::open(&root.join("link")).unwrap_err(),
        ToolFailure::PathChanged
    );
    let pinned = PinnedFolder::open(&root).unwrap();
    assert_eq!(
        pinned.child(name("link")).unwrap_err(),
        ToolFailure::PathChanged
    );
    assert!(matches!(
        pinned.entry(name("link")).unwrap(),
        Some((_, EntryType::Link))
    ));
}

#[test]
fn deleting_a_tree_deletes_links_but_not_their_targets() {
    let (_temporary, root) = folder();
    let outside = root.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep.txt"), "keep").unwrap();
    let doomed = root.join("doomed");
    fs::create_dir_all(doomed.join("deeper")).unwrap();
    fs::write(doomed.join("deeper").join("file.txt"), "gone").unwrap();
    link_folder(&doomed.join("escape"), &outside);
    let pinned = PinnedFolder::open(&root).unwrap();

    pinned.remove_tree(name("doomed")).unwrap();

    assert_eq!(names(&root), ["outside"]);
    assert_eq!(
        fs::read_to_string(outside.join("keep.txt")).unwrap(),
        "keep"
    );
}

#[test]
fn deleting_a_link_keeps_its_target() {
    let (_temporary, root) = folder();
    let real = root.join("real");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("keep.txt"), "keep").unwrap();
    link_folder(&root.join("link"), &real);
    let pinned = PinnedFolder::open(&root).unwrap();

    pinned.remove_entry(name("link")).unwrap();

    assert_eq!(names(&root), ["real"]);
    assert_eq!(names(&real), ["keep.txt"]);
}

#[test]
fn an_empty_folder_is_deleted_and_a_full_one_is_refused() {
    let (_temporary, root) = folder();
    fs::create_dir(root.join("empty")).unwrap();
    fs::create_dir(root.join("full")).unwrap();
    fs::write(root.join("full").join("file.txt"), "x").unwrap();
    let pinned = PinnedFolder::open(&root).unwrap();

    pinned.remove_empty_folder(name("empty")).unwrap();
    let failure = pinned.remove_empty_folder(name("full")).unwrap_err();

    assert!(
        matches!(failure, ToolFailure::FolderNotEmpty { .. }),
        "{failure:?}"
    );
    assert_eq!(names(&root), ["full"]);
}

#[test]
fn moves_never_replace_and_keep_the_identity() {
    let (_temporary, root) = folder();
    fs::write(root.join("a.txt"), "a").unwrap();
    fs::write(root.join("b.txt"), "b").unwrap();
    fs::create_dir(root.join("into")).unwrap();
    let pinned = PinnedFolder::open(&root).unwrap();
    let into = pinned.child(name("into")).unwrap();
    let (identity, _) = pinned.entry(name("a.txt")).unwrap().unwrap();

    let refused = pinned
        .move_entry(name("a.txt"), &pinned, name("b.txt"))
        .unwrap_err();
    pinned
        .move_entry(name("a.txt"), &into, name("moved.txt"))
        .unwrap();

    assert!(
        matches!(refused, ToolFailure::AlreadyExists { .. }),
        "{refused:?}"
    );
    assert_eq!(fs::read_to_string(root.join("b.txt")).unwrap(), "b");
    assert_eq!(names(&root), ["b.txt", "into"]);
    assert_eq!(into.entry(name("moved.txt")).unwrap().unwrap().0, identity);
}

#[test]
fn folders_move_and_refuse_an_existing_destination() {
    let (_temporary, root) = folder();
    fs::create_dir_all(root.join("src").join("inner")).unwrap();
    fs::create_dir(root.join("taken")).unwrap();
    let pinned = PinnedFolder::open(&root).unwrap();

    let refused = pinned
        .move_entry(name("src"), &pinned, name("taken"))
        .unwrap_err();
    pinned
        .move_entry(name("src"), &pinned, name("renamed"))
        .unwrap();

    assert!(
        matches!(refused, ToolFailure::AlreadyExists { .. }),
        "{refused:?}"
    );
    assert_eq!(names(&root), ["renamed", "taken"]);
    assert_eq!(names(&root.join("renamed")), ["inner"]);
}

#[test]
fn folders_are_created_once() {
    let (_temporary, root) = folder();
    let pinned = PinnedFolder::open(&root).unwrap();

    pinned.create_folder(name("new")).unwrap();
    let again = pinned.create_folder(name("new")).unwrap_err();

    assert!(
        matches!(again, ToolFailure::AlreadyExists { .. }),
        "{again:?}"
    );
    assert!(root.join("new").is_dir());
}
