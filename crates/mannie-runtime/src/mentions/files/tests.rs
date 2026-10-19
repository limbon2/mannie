use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use mannie_plugin_sdk::points::mentions::RenderedMention;
use mannie_types::mentions::MentionEntry;

use super::FileMentions;
use crate::mentions::{MentionContext, MentionProvider};
use crate::policy::PolicyGrants;

/// A workspace in canonical form, as projects store their folders.
fn workspace() -> (tempfile::TempDir, PathBuf) {
    let folder = tempfile::tempdir().unwrap();
    let canonical = PathBuf::from(mannie_tools::canonical_folder(folder.path()).unwrap());
    (folder, canonical)
}

fn context(workspace: Option<&Path>, protected: Vec<PathBuf>, full_access: bool) -> MentionContext {
    MentionContext {
        conversation: None,
        project: None,
        workspace: workspace.map(Path::to_path_buf),
        full_access,
        grants: PolicyGrants::default(),
        protected,
        ceiling: None,
    }
}

async fn entries(
    provider: &FileMentions,
    context: &MentionContext,
    query: &str,
) -> Vec<MentionEntry> {
    provider.suggest(context, query, 50).await.unwrap().entries
}

async fn rendered(provider: &FileMentions, context: &MentionContext, key: &str) -> RenderedMention {
    provider
        .resolve(context, key, 64 * 1024)
        .await
        .unwrap()
        .render(64 * 1024)
}

#[tokio::test]
async fn files_are_offered_only_where_there_is_a_workspace() {
    let (_folder, root) = workspace();
    let provider = FileMentions::new();

    assert!(provider.offered(&context(Some(&root), Vec::new(), false)));
    assert!(!provider.offered(&context(None, Vec::new(), false)));
}

#[tokio::test]
async fn suggestions_include_hidden_ignored_and_secret_files() {
    let (_folder, root) = workspace();
    fs::write(root.join(".gitignore"), "build/\n").unwrap();
    fs::write(root.join(".env"), "TOKEN=1\n").unwrap();
    fs::create_dir_all(root.join("build")).unwrap();
    fs::write(root.join("build").join("out.txt"), "").unwrap();
    fs::create_dir_all(root.join("my notes")).unwrap();
    fs::write(root.join("my notes").join("plan.md"), "").unwrap();
    let provider = FileMentions::new();
    let place = context(Some(&root), Vec::new(), false);

    let env = entries(&provider, &place, ".env").await;
    let build = entries(&provider, &place, "build").await;
    let notes = entries(&provider, &place, "my notes").await;

    assert_eq!(env[0].key, ".env");
    assert_eq!(env[0].token, "@.env");
    assert_eq!(env[0].warning.as_deref(), Some("secret"));
    assert_eq!(
        (
            build[0].label.as_str(),
            build[0].token.as_str(),
            build[0].variant.as_str()
        ),
        ("build/", "@build/", "folder")
    );
    assert!(build[0].expands);
    assert!(build.iter().any(|entry| entry.key == "build/out.txt"));
    assert_eq!(notes[0].token, "@\"my notes/\"");
    assert!(notes.iter().all(|entry| entry.warning.is_none()));
}

#[tokio::test]
async fn an_index_serves_the_queries_that_follow_it() {
    let (folder, root) = workspace();
    fs::write(root.join("kept.txt"), "").unwrap();
    let provider = FileMentions::new();
    let place = context(Some(&root), Vec::new(), false);

    let before = entries(&provider, &place, "kept").await;
    fs::remove_file(root.join("kept.txt")).unwrap();
    let after = entries(&provider, &place, "kept").await;
    drop(folder);
    let fresh = FileMentions::new();

    assert_eq!(before, after);
    assert_eq!(
        fresh.suggest(&place, "kept", 50).await.unwrap_err(),
        "folder_unreadable"
    );
}

#[tokio::test]
async fn a_file_gives_its_page_and_a_folder_its_listing() {
    let (_folder, root) = workspace();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("docs").join("guide.md"), "one\ntwo\n").unwrap();
    let provider = FileMentions::new();
    let place = context(Some(&root), Vec::new(), false);

    let file = rendered(&provider, &place, "docs/guide.md").await;
    let folder = rendered(&provider, &place, "docs/").await;

    assert!(file.text.starts_with(
        "The owner mentioned the file docs/guide.md. Its text follows, as read_file shows it.\nFile: "
    ));
    assert!(
        file.text.contains("     1\tone\n     2\ttwo\n"),
        "{}",
        file.text
    );
    assert_eq!(
        (file.label.as_str(), file.variant.as_str()),
        ("docs/guide.md", "file")
    );
    assert_eq!(file.summary.code, "lines");
    assert_eq!(file.summary.values["total"], 2);
    assert_eq!(file.summary.values["last"], 2);
    assert!(
        folder
            .text
            .starts_with("The owner mentioned the folder docs.")
    );
    assert!(
        folder.text.ends_with("docs/guide.md (8 bytes)\n"),
        "{}",
        folder.text
    );
    assert_eq!(
        (folder.label.as_str(), folder.variant.as_str()),
        ("docs/", "folder")
    );
    assert_eq!(folder.summary.code, "entries");
    assert_eq!(folder.summary.values["shown"], 1);
}

#[tokio::test]
async fn a_page_keeps_within_a_small_share() {
    let (_folder, root) = workspace();
    let mut text = String::new();
    for number in 1..=2_000 {
        writeln!(text, "line {number}").expect("writing to a String cannot fail");
    }
    fs::write(root.join("long.txt"), text).unwrap();
    let provider = FileMentions::new();
    let place = context(Some(&root), Vec::new(), false);

    let content = provider
        .resolve(&place, "long.txt", 64 * 1024)
        .await
        .unwrap();
    let small = content.render(3_000);

    assert!(small.text.len() <= 3_000, "{}", small.text.len());
    assert!(
        small
            .text
            .contains("More follows: read_file reads on with offset")
    );
    assert!(small.summary.values["last"] < 2_000);
}

#[tokio::test]
async fn keys_outside_the_workspace_are_refused() {
    let (_folder, root) = workspace();
    let provider = FileMentions::new();
    let place = context(Some(&root), Vec::new(), false);
    let general = context(None, Vec::new(), false);

    // On Windows, a path that starts with a slash depends on the current
    // drive, which the checks of paths refuse as well.
    let mut outside = vec!["../escape.txt", "docs/../../escape.txt", "/etc/passwd"];
    if cfg!(windows) {
        outside.push("C:\\Windows\\win.ini");
    }
    for key in outside {
        assert_eq!(
            provider.resolve(&place, key, 1024).await.err(),
            Some("outside_workspace".to_owned()),
            "{key}"
        );
    }
    assert_eq!(
        provider.resolve(&general, "a.txt", 1024).await.err(),
        Some("no_workspace".to_owned())
    );
}

#[tokio::test]
async fn missing_binary_and_protected_files_give_notes() {
    let (_folder, root) = workspace();
    fs::write(root.join("logo.png"), b"\x89PNG\r\n\x1a\n\x00\x00").unwrap();
    fs::create_dir_all(root.join("mannie-data")).unwrap();
    fs::write(root.join("mannie-data").join("log.txt"), "secret log\n").unwrap();
    let protected = vec![root.join("mannie-data")];
    let provider = FileMentions::new();
    let place = context(Some(&root), protected.clone(), false);
    let full = context(Some(&root), protected, true);

    let missing = rendered(&provider, &place, "gone/").await;
    let binary = rendered(&provider, &place, "logo.png").await;
    let guarded = rendered(&provider, &place, "mannie-data/log.txt").await;
    let allowed = rendered(&provider, &full, "mannie-data/log.txt").await;

    assert_eq!(
        missing.text,
        "The owner mentioned gone, which does not exist."
    );
    assert_eq!(missing.summary.code, "not_found");
    assert_eq!(binary.summary.code, "not_text");
    assert!(binary.text.contains("which is not text"));
    assert_eq!(guarded.summary.code, "needs_approval");
    assert!(!guarded.text.contains("secret log"));
    assert_eq!(allowed.summary.code, "lines");
    assert!(allowed.text.contains("secret log"));
}
