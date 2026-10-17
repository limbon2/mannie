# Conversation mentions

|              |                                                                                                                                          |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Idea                                                                                                                                     |
| Milestone    | Unscheduled                                                                                                                              |
| Last updated | 2026-10-17                                                                                                                               |
| Part of      | [Chat](README.md)                                                                                                                        |
| Related      | [ADR-0032](../../adr/0032-let-features-and-plugins-add-kinds-of-mentions.md), [Mentions](mentions.md), [Conversations](conversations.md) |

## Intent

The owner may want to mention a sibling conversation in a message, so that the model knows what was said there (owner, 2026-10-17). It would be a mention provider of chat ([ADR-0032](../../adr/0032-let-features-and-plugins-add-kinds-of-mentions.md)) that lists the owner's other conversations, those of the same project first. What the model reads of a conversation within a mention's share, such as its checkpoint or its latest turns, is still open.
