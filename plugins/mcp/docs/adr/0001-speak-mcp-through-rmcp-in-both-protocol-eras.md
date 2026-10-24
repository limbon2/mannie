# ADR-0001: Speak MCP through rmcp in both protocol eras

|          |                                                                                                                                                                                                                                                                                                                                        |
| -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D42)                                                                                                                                                                                                                                                                                                                         |
| Date     | 2026-10-24                                                                                                                                                                                                                                                                                                                             |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                  |
| Related  | [MCP servers](../README.md), [ADR-0043](../../../../docs/adr/0043-let-plugins-offer-the-tools-of-tool-servers.md), [ADR-0017](../../../../docs/adr/0017-license-and-dependency-policy.md), [Technology stack](../../../../docs/architecture/technology-stack.md), [M12 validation results](../../../../docs/roadmap/m12-validation.md) |

## Context

- The plugin MCP is a client of MCP servers: local programs that speak MCP over their standard input and output, and remote servers that speak Streamable HTTP ([ADR-0043](../../../../docs/adr/0043-let-plugins-offer-the-tools-of-tool-servers.md)).
- MCP 2026-07-28 is stateless. It drops `initialize`, carries the protocol version and the capabilities in each request's metadata, and adds `server/discover`, `subscriptions/listen` for announcements and multi-round-trip requests. Most servers still speak 2025-11-25 or older, with the `initialize` handshake (verified 2026-10-24).
- rmcp 3.7.0 is the official Rust SDK of MCP (Apache-2.0, released 2026-10-22, rust-version 1.88). Its features `client`, `transport-async-rw`, `transport-streamable-http-client-reqwest`, `reqwest-tls-no-provider` and `auth` give a client over any reader and writer and over reqwest, with OAuth. Its defaults bring the server, `transport-child-process` starts processes itself, and `auth-client-credentials-jwt` brings aws-lc-rs (verified 2026-10-24).
- `ClientLifecycleMode::Auto` probes with `server/discover` and falls back to `initialize` when the server answers with an error of an earlier era or does not answer within 10 seconds, a timeout that cannot be changed (verified 2026-10-24).
- The spikes of step 0 passed cargo-deny on the four targets with these features, with one reqwest, ring and no aws-lc-rs or OpenSSL, and connected to the everything server of both eras through `npx`. The first start of an older server, while `npx` downloaded it, missed the probe's 10 seconds, and its late error ended the connection ([M12 validation results](../../../../docs/roadmap/m12-validation.md)).

## Decision

1. The plugin speaks MCP through rmcp, pinned at `=3.7.0` with `default-features = false` and the features `client`, `transport-async-rw`, `transport-streamable-http-client-reqwest`, `reqwest-tls-no-provider` and `auth`. Only this plugin may depend on rmcp and oauth2, which its manifest's `[crates]` records. An upgrade of rmcp is a change of its own that runs the tests against servers of both eras.
2. A local server's program runs in the core's supervisor through the service `local-servers` (ADR-0043, decision 4). rmcp reads and writes an in-memory pipe, and tasks of the plugin connect its other end to the program's output and input.
3. A remote server runs over the plugin's own reqwest client, with rustls on ring through the process's provider, the operating system's trust store, the system's proxy settings and no redirects, under the manifest's `[native] sockets`.
4. A server's first connection uses `ClientLifecycleMode::Auto`. The plugin stores the era that a connection found and starts the next one in it: `Initialize` for an earlier era, and `Discover` with 2026-07-28 for the current one. A connection that fails, other than a refused program, a sign-in, an unreachable URL or a timeout, is tried once more with `Auto`, so a server that `npx` was still downloading is not read as an earlier era.
5. Mannie declares no capabilities: no roots, no sampling and no elicitation. A server of the current era announces changes through a subscription, which the plugin opens when the server declares them; a server of an earlier era through its session, which rmcp's handler hears.
6. The plugin lists tools, resources and templates page by page itself, at most 32 pages of 30 seconds each, and keeps at most 512 tools, 500 resources and 100 templates, since rmcp's own listings follow a server's cursors without an end.
7. A call goes through `call_tool_once` within the server's timeout, without driving the rounds of a request that asks for input, which Mannie does not give, and without following a task that a server starts. Dropping the call's future makes rmcp tell the server that the call is cancelled.

## Consequences

- Positive: one implementation of both eras, Streamable HTTP and OAuth, which the protocol's own maintainers keep current, behind the plugin's connection modules. Local servers get the core's containment and environment.
- Negative: rmcp's API changes often, and the plugin pins it exactly and upgrades it on purpose. A server that announces nothing is listed again only when it connects.
- Risks: a server that answers neither era as rmcp expects fails to connect with `protocol`, which the owner's checks against real servers find (R33).

## Alternatives considered

| Option                                           | Why not chosen                                                                                                            |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------- |
| The plugin's own MCP client                      | Two protocol eras, Streamable HTTP and OAuth to maintain, when the official SDK can be pinned                             |
| rmcp's `transport-child-process`                 | It starts processes outside the core's supervisor, without containment, the environment of commands or a resolved program |
| `ClientLifecycleMode::Auto` for every connection | A slow server would wait for the probe's 10 seconds at every start                                                        |
| `ClientLifecycleMode::Initialize` only           | Servers of the current era answer `initialize` less and less                                                              |
| rmcp's `list_all_tools`                          | It follows a server's cursors without an end                                                                              |

## Validation

M12: the plugin's tests against servers of both eras over stdio and Streamable HTTP, and the owner's checks against real servers.
