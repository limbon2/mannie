# Summaries and notes

|              |                                                                                                                            |
| ------------ | -------------------------------------------------------------------------------------------------------------------------- |
| Status       | Idea                                                                                                                       |
| Milestone    | Unscheduled                                                                                                                |
| Last updated | 2026-10-24                                                                                                                 |
| Part of      | [Notebooks](README.md)                                                                                                     |
| Related      | [ADR-0034](../../../docs/adr/0034-build-plugins-on-a-plugin-sdk.md), [Providers](../../../docs/features/chat/providers.md) |

## Intent

NotebookLM writes a guide of each source when it is added, suggests questions, and keeps the owner's notes and saved answers in the notebook. Mannie's notebooks have none of these in version 1, since a plugin cannot call a chat model.

## Open questions

- Which service of the SDK lets a plugin call a chat model, with which profile, and how the owner sees what that costs.
- Whether a saved answer becomes a source of its notebook, so that later searches find it.
- Where notes show: on the notebook's page, in the sidebar, or in a side panel.
