use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use futures_util::FutureExt;
use futures_util::future::join_all;
use mannie_plugin_sdk::points::mentions::RenderedMention;
use mannie_tools::MentionIndex;
use mannie_types::mentions::MentionEntry;
use tokio::time::Instant;

use super::{Building, FileMentions, INDEX_LIFETIME, MAX_INDEXES, keep, lock};
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

async fn keys(provider: &FileMentions, context: &MentionContext, query: &str) -> Vec<String> {
    entries(provider, context, query)
        .await
        .into_iter()
        .map(|entry| entry.key)
        .collect()
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

/// How many times [`counted_walk`] walked.
static COUNTED_WALKS: AtomicUsize = AtomicUsize::new(0);

fn counted_walk(folder: &Path) -> Option<MentionIndex> {
    COUNTED_WALKS.fetch_add(1, Ordering::SeqCst);
    folder.is_dir().then(|| MentionIndex::build(folder))
}

#[tokio::test]
async fn queries_that_arrive_while_a_workspace_is_walked_wait_for_that_walk() {
    let (_folder, root) = workspace();
    fs::write(root.join("a.txt"), "").unwrap();
    let provider = FileMentions::walking(counted_walk);
    let place = context(Some(&root), Vec::new(), false);

    let answers = join_all((0..5).map(|_| provider.suggest(&place, "a.txt", 50))).await;

    assert_eq!(COUNTED_WALKS.load(Ordering::SeqCst), 1);
    for answer in answers {
        assert_eq!(answer.unwrap().entries[0].key, "a.txt");
    }
}

#[tokio::test(start_paused = true)]
async fn an_index_past_its_lifetime_answers_at_once_while_the_workspace_is_walked_again() {
    let (_folder, root) = workspace();
    fs::write(root.join("old.txt"), "").unwrap();
    let provider = FileMentions::new();
    let place = context(Some(&root), Vec::new(), false);

    let first = keys(&provider, &place, "txt").await;
    fs::remove_file(root.join("old.txt")).unwrap();
    fs::write(root.join("new.txt"), "").unwrap();
    tokio::time::advance(INDEX_LIFETIME / 2).await;
    let within = keys(&provider, &place, "txt").await;
    tokio::time::advance(INDEX_LIFETIME).await;
    let past = keys(&provider, &place, "txt").await;
    provider.walked(&root).await;
    let walked = keys(&provider, &place, "txt").await;

    assert_eq!(first, ["old.txt"]);
    assert_eq!(within, ["old.txt"]);
    assert_eq!(past, ["old.txt"]);
    assert_eq!(walked, ["new.txt"]);
}

#[tokio::test(start_paused = true)]
async fn a_workspace_that_can_no_longer_be_walked_drops_its_index() {
    let (folder, root) = workspace();
    fs::write(root.join("kept.txt"), "").unwrap();
    let provider = FileMentions::new();
    let place = context(Some(&root), Vec::new(), false);

    let before = keys(&provider, &place, "kept").await;
    drop(folder);
    tokio::time::advance(INDEX_LIFETIME).await;
    let past = keys(&provider, &place, "kept").await;
    provider.walked(&root).await;
    let after = provider.suggest(&place, "kept", 50).await;

    assert_eq!(before, ["kept.txt"]);
    assert_eq!(past, ["kept.txt"]);
    assert_eq!(after.unwrap_err(), "folder_unreadable");
}

#[tokio::test]
async fn a_walk_that_another_walk_of_its_workspace_replaced_keeps_nothing() {
    let (_folder, root) = workspace();
    fs::write(root.join("a.txt"), "").unwrap();
    let provider = FileMentions::new();
    keys(&provider, &context(Some(&root), Vec::new(), false), "a").await;
    let replacing: Building = futures_util::future::pending().boxed().shared();
    lock(&provider.indexes)[0].building = Some((7, replacing));

    keep(
        &provider.indexes,
        &root,
        0,
        Instant::now(),
        &Err("folder_unreadable".to_owned()),
    );

    let indexes = lock(&provider.indexes);
    assert!(indexes[0].built.is_some());
    assert!(matches!(indexes[0].building, Some((7, _))));
}

#[tokio::test(start_paused = true)]
async fn the_indexes_of_the_workspaces_used_longest_ago_are_dropped() {
    let folders: Vec<(tempfile::TempDir, PathBuf)> =
        (0..=MAX_INDEXES).map(|_number| workspace()).collect();
    let provider = FileMentions::new();

    for (_folder, root) in &folders {
        fs::write(root.join("a.txt"), "").unwrap();
        keys(&provider, &context(Some(root), Vec::new(), false), "a").await;
        tokio::time::advance(std::time::Duration::from_millis(1)).await;
    }

    let kept: Vec<PathBuf> = lock(&provider.indexes)
        .iter()
        .map(|workspace| workspace.root.clone())
        .collect();
    assert_eq!(kept.len(), MAX_INDEXES);
    assert!(!kept.contains(&folders[0].1), "{kept:?}");
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
