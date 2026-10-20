# Git conflicts and history

|              |                                                                                 |
| ------------ | ------------------------------------------------------------------------------- |
| Status       | Building                                                                        |
| Plugin       | `git`                                                                           |
| Milestone    | M9                                                                              |
| Last updated | 2026-10-20                                                                      |
| Part of      | [Git](README.md)                                                                |
| Related      | [Tools](tools.md), [edit history in memory](adr/0002-edit-history-in-memory.md) |

## Intent

Let agents merge, rebase, cherry-pick and revert, resolve what conflicts, and edit the commits of a branch, without git's interactive editor and without leaving the repository in a state that the owner's own git cannot continue.

## Behavior

### Operations that can stop

- `git_merge`, `git_rebase`, `git_cherry_pick`, `git_revert`, `git_operation` with `continue`, and `git_fetch` with `integrate` run git's own operation and report one of:
  - completed: the new HEAD and the commits that the operation made;
  - stopped: the operation, its step, such as "3 of 7, applying abc1234 Fix parser", and the conflicted files with their kinds;
  - refused: the local changes that the operation would overwrite, which git lists; nothing changed.
- git's editor never opens: the operations keep git's default messages, and `git_commit` with `amend` changes a message afterwards.
- The state of an operation stays git's own, so `git status` in the owner's terminal shows it, and the owner can continue or abort it there.

### Reading conflicts

- `git_conflicts` names the operation in progress and its step, its current commit, what HEAD holds, and its incoming commit, what is being applied. In a rebase, current is the rebased series and incoming is the commit being replayed, which git calls "ours" and "theirs" the other way around; the tools never use git's names.
- Each conflicted file has its kind: both changed, both added, changed on one side and deleted on the other, or added on one side.
- For a file, the tool reads the markers in the work tree, so the hunks that the agent resolved already no longer show. Each hunk has its number, its first and last line in the file, and its current, base and incoming lines. Operations start with the `zdiff3` style, so a hunk has its base.

### Resolving

- `git_resolve` takes one of:
  - `take`: one side for the whole file, current, incoming, base, or both, current first;
  - `hunks`: a side, or a text, for each hunk by its number;
  - `content`: the whole file;
  - `deleted`: true to delete a file that one side deleted, false to keep it;
  - `edited`: the file as the agent edited it with `edit_file`, refused while conflict markers remain.
- The plugin writes the result through git, as a blob that becomes the file's entry in the index and is then checked out to the work tree, so line endings and filters follow the repository's attributes, and stages it.
- `git_operation` with `continue` goes on to the next step or completes; `skip` drops the current step's commit; `abort` returns to where the operation started.

### Editing history

- `git_edit_history` takes a base and the commits after it in a new order, each with an action: `pick`, `reword` with a new message, `squash` into the commit before it with a message for both, `fixup` into the commit before it keeping that commit's message, or `drop`. Every commit between the base and the tip appears once.
- It builds the new commits without touching the work tree and moves the branch only when every step succeeded. When a step conflicts, nothing changes, and the result names the step and the files ([ADR 0002](adr/0002-edit-history-in-memory.md)).
- It refuses merge commits in the range, an operation in progress, a branch checked out in another worktree, and staged changes when the result's files differ from HEAD's.
- Commits keep their authors and dates, and are signed when the owner's configuration signs commits. No `post-rewrite` hook runs.
- When an edit conflicts, the agent can drop a commit with `git_rebase` onto its parent, fold staged changes into an older commit with `git_commit`'s fixup and `git_rebase` with `autosquash`, or move commits with `git_cherry_pick`, and resolve the conflicts as above.

## Acceptance

- Conflicts of a merge, a rebase, a cherry-pick and a revert read with the right sides and commits, resolve with each kind of resolution, and continue to completion; skip and abort leave what git leaves.
- An operation refused for local changes changes nothing, and lists them.
- Each action of a history edit, a conflicting step that changes nothing, a branch that moved meanwhile, staged changes, and a signed repository.
