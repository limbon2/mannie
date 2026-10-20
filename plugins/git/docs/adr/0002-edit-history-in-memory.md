# ADR-0002: Edit history in memory, all or nothing

|          |                                                                                                                                                  |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status   | Accepted (D37)                                                                                                                                   |
| Date     | 2026-10-20                                                                                                                                       |
| Deciders | Owner                                                                                                                                            |
| Related  | [Git](../README.md), [Conflicts and history](../conflicts-and-history.md), [run the owner's installed git](0001-run-the-owners-installed-git.md) |

## Context

- Agents need to edit the commits of a branch: reword a message, squash or fix up commits, drop one, change their order ([D37](../../../../docs/roadmap/milestones.md#5-decisions)).
- git edits history interactively with `rebase -i`, which reads its todo list from an editor. git skips an editor whose command is exactly `:`, and runs any other editor command, and every `exec` line of a todo, through a shell (verified in git's `editor.c` on 2026-10-20). The plugin runs no shell (ADR-0038, decision 7), and a helper program that writes the todo would have to ship with the desktop app.
- `git replay` rewrites commits without touching the work tree, and is still marked experimental. `git merge-tree --write-tree --merge-base` merges in memory and exits 1 on a conflict (2.40).

## Decision

1. The tool `git_edit_history` takes a base and the new order of the commits between it and the branch's tip, each with an action: pick, reword with a new message, squash or fix up into the commit before it, or drop.
2. It refuses merge commits in the range, an operation in progress, a branch that another worktree has checked out, and staged changes when the result's tree differs from HEAD's.
3. It builds the new commits in memory: a commit whose parent stays keeps its object, and every other commit is cherry-picked with `merge-tree --write-tree --merge-base=<parent> <new parent> <commit>` and written with `commit-tree`, keeping its author's name, address and date, and signing with `-S` when `commit.gpgSign` says so, or `--no-gpg-sign`.
4. When a step conflicts, nothing changes: the result names the step and the conflicted files, and the objects that the attempt wrote stay unreferenced for git's garbage collection.
5. When every step succeeds, the branch moves with `update-ref` on its old value, so a branch that moved meanwhile is left alone, or with `reset --keep` when the result's tree differs from HEAD's, which refuses to overwrite local changes. No `post-rewrite` hook runs, and the documents say so.
6. An edit that conflicts goes through the operations that stop at conflicts instead: `git_rebase` onto the parent of a dropped commit, a fixup commit followed by `git_rebase` with autosquash, or `git_cherry_pick` onto a new branch.

## Consequences

- Positive: an edit is all or nothing and never leaves an operation half done, a reword, a squash of neighbors and a reorder that applies cleanly never touch the work tree, and no shell or helper program is needed.
- Negative: an edit that conflicts needs the agent to take another route, and `post-rewrite` hooks do not run for edits.
- Risks: a merge in memory and git's own sequencer can disagree in rare cases of renames; the tests compare the results of both on a corpus of edits.

## Alternatives considered

| Option                                 | Why not chosen                                                                                                 |
| -------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| `rebase -i` with a todo from an editor | The editor and `exec` lines run through a shell, or a helper program must ship with the app                    |
| `git replay`                           | Still experimental, and it neither squashes nor rewords                                                        |
| Cherry-picks on a detached HEAD        | They leave an operation in progress that the plugin would have to continue on its own, outside git's own state |

## Validation

M9: the plugin's tests of every action, of a conflicting step that changes nothing, of a branch that moved meanwhile, and of signing.
