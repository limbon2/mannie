# Restored terminals

|              |                                                                                                                                                                                                     |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Idea                                                                                                                                                                                                |
| Milestone    | Unscheduled                                                                                                                                                                                         |
| Last updated | 2026-10-30                                                                                                                                                                                          |
| Part of      | [Terminals](README.md)                                                                                                                                                                              |
| Related      | [ADR-0053](../../../docs/adr/0053-let-plugins-offer-terminals.md), [keep a headless emulator](adr/0002-keep-a-headless-emulator-of-each-terminal-in-rust.md), [SSH hosts](../../ssh/docs/README.md) |

## Intent

Terminals live in memory, and none comes back after Mannie restarts ([Terminals](README.md#decisions)). A conversation's terminals would open again after a restart, on the same hosts, with the same shells and folders, so that the owner finds them where they were.

## Open questions

- Whether a restored terminal shows its earlier scrollback, which would store a terminal's output beyond what reaches the model, and for how long.
- Whether a terminal on an SSH host logs in again by itself, or waits for the owner, since opening a terminal on a host is the owner's action today.
- How the panel marks where the earlier session ended, so that nobody takes old output for new, and what it says about the commands that were running when Mannie quit and ended with it.
