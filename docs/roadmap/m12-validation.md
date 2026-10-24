# M12 validation results

|              |                                                                                                                                                                                                                             |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                                                                                            |
| Last updated | 2026-10-24                                                                                                                                                                                                                  |
| Related      | [Milestones](milestones.md#m12-mcp), [ADR-0043](../adr/0043-let-plugins-offer-the-tools-of-tool-servers.md), [ADR-0044](../adr/0044-let-tool-results-carry-images-to-the-model.md), [Security](../architecture/security.md) |

This document records what the spikes and checks of [M12](milestones.md#m12-mcp) showed, so that the owner can review the plugin MCP, ADR-0043 and ADR-0044, and see what remains before the next release. The owner accepted both ADRs and the plugin's first two ADRs with the plan (D42). Each section names the checks, where they ran, the result, and what is open.

The spikes ran on 2026-10-24 on the owner's Windows 11 workstation, in scratch projects outside the repository, with Rust 1.99.0, Node.js 24.15.0 and cargo-deny 0.20.2.

## 1. Summary

| Spike or check                                                                                                           | Result                                                                                                                                                                                                              | Open                                                                                                                       |
| ------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| S1: rmcp 3.7.0 with `client`, `transport-async-rw`, the Streamable HTTP client on reqwest without a provider, and `auth` | Passed: cargo-deny's advisories, bans, licenses and sources with the repository's policy on its four targets; no aws-lc-rs, no OpenSSL, only reqwest 0.13.5                                                         | None                                                                                                                       |
| S2: both protocol eras over stdio                                                                                        | Passed with the everything server 2025.12.18 (2025-11-25) and 2.0.0 (2026-07-28) under `ClientLifecycleMode::Auto`; a first start that `npx` downloads failed                                                       | Over HTTP, with the plugin's tests                                                                                         |
| S3: rmcp's transport over the supervisor's pipes                                                                         | Passed on Windows: the tests of the supervisor's server mode, and the plugin's tests, which speak MCP over a local server's pipes                                                                                   | The same tests on Linux                                                                                                    |
| S4: rmcp's OAuth                                                                                                         | Meets the plugin's rules: `iss` checked, a credential store and a state store of the plugin, its HTTP client, and `application_type` `native`                                                                       | None: the plugin refuses an authorization server without S256 in `code_challenge_methods_supported`, which its tests check |
| S5: the largest secret of the Windows credential store                                                                   | 1,280 UTF-16 characters                                                                                                                                                                                             | None                                                                                                                       |
| S6, S7, S9: images in tool results, the limits on the number of tools, and loose schemas on live providers               | Not run: this workstation's shell has no `MANNIE_LIVE_*` variables                                                                                                                                                  | The scenarios `tool_image`, `many_tools` and `loose_tool` of `task provider:live`                                          |
| S8: `npx.cmd` and `uvx.exe` through a suspended start and a job                                                          | A batch file starts through `cmd.exe` with each argument quoted, and an argument that `cmd.exe` would read as code is refused before anything runs; the end-to-end test connects a server that a `.cmd` file starts | `npx.cmd` and `uvx.exe` with real packages, in the owner's checks                                                          |
| The plugin's end-to-end spec                                                                                             | Passed its 8 tests on WebView2 155, with axe in both themes, after a fix of Tailwind's sources that it found                                                                                                        | The same spec on WebKitGTK                                                                                                 |

## 2. rmcp's dependencies (S1)

A scratch crate depended on rmcp 3.7.0 with `default-features = false` and the features `client`, `transport-async-rw`, `transport-streamable-http-client-reqwest`, `reqwest-tls-no-provider` and `auth`, beside reqwest 0.13.5 with `rustls-no-provider` and rustls 0.23.46 with ring, as `mannie-provider` uses them.

- `cargo deny --all-features check` with a copy of the repository's `deny.toml` passed advisories, bans, licenses and sources. Its warnings named only wrappers and ignored advisories that the scratch graph does not have.
- `cargo tree -i` found no aws-lc-rs, aws-lc-sys or openssl-sys. The graph holds one reqwest, 0.13.5, and ring 0.17.14. oauth2 5.0.0 comes without its default features, so without reqwest 0.12; it brings rand 0.8.8, getrandom 0.2.17 and thiserror 1.0.69 beside the newer versions in the root's lock. chrono 0.4.45 is a dependency of rmcp that no feature removes.
- rmcp's crate metadata names Apache-2.0, and its rust-version is 1.88. The repository's license notice keeps older contributions under MIT. Both licenses are on the policy's list.

## 3. Protocol eras (S2)

A scratch client started the everything server with `npx.cmd -y @modelcontextprotocol/server-everything@<version>` through tokio's `Command`, connected with `serve_client_with_lifecycle` and `ClientLifecycleMode::Auto`, listed the tools and called `echo`.

- 2025.12.18 answered as `example-servers/everything` 1.0.0 with the protocol version 2025-11-25, after 1.6 s, with 11 tools. Its `echo` returned `Echo: hello`, without `resultType`.
- 2.0.0 answered as `mcp-servers/everything` 2.0.0 with the protocol version 2026-07-28, after 4.7 s the first time and 1.5 s after, with 22 tools whose names hold hyphens, such as `get-annotated-message`. Its `echo` returned `resultType` `complete` and the server's identity in `_meta`.
- The schemas name `$schema`: draft-07 for the older server and 2020-12 for the newer one.
- The first start of 2025.12.18, while `npx` downloaded it, failed after 11.9 s: the probe of `server/discover` waited 10 s, the client fell back to `initialize`, and the server's late error for the probe ended the connection. The plugin therefore stores the era that a server's first connection found, connects in it from then on, and retries a first connection once.
- rmcp's timeout of the probe is fixed at 10 s (`DEFAULT_AUTO_DISCOVER_TIMEOUT`).

## 4. OAuth (S4)

rmcp 3.7.0's `transport::auth` was read in its source:

- `OAuthState` drives discovery, registration and the authorization code flow with PKCE S256, and `handle_callback_url` checks the response's `iss` against the issuer of the metadata, required when the metadata sets `authorization_response_iss_parameter_supported`.
- `CredentialStore` and `StateStore` are traits that the plugin implements over its secrets. Stored credentials name their issuer, and a changed issuer clears them.
- Dynamic client registration sends `application_type` `native` unless told otherwise.
- The client for OAuth's requests is the caller's reqwest client, or an `OAuthHttpClient`, and discovery refuses a redirect to another origin.
- A server whose metadata omits `code_challenge_methods_supported` gets a warning and an S256 challenge, where the specification asks the client to refuse. The plugin checks the field itself before it starts a sign-in.

## 5. The credential store (S5)

A scratch program stored values of growing length through keyring-core 1.0 and windows-native-keyring-store 1.1 with `set_password` and the target and persistence that `mannie-secrets` uses. Values of up to 1,280 characters were stored and read back whole; from 1,281 on, the store refused them: "Value of 'password encoded as UTF-16' is longer than the platform limit of 2560 chars". A value of 1,280 two-byte characters was stored too, so the limit counts UTF-16 units. The plugin keeps a longer value, such as an OAuth token, in numbered entries of its family.

## 6. The build's checks

The checks ran on 2026-10-24 on the same workstation, with WebView2 155 for the end-to-end tests.

- The supervisor's server mode (`crates/mannie-tools/tests/servers.rs`, 10 tests) passed. A server reads its input in order and answers on its output, closing its input ends it with its exit code, ending a server that ignores its input ends its whole tree, and a dropped server ends its tree. A batch file starts through `cmd.exe` with each argument quoted, and an argument that `cmd.exe` cannot take quoted is refused before anything runs. The finder takes an `.exe` before a batch file and never a PowerShell script, skips the projects and Mannie's folders on the `PATH`, marks a program inside a project and refuses one in Mannie's folders.
- The runtime's tests with fake plugins passed: tool sources (`tool_sources.rs`, 7 tests), external tools (`external_tools.rs`, 6), local servers (`local_servers.rs`, 6, with a secret that reaches only its own server), the images of tool results (`plugin_images.rs`, 3), and the injection corpus, which gained a tool server's tool whose description and result ask for a deletion, a command and another call (`injection.rs`, 3).
- The plugin's 72 tests passed against the test servers of both protocol eras over stdio and HTTP, with the fake authorization server.
- A review of the plugin during these checks found that a server's own words reached the model, the store and the settings without its secrets replaced: the message of a protocol error, its instructions, and the titles, descriptions and schemas of its tools and resources. They are replaced now, before the plugin cuts, stores or shows them, and a tool or resource whose name or URI holds a secret is left out, since a call or a read sends it back. The test server repeats its planted token in all of these while a test sets it, and the plugin's test of the planted secret and the end-to-end spec check that none of it reaches a request to the model, the tools' views or the log.
- The plugin's end-to-end spec (`plugins/mcp/tests/e2e/mcp.test.ts`) passed its 8 tests, with axe in both themes. Its first run found that Tailwind read no file of the plugins' folders: the pattern `../../../plugins/*/ui` of `apps/desktop/src/app.css` matched the folders and none of their files, so the classes that only a plugin's interface used were missing. The server dialog lost its height limit, which left its buttons outside the window, and SSH's host dialog lost the row of a host's name and port. The patterns now name the files.
- The whole end-to-end suite ran its 25 spec files: 23 passed, those of every plugin among them, and 2 failed for a reason that predates M12. The specs of child agents and of Workers expect a task's report to start with `Task 1 reported:`, which the change of task reports in commit `ffa11d4` made `Task 1 reported. The evidence in this report comes from the child's own tool calls:`.
- `task ci` ran its 14 steps. Eleven passed, among them Clippy, the Rust tests of every workspace (1,339 of the root's and 72 of the plugin MCP) and the 532 tests of the UI. `lint:deps` failed on two advisories that predate M12: RUSTSEC-2026-0386 in tokio 1.53.2, which every workspace's lock holds, and RUSTSEC-2026-0387 in crypto-bigint 0.7.5, which the plugin SSH's lock holds. `fmt:check` and `test:scripts` failed on two slips of this work, a line that rustfmt wraps and the release script's test, whose list of the plugins' changelogs lacked MCP's; both are fixed, and both steps passed when run again.

## 7. Open

- The spikes that need live providers: `task provider:live` runs the scenarios `tool_image`, an image in a tool's result in the placement of each interface; `many_tools`, a request with 128 tools; and `loose_tool`, a schema without strict mode, against the server that the `MANNIE_LIVE_*` variables name.
- Linux: `task test:linux` in Docker, the supervisor's server mode with the Unix wrapper's input kept open, and the end-to-end tests on WebKitGTK. FreeBSD: the worker's build with the core's changes.
- The two advisories of `lint:deps` and the two end-to-end specs that expect the old text of a task's report, which predate M12 and need changes of their own.
- The owner's checks with real servers: `@modelcontextprotocol/server-everything` through `npx`, the GitHub server with a secret token, a remote server with a bearer header, a remote server with OAuth such as Linear, and images through the Responses API and one Chat Completions server.
