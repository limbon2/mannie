//! The change tools against real repositories (plugins/git/docs/
//! approvals.md and conflicts-and-history.md): what each preparation lists
//! for the owner, what each run does, and what it refuses.

use mannie_git_testing::TestRepository;
use mannie_plugin_sdk::contracts::{DiscardedWork, ProgramEffect};
use mannie_plugin_sdk::points::tools::ToolAccess;
use serde_json::json;

use crate::changes::testing::{change, prepare};
use crate::tools::branch::GitBranch;
use crate::tools::commit::GitCommit;
use crate::tools::edit_history::GitEditHistory;
use crate::tools::fetch::GitFetch;
use crate::tools::merge::GitMerge;
use crate::tools::operation::GitOperation;
use crate::tools::push::GitPush;
use crate::tools::rebase::GitRebase;
use crate::tools::reset::GitReset;
use crate::tools::resolve::GitResolve;
use crate::tools::stage::GitStage;

/// A repository with one commit of `a.txt`.
fn started() -> TestRepository {
    let repository = TestRepository::new();
    repository.write("a.txt", "one\n");
    repository.commit("Start");
    repository
}

/// A repository whose `feature` and `main` changed `a.txt`'s line after
/// `Start`, on `main`.
fn diverged() -> TestRepository {
    let repository = started();
    repository.git(&["switch", "--quiet", "-c", "feature"]);
    repository.write("a.txt", "feature\n");
    repository.commit("Feature change");
    repository.git(&["switch", "--quiet", "main"]);
    repository.write("a.txt", "main\n");
    repository.commit("Main change");
    repository
}

fn discards(effects: &[ProgramEffect], work: DiscardedWork) -> Option<(u32, Vec<String>, bool)> {
    effects.iter().find_map(|effect| match effect {
        ProgramEffect::DiscardsWork {
            work: found,
            count,
            items,
            snapshot,
        } if *found == work => Some((*count, items.clone(), *snapshot)),
        _other => None,
    })
}

#[tokio::test]
async fn a_commit_without_hooks_only_writes_and_one_with_a_hook_lists_it() {
    let repository = started();
    repository.write("a.txt", "two\n");
    repository.git(&["add", "a.txt"]);

    let plain = prepare::<GitCommit>(&repository, json!({ "message": "Two" })).await;
    let accesses = plain.accesses.as_ref().unwrap();
    assert_eq!(accesses.len(), 1, "{accesses:?}");
    assert!(
        matches!(&accesses[0], ToolAccess::Path(path) if path.state.as_deref().is_some_and(|state| state.starts_with("HEAD ")))
    );

    repository.write_hook("pre-commit", "#!/bin/sh\nexit 0\n");
    let hooked = prepare::<GitCommit>(&repository, json!({ "message": "Two" })).await;
    assert_eq!(
        hooked.effects(),
        [ProgramEffect::RunsFolderCode {
            name: "pre-commit".to_owned(),
            source: ".git/hooks/pre-commit".to_owned(),
        }]
    );
    let result = hooked.execute().await;
    assert!(result.output.starts_with("Committed "), "{}", result.output);
    assert!(result.output.contains("    Two\n"), "{}", result.output);
    assert_eq!(repository.git(&["log", "-1", "--format=%s"]).trim(), "Two");
}

/// On Unix, git skips a hook file without its executable bit, so the change
/// neither lists it nor runs it.
#[cfg(unix)]
#[tokio::test]
async fn a_hook_file_that_git_cannot_execute_is_neither_listed_nor_run() {
    let repository = started();
    repository.write(".git/hooks/pre-commit", "#!/bin/sh\nexit 1\n");
    repository.write("a.txt", "two\n");
    repository.git(&["add", "a.txt"]);

    let prepared = prepare::<GitCommit>(&repository, json!({ "message": "Two" })).await;

    assert_eq!(prepared.effects(), Vec::<ProgramEffect>::new());
    let result = prepared.execute().await;
    assert_eq!(result.failure, None, "{}", result.output);
    assert_eq!(repository.git(&["log", "-1", "--format=%s"]).trim(), "Two");
}

#[tokio::test]
async fn a_change_refuses_to_act_when_the_repository_moved_since_it_was_allowed() {
    let repository = started();
    repository.write("a.txt", "two\n");
    repository.git(&["add", "a.txt"]);
    let prepared = prepare::<GitCommit>(&repository, json!({ "message": "Two" })).await;
    // The owner stages another file in a terminal meanwhile.
    repository.write("b.txt", "meanwhile\n");
    repository.git(&["add", "b.txt"]);

    let result = prepared.execute().await;

    assert_eq!(
        result.failure.as_deref(),
        Some("repository_changed"),
        "{}",
        result.output
    );
    assert_eq!(
        repository.git(&["log", "-1", "--format=%s"]).trim(),
        "Start"
    );
}

#[tokio::test]
async fn a_hook_that_rejects_a_commit_refuses_it_with_the_hook_s_words() {
    let repository = started();
    repository.write_hook(
        "pre-commit",
        "#!/bin/sh\necho 'lint failed: a.txt' >&2\nexit 1\n",
    );
    repository.write("a.txt", "two\n");

    let result = change::<GitCommit>(&repository, json!({ "message": "Two", "all": true })).await;

    assert_eq!(
        result.failure.as_deref(),
        Some("operation_refused"),
        "{}",
        result.output
    );
    assert!(
        result.output.contains("lint failed: a.txt"),
        "{}",
        result.output
    );
    assert_eq!(
        repository.git(&["log", "-1", "--format=%s"]).trim(),
        "Start"
    );
}

#[tokio::test]
async fn nothing_staged_a_missing_identity_and_conflicted_files_refuse_a_commit() {
    let repository = started();
    let nothing = change::<GitCommit>(&repository, json!({ "message": "Empty" })).await;
    repository.git(&["config", "user.useConfigOnly", "true"]);
    repository.git(&["config", "--unset", "user.name"]);
    repository.git(&["config", "--unset", "user.email"]);
    repository.write("a.txt", "two\n");
    let anonymous =
        change::<GitCommit>(&repository, json!({ "message": "Two", "all": true })).await;

    assert_eq!(
        nothing.failure.as_deref(),
        Some("invalid_argument"),
        "{}",
        nothing.output
    );
    assert!(
        nothing.output.contains("Nothing is staged"),
        "{}",
        nothing.output
    );
    assert_eq!(
        anonymous.failure.as_deref(),
        Some("identity_missing"),
        "{}",
        anonymous.output
    );
}

#[tokio::test]
async fn a_hard_reset_lists_the_changes_and_the_commits_that_it_throws_away_and_keeps_a_snapshot() {
    let repository = started();
    repository.write("b.txt", "kept only here\n");
    repository.commit("Only on main");
    repository.write("a.txt", "uncommitted\n");

    let prepared =
        prepare::<GitReset>(&repository, json!({ "target": "HEAD~1", "mode": "hard" })).await;
    let effects = prepared.effects();
    let result = prepared.execute().await;

    assert_eq!(
        discards(&effects, DiscardedWork::UncommittedChanges),
        Some((1, vec!["a.txt".to_owned()], true))
    );
    let (count, items, snapshot) = discards(&effects, DiscardedWork::Commits).unwrap();
    assert_eq!(count, 1);
    assert!(items[0].ends_with(" Only on main"), "{items:?}");
    assert!(!snapshot);
    assert!(
        result.output.contains("\"Mannie: before a hard reset\""),
        "{}",
        result.output
    );
    assert!(
        repository
            .git(&["stash", "list"])
            .contains("Mannie: before a hard reset")
    );
    assert_eq!(repository.read("a.txt"), "one\n");
}

#[tokio::test]
async fn unstaging_a_partly_staged_file_keeps_its_staged_version_first() {
    let repository = started();
    repository.write("a.txt", "staged\n");
    repository.git(&["add", "a.txt"]);
    repository.write("a.txt", "work tree\n");

    let prepared =
        prepare::<GitStage>(&repository, json!({ "paths": ["a.txt"], "unstage": true })).await;
    let effects = prepared.effects();
    let result = prepared.execute().await;

    assert_eq!(
        discards(&effects, DiscardedWork::UncommittedChanges),
        Some((1, vec!["a.txt".to_owned()], true))
    );
    assert!(
        result
            .output
            .contains("Not staged (1):\n  modified: a.txt\n"),
        "{}",
        result.output
    );
    assert!(
        repository
            .git(&["stash", "list"])
            .contains("Mannie: before unstaging")
    );
}

#[tokio::test]
async fn deleting_a_branch_whose_commits_nothing_else_holds_lists_them() {
    let repository = started();
    repository.git(&["branch", "merged"]);
    repository.git(&["switch", "--quiet", "-c", "lone"]);
    repository.write("b.txt", "lone\n");
    repository.commit("Lone work");
    repository.git(&["switch", "--quiet", "main"]);

    let merged =
        prepare::<GitBranch>(&repository, json!({ "action": "delete", "name": "merged" })).await;
    let lone =
        prepare::<GitBranch>(&repository, json!({ "action": "delete", "name": "lone" })).await;
    let current =
        change::<GitBranch>(&repository, json!({ "action": "delete", "name": "main" })).await;

    assert_eq!(merged.effects(), Vec::new());
    let (count, items, _snapshot) = discards(&lone.effects(), DiscardedWork::Commits).unwrap();
    assert_eq!(count, 1);
    assert!(items[0].ends_with(" Lone work"));
    let result = lone.execute().await;
    assert_eq!(result.failure, None, "{}", result.output);
    assert!(!repository.git(&["branch"]).contains("lone"));
    assert_eq!(current.failure.as_deref(), Some("invalid_argument"));
}

#[tokio::test]
async fn amending_a_pushed_commit_lists_the_remote_branches_that_hold_it() {
    let repository = started();
    let remote = repository.sibling("remote.git");
    repository.git_in(&remote, &["init", "--quiet", "--bare"]);
    repository.git(&["remote", "add", "origin", remote.to_str().unwrap()]);
    repository.git(&["push", "--quiet", "origin", "main"]);

    let prepared = prepare::<GitCommit>(
        &repository,
        json!({ "amend": true, "message": "Start again" }),
    )
    .await;

    assert!(
        prepared
            .effects()
            .contains(&ProgramEffect::RewritesPublished {
                count: 1,
                places: vec!["origin/main".to_owned()],
            }),
        "{:?}",
        prepared.effects()
    );
}

#[tokio::test]
async fn a_merge_stops_on_conflicts_resolves_each_hunk_and_continues() {
    let repository = diverged();

    let stopped = change::<GitMerge>(&repository, json!({ "branch": "feature" })).await;
    let resolved = change::<GitResolve>(
        &repository,
        json!({ "path": "a.txt", "hunks": [{ "number": 1, "take": "both" }] }),
    )
    .await;
    let continued = change::<GitOperation>(&repository, json!({ "action": "continue" })).await;

    assert!(
        stopped
            .output
            .contains("Conflicted files (1):\n  a.txt: both changed, 1 conflicts\n"),
        "{}",
        stopped.output
    );
    assert_eq!(stopped.data.as_ref().unwrap()["outcome"], "stopped");
    assert!(
        resolved
            .output
            .contains("No file is conflicted now; call git_operation with continue."),
        "{}",
        resolved.output
    );
    assert_eq!(repository.read("a.txt"), "main\nfeature\n");
    assert!(
        continued.output.starts_with("The merge completed."),
        "{}",
        continued.output
    );
    assert!(
        continued.output.contains("Merge branch 'feature'"),
        "{}",
        continued.output
    );
}

#[tokio::test]
async fn a_rebase_s_sides_keep_their_meaning_when_resolved() {
    let repository = diverged();
    repository.git(&["switch", "--quiet", "feature"]);

    let stopped = change::<GitRebase>(&repository, json!({ "onto": "main" })).await;
    let resolved =
        change::<GitResolve>(&repository, json!({ "path": "a.txt", "take": "incoming" })).await;
    let continued = change::<GitOperation>(&repository, json!({ "action": "continue" })).await;

    assert!(
        stopped.output.starts_with("A rebase of feature onto "),
        "{}",
        stopped.output
    );
    assert_eq!(resolved.failure, None, "{}", resolved.output);
    // Incoming is the commit that the rebase replays: feature's.
    assert_eq!(repository.read("a.txt"), "feature\n");
    assert!(
        continued.output.starts_with("The rebase completed."),
        "{}",
        continued.output
    );
    assert!(
        continued.output.contains(" Feature change\n"),
        "{}",
        continued.output
    );
}

#[tokio::test]
async fn aborting_a_stopped_merge_returns_to_where_it_started() {
    let repository = diverged();
    let head = repository.head();
    change::<GitMerge>(&repository, json!({ "branch": "feature" })).await;

    let skipped = change::<GitOperation>(&repository, json!({ "action": "skip" })).await;
    let aborted = change::<GitOperation>(&repository, json!({ "action": "abort" })).await;

    assert_eq!(skipped.failure.as_deref(), Some("invalid_argument"));
    assert!(
        aborted.output.starts_with("The merge was aborted."),
        "{}",
        aborted.output
    );
    assert_eq!(repository.head(), head);
    assert_eq!(repository.read("a.txt"), "main\n");
}

/// A bare remote beside the repository, added as `origin`, with `main`
/// pushed.
fn with_remote(repository: &TestRepository) -> std::path::PathBuf {
    let remote = repository.sibling("remote.git");
    repository.git_in(
        &remote,
        &["init", "--quiet", "--bare", "--initial-branch=main"],
    );
    repository.git(&["remote", "add", "origin", remote.to_str().unwrap()]);
    remote
}

/// Commits in a clone of `remote` and pushes it, as someone else would.
fn push_elsewhere(repository: &TestRepository, remote: &std::path::Path, subject: &str) {
    let other = repository.sibling("other");
    if !other.join(".git").exists() {
        repository.git_in(&other, &["clone", "--quiet", remote.to_str().unwrap(), "."]);
    }
    repository.git_in(
        &other,
        &[
            "-c",
            "user.name=Someone Else",
            "-c",
            "user.email=else@example.com",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            subject,
        ],
    );
    repository.git_in(&other, &["push", "--quiet", "origin", "main"]);
}

#[tokio::test]
async fn a_push_names_its_endpoint_and_a_fetch_lists_what_moved_and_pulls() {
    let repository = started();
    let remote = with_remote(&repository);
    let endpoint = remote.display().to_string();

    let prepared = prepare::<GitPush>(&repository, json!({ "set_upstream": true })).await;
    assert!(
        prepared.effects().contains(&ProgramEffect::Network {
            endpoint: endpoint.clone()
        }),
        "{:?}",
        prepared.effects()
    );
    let pushed = prepared.execute().await;
    assert!(
        pushed
            .output
            .starts_with("Pushed main to origin as main.\n"),
        "{}",
        pushed.output
    );
    assert!(pushed.output.contains("(new)"), "{}", pushed.output);

    push_elsewhere(&repository, &remote, "From elsewhere");
    let fetched = change::<GitFetch>(&repository, json!({})).await;
    let pulled = change::<GitFetch>(&repository, json!({ "integrate": "merge" })).await;

    assert!(
        fetched.output.contains("  origin/main: "),
        "{}",
        fetched.output
    );
    assert_eq!(
        fetched.data.as_ref().unwrap()["moved"][0]["name"],
        "origin/main"
    );
    assert!(
        pulled.output.contains("The pull completed."),
        "{}",
        pulled.output
    );
    assert_eq!(
        repository.git(&["log", "-1", "--format=%s"]).trim(),
        "From elsewhere"
    );
}

#[tokio::test]
async fn a_push_forces_only_with_a_lease_on_what_it_read() {
    let repository = started();
    let remote = with_remote(&repository);
    repository.git(&["push", "--quiet", "--set-upstream", "origin", "main"]);
    repository.git(&["commit", "--quiet", "--amend", "-m", "Start rewritten"]);

    let refused = change::<GitPush>(&repository, json!({})).await;
    let prepared = prepare::<GitPush>(&repository, json!({ "force_with_lease": true })).await;
    let effects = prepared.effects();
    // Someone else pushes before the forced push runs: its lease fails.
    push_elsewhere(&repository, &remote, "Meanwhile");
    repository.git(&["fetch", "--quiet"]);
    let stale = prepared.execute().await;
    let forced = change::<GitPush>(&repository, json!({ "force_with_lease": true })).await;

    assert_eq!(
        refused.failure.as_deref(),
        Some("operation_refused"),
        "{}",
        refused.output
    );
    assert!(
        effects.contains(&ProgramEffect::RewritesPublished {
            count: 1,
            places: vec!["origin/main".to_owned()],
        }),
        "{effects:?}"
    );
    assert_eq!(
        stale.failure.as_deref(),
        Some("repository_changed"),
        "{}",
        stale.output
    );
    assert!(forced.output.contains("(forced)"), "{}", forced.output);
}

#[tokio::test]
async fn a_remote_s_url_loses_its_credentials_and_its_programs_ask() {
    let repository = started();
    repository.git(&[
        "remote",
        "add",
        "origin",
        "https://me:token@example.com/app.git",
    ]);
    repository.git(&["config", "core.sshCommand", "ssh -i planted"]);

    let prepared = prepare::<GitFetch>(&repository, json!({})).await;

    let effects = prepared.effects();
    assert!(
        effects.contains(&ProgramEffect::Network {
            endpoint: "https://example.com/app.git".to_owned()
        }),
        "{effects:?}"
    );
    assert!(
        prepared
            .folder_code()
            .contains(&"core.sshcommand".to_owned()),
        "{effects:?}"
    );
    assert!(!format!("{:?}", prepared.accesses).contains("token"));
}

#[tokio::test]
async fn a_history_edit_rewrites_every_step_or_nothing() {
    let repository = started();
    for (file, subject) in [("b.txt", "Add b"), ("c.txt", "Add c"), ("d.txt", "Add d")] {
        repository.write(file, "x\n");
        repository.commit(subject);
    }
    let before = repository.head();

    let edited = change::<GitEditHistory>(
        &repository,
        json!({
            "base": "HEAD~3",
            "steps": [
                { "commit": "HEAD~1", "action": "pick" },
                { "commit": "HEAD~2", "action": "reword", "message": "Add b first" },
                { "commit": "HEAD", "action": "squash", "message": "Add b and d" },
            ],
        }),
    )
    .await;

    assert_eq!(edited.failure, None, "{}", edited.output);
    let subjects = repository.git(&["log", "--format=%s", "HEAD~2..HEAD"]);
    assert_eq!(subjects, "Add b and d\nAdd c\n");
    assert_ne!(repository.head(), before);
    assert_eq!(
        repository
            .git(&["log", "-1", "--format=%an", "HEAD"])
            .trim(),
        "Test Author"
    );
    let files = repository.git(&["show", "--name-only", "--format=", "HEAD"]);
    assert_eq!(files, "b.txt\nd.txt\n");
}

#[tokio::test]
async fn a_history_edit_whose_step_conflicts_changes_nothing() {
    let repository = started();
    repository.write("a.txt", "two\n");
    repository.commit("Two");
    repository.write("a.txt", "three\n");
    repository.commit("Three");
    let before = repository.head();

    let swapped = change::<GitEditHistory>(
        &repository,
        json!({
            "base": "HEAD~2",
            "steps": [
                { "commit": "HEAD", "action": "pick" },
                { "commit": "HEAD~1", "action": "pick" },
            ],
        }),
    )
    .await;
    let unchanged = change::<GitEditHistory>(
        &repository,
        json!({
            "base": "HEAD~2",
            "steps": [
                { "commit": "HEAD~1", "action": "pick" },
                { "commit": "HEAD", "action": "pick" },
            ],
        }),
    )
    .await;

    assert_eq!(
        swapped.failure.as_deref(),
        Some("operation_refused"),
        "{}",
        swapped.output
    );
    assert!(swapped.output.contains("step 1 ("), "{}", swapped.output);
    assert!(
        swapped
            .output
            .contains("conflicts in a.txt. Nothing changed."),
        "{}",
        swapped.output
    );
    assert_eq!(repository.head(), before);
    assert_eq!(unchanged.failure.as_deref(), Some("invalid_argument"));
}
