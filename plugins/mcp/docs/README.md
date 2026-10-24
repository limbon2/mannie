# MCP servers

|              |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Plugin       | `mcp`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| Milestone    | M12                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Last updated | 2026-10-24                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Related      | [ADR-0043](../../../docs/adr/0043-let-plugins-offer-the-tools-of-tool-servers.md), [ADR-0044](../../../docs/adr/0044-let-tool-results-carry-images-to-the-model.md), [speak MCP through rmcp](adr/0001-speak-mcp-through-rmcp-in-both-protocol-eras.md), [sign in with OAuth](adr/0002-sign-in-to-remote-servers-with-oauth.md), [ADR-0013](../../../docs/adr/0013-journal-every-side-effect.md), [Plugins](../../../docs/features/plugins.md), [Operations and approvals](../../../docs/features/system-tools/operations-and-approvals.md), [M12 validation results](../../../docs/roadmap/m12-validation.md) |

## Intent

The owner wants Mannie to use the tools of MCP servers, the Model Context Protocol's servers that other agents use: programs that run on the owner's computer, such as a server that `npx` starts, and remote servers at a URL, with a secret header or an OAuth sign-in. In the owner's words: "I need MCP support for Mannie. Implement it as a plugin". Version 1 covers the servers' tools, their resources and the images that their tools return ([D42](../../../docs/roadmap/milestones.md#5-decisions)).

## Behavior

### Turning it on

- MCP is a plugin on the Plugins page, with a switch. It starts off on a new installation, like every plugin ([D28](../../../docs/roadmap/milestones.md#5-decisions)).
- While MCP is off, agents get no MCP tools, the system prompt lists no servers and "@" offers no resources. The servers, their secrets and their cached tools stay.
- Turning MCP off while calls run asks first, ends the calls, whose outcome is then unknown, and ends every local server's program.

### Registering a server

- The settings section MCP servers lists the servers and has "Add server" and "Paste JSON…". The command palette has "Add MCP server…".
- Each server shows its status: Off, Starting, Connected (with its protocol version, its tools and its resources), Sign-in needed, More access needed, or Failed (with the reason). Its row has a switch, "Sign in…" while a server that signs in with OAuth asks for it, and a menu behind a button with vertical dots: "Sign out" for a server that signed in, "Restart", "Show tools…", "Show log…", "Edit…" and, set apart, "Remove…".
- A server has a name, unique without regard to case and shown in approvals, and a prefix: the start of its tools' names after `mcp_`, 1 to 16 lower-case letters and digits, starting with a letter, and not `read`. An empty prefix is taken from the name, and a server keeps its prefix when it is edited.
- A local server has a program, its arguments, a working folder and variables:
  - The program is a full path, or a name that the core finds on Mannie's `PATH`, outside the owner's projects and Mannie's own folders. On Windows a name finds `.exe`, `.cmd` or `.bat`, so `npx` finds `npx.cmd`. "Find program" shows the path that Mannie starts, and warns when it lies inside a project.
  - The arguments are one on each line, as the program receives them. An argument that looks like a token gets a warning, since approvals and the journal show arguments.
  - The working folder is a full path, or the owner's home folder when it is empty.
  - Plain variables join the environment of commands. Secret variables reach only this server's environment, and a variable whose name looks like a secret's, such as `GITHUB_TOKEN`, is always secret ([D42](../../../docs/roadmap/milestones.md#5-decisions)).
- A remote server has an `https` URL, or a plain `http` one only to this computer or a private network, without a user, a fragment or a secret in its query. Its plain headers go with every request, and its secret headers too; `Authorization`, `Cookie`, `Proxy-Authorization`, `X-Api-Key` and names that look like a secret's are always secret. It signs in with OAuth when the owner turns that on, with a client ID that the owner registered, and its client secret, or without one, when the authorization server registers Mannie itself.
- Each server has two switches, "Give images to the model" and "Give the server's instructions to the model", both on, and a timeout of its calls, 300 seconds unless the owner picks 10 to 3,600.
- Secret values never show again: the settings name each secret, show whether its value is stored, and keep a stored one when its field stays empty.
- Removing a server asks first, then stops it, deletes its secrets and tokens and forgets it and its tools. The permissions for its tools stay listed in the Tools settings until the owner revokes them, and cover nothing without the server.

### Paste JSON

- Paste JSON reads the servers of another client's configuration, under `mcpServers` or `servers`, without running anything. The preview lists each server with what it runs or reaches and the names of its secrets, never their values.
- A server that speaks only HTTP with server-sent events, the transport of 2024, an entry without a program or a URL, and one that breaks the rules of servers are listed with the reason and cannot be added. So is a name that another server has.
- The owner chooses the servers to add. A placeholder of another client, such as `${input:token}`, becomes a secret without a value, which the owner enters after the import; such a server, and one that the configuration turns off, starts off.

### Tools for agents

- The conversation agent and the `general` child agent get each tool that is on, of each server that is on and connected or connecting, as `mcp_<prefix>_<tool>` with the server's description. The `explore` child agent and the tasks of workers get none.
- A tool's name is the server's name for it in snake case. A name that would be longer than 64 characters, or that another tool of the server already has, is cut and ends with a hash of the server's name for it, so it stays the same from one listing to the next.
- The model's copy of a schema has its local references replaced by what they point to, drops `$schema`, `$id`, `$comment` and the keywords that start with `x-`, and has an object at its root. A top-level property named `activity`, which the core adds to every call, is `activity_value` for the model, and the plugin names it back for the server. A schema that is not an object, or that has both names, keeps its tool from agents.
- Show tools lists each tool with its name for agents, its title, its description, what the server says of it, a mark when it is new and a switch. At most 64 tools of all servers are on.
- The tools of a server's first listing start on, as long as fewer than 64 are on. A tool that a server adds later starts off and is marked new, so a server cannot slip a tool in after the owner trusted it. The mark clears once the owner closes Show tools.

### Calling a tool

- Each call asks the owner. The request shows the server, where it runs, the tool, the arguments and what the server says of the tool, which the approval labels as the server's claims and nothing decides on. The owner can allow it once, for this conversation, for this project or for every conversation, or deny it. A grant covers the same tool of the same server at the same endpoint with any arguments, so the same tool on another server, or at another endpoint, asks again. Full access runs every call. MCP tools get no rules, since a rule would cover what the grant for every conversation covers.
- A call's arguments are parsed into an object, where empty arguments and `null` read as `{}`, and the approval shows them with their keys sorted.
- A call waits for its server while it connects, and connects a server that is on and has no connection. It ends after the server's timeout; Stop ends it too, and rmcp tells the server that the call is cancelled. Either way, whether the call took effect is unknown.
- A server that stops during a call leaves its outcome unknown. A local server that ends is started again after 1, 2, 4, 8 and 16 seconds, at most 5 times in 5 minutes, and then shows as Failed until the owner restarts it.

### What the model reads

- Text as it is, and PNG and JPEG images as images, at most 4 of a result, without their metadata ([ADR-0044](../../../docs/adr/0044-let-tool-results-carry-images-to-the-model.md)), when the server's switch lets them through.
- Audio and other binary content become a note with their type and size, a resource link a line with its URI, and an embedded text resource its text.
- Structured content becomes JSON when the result has no text, and a result that the tool marks as an error fails the call.
- Output past the result's room keeps its start and its end and is archived for `read_tool_output`. Every secret of the server, its tokens included, is replaced with `[REDACTED]` when it has at least 8 bytes, also in the server's errors, its instructions, and the titles, descriptions and schemas of its tools and resources, before the plugin stores or shows them. A tool or resource whose name or URI holds a secret is left out, since a call or a read sends its name or URI back.
- A server that asks for input, which Mannie does not give, and one that answers with a task of its own fail the call with a note.

### Resources

- "@" lists the resources of the connected servers, from what their connections listed: at most 500 of each server, listed again when the server announces a change and when it connects. A mention attaches the resource's text to the owner's message, read when the owner sends it; binary content becomes a note.
- Agents get `mcp_read_resource`, which takes a server's prefix and a URI, while some connected server lists resources or resource templates. It asks like a tool call, as the server's tool `read_resource` with the URI as its argument, and its images reach the model as a tool's do.
- Mannie never fetches a resource from the web itself.

### The instruction section

- In runs that offer MCP tools, the system prompt lists the connected servers with their prefixes and how many resources they offer, the server's instructions when its switch lets them through, marked as the server's words and cut to 2,000 characters, and up to 20 of its resource templates.

### Signing in

- "Sign in…" shows the authorization server's host and opens its page in the browser. Mannie waits five minutes for the browser to come back to a port on this computer, then connects. The tokens stay in the credential store, Mannie refreshes them itself, and "Sign out" deletes them.
- A server that asks for more permissions than the sign-in has shows More access needed, and signing in again asks for them.

### What it does not do

- It offers no prompts and answers no request of a server: no elicitation, no sampling and no roots, so a server never asks the owner anything through it. These and the other parts below are [ideas](#parts) for later.
- It speaks no HTTP with server-sent events, follows no task of a server, shows no MCP App, and has no servers of a project or from a repository's `.mcp.json`.

## Fit

- Builds on the extension points of [ADR-0043](../../../docs/adr/0043-let-plugins-offer-the-tools-of-tool-servers.md): a tool source whose tools change while the plugin runs, tools with loose schemas, the external-tool access that the core's policy decides and its journal records, the service `local-servers`, which starts the programs of local servers in the core's supervisor, and the UI SDK's `openLink` and its tool kind for every tool. It also adds an instruction section, a mention provider, a settings section, an overlay, a command of the palette and a kind of mention.
- Builds on [ADR-0044](../../../docs/adr/0044-let-tool-results-carry-images-to-the-model.md) for the images of results.
- Follows [Plugins](../../../docs/features/plugins.md): a switch, a start and a stop that take effect at once, the plugin's code in `plugins/mcp/`, its UI files named `mcp` and its messages under `mcp_`.
- Opens its own sockets under the manifest's `[native] sockets` grant, since the SDK has no network service.
- Must not change: the core's tools, their rules and grants, which never match a call of a server's tool, and the other plugins.

## Open questions

None.

## Decisions

- 2026-10-24: the owner decided ([D42](../../../docs/roadmap/milestones.md#5-decisions)):
  - Version 1 covers the servers' tools, their resources and the images that their tools return.
  - It reaches local servers that Mannie starts and remote servers over Streamable HTTP, with secret headers and OAuth sign-in.
  - A secret that the owner enters for a local server goes into that server's environment and nowhere else, an exception to NFR-SEC-05.
  - Every call asks. A grant covers one tool on one server with any arguments, full access runs every call, and the servers' hints decide nothing.
  - Approving the plan accepts ADR-0043, ADR-0044 and the plugin's first two ADRs.
- 2026-10-24: the plan refined the approvals for the owner's review: MCP tools get no rules, since a rule would cover what the grant for every conversation covers.
- 2026-10-24: the defaults below were chosen when shaping, and wait for the owner's review:
  - At most 64 tools of all servers are on, and a call ends after 300 seconds.
  - A local server has 120 seconds to start and list its tools, a remote one 30, and each page of a listing 30.
  - A server ends at most 5 times in 5 minutes before it shows as Failed, and its log keeps 64 KiB.
  - A listing keeps at most 512 tools, 500 resources and 100 templates, and the section shows 2,000 characters of instructions and 20 templates.
  - A sign-in waits 5 minutes for the browser.

## Parts

- [Prompts](prompts.md): the servers' prompts as commands of the message box. An idea.
- [Requests of servers](server-requests.md): elicitation, sampling, roots and the servers' log messages. An idea.
- [Servers of a project](project-servers.md): servers that belong to a project, and a repository's `.mcp.json`. An idea.
- [More transports and sign-ins](more-transports.md): HTTP with server-sent events and Client ID Metadata Documents. An idea.
- [Long calls and apps](long-calls.md): the tasks extension and MCP Apps. An idea.

## Design

### The plugin

`plugins/mcp/crates/mannie-mcp` defines the plugin `mcp`:

- Its manifest, `plugins/mcp/plugin.toml`, declares the services `local-servers`, `secrets` and `store`, the keyed secret families `variable`, `header` and `oauth`, the crates `rmcp` and `oauth2` that only it may use, and the native grant `sockets`. Its default is off.
- Its start reads the servers and their tools, contributes the tool source `mcp`, the section `mcp-servers` at position 340 and the mention provider `mcp`, and connects to every server that is on. Its stop ends every connection and sign-in; the host's release of the service `local-servers` ends any program that is left.
- One task for each server, the runner, owns its connection (`src/servers/runner.rs`). Every change that a runner makes carries its generation, so a runner that the owner replaced changes nothing.
- Its UI registers one tool kind with `everyTool`, which gives every published tool the plug icon and words its failure codes; the core names each call by its external-tool access.
- The test-only crate `mannie-mcp-testing` runs an rmcp server over Streamable HTTP in a test's process, with a fake authorization server, and its example program `mcp-test-server` serves stdio in the current era and, with `--legacy`, as a scripted server of 2025-06-18.

### Contracts

- Commands, which the host's dispatch answers with `plugin_off` while MCP is off ([ADR-0034](../../../docs/adr/0034-build-plugins-on-a-plugin-sdk.md), decision 8):
  - `add_server` takes a server's fields and returns its view; `update_server` takes the server's ID and its fields, keeps the stored secrets that the owner does not replace and refuses another transport; `remove_server`, `set_server_enabled` and `restart_server` take the server's ID.
  - `server_tools` returns the tools that a server listed last, `set_tool_enabled` switches one, within 64 of all servers, and `acknowledge_tools` clears the marks of new tools.
  - `server_log` returns a server's log; `parse_configuration` reads a pasted configuration, and `import_configuration` reads it again and adds the chosen servers; `locate_program` says where the core finds a program.
  - `begin_sign_in` takes the server's ID and the texts of the page that the browser shows, from the UI's catalog, and returns the authorization server's page and host; `sign_out` deletes the tokens.
  - The channel `follow_servers` sends the whole list of servers with how many tools are on, then again after every change.
  - A server's view has its fields, the names of its secrets and of the missing ones, whether a client secret and tokens are stored, its switches, its status with the reason of a failure, its protocol version, the counts of its tools and resources, and its warnings: `token_in_argument`, `new_tools` and `unusable_tools`. It never holds a secret's value.
  - Errors: `name_taken`, `prefix_taken`, `server_not_found`, `transport_changed`, `tool_not_found`, `too_many_tools`, the four `configuration_*` errors, `program_not_found`, `program_refused`, `no_sign_in`, `authorization_failed`, `authorization_unsupported`, `client_id_needed`, `credential_store_unavailable` and `storage_failed`. A field that breaks a rule is the host's `invalid_input` with the field's name.
- Each published tool's preparation returns one external-tool access: the server's ID as the target, its name as the label, its endpoint, the tool as the server names it with its title, the arguments as JSON with sorted keys, and the server's hints. `mcp_read_resource` returns the tool `read_resource` with the URI as the arguments. Execution checks that the core authorized exactly these accesses and that the server's endpoint is still theirs.
- The failure codes of results are `not_called`, `timed_out`, `server_stopped`, `server_error`, `input_required`, `task_started` and `tool_error`.
- The mention provider `mcp` lists resources with the variant `resource` under keys `<server ID>:<URI>`, refuses a key that no connected server lists with `resource_not_found`, and one that the server does not give with `read_failed`. A mentioned resource's summary code is `resource`.

### Data

- The plugin's migration 1 creates `mcp_servers`: the ID, the name, unique without regard to case, the prefix, unique, the transport, the program, its arguments and folder or the URL, the plain variables or headers, the names of the secrets, how many parts the credential store keeps of them and which have no value, whether the server signs in with OAuth, its client ID, whether a client secret and tokens are stored and how many parts their text has, the switches, the timeout, the protocol era that its last connection found, whether it listed its tools once, and the times of creation and change. A CHECK keeps a local server without a URL or OAuth, and a remote one without a program, arguments or folder.
- It also creates `mcp_tools`: the server's ID, the tool's name, its place, its title, its description cut to 8 KiB, its schema as the server gave it, or `null` past 256 KiB, its hints, the owner's switch and the mark of a new tool. A listing replaces a server's tools in one transaction.
- A server's secrets are one text for each family, its secret variables or headers, and its OAuth state with the client secret and rmcp's credentials. Each text is split into parts of at most 1,000 UTF-16 units, below the 1,280 that the Windows credential store keeps ([M12 validation results](../../../docs/roadmap/m12-validation.md)), stored as `mcp/<family>/<server ID>-<part>`. Adding a server stores its row first, then its secrets, then counts them in the row; a save writes the new parts, counts them, then deletes the parts past them; removing a server deletes its secrets first.

### Connections

- The plugin speaks MCP through rmcp 3.7.0 in both protocol eras ([speak MCP through rmcp](adr/0001-speak-mcp-through-rmcp-in-both-protocol-eras.md)). A first connection probes with `server/discover` and falls back to `initialize`; the era that a connection found is stored, the next one starts in it, and a connection that fails is tried once more with the probe.
- A local server runs as a program that the service `local-servers` starts in the core's supervisor, with the environment of commands and the server's variables. rmcp reads and writes an in-memory pipe whose other end two tasks connect to the program's output and input; a third keeps the lines of its error output in the log. Stopping a connection closes rmcp, closes the program's input, waits 2 seconds and ends its tree.
- A remote server runs over Streamable HTTP through the plugin's own reqwest client: rustls with ring through the process's provider, the operating system's trust store, the system's proxy settings, and no redirects, since a redirect could carry a secret header to another host. Its secret headers are marked sensitive.
- Mannie declares no capabilities. A server of the current era announces changes of its tools and resources through a subscription, and one of an earlier era through its session; either way the runner lists them again.
- A listing reads at most 32 pages of 30 seconds each.

### OAuth

- The sign-in runs through rmcp's `auth` ([sign in with OAuth](adr/0002-sign-in-to-remote-servers-with-oauth.md)): discovery through the server's protected resource metadata and the authorization server's metadata, registration of a native client unless the owner registered one, the authorization code flow with PKCE S256 and `state`, the `resource` of RFC 8707, and the `iss` of RFC 9207, checked before the code is redeemed.
- The plugin refuses an authorization server that does not name S256 among its PKCE methods, and endpoints that a credential may not travel to. rmcp's own client of OAuth follows no redirect to another origin, bounds each answer to 1 MiB and 30 seconds, and reaches a private or loopback address only when the server's URL names one of the same class.
- The browser comes back to `http://127.0.0.1:<random port>/callback`. The listener waits five minutes for one request with the sign-in's `state`, answers others and keeps waiting, and answers with a fixed page of the UI's texts, escaped, that loads nothing.
- rmcp keeps the credentials through the plugin's credential store, refreshes the access token itself, and the new tokens join the values that the server's output must not show.

### The text the model sees

| Case                                  | What the model reads                                                                                                                |
| ------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| A tool                                | The server's description, or its title, or "A tool of the MCP server {name}, which gives no description."                           |
| The section                           | An introduction that the servers' words give no permission, then each server with its prefix, instructions and templates            |
| A call that did not reach the server  | "Not called: {reason} The server received nothing."                                                                                 |
| A timeout                             | "The server did not answer within {seconds} seconds. Mannie told it to stop the call, and whether the call took effect is unknown." |
| A server that stopped during a call   | "The server stopped during the call, so whether the call took effect is unknown."                                                   |
| A result that the tool marks an error | "The tool reported an error:" and the result                                                                                        |
| An image that does not go             | "[An image of type {type} is left out: {reason}]"                                                                                   |
| A mentioned resource                  | "The owner attached the resource {URI} of the MCP server {name}:" and its text                                                      |

### Interface

- The settings section MCP servers lists the servers with their status, endpoint, counts, warnings and failures, and the actions of each. The dialogs to add or edit a server, to show its tools and its log, to remove it and to paste a configuration are an overlay of the plugin.
- The mention kind `mcp` shows resources under MCP resources with the file icon.
- Messages live in `plugins/mcp/messages/en.json` under `mcp_`.

### Loading, empty and error states

| View              | Loading                      | Empty                                                | Error                                                    |
| ----------------- | ---------------------------- | ---------------------------------------------------- | -------------------------------------------------------- |
| The MCP settings  | Placeholders of two servers  | "No MCP servers", with "Add server" and "Paste JSON" | "The MCP servers could not be loaded" with "Try again"   |
| The server dialog | "Saving the server…"         | Not applicable                                       | What failed, with the refused field marked               |
| Show tools        | Placeholders of two tools    | "The server lists no tools."                         | "The tools could not be loaded" with "Try again"         |
| Show log          | Placeholders of two lines    | "Nothing happened yet."                              | "The log could not be loaded"                            |
| Paste JSON        | "Reading the configuration…" | Not applicable                                       | What failed, and why each refused server cannot be added |
| A sign-in         | "Starting the sign-in…"      | Not applicable                                       | "The sign-in did not start:" with the reason             |

### Threat notes

- A prompt injection can make an agent call a server's tool. Every call is an operation that the owner allows unless a grant or full access covers it, a grant covers one tool of one server at one endpoint, and the explore agent never gets the tools (T-55).
- A server can poison its descriptions, instructions, results and images. They reach the model as the server's words, grant nothing, and a tool that it adds later starts off (T-56).
- A server's secret reaches only its process or its URL, and never the database, logs, IPC output or another process; results, errors, the log, the instructions and the listings of tools and resources replace it, and a tool or resource whose name or URI holds it is left out (T-57).
- An attacker can try to catch a sign-in's code. PKCE, `state`, the `iss` check and a listener on a random loopback port protect it (T-58).
- A local server's program ends with the plugin and with Mannie (T-59). A compromised webview can register a program of its choice, which adds nothing to what full access gives it (T-60).
- A server's metadata can point the client at internal addresses; rmcp's client of OAuth reaches them only when the server's own URL does (T-61).
- `npx` and `uvx` may fetch a new package at each start (T-62), and a program inside a project can change under an agent's hands, which the settings warn about (T-63).
- Logs at `info` name servers by ID and calls by their outcome. Errors of HTTP and OAuth are logged by their kind, without their text, which may quote a server's answer.

### Platform differences

- On Windows, a program's name finds `.exe`, `.cmd` and `.bat`, and the core refuses an argument of a batch file that `cmd.exe` would read as code. On Linux and FreeBSD, the name is found as it is.

## Acceptance

- The rules of servers, the names and schemas of tools, the parsing of pasted configurations, the split of secrets, the log, the results, the section, the mentions, the callback and the checks of OAuth pass their unit tests in `plugins/mcp/crates/mannie-mcp/src/`.
- Through the real host (`plugins/mcp/crates/mannie-mcp/tests/plugin.rs`): a server keeps its secrets only in the credential store and its view only names them; a change keeps the secrets that the owner does not replace; removal deletes them; names and prefixes stay apart; Paste JSON previews without values and imports the chosen servers; and the plugin passes the SDK's conformance check.
- Through the core's tool loop with real servers (`plugins/mcp/crates/mannie-mcp/tests/servers.rs`): a local server's tool runs once the owner allows it; a grant covers one tool with any arguments and no other tool; the planted secret reaches the server and never the result, the log or the model; an image reaches the model; `activity` is renamed and named back; a late tool starts off and marked new; a crash leaves its call's outcome unknown and the server starts again; a timeout ends the call; `mcp_read_resource` reads a resource under an approval; a server of 2025 connects with `initialize`; a remote server takes its secret header and refuses a wrong one; a mention attaches a resource's text; a sign-in registers, redeems its code with PKCE and connects; an authorization server without S256 is refused; an expired token is refreshed without a new sign-in; and turning MCP off takes the tools.
- The plugin's UI passes its Vitest tests in `plugins/mcp/tests/ui/`.
- In the app (`plugins/mcp/tests/e2e/mcp.test.ts`), with axe in both themes: the Plugins page lists what MCP may use; a local server with a secret variable connects, lists its tools and logs without the secret; a call asks with the server, its program and the tool, and runs once allowed; the secret that the server returns shows only as `[REDACTED]`; an image shows in the call's details; "@" mentions a resource whose text the model reads; on Windows, a server that a `.cmd` file starts connects; and removing a server asks first.
- The owner's checks: `@modelcontextprotocol/server-everything` through `npx`, the GitHub server with a secret token, a remote server with a bearer header, a remote server with OAuth such as Linear, and images through the Responses API and one Chat Completions server.
