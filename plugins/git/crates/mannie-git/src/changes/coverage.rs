//! The hooks that each change lists against the hooks that git runs for it
//! (plugins/git/docs/approvals.md, which hooks a change runs): every client
//! hook records itself, each tool's preparation lists the hooks of its
//! run, and every hook that ran must be among them, or the change would run
//! the repository's code without the owner's word (T-44).

use mannie_git_testing::TestRepository;
use serde_json::{Value, json};

use crate::changes::ChangeTool;
use crate::changes::testing::prepare;
use crate::tools::apply::{GitCherryPick, GitRevert};
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
use crate::tools::restore::GitRestore;
use crate::tools::stage::GitStage;
use crate::tools::stash::GitStash;
use crate::tools::switch::GitSwitch;

/// Runs a call of `T`, and checks that it succeeded and that every hook
/// that git started was listed. Returns the result's text.
async fn listed<T: ChangeTool>(
    repository: &TestRepository,
    step: &str,
    arguments: Value,
) -> String {
    repository.hooks_ran();
    let prepared = prepare::<T>(repository, arguments).await;
    let listed = prepared.folder_code();
    let result = prepared.execute().await;
    assert_eq!(result.failure, None, "{step}: {}", result.output);
    let ran = repository.hooks_ran();
    let unlisted: Vec<&String> = ran.iter().filter(|hook| !listed.contains(hook)).collect();
    assert!(
        unlisted.is_empty(),
        "{step} ran hooks that it did not list: {unlisted:?} (listed {listed:?})"
    );
    result.output
}

/// Stages, commits, branches and switches, then restores, stashes and
/// resets. Leaves `main` with one commit and `feature` one ahead of it.
async fn local_changes(repository: &TestRepository) {
    repository.write("a.txt", "one\n");
    listed::<GitStage>(repository, "stage", json!({ "paths": ["a.txt"] })).await;
    listed::<GitCommit>(repository, "commit", json!({ "message": "Start" })).await;
    let create = json!({ "action": "create", "name": "feature" });
    listed::<GitBranch>(repository, "branch", create).await;
    listed::<GitSwitch>(repository, "switch", json!({ "target": "feature" })).await;
    repository.write("b.txt", "feature\n");
    listed::<GitStage>(repository, "stage all", json!({ "all": true })).await;
    listed::<GitCommit>(
        repository,
        "commit on feature",
        json!({ "message": "Add b" }),
    )
    .await;
    listed::<GitSwitch>(repository, "switch back", json!({ "target": "main" })).await;

    repository.write("a.txt", "changed\n");
    listed::<GitRestore>(repository, "restore", json!({ "paths": ["a.txt"] })).await;
    repository.write("a.txt", "stashed\n");
    let push = json!({ "action": "push", "message": "saved" });
    listed::<GitStash>(repository, "stash push", push).await;
    listed::<GitStash>(repository, "stash apply", json!({ "action": "apply" })).await;
    let hard = json!({ "target": "HEAD", "mode": "hard" });
    listed::<GitReset>(repository, "reset hard", hard).await;
    listed::<GitStash>(repository, "stash drop", json!({ "action": "drop" })).await;
}

/// A fast-forward, a reset, a merge commit and an amended merge.
async fn merges(repository: &TestRepository) {
    let fast_forward = json!({ "branch": "feature" });
    listed::<GitMerge>(repository, "merge fast-forward", fast_forward).await;
    let keep = json!({ "target": "HEAD~1", "mode": "keep" });
    listed::<GitReset>(repository, "reset keep", keep).await;
    repository.write("c.txt", "main\n");
    let empty = json!({ "message": "Add c", "all": false, "allow_empty": true });
    listed::<GitCommit>(repository, "commit all", empty).await;
    let no_fast_forward = json!({ "branch": "feature", "mode": "no_fast_forward" });
    listed::<GitMerge>(repository, "merge commit", no_fast_forward).await;
    let amend = json!({ "amend": true, "message": "Merge feature" });
    listed::<GitCommit>(repository, "amend", amend).await;
}

/// A merge that stops on a conflict, a resolution and a continue.
async fn a_merge_that_stops(repository: &TestRepository) {
    let left = json!({ "target": "left", "create": true });
    listed::<GitSwitch>(repository, "switch new", left).await;
    repository.write("a.txt", "left\n");
    let commit_left = json!({ "message": "Left", "all": true });
    listed::<GitCommit>(repository, "commit left", commit_left).await;
    let right = json!({ "target": "right", "create": true, "start": "main" });
    listed::<GitSwitch>(repository, "switch right", right).await;
    repository.write("a.txt", "right\n");
    let commit_right = json!({ "message": "Right", "all": true });
    listed::<GitCommit>(repository, "commit right", commit_right).await;

    let stopped =
        listed::<GitMerge>(repository, "merge stopping", json!({ "branch": "left" })).await;
    assert!(stopped.contains("Conflicted files (1):"), "{stopped}");
    let take = json!({ "path": "a.txt", "take": "incoming" });
    listed::<GitResolve>(repository, "resolve", take).await;
    let continued = json!({ "action": "continue" });
    listed::<GitOperation>(repository, "continue merge", continued).await;
}

/// A rebase that stops and its abort, then a rebase that completes.
async fn rebases(repository: &TestRepository) {
    let ours = json!({ "target": "ours", "create": true, "start": "main" });
    listed::<GitSwitch>(repository, "switch ours", ours).await;
    repository.write("d.txt", "ours\n");
    listed::<GitStage>(repository, "stage d", json!({ "paths": ["d.txt"] })).await;
    listed::<GitCommit>(repository, "commit ours", json!({ "message": "Ours" })).await;
    let theirs = json!({ "target": "theirs", "create": true, "start": "main" });
    listed::<GitSwitch>(repository, "switch theirs", theirs).await;
    repository.write("d.txt", "theirs\n");
    listed::<GitStage>(repository, "stage d again", json!({ "paths": ["d.txt"] })).await;
    listed::<GitCommit>(repository, "commit theirs", json!({ "message": "Theirs" })).await;

    let rebasing =
        listed::<GitRebase>(repository, "rebase stopping", json!({ "onto": "ours" })).await;
    assert!(rebasing.contains("A rebase"), "{rebasing}");
    listed::<GitOperation>(repository, "abort rebase", json!({ "action": "abort" })).await;
    let onto_right = json!({ "onto": "right", "upstream": "main" });
    listed::<GitRebase>(repository, "rebase", onto_right).await;
}

/// A cherry-pick, a revert, an edit of the history and a deleted branch.
async fn picks_and_edits(repository: &TestRepository) {
    let extra = json!({ "target": "extra", "create": true, "start": "main" });
    listed::<GitSwitch>(repository, "switch extra", extra).await;
    repository.write("e.txt", "extra\n");
    listed::<GitStage>(repository, "stage e", json!({ "paths": ["e.txt"] })).await;
    listed::<GitCommit>(repository, "commit extra", json!({ "message": "Extra" })).await;
    listed::<GitSwitch>(
        repository,
        "switch to theirs",
        json!({ "target": "theirs" }),
    )
    .await;

    listed::<GitCherryPick>(repository, "cherry-pick", json!({ "commits": ["extra"] })).await;
    listed::<GitRevert>(repository, "revert", json!({ "commits": ["HEAD"] })).await;
    let edit = json!({
        "base": "HEAD~2",
        "steps": [
            { "commit": "HEAD~1", "action": "reword", "message": "Pick feature's b" },
            { "commit": "HEAD", "action": "pick" },
        ],
    });
    listed::<GitEditHistory>(repository, "edit history", edit).await;
    let delete = json!({ "action": "delete", "name": "feature" });
    listed::<GitBranch>(repository, "delete", delete).await;
}

/// A push, a fetch and a pull through a remote in a folder.
async fn transfers(repository: &TestRepository) {
    let remote = repository.sibling("remote.git");
    repository.git_in(&remote, &["init", "--quiet", "--bare"]);
    repository.git(&["remote", "add", "origin", remote.to_str().unwrap()]);
    listed::<GitPush>(repository, "push", json!({ "set_upstream": true })).await;
    listed::<GitFetch>(repository, "fetch", json!({ "prune": true })).await;
    listed::<GitFetch>(repository, "pull", json!({ "integrate": "rebase" })).await;
}

#[tokio::test]
async fn every_change_lists_the_hooks_that_git_runs_for_it() {
    let repository = TestRepository::new();
    repository.record_hooks();

    local_changes(&repository).await;
    merges(&repository).await;
    a_merge_that_stops(&repository).await;
    rebases(&repository).await;
    picks_and_edits(&repository).await;
    transfers(&repository).await;
}
