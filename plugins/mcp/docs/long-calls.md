# Long calls and apps

|              |                                                                                                                                                                                                                          |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Idea                                                                                                                                                                                                                     |
| Milestone    | Unscheduled                                                                                                                                                                                                              |
| Last updated | 2026-10-24                                                                                                                                                                                                               |
| Part of      | [MCP servers](README.md)                                                                                                                                                                                                 |
| Related      | [Background commands](../../../docs/features/system-tools/background-commands.md), [ADR-0043](../../../docs/adr/0043-let-plugins-offer-the-tools-of-tool-servers.md), [Security](../../../docs/architecture/security.md) |

## Intent

The tasks extension of MCP lets a server answer a call with a task that runs on its own, which the client polls until it ends. MCP Apps let a tool return an interface that the client shows. Version 1 fails a call that becomes a task, with a note, and shows no app ([D42](../../../docs/roadmap/milestones.md#5-decisions)).

## Open questions

- Whether a server's task runs like a command in the background: listed with Stop, read with a tool, and waking the agent when it ends.
- Whether an app's interface could run in the webview at all, since the webview is not trusted and a server's HTML is content ([ADR-0004](../../../docs/adr/0004-keep-system-access-in-the-rust-core.md)), or only in a separate window without Mannie's bridge.
