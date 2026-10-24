# Requests of servers

|              |                                                                                                                                       |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Idea                                                                                                                                  |
| Milestone    | Unscheduled                                                                                                                           |
| Last updated | 2026-10-24                                                                                                                            |
| Part of      | [MCP servers](README.md)                                                                                                              |
| Related      | [ADR-0043](../../../docs/adr/0043-let-plugins-offer-the-tools-of-tool-servers.md), [Security](../../../docs/architecture/security.md) |

## Intent

An MCP server can ask its client for things: elicitation asks the owner a question during a call, sampling asks the client's model to complete a text, roots asks which folders the server may work in, and log messages report what the server does. Version 1 declares none of these capabilities, so a server never asks the owner anything through Mannie, and a call that asks for input fails with a note ([D42](../../../docs/roadmap/milestones.md#5-decisions)).

## Open questions

- How elicitation shows the server's question so that the owner can tell it apart from Mannie's own approvals, and what a server may ask for: plain answers only, never secrets.
- Whether sampling runs on the conversation's profile, under its budget, and whether every request asks the owner first.
- Which folders roots names: the conversation's project, or folders that the owner picks for each server.
- Whether the servers' log messages join the log of Show log, by their level.
