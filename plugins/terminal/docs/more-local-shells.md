# More local shells

|              |                                                                                                                                                                                                                          |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Idea                                                                                                                                                                                                                     |
| Milestone    | Unscheduled                                                                                                                                                                                                              |
| Last updated | 2026-10-30                                                                                                                                                                                                               |
| Part of      | [Terminals](README.md)                                                                                                                                                                                                   |
| Related      | [ADR-0053](../../../docs/adr/0053-let-plugins-offer-terminals.md), [mark commands with shell integration](adr/0001-mark-commands-with-shell-integration.md), [Commands](../../../docs/features/system-tools/commands.md) |

## Intent

A terminal on this machine runs a shell that the core lists: on Windows, PowerShell 7, Windows PowerShell, Command Prompt and Git Bash, and on Linux, the account's login shell and the shells of `/etc/shells` ([ADR-0053](../../../docs/adr/0053-let-plugins-offer-terminals.md), decision 2). The owner would also open the shells of WSL's distributions on Windows, and shells installed where Mannie does not look.

## Open questions

- How Mannie lists WSL's distributions without starting a process, since the list of shells comes from checks of files.
- Whether the supervisor can contain what a shell of WSL starts, since its Linux processes run outside the Windows process tree, and whether such a shell counts as this machine for grants and rules.
- Which setups other shells need to mark their commands ([mark commands with shell integration](adr/0001-mark-commands-with-shell-integration.md)).
- Whether the owner may add a shell by its path in the settings, and how the core then checks the path, as it checks programs on the `PATH`.
