# File transfer

|              |                                                                                                                              |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------- |
| Status       | Idea                                                                                                                         |
| Milestone    | Unscheduled                                                                                                                  |
| Last updated | 2026-10-20                                                                                                                   |
| Part of      | [SSH hosts](README.md)                                                                                                       |
| Related      | [ADR-0036](../../../docs/adr/0036-let-plugins-run-commands-on-remote-hosts.md), [Tools](../../../docs/architecture/tools.md) |

## Intent

The owner deferred file transfer from the first version ([D35](../../../docs/roadmap/milestones.md#5-decisions)). Agents would copy files between the master's workspace and a registered host over SFTP, such as a build's output to a server, without writing the file's content into a command.

## Open questions

- Which accesses a transfer names, so that the core's policy decides on the local path, with its mode, and on the remote path on its host. Each is an SDK change with an ADR, as ADR-0036 decision 2 says.
- Whether the owner's grants and rules for transfers follow the local ones, by folder, on the host's side too.
- The largest file that a transfer moves, and how a transfer shows its progress.
