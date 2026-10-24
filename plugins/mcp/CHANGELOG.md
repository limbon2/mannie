# Changelog of the MCP plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Added

- The plugin MCP (ADR-0043): agents use the tools of MCP servers that the owner registers in the settings section MCP servers, programs that Mannie starts on this computer and remote servers over Streamable HTTP, in the protocol era of 2026-07-28 and in the earlier one of `initialize`, through rmcp 3.7.0. Each call asks the owner unless a grant for the tool or full access covers it, and the servers' hints decide nothing.
- Secret variables of a local server reach only that server's environment, and secret headers of a remote server only its URL; both stay in the credential store, and results, errors, the log, the server's instructions and its listings of tools and resources replace them.
- Signing in to a remote server with OAuth: discovery, registration of Mannie as a native client or a client that the owner registered, the code flow with PKCE S256, `state` and the `iss` check, a browser that comes back to a loopback port, and tokens that Mannie refreshes itself.
- Show tools switches each tool of a server, at most 64 of all servers; a tool that a server adds later starts off and is marked new. Show log shows a server's error output and what happened to its connections.
- Paste JSON adds the servers of another client's configuration, under `mcpServers` or `servers`, after a preview that names their secrets without their values.
- The images that a server's tools return reach the model (ADR-0044), with a switch for each server, and so do the server's instructions, marked as its words.
- "@" attaches a connected server's resource to a message, and agents read resources with `mcp_read_resource`.
