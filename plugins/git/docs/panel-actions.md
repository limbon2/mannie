# Git panel actions

|              |                                                                                                                                                                                                                                                                            |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Shipped                                                                                                                                                                                                                                                                    |
| Plugin       | `git`                                                                                                                                                                                                                                                                      |
| Milestone    | M9                                                                                                                                                                                                                                                                         |
| Last updated | 2026-11-22                                                                                                                                                                                                                                                                 |
| Part of      | [Git](README.md)                                                                                                                                                                                                                                                           |
| Related      | [Git panel](panel.md), [ADR-0038](../../../docs/adr/0038-let-plugins-run-installed-programs-in-the-owners-projects.md), [ADR-0078](../../../docs/adr/0078-let-plugins-run-installed-programs-for-owner-actions.md), [D72](../../../docs/roadmap/milestones.md#5-decisions) |

## Intent

Let the owner stage, unstage, discard, commit, switch and manage branches and tags, push and pop stashes, fetch/pull/push, merge, rebase, cherry-pick, revert, and resolve or abort conflicted operations directly from the Git side panel without asking an agent.

## Behavior

- Actions supported from the panel:
  - Working tree and index: stage or unstage files or all changes, stage/unstage/discard individual diff hunks, discard changes to files (saving a recovery stash snapshot first when tracked changes are discarded), and commit or amend staged changes.
  - Branches and tags: switch to a branch or commit, create, rename or delete a branch, and create or delete a tag.
  - Stashes: push a stash (with optional message and untracked files), apply, pop or drop a stash.
  - Remotes: fetch a remote, pull from upstream (merge or rebase), and push the current branch (with optional upstream setup or force-with-lease).
  - History and conflicts: merge, rebase, cherry-pick, revert, resolve a conflicted file or individual hunks, mark a file resolved, and continue, skip or abort an in-progress operation.
- Server-enforced confirmation before dangerous or repository-code-executing actions ([ADR-0078](../../../docs/adr/0078-let-plugins-run-installed-programs-for-owner-actions.md)):
  - Safe local actions (staging, unstaging, switching a clean work tree, creating a branch or tag, resolving a conflict, fetching) execute immediately when clicked.
  - Any action that runs repository or global Git hooks, runs programs named in the repository's own Git configuration, discards uncommitted work or drops a stash, rewrites commits held by a remote-tracking branch, or deletes a branch or tag returns `needs_confirmation` with the exact list of effects, a repository state token and an `effects_digest`.
  - The panel opens a confirmation dialog listing every hook, configured program or destructive effect. Submitting calls `repository_action_run` again with the confirmation token. If the repository changed in the meantime, the server refuses with `repository_changed` so the owner reviews the updated effects before confirming again.
- Concurrency with agents:
  - Owner actions acquire the same per-repository `RepositoryLocks` as agent tools and notify `follow_repository` on completion, so panel actions and agent tool calls serialize cleanly and refresh the UI together.

## Design

- Commands: `repository_action_preview` and `repository_action_run` in `plugins/git/crates/mannie-git/src/commands.rs`, implemented in `plugins/git/crates/mannie-git/src/panel/actions.rs`.
- Every executed owner action is journaled in `tools_program_operations` through `ProcessService::begin_owner_operation` and `finish_owner_operation` ([ADR-0078](../../../docs/adr/0078-let-plugins-run-installed-programs-for-owner-actions.md)), with `origin = 'owner_action'`, an owner-supplied `operation_id` for idempotency (`already_run` on replay), and crash recovery on startup (`uncertain` for unfinished runs).
- Folder boundary enforcement reuses the core's workspace policy checks (`write_not_allowed` when the repository's git folder lies outside the project without an existing project grant).

## Decisions

- 2026-11-22: accepted [ADR-0078](../../../docs/adr/0078-let-plugins-run-installed-programs-for-owner-actions.md) ([D72](../../../docs/roadmap/milestones.md#5-decisions)) to allow journaled owner-initiated installed-program actions from plugin views with server-enforced confirmation for hooks, repository-configured programs, discarded work, published rewrites, and branch/tag deletions.
