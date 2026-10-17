# Worker mentions

|              |                                                                                                                                                        |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Idea                                                                                                                                                   |
| Milestone    | Unscheduled                                                                                                                                            |
| Last updated | 2026-10-17                                                                                                                                             |
| Part of      | [Workers](README.md)                                                                                                                                   |
| Related      | [ADR-0032](../../adr/0032-let-features-and-plugins-add-kinds-of-mentions.md), [Mentions](../chat/mentions.md), [Host and ceiling](host-and-ceiling.md) |

## Intent

The owner may want to mention a worker in a message, so that the master knows which host the owner means (owner, 2026-10-17). It would be the Workers plugin's mention provider ([ADR-0032](../../adr/0032-let-features-and-plugins-add-kinds-of-mentions.md)), offered while the plugin is on, which lists the paired workers and gives the model a worker's name, purpose and host facts.
