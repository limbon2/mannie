# Hosting services

|              |                                                                                                                          |
| ------------ | ------------------------------------------------------------------------------------------------------------------------ |
| Status       | Idea                                                                                                                     |
| Milestone    | Unscheduled                                                                                                              |
| Last updated | 2026-10-20                                                                                                               |
| Part of      | [Git](README.md)                                                                                                         |
| Related      | [Plugins](../../../docs/features/plugins.md), [ADR-0030](../../../docs/adr/0030-split-mannie-into-a-core-and-plugins.md) |

## Intent

Agents would work with the services that host repositories: open and update pull requests, read and answer reviews, and read issues, on GitHub, GitLab and others.

## Open questions

- Whether it is a plugin of its own, since it speaks each service's HTTP API with the owner's tokens and needs no git, and a plugin never depends on another ([ADR-0030](../../../docs/adr/0030-split-mannie-into-a-core-and-plugins.md)).
- Where the tokens live: the master's credential store, as a provider's key does ([ADR-0010](../../../docs/adr/0010-keep-provider-credentials-on-the-master.md)).
- Which of its calls ask the owner, since posting a review or a comment publishes text under the owner's name.
