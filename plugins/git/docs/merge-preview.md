# Merge preview

|              |                                                                                                 |
| ------------ | ----------------------------------------------------------------------------------------------- |
| Status       | Idea                                                                                            |
| Milestone    | Unscheduled                                                                                     |
| Last updated | 2026-10-20                                                                                      |
| Part of      | [Git](README.md)                                                                                |
| Related      | [Conflicts and history](conflicts-and-history.md), [Approvals](approvals.md), [Tools](tools.md) |

## Intent

An agent would learn whether a merge, a rebase or a cherry-pick would conflict, and in which files, before it starts one, so it can choose another order or warn the owner.

## Open questions

- `git merge-tree --write-tree` answers the question without touching the index or the work tree, but it writes objects into the repository, so it cannot run as a read that never asks. A preview would be a change that git can recover from, or the objects would go to a temporary object folder through `GIT_OBJECT_DIRECTORY` with the repository's objects as alternates, which needs a check that hooks and filters stay out.
- How a preview of a rebase reports the step that would stop, since each commit replays onto the result of the one before.
