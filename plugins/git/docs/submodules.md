# Submodules

|              |                                                                                                                         |
| ------------ | ----------------------------------------------------------------------------------------------------------------------- |
| Status       | Idea                                                                                                                    |
| Milestone    | Unscheduled                                                                                                             |
| Last updated | 2026-10-20                                                                                                              |
| Part of      | [Git](README.md)                                                                                                        |
| Related      | [Tools](tools.md), [Approvals](approvals.md), [run the owner's installed git](adr/0001-run-the-owners-installed-git.md) |

## Intent

The tools show a submodule only by the commit that the repository records for it, and never look inside one. Agents would read and change a submodule's own work tree, and update a submodule to the commit that the repository records.

## Open questions

- git runs inside a submodule with the submodule's own configuration, whose filters and file system monitor a read would have to blank as it does for the repository (T-44). Each submodule is a repository of its own, with its own git folder inside the repository's.
- Whether `git submodule update` counts as a network operation, since it may fetch, and how its endpoints are shown to the owner.
- Whether a tool takes the submodule as its `repository`, which already works for reading, or follows submodules from the top.
