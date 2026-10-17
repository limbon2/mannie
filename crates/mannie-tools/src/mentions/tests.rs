use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use super::{
    FileMention, FolderMention, MentionPathError, MentionReadFailure, PageShows, resolve_mention,
};
use crate::paths::is_inside;

/// A workspace in canonical form, as projects store their folders.
fn workspace() -> (tempfile::TempDir, std::path::PathBuf) {
    let folder = tempfile::tempdir().unwrap();
    let canonical = crate::paths::resolve(folder.path(), crate::paths::FinalLink::Follow)
        .unwrap()
        .path;
    (folder, canonical)
}

/// A text of `count` numbered lines.
fn numbered_lines(count: u32) -> String {
    let mut text = String::new();
    for number in 1..=count {
        writeln!(text, "line {number}").expect("writing to a String cannot fail");
    }
    text
}

#[test]
fn mentions_name_files_and_folders_below_the_workspace() {
    let (_folder, root) = workspace();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src").join("main.rs"), "fn main() {}\n").unwrap();

    let file = resolve_mention(&root, "src/main.rs").unwrap();
    let folder = resolve_mention(&root, "src/").unwrap();
    let itself = resolve_mention(&root, ".").unwrap();

    assert_eq!(file.shown, "src/main.rs");
    assert!(!file.folder);
    assert!(is_inside(&file.path, &root));
    assert_eq!((folder.shown.as_str(), folder.folder), ("src", true));
    assert_eq!((itself.shown.as_str(), itself.folder), (".", true));
    assert_eq!(
        resolve_mention(&root, "src/missing.rs"),
        Err(MentionPathError::NotFound)
    );
}

#[test]
fn keys_outside_the_workspace_are_refused_before_any_access() {
    let (_folder, root) = workspace();
    fs::write(root.join("a.txt"), "").unwrap();
    let absolute = root.join("a.txt").to_str().unwrap().to_owned();

    for key in [
        absolute.as_str(),
        "../a.txt",
        "src/../../a.txt",
        "",
        "a\u{7}.txt",
    ] {
        assert_eq!(
            resolve_mention(&root, key),
            Err(MentionPathError::OutsideWorkspace),
            "{key:?}"
        );
    }
}

#[cfg(windows)]
#[test]
fn windows_keys_take_backslashes_and_refuse_device_paths() {
    let (_folder, root) = workspace();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src").join("main.rs"), "").unwrap();

    assert_eq!(
        resolve_mention(&root, "src\\main.rs").unwrap().shown,
        "src/main.rs"
    );
    for key in [
        "\\\\server\\share\\a.txt",
        "\\\\?\\C:\\a.txt",
        "C:a.txt",
        "\\a.txt",
    ] {
        assert_eq!(
            resolve_mention(&root, key),
            Err(MentionPathError::OutsideWorkspace),
            "{key:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_link_may_lead_out_of_the_workspace_for_the_policy_to_decide() {
    let (_folder, root) = workspace();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("notes.txt"), "").unwrap();
    std::os::unix::fs::symlink(outside.path().join("notes.txt"), root.join("notes.txt")).unwrap();

    let target = resolve_mention(&root, "notes.txt").unwrap();

    assert_eq!(target.shown, "notes.txt");
    assert!(!is_inside(&target.path, &root));
}

#[test]
fn a_file_reads_as_read_file_shows_it() {
    let (_folder, root) = workspace();
    let path = root.join("notes.txt");
    fs::write(&path, "first\nsecond\nthird\n").unwrap();

    let page = FileMention::read(&path, 64 * 1024)
        .unwrap()
        .render(64 * 1024);

    assert!(page.text.starts_with("File: "), "{}", page.text);
    assert!(page.text.contains("\nRevision: sha256:"), "{}", page.text);
    assert!(
        page.text
            .contains("Lines 1 to 3 of 3.\n     1\tfirst\n     2\tsecond\n     3\tthird\n")
    );
    assert!(!page.text.contains("More follows"));
    assert_eq!(
        page.shows,
        PageShows::Lines {
            first: 1,
            last: 3,
            total: 3
        }
    );
}

#[test]
fn a_long_file_is_cut_within_its_share_with_a_note_to_read_on() {
    let (_folder, root) = workspace();
    let path = root.join("long.txt");
    fs::write(&path, numbered_lines(3_000)).unwrap();
    let mention = FileMention::read(&path, 64 * 1024).unwrap();

    let page = mention.render(4 * 1024);
    let none = mention.render(600);

    assert!(page.text.len() <= 4 * 1024, "{}", page.text.len());
    let PageShows::Lines { last, total, .. } = page.shows else {
        panic!("a file shows lines");
    };
    assert_eq!(total, 3_000);
    assert!(last > 100 && last < 3_000, "{last}");
    assert!(
        page.text.ends_with(&format!(
            "More follows: read_file reads on with offset {}.\n",
            last + 1
        )),
        "{}",
        page.text
    );
    assert!(
        none.text.contains("Lines shown: none of 3000."),
        "{}",
        none.text
    );
}

#[test]
fn files_that_are_empty_binary_too_large_or_missing_give_no_text() {
    let (_folder, root) = workspace();
    fs::write(root.join("empty.txt"), "").unwrap();
    fs::write(root.join("image.png"), b"\x89PNG\r\n\x1a\n\x00\x00").unwrap();
    let huge = fs::File::create(root.join("huge.log")).unwrap();
    huge.set_len(16 * 1024 * 1024 + 1).unwrap();
    drop(huge);

    let empty = FileMention::read(&root.join("empty.txt"), 1024)
        .unwrap()
        .render(4096);

    assert!(
        empty.text.ends_with("The file is empty.\n"),
        "{}",
        empty.text
    );
    assert_eq!(
        empty.shows,
        PageShows::Lines {
            first: 1,
            last: 0,
            total: 0
        }
    );
    assert_eq!(
        FileMention::read(&root.join("image.png"), 1024).unwrap_err(),
        MentionReadFailure::NotText
    );
    assert_eq!(
        FileMention::read(&root.join("huge.log"), 1024).unwrap_err(),
        MentionReadFailure::TooLarge {
            limit_bytes: 16 * 1024 * 1024
        }
    );
    assert!(matches!(
        FileMention::read(&root.join("gone.txt"), 1024),
        Err(MentionReadFailure::Unreadable { .. })
    ));
}

#[test]
fn crlf_files_say_how_they_store_their_text() {
    let (_folder, root) = workspace();
    let path = root.join("windows.txt");
    fs::write(&path, "one\r\ntwo\r\n").unwrap();

    let page = FileMention::read(&path, 4096).unwrap().render(4096);

    assert!(
        page.text.contains("Text: UTF-8 with CRLF line endings"),
        "{}",
        page.text
    );
    assert!(
        page.text.contains("     1\tone\n     2\ttwo\n"),
        "{}",
        page.text
    );
}

/// The folder `docs` of a workspace with a few entries in two levels.
fn docs(root: &Path) {
    fs::create_dir_all(root.join("docs").join("adr")).unwrap();
    fs::create_dir_all(root.join("docs").join(".git").join("objects")).unwrap();
    fs::write(root.join("docs").join("README.md"), "hello").unwrap();
    fs::write(root.join("docs").join("adr").join("0001.md"), "ab").unwrap();
}

#[test]
fn a_folder_lists_its_entries_breadth_first_sorted_by_path() {
    let (_folder, root) = workspace();
    docs(&root);

    let page = FolderMention::list(&root.join("docs"), "docs", 64 * 1024)
        .unwrap()
        .render(64 * 1024);

    assert_eq!(
        page.text,
        "docs/.git/ (entries not shown)\ndocs/adr/\ndocs/adr/0001.md (2 bytes)\ndocs/README.md (5 bytes)\n"
    );
    assert_eq!(
        page.shows,
        PageShows::Entries {
            shown: 4,
            complete: true
        }
    );
}

#[test]
fn a_listing_shows_fifty_entries_of_each_folder() {
    let (_folder, root) = workspace();
    fs::create_dir_all(root.join("many").join("inner")).unwrap();
    for number in 0..60 {
        fs::write(
            root.join("many")
                .join("inner")
                .join(format!("{number:02}.txt")),
            "",
        )
        .unwrap();
    }

    let page = FolderMention::list(&root.join("many"), "many", 64 * 1024)
        .unwrap()
        .render(64 * 1024);

    assert!(
        page.text
            .starts_with("many/inner/ (50 of 60 entries shown)\n"),
        "{}",
        page.text
    );
    assert!(page.text.contains("many/inner/49.txt (0 bytes)\n"));
    assert!(!page.text.contains("many/inner/50.txt"));
    assert!(
        page.text
            .ends_with("Not every entry is shown: list_directory and find_files show the rest.\n")
    );
    assert_eq!(
        page.shows,
        PageShows::Entries {
            shown: 51,
            complete: false
        }
    );
}

#[test]
fn a_listing_is_cut_within_its_share() {
    let (_folder, root) = workspace();
    for folder in ["a", "b", "c"] {
        fs::create_dir_all(root.join(folder)).unwrap();
        for number in 0..40 {
            fs::write(root.join(folder).join(format!("file-{number:02}.txt")), "").unwrap();
        }
    }
    let listing = FolderMention::list(&root, ".", 64 * 1024).unwrap();

    let small = listing.render(2 * 1024);
    let empty_folder = {
        fs::create_dir_all(root.join("empty")).unwrap();
        FolderMention::list(&root.join("empty"), "empty", 1024)
            .unwrap()
            .render(1024)
    };

    assert!(small.text.len() <= 2 * 1024, "{}", small.text.len());
    assert!(small.text.starts_with("a/ ("), "{}", small.text);
    let PageShows::Entries { shown, complete } = small.shows else {
        panic!("a folder shows entries");
    };
    assert!(!complete);
    assert!(shown > 3 && shown < 123, "{shown}");
    assert_eq!(empty_folder.text, "The folder is empty.\n");
    assert_eq!(
        empty_folder.shows,
        PageShows::Entries {
            shown: 0,
            complete: true
        }
    );
}
