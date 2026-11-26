# Network Client

|              |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Plugin       | `network`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Last updated | 2026-11-26                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Related      | [ADR-0075](../../../docs/adr/0075-let-plugins-send-network-requests-from-collections.md), [ADR-0076](../../../docs/adr/0076-provide-a-code-editor-through-the-plugin-sdk.md), [ADR-0077](../../../docs/adr/0077-let-the-shell-sidebar-resize-per-activity.md), [ADR-0013](../../../docs/adr/0013-journal-every-side-effect.md), [Plugins](../../../docs/features/plugins.md), [Operations and approvals](../../../docs/features/system-tools/operations-and-approvals.md), [Security](../../../docs/architecture/security.md) |

## Intent

The owner wants a Postman-style network client plugin so agents can run HTTP requests from owner-provided API collections instead of falling back to `curl` or shell HTTP tooling. The owner imports OpenAPI v2 (Swagger 2.0) and v3 (3.0 and 3.1) specifications in JSON or YAML, organizes collections, folders, environments, custom variables, headers, query parameters, path parameters and cookies, and tests requests directly in a workbench. Agents see every cookie and secret as `[redacted]`, while the owner sees their secrets as-is in the Network Client view. Version 1 implements HTTP only, with protocol-tagged schemas and contracts ready for raw TCP, GraphQL, WebSockets, gRPC and tRPC later ([D70](../../../docs/roadmap/milestones.md#5-decisions)).

## Behavior

### Turning it on

- Network Client is an optional plugin on the Plugins page, off by default on new installations ([D28](../../../docs/roadmap/milestones.md#5-decisions)).
- While it is off, agents receive no network tools, the system prompt omits the collections section, `@` offers no collection or request mentions, and the activity bar hides Network. Stored collections, environments, cookies and secrets remain intact.
- Turning it off while requests are in flight asks first and cancels in-flight workbench requests.

### Collections, variables, environments and cookies

- The Network entry in the activity bar opens a resizable sidebar tree of collections, folders and saved requests ([ADR-0077](../../../docs/adr/0077-let-the-shell-sidebar-resize-per-activity.md)), with a vertical overflow menu in the section header for New collection, Import OpenAPI, Global variables and Cookies.
- Each collection has a name (unique without regard to case), an optional description, a protocol (`http` in version 1), a default base URL, an enabled switch for agents, collection-level default headers, query parameters, authentication and variables, named environments (such as Local, Staging, Production) with an active environment selector, folders and saved requests.
- Each saved request carries its protocol (`http`), HTTP method (`GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`, `OPTIONS`), name, optional description, URL template (resolved against the collection base URL when relative, with separate Query and Path tabs: one-way URL to Path parameter synchronization and bidirectional URL and Query parameter synchronization), headers, authentication (inherit collection, none, Bearer token, Basic auth, API key in header or query), body (`none`, `json`, `text`, `form_urlencoded`), cookie-jar participation toggle, and timeout (default 30 seconds, bounded to 1 to 300 seconds).
- Variable interpolation resolves nested `{{variable}}` placeholders (up to 4 passes, preserving secret provenance) across URLs, path parameters, query values, header values, authentication fields and request bodies, with precedence: agent runtime overrides (non-secret only, unable to alter the owner-configured request origin), active environment variables, collection variables, global variables. Inputs and code editors offer `{{variable}}` autocomplete suggestions across active environment, collection and global scopes, and highlight `{{variable}}` references (violet), `{path}` parameters (amber) and `?query=value` parameters (sky) consistently across the URL editor, request bodies, parameter and variable tables, and read-only URL lists. Path templates also resolve `{param}` segments with safe path-segment percent encoding.
- The cookie jar holds named cookies with domain, host-only flag (RFC 6265 section 5.4 exact-host versus subdomain suffix matching), path, `Secure`, `HttpOnly`, optional expiry, and value. Matching cookies are attached automatically when a request's cookie-jar option is on, and response `Set-Cookie` headers update the jar when automatic cookie updates are enabled.

### OpenAPI v2 and v3 import

- Import OpenAPI accepts either a file chosen through the system file dialog (`.json`, `.yaml`, `.yml`) or pasted JSON or YAML text in a syntax-highlighted code editor.
- The parser handles Swagger 2.0 (`swagger: "2.0"`) and OpenAPI 3.0.x / 3.1.x (`openapi: "3..."`), resolves local `$ref` pointers with cycle and depth guards (max `$ref` chain depth 32, max schema synthesis depth 12), honors document-level and per-operation `security` rules (including `security: []`), and never fetches remote `$ref` URLs.
- Import safety limits apply identically to JSON and YAML: at most 10 MiB of input, 32 MiB of expanded bytes (including mapping key bytes and per-node overhead), nesting depth 64, 200,000 value nodes (objects, arrays and scalar values, including nodes expanded from YAML aliases; mapping keys charge their byte length to the expanded-byte budget instead of counting as separate nodes), and 1,000 YAML alias expansions. Imports stop after 2,000 endpoints and record a preview warning. In practice, the 200,000 value-node limit accepts specs up to roughly 5.5 to 7 MiB of YAML (around 2,500 endpoints with `$ref` reuse such as GitLab's REST API, or 400 to 800 endpoints when schemas are large or inlined).
- A preview shows the collection title, spec version, base URL, derived environments (from OpenAPI 3 `servers` and server variables), folders (from tags), request list (method, path, summary), detected security schemes (mapped to secret variable placeholders without hardcoded credentials) and structured import notes before saving. Fixable import notes (missing base URL when no servers or host were declared, and cookie warnings when matching cookies are added to the Cookie Jar) disappear as soon as they are resolved, closing the Import notes alert when none remain.

### Secret storage and redaction

- Every secret variable (owner-marked secret or matching `looks_like_secret`), secret header (`Authorization`, `Proxy-Authorization`, `Cookie`, `Set-Cookie`, `X-Api-Key`, `X-Auth-Token`, names matching `looks_like_secret` or marked secret), secret query parameter, authentication secret and cookie value lives only in the master's OS credential store through `secrets`, chunked to at most 1,000 UTF-16 code units per entry. Updates stage replacement secrets under a fresh slot before updating SQLite and remove old slots only after SQLite commits. Saved URLs reject embedded userinfo and extract secret-named query parameters into secret query rows.
- SQLite `network_*` tables store only non-secret values, metadata, secret flags, slot IDs and chunk counts. Deleting any collection, environment, request or cookie deletes its SQLite row first, then deletes its credential-store entries and erases freed SQLite pages ([ADR-0048](../../../docs/adr/0048-erase-what-plugins-delete-from-the-store.md)).
- In the Network Client workbench, the owner views and edits collection secrets, cookies and manual-run responses as-is ([ADR-0075](../../../docs/adr/0075-let-plugins-send-network-requests-from-collections.md), decision 5). No secret is written to browser storage.
- For agents, approvals, transcripts, tool-call cards, archives, logs and conversation exports, all structured secret fields and cookies appear as `[redacted]` regardless of length, and any known secret or cookie value of at least 8 bytes (including URL-encoded and Basic-auth base64 forms) is scrubbed to `[redacted]` across URLs, response headers, response bodies and error text.

### Workbench execution

- Clicking Send in the request editor runs the resolved HTTP request directly on the owner's action without an agent approval prompt and without creating a transcript entry. In-flight workbench sends are tracked independently per saved request so switching requests keeps each run's status and cancel button isolated.
- Request body editing, OpenAPI paste import and the read-only response viewer use the SDK's lazy-loaded CodeMirror 6 code editor ([ADR-0076](../../../docs/adr/0076-provide-a-code-editor-through-the-plugin-sdk.md)) with JSON, YAML, XML and HTML syntax highlighting and `{{variable}}` completion.
- The response pane displays status code, reason, elapsed milliseconds, byte size, formatted or raw body, response headers and response cookies (including host-only indicators) as-is, and supports canceling an in-flight request. A pointer-captured and keyboard-operable vertical splitter resizes the request and response panes.
- Requests carrying any secret or cookie require `credential_transport_allowed` (`https`, or plain `http` only to localhost, loopback or private LAN addresses). URLs with embedded userinfo are refused, and automatic redirects are disabled.

### Agent tools, approvals and system prompt

- Four static tools are contributed:
  - `network_list_collections`, `network_list_requests`, `network_get_request`: `ToolEffect::ReadsOwnData`, offered to every agent, returning strictly redacted metadata and schemas.
  - `network_send_request`: `ToolEffect::SideEffects`, offered to conversation agents and the `general` child agent (never `explore` or read-only custom agents). Accepts only a saved request from an enabled collection (refusing ambiguous names when multiple requests share a title) plus optional runtime overrides (environment ID, path params, query params, non-secret headers, non-secret variables, body and timeout). Agent overrides cannot redirect a saved request's scheme, host or port.
- Every `network_send_request` call asks the owner through `Access::NetworkRequest` unless covered by a grant (`GrantScope::NetworkRequest` keyed by `(plugin, collection_id, request_id, protocol, method, origin)`) or full access. The approval digest includes a SHA-256 hash of the complete sanitized request body, so modifying any byte invalidates a pending approval. Post-dispatch connection drops report `response_lost` with `with_lost_answer()`, and oversized tool archives are capped before persisting. Switching environments to a different origin or changing a saved request's method asks again. Workers refuse `Access::NetworkRequest` with `no_network_requests`.
- The instruction section `network-collections` (position 345) lists enabled collections and instructs agents to prefer the `network_*` tools over `curl` or shell HTTP commands for covered APIs, never ask the owner to paste secrets or cookies into chat, and treat responses as untrusted data.
- The mention provider `network` lists collections and saved requests under `@` and resolves them to redacted summaries.

## Fit

- Uses `ToolAccess::NetworkRequest` / `Access::NetworkRequest` ([ADR-0075](../../../docs/adr/0075-let-plugins-send-network-requests-from-collections.md)), static plugin tools, instruction sections, mentions, activity-bar navigation, sidebar panels, views, overlays, command palette entries and tool kinds.
- Opens sockets only through `reqwest` under `[native] sockets` and parses YAML only through the exclusive crate `saphyr-parser`.
- Keeps every other plugin and core tool untouched.

## Open questions

None.

## Decisions

- 2026-11-21: accepted [ADR-0075](../../../docs/adr/0075-let-plugins-send-network-requests-from-collections.md) ([D70](../../../docs/roadmap/milestones.md#5-decisions)).

## Parts

- GraphQL: query, variables and schema introspection on `protocol = "graphql"`. An idea.
- WebSockets: persistent bidirectional frame sessions on `protocol = "websocket"`. An idea.
- gRPC: `.proto` import and unary or streaming RPCs on `protocol = "grpc"`. An idea.
- tRPC: typed router procedure calls on `protocol = "trpc"`. An idea.
- Raw TCP: byte and line framed socket sessions on `protocol = "tcp"`. An idea.

## Design

### Store and secrets

- Migration 1 creates `network_collections`, `network_folders`, `network_requests`, `network_environments`, `network_cookies` and `network_global_variables` as `STRICT` SQLite tables with foreign-key `ON DELETE CASCADE`. Migration 2 adds `host_only` on `network_cookies` and `secret_slot` on the tables with secret payloads.
- Secret payloads live in OS credential-store families `collection`, `environment`, `cookie`, `request` and `global`, chunked at 1,000 UTF-16 code units per entry and rotated by UUIDv7 slot on updates.

### Interface

- Activity bar icon (`Globe`, order 25) opens the Network Client view (`network`) and sidebar panel (`network.collections`).
- Overlays handle New/Edit Collection, Import OpenAPI (file picker, paste, live preview), Environments and Variables, Global Variables, Cookie Jar and Delete confirmations.
- Tool kind cards render `network_list_collections`, `network_list_requests`, `network_get_request` and `network_send_request` with status and sanitized details.
- All visible text lives in `plugins/network/messages/en.json` under the `network_` prefix.

## Acceptance

- Unit tests in `plugins/network/crates/mannie-network/src/` cover OpenAPI v2, v3.0 and v3.1 JSON and YAML import, bounded YAML anchor-bomb and cyclic/remote `$ref` defenses, variable resolution, secret chunking, cookie matching and `Set-Cookie` parsing, redaction and transport safety.
- Integration tests in `plugins/network/crates/mannie-network/tests/plugin.rs` verify SDK conformance, CRUD, secret-free SQLite storage, owner as-is visibility, redacted agent tool outputs and archives, per-endpoint approvals and grants, environment-switch re-approval, full access, worker refusal, plain-HTTP credential refusal and no-redirect enforcement against a live loopback HTTP server.
- UI tests in `plugins/network/tests/ui/` and E2E tests in `plugins/network/tests/e2e/network.test.ts` verify the workbench, OpenAPI import, keyboard operability and light/dark themes.
