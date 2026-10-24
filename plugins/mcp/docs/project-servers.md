# Servers of a project

|              |                                                                                                                                                                                            |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Idea                                                                                                                                                                                       |
| Milestone    | Unscheduled                                                                                                                                                                                |
| Last updated | 2026-10-24                                                                                                                                                                                 |
| Part of      | [MCP servers](README.md)                                                                                                                                                                   |
| Related      | [Projects](../../../docs/features/chat/projects.md), [ADR-0043](../../../docs/adr/0043-let-plugins-offer-the-tools-of-tool-servers.md), [Security](../../../docs/architecture/security.md) |

## Intent

In version 1, every conversation gets the tools of every server that is on ([D42](../../../docs/roadmap/milestones.md#5-decisions)). Some servers belong to one project, such as a database of one application, and many repositories keep a `.mcp.json` that names the servers their developers use. The owner could give a server to some projects only, and add the servers that a repository's `.mcp.json` names.

## Open questions

- Whether a server of a project is offered only in the project's conversations, or also with its own approvals elsewhere.
- How Mannie treats a repository's `.mcp.json`: as a proposal that the owner reviews and adds with Paste JSON, never as servers that run when the repository opens, since its programs are the repository's code (T-63).
- Whether one process serves every conversation of a project, or one each.
