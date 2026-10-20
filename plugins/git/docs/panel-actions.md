# Git panel actions

|              |                                                                                                                                                                                |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Idea                                                                                                                                                                           |
| Milestone    | Unscheduled                                                                                                                                                                    |
| Last updated | 2026-10-20                                                                                                                                                                     |
| Part of      | [Git](README.md)                                                                                                                                                               |
| Related      | [Git panel](panel.md), [ADR-0038](../../../docs/adr/0038-let-plugins-run-installed-programs-in-the-owners-projects.md), [D37](../../../docs/roadmap/milestones.md#5-decisions) |

## Intent

The owner kept the Git side panel to reading in the first version ([D37](../../../docs/roadmap/milestones.md#5-decisions)). The owner would stage and unstage files, commit, and continue or abort an operation that stopped from the panel, without asking an agent.

## Open questions

- ADR-0038 decision 10 lets a plugin's commands only read. A command that changes a repository needs an ADR of its own: how the owner's own action is journaled, and whether it asks where an agent's change would, such as before running hooks.
- Whether a commit from the panel opens a dialog for the message, and how it shows the hooks that will run.
- How the panel shows an agent's change that runs while the owner acts in it.
