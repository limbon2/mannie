# Mentions of sources

|              |                                                                                                                                           |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Idea                                                                                                                                      |
| Milestone    | Unscheduled                                                                                                                               |
| Last updated | 2026-10-24                                                                                                                                |
| Part of      | [Notebooks](README.md)                                                                                                                    |
| Related      | [ADR-0032](../../../docs/adr/0032-let-features-and-plugins-add-kinds-of-mentions.md), [Mentions](../../../docs/features/chat/mentions.md) |

## Intent

A short source, such as a page of notes, is better read whole than in passages. A mention after "@" would attach a source's whole text to one message, within the room that mentions share, whether or not its notebook is attached.

## Open questions

- Whether the list offers every source of every notebook, or those of the attached notebooks first.
- What the model reads of a source that does not fit the room: its start, or its best passages for the message.
