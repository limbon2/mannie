# More transports and sign-ins

|              |                                                                                                                                                            |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Idea                                                                                                                                                       |
| Milestone    | Unscheduled                                                                                                                                                |
| Last updated | 2026-10-24                                                                                                                                                 |
| Part of      | [MCP servers](README.md)                                                                                                                                   |
| Related      | [sign in with OAuth](adr/0002-sign-in-to-remote-servers-with-oauth.md), [speak MCP through rmcp](adr/0001-speak-mcp-through-rmcp-in-both-protocol-eras.md) |

## Intent

Version 1 speaks stdio and Streamable HTTP, and signs in with a client that the owner registered or that the authorization server registers ([D42](../../../docs/roadmap/milestones.md#5-decisions)). Some servers still speak only HTTP with server-sent events, the transport of 2024, and some authorization servers take only Client ID Metadata Documents, where the client's ID is the URL of a document that describes it.

## Open questions

- Whether servers of server-sent events are common enough to speak a deprecated transport, which rmcp no longer offers.
- Where Mannie would host its client's metadata document, and how a release would keep it current.
