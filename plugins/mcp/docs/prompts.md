# Prompts

|              |                                                                                                                                        |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Idea                                                                                                                                   |
| Milestone    | Unscheduled                                                                                                                            |
| Last updated | 2026-10-24                                                                                                                             |
| Part of      | [MCP servers](README.md)                                                                                                               |
| Related      | [ADR-0043](../../../docs/adr/0043-let-plugins-offer-the-tools-of-tool-servers.md), [Mentions](../../../docs/features/chat/mentions.md) |

## Intent

MCP servers offer prompts: messages with arguments that the owner picks, such as a server's "review this pull request". Version 1 leaves them out ([D42](../../../docs/roadmap/milestones.md#5-decisions)). The owner would pick a server's prompt in the message box, fill in its arguments, and send what the server returns as the message.

## Open questions

- Whether prompts show as a kind of mention after "@", as commands after "/", or in a menu of the message box.
- How a prompt's arguments are asked for: a small form, or the text after the prompt's name.
- Whether the server's text is sent as the owner's words, which the owner then sees before sending, or as content that grants nothing.
