# ADR-0002: Sign in to remote servers with OAuth through rmcp

|          |                                                                                                                                                                                                                                                                                                                  |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D42)                                                                                                                                                                                                                                                                                                   |
| Date     | 2026-10-24                                                                                                                                                                                                                                                                                                       |
| Deciders | Owner                                                                                                                                                                                                                                                                                                            |
| Related  | [MCP servers](../README.md), [ADR-0001](0001-speak-mcp-through-rmcp-in-both-protocol-eras.md), [ADR-0043](../../../../docs/adr/0043-let-plugins-offer-the-tools-of-tool-servers.md), [Security](../../../../docs/architecture/security.md), [M12 validation results](../../../../docs/roadmap/m12-validation.md) |

## Context

- Many remote MCP servers sign their users in with OAuth: the MCP authorization specification asks for the authorization code flow with PKCE, discovery through the server's protected resource metadata (RFC 9728) and the authorization server's metadata (RFC 8414), the `resource` parameter (RFC 8707), and a client that is registered beforehand, through dynamic client registration or by a Client ID Metadata Document (verified 2026-10-24).
- rmcp 3.7.0's `auth` drives discovery, registration with `application_type` `native` and the code flow with PKCE S256, and checks the `iss` of RFC 9207 against the issuer of the metadata before it redeems the code. Its stores of credentials and of the flow's state are traits that a caller implements. An authorization server whose metadata omits its PKCE methods only gets a warning (verified 2026-10-24, [M12 validation results](../../../../docs/roadmap/m12-validation.md)).
- rmcp's own HTTP client of OAuth bounds answers to 1 MiB and 30 seconds, follows no redirect to another origin, and resolves the hosts that metadata names so that they reach a private or loopback address only when the server's URL names one of the same class (verified 2026-10-24).
- A browser that the owner signs in with must come back to Mannie, and the core opens links in the system browser through its own checks ([ADR-0043](../../../../docs/adr/0043-let-plugins-offer-the-tools-of-tool-servers.md), decision 8).
- The Windows credential store keeps at most 1,280 UTF-16 units in one entry, fewer than an OAuth token can have ([M12 validation results](../../../../docs/roadmap/m12-validation.md)).

## Decision

1. The sign-in runs through rmcp's `auth`, with `AuthorizationManager::new` and its own client of OAuth, the authorization code flow with PKCE S256 and `state`, and the `iss` checked before the code is redeemed.
2. The plugin refuses an authorization server that does not name S256 among its PKCE methods, and one whose authorization, token or registration endpoint a credential may not travel to: `https`, or plain `http` only to this computer or a private network.
3. The client is the one that the owner registered, with its client ID and its client secret if it has one; without one, the authorization server registers Mannie as a native client named "Mannie". An authorization server that registers no client asks the owner for a client ID.
4. The browser comes back to `http://127.0.0.1:<random port>/callback`. One listener for each sign-in waits five minutes for one request to that path with the sign-in's `state`, answers any other request and keeps waiting, and answers with a fixed page whose texts the UI passes in from its catalog, escaped, with a policy that loads nothing.
5. The settings show the authorization server's host and open its page through `openLink` (ADR-0043, decision 8). A run never opens a browser.
6. rmcp keeps the credentials through a store of the plugin over the server's OAuth state in the credential store, split into parts of 1,000 UTF-16 units, and refreshes the access token itself. The tokens of a refresh join the values that the server's output must not show.
7. A server that answers `insufficient_scope` shows More access needed, and the owner signs in again. Signing out deletes the tokens and keeps a client secret.

## Consequences

- Positive: the sign-ins that most remote servers offer work without a client of Mannie's own, and the address rules of discovery come with rmcp.
- Negative: an authorization server that offers only Client ID Metadata Documents, or that omits its PKCE methods, needs a client ID that the owner registered, or cannot be used (R35).
- Risks: rmcp's `auth` changes with its releases; the plugin's tests against the fake authorization server cover discovery, registration, the code flow and a refresh.

## Alternatives considered

| Option                                     | Why not chosen                                                                                          |
| ------------------------------------------ | ------------------------------------------------------------------------------------------------------- |
| The plugin's own OAuth client over reqwest | Discovery, registration, the flow, the `iss` check and refreshes to maintain, which rmcp already checks |
| A new `browser` service of the SDK         | The core's `open_link` already checks and opens links, and the page's URL holds no secret               |
| A redirect to a custom URL scheme          | It needs a scheme registered with the system, which a desktop app of several systems handles unevenly   |
| Client ID Metadata Documents               | They need a document that Mannie hosts on the web; an idea for later                                    |

## Validation

M12: the plugin's tests against the fake authorization server of `mannie-mcp-testing`, and the owner's check against a real server that signs in with OAuth.
