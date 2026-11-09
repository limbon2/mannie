# Changelog

All notable changes to Mannie are recorded in this file. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/). Each change adds one line under Unreleased, and a release turns that section into its notes. A change to a plugin goes into the plugin's own changelog, `plugins/<id>/CHANGELOG.md`, which a release moves into its section here under the plugin's name.

## Unreleased

## 0.6.1 - 2026-11-09

### Changed

#### SSH

- Hosts whose only host key is RSA can be scanned, trusted and used, with `rsa-sha2-512` or `rsa-sha2-256` signatures and keys of at least 2048 bits; SHA-1 `ssh-rsa` and shorter keys are still refused ([SSH ADR-0002](plugins/ssh/docs/adr/0002-accept-rsa-host-keys.md)).

### Fixed

- A mention whose name starts with another mention's name and a space, such as a database "test db" and a notebook "test", no longer carries both: where tokens in the message overlap, the longer one counts.

## 0.6.0 - 2026-11-09

### Added

- Plugins publish tools while they run, through the point `tool-sources`: each set replaces the source's tools, a run keeps the set that it started with, and a published tool gives way to any tool of the same name (ADR-0043).
- A plugin's tool may call a tool server's tool as an external-tool access, which the owner allows once, for the conversation, for the project or for every conversation, covering the same tool of the same server at the same endpoint with any arguments; full access runs every call, a worker's ceiling refuses them, and the approval shows the server's hints as its claims (ADR-0043).
- Plugins start the programs of local tool servers through the service `local-servers`, which finds the program on Mannie's `PATH`, runs it in the supervisor with the environment of commands and the server's own variables and secrets, and ends it when the plugin stops or Mannie quits (ADR-0043).
- The images of tool results reach the model, PNG and JPEG without their metadata, at most 4 of a result, through the Responses API's tool outputs and a message after the tool results on Chat Completions; the call's details show them and the export keeps them (ADR-0044).
- A plugin's tool may have a schema that is not strict, which the providers' strict mode leaves as it is, and a plugin's UI opens a link through the core's checks with `openLink` and describes every tool that it publishes with one tool kind (ADR-0043).
- The evaluations count, for each report of a child agent, the parent's reads and searches that repeat what a child read or searched, apart from its checks of files that a child changed. A new task fails when the parent repeats more than two of them for its report, and `MANNIE_EVAL_RUNS` runs each task several times.
- An activity bar at the far left of the window switches the sidebar between Chat and the views of plugins, such as Notebooks, with "Find a command" and "Settings" at its foot; each entry shows its own sidebar panels and returns to where the main area last was under it, and Ctrl+B collapses the sidebar to the bar (ADR-0045).
- Conversations hold attachments, at most 8: a plugin's context, such as a notebook. A reply searches the conversation's home and the attachments that the message names, with the owner's two earlier messages, before the model answers, under "Searching the attached sources…". The owner's message shows what was found as a row that opens to the excerpts, and the model reads them after the message's mentions with their cite links, and the instructions with an outline of what the attachments hold and how to search, read and cite them. The message box shows the attachments as chips that detach them, and the new conversation pane offers the controls of the message box that work on a draft (ADR-0046, ADR-0049).
- Replies cite by links such as `[guide.pdf p. 12](cite:notebooks/<key>)`, which the transcript shows as buttons with their labels that open the excerpt, also while its plugin is off, or open where a passage that a tool returned comes from; compaction keeps the cite links that the replies relied on (ADR-0049).
- A mention whose provider is also a context provider, such as a notebook after "@", attaches what it names when the message is sent, and that message gets what the attachment holds for it (ADR-0050).
- A conversation can belong to one of its attachments, such as a notebook's chat: the attachment stays attached, every message gathers from it, the conversation never joins a project, and the Chat list leaves it to the plugin, which starts it with `startConversation({ home })` and deletes it with `deleteConversation` (ADR-0051).
- A plugin's tool can be offered only where the conversation, or the one that gave a child its task, holds attachments of the plugin's own context provider, and its calls carry their keys (ADR-0052).
- A new embedding profile starts with the query text that instruction-aware models such as Qwen3-Embedding want, which the owner can change or clear, and a plugin's `openView` can replace the address in the history.
- Search profiles in the Providers settings, "Embedding and rerank models": an embedding or rerank model on a server that the owner names, with an optional key and the texts that go before queries and documents, and a test that reports the vectors' dimensions or the scores of two passages. Plugins embed and rerank through them with the service `models`, and never see a key (ADR-0047).
- A plugin's write to the store can erase what it deleted from every file of the store, as deleting a conversation does (ADR-0048).
- ChatGPT plan: "Continue with ChatGPT" in Settings > Providers signs in to one ChatGPT account through OpenAI's Sign in with ChatGPT, and a provider profile can use the ChatGPT plan in place of an API endpoint, with a model from the account's catalog, so that its replies bill the owner's ChatGPT subscription. The tokens stay in the credential store and refresh by themselves, "Using ChatGPT plan" shows beside the profile menu, a usage limit offers "Manage usage" and is never retried, and runs on the plan are not offered workers (ADR-0062).
- Skills: agents see the skills that the `skills` CLI installs with `--agent universal` in `~/.agents/skills` and in a project's `.agents/skills`, and read them with `read_skill` without asking. Settings > Skills lists them with a switch each: a skill of this computer is on until the owner turns it off, and a project's skill is off until the owner turns it on, and off again when its name or description changes. "@" mentions a skill for one message, and nothing in a skill grants a permission (ADR-0061).
- Custom agents: an Agents page creates and edits agents without code, with a name, a kind, instructions, steps, a model, groups of tools, plugins, the child agents it may use and limits, and a picker in the message box chooses each conversation's agent. A custom agent can only have less than the built-in agent that it is shaped like, and an agent that only reads gets no tool with side effects, no executor and no child that can act. The starters are an Orchestrator, which hands the work to child agents, and a Plan agent (ADR-0059).
- Plans: the tool `save_plan` saves a conversation's plan in Mannie's database, where the owner reads, edits and deletes it, and "@" mentions it in any conversation. Running a plan fills the message box of a new conversation or of its own, and nothing runs before Send. A plan is model output and allows nothing (ADR-0060).
- A side panel lists a conversation's child agents under Working and Ended, and opens by itself without taking the focus on the first task of a conversation whose agent is custom.
- Agents can ask the owner questions with options during a reply: the tool `ask_owner` pauses the run, a card in the transcript shows up to 4 questions of 2 to 6 options with a choice of the owner's own words, and the reply goes on with the answer. Only the conversation agent asks, an answer allows nothing, and Stop, a quit or a crash cancels the question (ADR-0058).
- Plugins open terminals through the service `terminals`: this machine's shells, which the supervisor starts on a pseudo console with a setup that the plugin passes and contains as a command, and the hosts that plugins offer through the point `terminal-hosts`. Each terminal belongs to a conversation, at most 8 to one and 32 on the machine, and ends with its conversation, its plugin and Mannie (ADR-0053).
- A plugin's tool may type a command into a terminal as a terminal-command access, which the owner allows once, for the conversation, for the project or for every conversation, covering the same command on the same host and, on this machine, in the same shell; rules of its leading words apply on their host only and show in the Tools settings, full access runs it, and a worker's ceiling refuses it (ADR-0053).
- A tool's result may keep up to 8 items that it cites, which the call's part stores, so a citation's popover shows their text after their source is gone; a cite key may end with `#L<first>-L<last>`, whose lines the popover marks, and a context kind without a provider serves citations only (ADR-0054).
- A plugin's view puts a mention into the open conversation's message box through `@mannie/plugin-sdk/message-box`, as if the owner had chosen it from the "@" list (ADR-0054).
- A command of the palette may have a key, which the shell binds while the command is available and the palette shows (ADR-0053).
- The warning before closing or quitting counts the terminals that run a command (ADR-0053).
- Plugins may have optional features, each a table `[features.<id>]` of the manifest with the services that only it uses, which start off and start on the running plugin when the owner turns them on. A feature's contributions, commands and channels have a generation of their own, and turning it off removes them, ends its calls and channels with `feature_off`, waits for its work and releases its services; turning the plugin off stops its features first (ADR-0055).
- Plugins work with the owner's SQLite files through the service `sqlite-files`: the owner chooses a file in the system's file dialog, the core registers it for the plugin, refuses links, network paths, Mannie's own folders and files without SQLite's header, opens it with hardened connections of its own, and journals each change that the owner makes from the plugin's view before it runs, never running one twice (ADR-0056).
- A plugin's tool may run a statement on a registered database as a database access: a read runs in every conversation without asking, a write asks and its grant covers the same statement on the same database with any values, a change of the schema always asks and gets no grant, and the approval shows the statement with its values; full access runs every statement, and a worker's ceiling refuses them (ADR-0056).
- A plugin's tool may name a database on a worker's host with what the worker reported of it: the core classifies its statements against the copy of the schema, narrows the master's policy to the host, gives the tool the evidence of exactly the write that was allowed, with what allowed it, for the worker to check, and journals the owner's change there before the plugin sends it. A plugin's result may say that it lost its answer, which makes a call with side effects uncertain (ADR-0057).
- The Plugins page lists each plugin's optional features under it, with what each adds and may use and a switch that works while the plugin is on, asks first when the feature has work in progress and offers "Try again" after a failed start. A plugin's UI registers a feature's entries through `context.feature(id)`, which show only while the plugin and the feature are on, and the address of a view, settings page or side panel of a feature that is off says so and leads to the Plugins page (ADR-0055).

#### MCP

- The plugin MCP (ADR-0043): agents use the tools of MCP servers that the owner registers in the settings section MCP servers, programs that Mannie starts on this computer and remote servers over Streamable HTTP, in the protocol era of 2026-07-28 and in the earlier one of `initialize`, through rmcp 3.7.0. Each call asks the owner unless a grant for the tool or full access covers it, and the servers' hints decide nothing.
- Secret variables of a local server reach only that server's environment, and secret headers of a remote server only its URL; both stay in the credential store, and results, errors, the log, the server's instructions and its listings of tools and resources replace them.
- Signing in to a remote server with OAuth: discovery, registration of Mannie as a native client or a client that the owner registered, the code flow with PKCE S256, `state` and the `iss` check, a browser that comes back to a loopback port, and tokens that Mannie refreshes itself.
- Show tools switches each tool of a server, at most 64 of all servers; a tool that a server adds later starts off and is marked new. Show log shows a server's error output and what happened to its connections.
- Paste JSON adds the servers of another client's configuration, under `mcpServers` or `servers`, after a preview that names their secrets without their values.
- The images that a server's tools return reach the model (ADR-0044), with a switch for each server, and so do the server's instructions, marked as its words.
- "@" attaches a connected server's resource to a message, and agents read resources with `mcp_read_resource`.

#### Notebooks

- The plugin Notebooks, off by default: notebooks of PDF, Markdown and text files of any size and pasted texts, indexed and searched through the owner's search profiles of an embedding model and, if the owner chooses one, a reranker. The Notebooks entry of the activity bar opens the latest notebook, whose page shows each source's state and progress, with "Use in answers", "Try again" and its text in a viewer that fills the window. The sidebar lists the notebooks, each with "New chat" and its own chats, which search the notebook with every message and stay out of the Chat list; "@" attaches a notebook to any other chat. Replies get the best passages, searched with the message and the owner's earlier messages, and the model lists, searches and reads the attached notebooks with three tools, and cites passages by links that open them in the viewer. A PDF is read page by page, so a page that fails costs only that page.

#### Workers

- Databases, an optional feature that the owner turns on under Workers on the Plugins page, registers SQLite files on this computer through the system's file dialog, with a name that no other database has, and gives the view commands to list, rename, mark, accept and remove them, read their structures, counts, pages and statements, stop a read and run the owner's changes, which the core journals (ADR-0055, ADR-0056).
- With Databases on, conversations and the general child agent read registered databases with `describe_database` and `query_database` without asking and change them with `change_database` after the owner's approval; the explore agent gets the reading tools, a section lists the databases with their states, and "@" names a database and gives the model its structure.
- The Databases view, in the activity bar while the feature is on: the databases of this computer in the sidebar with their states and menus, "Add database…" with a name and a read-only mark, and a page for each database with the tabs Tables, with row counts on request, Structure, Data, which pages, sorts, filters, edits, inserts, deletes and copies rows as CSV, and SQL, which runs a statement with Stop for reads and asks before a change of the schema. The cards of the database tools say what each call read or changed.
- Databases reach the paired workers' hosts (ADR-0057): the owner registers a SQLite file on a worker by its path, which the worker checks under its ceiling once its `worker.toml` sets `enabled = true` under `[databases]`. The view groups the databases by machine and says when a worker is offline, needs an update or does not allow databases, and the master keeps each worker's list to show it while the worker is offline. Agents read a worker's databases without asking and change them under the master's policy and the worker's ceiling; both machines journal each write, and one whose answer the link lost ends uncertain.
- The worker serves the link's optional package `databases`, runs each request under its ceiling, journals each write under its operation ID before it runs, and forgets the host's registrations when it is unpaired. A worker without the package stays connected, and its databases say that it needs an update.

#### SSH

- Terminals on the registered hosts: the plugin offers its hosts to the terminal plugin, whose terminals run the account's login shell on a pseudo terminal over the host's session, with the password replaced in their output ([ADR-0053](docs/adr/0053-let-plugins-offer-terminals.md)).

#### Terminal

- Terminals beside a conversation, as tabs of the side panel area, on this computer in its shells and on the hosts that plugins offer, such as SSH hosts, opened from the toolbar's Terminal menu, the command palette or Ctrl+`, with Find, a list of commands, End and Start again (ADR-0053).
- Agents run one command at a time in the conversation's terminals with `run_in_terminal`, under the master's policy, at an idle prompt and never while the owner types, and list them with `list_terminals`; the result cites the command's output by a link, which a reply's citation shows and marks in the terminal (ADR-0053, ADR-0054).
- Shells that mark their prompts and commands with a setup that the plugin injects on this computer and types on a remote host: bash, zsh, fish, sh, PowerShell and Command Prompt ([mark commands with shell integration](plugins/terminal/docs/adr/0001-mark-commands-with-shell-integration.md)).
- The owner points an agent at a command or a selection with "Mention in message", "Mention selection" and the "@" group Terminal.
- A headless emulator of each terminal in Rust, the only source of the text that a model reads, with a text log of 2 MiB for each terminal and 32 MiB in all, and views that get snapshots instead of a flood and nothing while hidden ([keep a headless emulator of each terminal in Rust](plugins/terminal/docs/adr/0002-keep-a-headless-emulator-of-each-terminal-in-rust.md)).
- Settings for the default shell of this computer, the scrollback, the font size, a blinking cursor, the renderer and screen reader mode.

### Changed

- `task dev` runs the app as `io.github.limbon2.mannie.dev`, with a data folder, credentials and logs of its own, so a development build never migrates the owner's database and runs beside a release build.
- The "@" list shows each kind of mention as soon as it answers, and while more than one kind shows entries, each shows its first 5 with "Show N more", so Git's and SSH's entries no longer sit below 50 files. A space no longer ends the query while an entry that the list shows holds it, so a name with spaces, such as a commit's subject or a path with a space, can be typed out.
- Child agents write their reports in sections: status, answer, established findings with their evidence, what they did not verify, changes, what is left open and their assumptions. A report's result says that its evidence comes from the child's own tool calls, and the guidance on tasks treats what a report backs with evidence as established, with at most two tool calls to check one report, on the files that a task changed and the claims that it did not verify.
- Checkpoints are written under fixed headings: the goal, what is done with what shows it, established facts with their sources, decisions, what nobody verified, open questions and the next steps.
- The "Plans" and "Conversations" groups of Chat's sidebar collapse from their headings, as "Projects" does, and the webview's storage remembers them.
- A new conversation shows the profile menu before its first message, with the ChatGPT plan's indicator beside it, and the first message starts the conversation with the chosen profile.

#### Workers

- The "Workers" group of the sidebar collapses from its heading, as "Projects" does.
- The Databases view lists each database's tables under it in the sidebar, which choose the table of the Data and Structure tabs in place of the Tables tab and the tabs' table pickers. "Count rows" moves to the Structure tab.
- The Data tab puts the order, the conditions' buttons, the pages and the actions on rows in one row, with Insert, Delete and Copy as icons whose names show on hover.
- The row dialog writes the kind of value that a column's declared type gives and offers the kind only for a column without one. Each field starts with the row's value, or with the column's default for a new row, can be set to NULL and can be undone, in place of the choices "Unchanged" and "The column's default".

#### Git

- The counts of added and removed lines of a diff's files take the colors of those lines.

#### Notebooks

- A notebook's chat can be deleted from the sidebar: a button on its row asks first, and the notebook's page shows in place of a chat that showed.

### Fixed

- CI checks of local servers compile without an unused Windows import on Linux and FreeBSD.
- The "@" list keeps up in large projects: queries that arrive while the project's folder is read share that one reading instead of each reading the folder again, an index older than 15 seconds answers at once while the folder is read again in the background, and a query no longer sorts every match.
- A summary that reaches the output limit no longer becomes a checkpoint: the compaction fails with `summary_cut`, which asks the owner to raise the profile's output limit.
- The styles that only a plugin's interface uses reach the app, such as the row of a host's name and port in SSH's host dialog: Tailwind read no file of the plugins' folders, because their source pattern named the folders instead of their files.
- The end-to-end test of child agents expects a task report's result as the runtime now writes it, with the sentence that its evidence comes from the child's own tool calls.
- On Windows, a channel to the webview held its first message after a pause until the system's next timer tick, up to 15.6 ms, and stretched the 16 ms between its batches to as much as 31 ms. A channel now sends up to 4 batches at once after a pause, without a timer, and then one every 16 ms at most on average, so a terminal's echo of a key reaches the page as soon as the shell sends it.
- Every change of the page made the webview restyle the whole page, 5 to 6 ms on the reference machine and four times that on the low-end laptop profile, because Tailwind's `group-has-*` utilities in the shadcn-svelte components and the cursor rule of the side panel's handle put `*` after a `:has()`. These utilities stay out of the build, rules that cost nothing between changes do what Mannie needs of them, and `task build` refuses a stylesheet with such a selector.
- A plugin that turned on or off could keep showing the state that the command returned, when the list with its newer state arrived before the command's answer. The command's plugin now goes in only while no list arrived during the command.
- Showing another side panel tab, or opening or closing one, no longer makes the Git panel start over and read its repository again: the side panel area hands a panel the same argument for as long as it stays open.
- On Linux, ending a terminal also ends the processes that it started in their own sessions, even after the terminal shell exits during shutdown.
- The Linux credential store uses async-executor 1.14.1 after the upstream project yanked 1.14.0.

#### Workers

- The end-to-end test of workers expects a task report's result as the runtime now writes it, with the sentence that its evidence comes from the worker's own tool calls.
- A worker that connected while it was being paired, or while it was renamed, could keep showing the status that the command returned, when the list with its newer status arrived before the command's answer. The command's worker now goes in only while no list arrived during the command.
- When a worker goes offline, its database row opens the database's status page without announcing a table list that is no longer available.

#### SSH

- CI uses crypto-bigint 0.7.10 after the upstream project yanked 0.7.6, with its required ctutils 0.4.6 dependency.
- A host that a command saved or trusted could keep showing the state that the command returned, when the list with its newer state arrived before the command's answer. The command's host now goes in only while no list arrived during the command.

#### Git

- The "@" list reads the repository once for a conversation and matches what the owner types against that reading for 10 seconds, instead of running git, `git status` included, for every key.
- Choosing the open commit in the panel's History again closes it, where before it stayed open until another commit was chosen.

#### MCP

- The JSON import test uses an absolute working folder for the platform that runs it, so it passes on Linux as well as Windows.
- A server that started while it was being added, turned on or signed out could keep showing the status that the command returned, when the list with its newer status arrived before the command's answer. The command's server now goes in only while no list arrived during the command.

### Security

- ctutils 0.4.6, which fixes the advisory RUSTSEC-2026-0385 in the constant-time masks and `u128` comparisons that the Secret Service credential store reaches through its hashes.
- tokio 1.53.3, which fixes the advisory RUSTSEC-2026-0386 in the constructors of `AsyncFd` on Unix, which neither Mannie's crates nor their dependencies call.

#### Workers

- tokio 1.53.3, which fixes the advisory RUSTSEC-2026-0386 in the constructors of `AsyncFd` on Unix, which neither the plugin's crates nor their dependencies call.

#### Images

- tokio 1.53.3, which fixes the advisory RUSTSEC-2026-0386 in the constructors of `AsyncFd` on Unix, which neither the plugin's crates nor their dependencies call.

#### SSH

- ctutils 0.4.6, which fixes the advisory RUSTSEC-2026-0385 in the constant-time masks and `u128` comparisons that russh's ciphers and elliptic curves use.
- crypto-bigint 0.7.10, which fixes the advisory RUSTSEC-2026-0387 in the constant-time subtraction, division and Montgomery multiplication that russh's P-256, P-384 and P-521 curves use.
- tokio 1.53.3, which fixes the advisory RUSTSEC-2026-0386 in the constructors of `AsyncFd` on Unix, which neither the plugin's crates nor their dependencies call.

#### Git

- tokio 1.53.3, which fixes the advisory RUSTSEC-2026-0386 in the constructors of `AsyncFd` on Unix, which neither the plugin's crates nor their dependencies call.

#### MCP

- tokio 1.53.3, which fixes the advisory RUSTSEC-2026-0386 in the constructors of `AsyncFd` on Unix, which neither the plugin's crates nor their dependencies call.

#### Notebooks

- Accept the maintenance advisory RUSTSEC-2026-0192 for `ttf-parser`, which the pinned PDF library requires for font embedding that Notebooks never uses, with the owner's approval. Recheck when `pdf-extract` supports `lopdf` 0.44 or newer.
- tokio 1.53.3, which fixes the advisory RUSTSEC-2026-0386 in the constructors of `AsyncFd` on Unix, which neither the plugin's crates nor their dependencies call.

## 0.5.0 - 2026-10-22

### Added

- Agents run commands in the background, such as servers, watchers, `journalctl -f` and long test runs (ADR-0041): `start_command` returns at once, `read_command_output` reads the new output later or waits for a line, and `stop_command` ends the command with everything it started. A command keeps running after the reply until it ends, it is stopped, its conversation is deleted, its task ends or Mannie quits; the submenu "Commands" of the conversation's menu lists the commands by what their calls said they do, such as "Command 2: Running test ticks", with their output in a side panel that has Stop, and a command that ends on its own wakes an idle agent, which a Tools setting turns off.
- A plugin's tool may hand the core a command that it starts on a remote host in the background, through the service `background-commands`, and the core follows, reads and ends it as it does a local one (ADR-0041).
- Export a conversation and its child conversations as linked Markdown documents, with stored images, complete tool outputs, and explicit warnings for unavailable worker records.

#### Workers

- Complete saved task-conversation records for Markdown export, through an authenticated level-2 events read.

- A worker task's agent runs commands in the background under the host's ceiling, and they end with the task. The task's conversation on the master lists them with their states.

#### SSH

- The tool `start_ssh_command` starts a command on a registered host in the background, such as `journalctl -f`, which `read_command_output` reads and `stop_command` ends with TERM and KILL. Its approval says that it runs in the background, its lines never show the host's password, and editing or removing the host, turning SSH off and quitting end it.

### Changed

- A conversation's toolbar keeps the profile menu and the Git plugin's branch, and moves "Child agents", "Commands" and the export into a menu behind a button with vertical dots, which counts running commands and working child agents (ADR-0042). Full access and the context meter move into the message box, which becomes one framed field with its buttons in a row below the text. The toolbar's own menu of Rename, Compact now and Delete is gone, since the conversation list renames and deletes and the context meter compacts.
- Dialogs no longer blur the window behind them, which made slower machines stutter; the window behind is only dimmed.
- The output panel of a background command shows "Follow new lines" on the row of the command's state.
- Manual release builds can select Windows and FreeBSD artifacts, with worker signatures, checksums, SBOMs and notices; the static Linux worker remains an internal signature check.

#### Images

- The button that adds images sits in the row below the message box's text, round and without a border, like the row's other buttons.

#### SSH

- The host dialog's Password and Note for agents fields explain themselves behind a "(?)" button beside their labels, as the other fields do.

### Fixed

- Preserve shell termination output in background-command lifecycle tests, wait for plugin startup by elapsed time instead of a yield count, and compile the background supervisor without Unix async warnings.
- Keep background-command output readers open until process cleanup ends, so a Unix shell reporting a terminated child cannot exit early and leave processes running.
- Wait for the first rendered background-command output in the desktop test, so a cold Windows shell can initialize before the existing streaming assertions begin.
- Make subscription stress-test replies contain visible text so they follow their scripted turns, and report a closed test driver instead of spinning until the test times out.

- A dialog taller than the window scrolls, so that its last fields and its buttons can be reached.
- Reserve the test pool for the one-second parent-stop check so concurrent tests cannot consume its deadline.
- Wait for the child conversation to exist and attach its command listener before approval in the task crash-recovery test; report a child that exits before the intended kill point.
- Run desktop unit tests without concurrent Rust compilation so cold message evaluation stays within the existing timeout on Windows CI.
- Follow the current selected tab in desktop text assertions and compare native canonical export paths on Windows.
- Validation builds refuse the normal app identifier before opening owner data, including when a parallel build replaces the native test executable.

## 0.4.0 - 2026-10-21

### Added

- A plugin's tool may run commands on remote hosts that the owner registered with the plugin (ADR-0036): each command is an operation that the policy decides and the journal records, its approval shows the host, its address and its key, grants and rules for it cover only that host, and the Tools settings list those rules with their host.
- A plugin's tool may return archives with its result, which `read_tool_output` reads, and a plugin may keep a family of secrets with one entry for each thing it knows, such as a password for each host, which the Plugins page and `task plugins:grants` list.
- A plugin's tool learns the conversation's project folder and may read and change paths on the owner's machine (ADR-0038): its reads run without asking inside the project and ask outside it, its changes inside the project follow the setting for changes, and it can be offered only in projects, or only where a folder such as `.git` lies.
- A plugin's tool may give its call's card in the transcript data and a failure code, which the model never reads, and the plugin's interface shows the calls of its tools with its own icon, line and details, while the core keeps their accesses and approvals (ADR-0039).
- The process supervisor runs installed programs without a shell: a program receives its arguments exactly, reads its input and writes its output as bytes, and ends with its whole process tree at its timeout, when its output passes its limit, or when its caller lets it go (ADR-0038).
- A plugin may run an installed program that its manifest declares, such as git, through the core's `processes` service, which the Plugins page and `task plugins:grants` list with the program's fixed arguments (ADR-0038). A run by a plugin's tool is an operation whose approval shows the program, its folder and what it does beyond the workspace, such as running a repository's hooks, discarding work, rewriting commits that a remote already has or connecting to a remote. Grants and saved rules can allow such runs, a rule keeps the run's first words and what it does, and the Tools settings list these rules.

#### SSH

- The SSH plugin lets agents run commands on computers without Mannie. The owner registers a host in the SSH settings with its address, port, user name and password, which Mannie keeps in the system's credential store, and trusts its key after comparing the fingerprint; Mannie sends the password only to a host that shows that key, and refuses hosts that offer only RSA keys.
- The tool `run_ssh_command` runs one command on a registered host for the conversation agent and the `general` child agent. Each command asks the owner like a local command, with the host, its address and its key, and grants and rules cover only that host; full access runs commands without asking.
- A command's output reads as `run_command`'s, with long streams archived for `read_tool_output` and the host's password replaced. The timeout and Stop ask the host to end the command and close its channel.
- The system prompt lists the registered hosts, and "@" in the message box points an agent at one.

#### Git

- The git plugin gives agents tools for the owner's repositories, which run the installed git 2.40 or later without a shell: reading the status, the history, diffs, blame, searches, branches and tags and the conflicts; staging, committing, branching, switching, restoring, resetting and stashing; merging, rebasing, cherry-picking and reverting; fetching, pulling and pushing. The tools show only in a conversation whose project is in a repository.
- Reads run without asking inside the project. A change asks like a file change, and a change that runs the repository's hooks or programs that its configuration names, discards work, rewrites commits that a remote already has or connects to a remote asks first and says so. Grants and saved rules can allow these, and full access runs everything without asking. Uncommitted work that a change discards is kept in a stash entry first. Only fetches, pulls and pushes connect to a remote, so in a partial clone, a read or a change that needs an object that the clone lacks fails instead of fetching it.
- A merge, a rebase, a cherry-pick or a revert that stops on conflicts says where it stopped and which files conflict, with each conflict's current, base and incoming lines named by their commits. `git_resolve` resolves a conflict by taking a side, both sides or new content, and `git_operation` continues, skips or aborts. The operation stays git's own, so the owner can finish it in a terminal.
- `git_edit_history` rewords, squashes, fixes up, drops and reorders a branch's commits in one step, which changes nothing when a step conflicts.
- Each call of a git tool shows a card in the transcript: the changed files with their diffs, the commits, the branches and tags, the conflicts, what a fetch moved or a push updated, and how an operation ended.
- "@" in the message box mentions the uncommitted changes, a commit or a branch, which the model reads as a diff, a commit's message and patch, or a branch's commits and changed files.
- The Git button in a conversation's toolbar names the project's branch and opens the Git side panel: the branch with its upstream, the operation in progress, the changed files with their diffs, the history with each commit's changes, the branches and tags, and the conflicts. The panel reads the repository again after each change of the plugin's tools, on Refresh and every 15 seconds.

### Changed

- Turning a plugin off waits up to 5 seconds for the calls of its tools that run, then lets them go: an operation becomes uncertain, and a read ends as not run. An operation of a plugin's tool that the owner stopped while it ran is uncertain too, and each says why.
- `task ci` runs its checks side by side within limits that keep the machine usable, with Clippy in a target folder of its own, `target/clippy`, and ends with every step's result and time; a failed step no longer stops the others (ADR-0037). The Rust tests run with cargo-nextest, which `task setup` installs, dev builds keep line tables and no debug information for other crates, and Windows builds link with rust-lld.

- Refresh an existing release draft on retry and validate its version before starting CI or FreeBSD tests.
- Cache Rust compilations across hosted jobs, retain Clippy metadata and run selected Rust tests or desktop specs when repairing CI.
- Run all CI on standard GitHub-hosted runners for the public repository, and build and install-check Windows, Linux and FreeBSD artifacts through the Release workflow, including manual branch builds.

#### Workers

- The documents of tool calls on the link gain the field `interruption` (ADR-0036), which a master or a worker one minor version behind ignores; their fixtures for v0.4 are in `crates/mannie-link/tests/fixtures/documents/v0.4/`.

### Fixed

- The note in a command's cut output says how many bytes of a long line it leaves out, where it said the whole line was left out while showing part of it, and calls the archive whole only when Mannie kept all of the output.
- `read_tool_output` shows the beginning of a line longer than a page, marked as cut, and the next page starts after it; it showed no line and could not get past it.

- Wait for a sent message to appear before checking its reply in desktop tests.
- Make CI's text assertions work on WebKit and wait for completed runtime cleanup before retrying; measure parser work without time given to other CI processes.

#### Workers

- Check full displayed text in end-to-end tests on both WebView2 and WebKit.
- Apply read-only permissions to both the file and its directory in the Windows access test.

#### SSH

- Check full displayed text in end-to-end tests on both WebView2 and WebKit.

#### Git

- Check full displayed text in end-to-end tests on both WebView2 and WebKit.

### Security

- Patch certificate verification for RUSTSEC-2026-0382, rejecting the distrusted intermediates.

## 0.3.0 - 2026-10-19

### Security

- Patch certificate verification for RUSTSEC-2026-0382, rejecting the distrusted intermediates.

### Added

- Typing "@" in the message box lists the files and folders of the conversation's project, hidden and ignored ones included, and a file that may hold secrets shows a warning. The model reads a bounded page of each mentioned file and a listing of each mentioned folder after the message, and the transcript shows what it got.
- The Plugins settings list the plugins built into Mannie, each with a switch that takes effect at once, without a restart; a plugin that starts, stops or failed says so, a failed one offers "Try again", and turning off a plugin with work in progress asks first.
- While a plugin is off, its parts of the interface are gone, the address of one of its pages says that it is off and leads to the Plugins settings, and what it added to conversations stays readable.
- The Images plugin adds images to a message with a button that opens the file dialog, by pasting them or by dropping them on the message box. Mannie turns each image upright, shrinks it to at most 2048 pixels and drops its metadata before the model reads it, and the transcript shows thumbnails that open at full size.
- The Plugins settings list what each plugin may use: the services and credential entries that its manifest declares, its commands and channels, the programs that ship with it, the libraries that only it may use and its system access outside Mannie's services.

### Changed

- Workers are the first plugin: turning Workers off closes every link and lets running worker tasks go, which keep running on their workers and are followed again when Workers is back on. After the update from 0.2.0, Workers is on when a worker is paired; a new installation starts with it off.
- A failure while Workers starts, such as a list of paired workers that cannot be read, leaves Workers failed on the Plugins page and no longer keeps Mannie from starting.
- The worker service, the link and its contracts moved into `plugins/workers/` with their names unchanged, so workers of 0.2.0 keep working.
- `plugin_work` returns a plugin's work in progress as items, each a code of the plugin's and a count, so that every plugin can report kinds of work of its own.
- Plugins build on a plugin SDK: a plugin's commands go through the plugin host, which refuses them while the plugin is off and ends the plugin's channels when it stops, and a plugin's database statements and migrations reach only its own tables.
- Each plugin is a Cargo workspace with its own `Cargo.lock`, and `task gen` writes the desktop app's launcher and its lock from the plugins' manifests and the committed locks. `task setup` also fetches the crates and runs `task gen`, the worker builds with `cargo build --manifest-path plugins/workers/Cargo.toml -p mannie-worker`, and `task plugins:lock` and `task deps:align` keep the plugins' locks.
- `task gen` refuses a plugin whose dependencies cross the boundaries between the core and the plugins, a plugin's manifest names the crates that only it may use, and the desktop app no longer builds when the plugins changed since `task gen` ran, with a message that says to run it.
- `task plugins:new ID=<id>` makes a new plugin in `plugins/<id>/` from the fixture plugin, which validation builds bundle, and `task gen` bundles the new plugin with no change outside its folder.
- Each plugin keeps its messages in its own catalogs, which `task gen` checks and compiles with the core's into one catalog, and the pseudo-locale of the end-to-end tests covers them too.
- The fixture plugin, which validation builds bundle, has a UI that adds notes, so the core's end-to-end tests of the Plugins page use it in place of the plugins that ship, and `task plugins:new` starts a plugin with a UI and its tests.
- The Workers plugin builds on the plugin SDK alone: the worker's side of the link moved into `mannie-link-worker`, the worker and that crate reach the core through the host kit, `mannie-host-kit`, the plugin's test tools moved into `mannie-workers-testing`, and the boundary check has no exceptions left.
- The documents describe the plugin SDK: how a plugin builds on it alone, where a plugin's documents and changes live, the threats it guards against and how to add a plugin with `task plugins:new`.
- The documents of Workers and Images, the guide to setting up a worker included, moved into their plugins' folders, `plugins/workers/docs/` and `plugins/images/docs/`.
- Each plugin keeps its changes in its own `plugins/<id>/CHANGELOG.md`, which `task release:prepare` moves into Mannie's changelog under the plugin's name, and `task docs:check` reads the plugins' documents and their own ADRs in `plugins/<id>/docs/`.
- A plugin's manifest may grant its library crates direct file writes, processes or sockets under `[native]`, each with its reason, `task lint:rust` refuses every other such call in the library crates that the plugin's entry crate ships, and `task plugins:grants` lists what each plugin may use. Workers grants the link its sockets.
- Plugins' tests reach the core through the plugin harness, `mannie-plugin-harness`, which hosts one plugin through the real plugin host and checks that a plugin keeps the SDK's rules; Images' tests and the fixture plugin's use it.
- Each plugin's UI and its tests live in the plugin's folder. The UI imports only the TypeScript plugin SDK, `@mannie/plugin-sdk`, and its own files, `task gen` writes the plugin lists that the app imports and each plugin's barrel of its own messages, and ESLint and the app's build refuse an import across the boundaries between the core and the plugins.
- The programs of the plugins, such as the worker, come from their manifests: `task build`, `task e2e`, CI's smoke tests and cross builds, the nightly FreeBSD run and the release build, check, sign and describe every program that a manifest declares, and the worker's smoke tests moved into `plugins/workers/ci/`.
- The architecture decisions that were Proposed are Accepted, and a failed or missing CI run no longer holds back a change, a milestone or a release.
- `task test` runs every Rust test binary even when one fails, so a run reports every failing test.
- CI, the live provider tests and the evaluations run on the owner's self-hosted runners, which GitHub does not bill; the FreeBSD tests and the release's own jobs stay on GitHub's runners.

### Fixed

- Set denied writes explicitly in the Windows worker permission test when it runs under an administrator account.
- Commands on Windows receive `PSModuleAnalysisCachePath`, so Windows PowerShell reads the module cache that the system prepared; without it, a command on GitHub's Windows runners took about 20 seconds to start.
- CI and the FreeBSD tests build with line tables in place of full debug information, which outgrew the disk of the Linux runners, and the runtime's test helpers wait up to two minutes for a state, as the link's do.

## 0.2.0 - 2026-10-16

### Added

- System tools for the model on the owner's computer: system information, listing, finding and searching files, reading, writing and editing text files with revisions, creating, copying, moving and deleting files and folders, running commands in Windows PowerShell or `/bin/sh`, and reading back long outputs.
- Projects in the sidebar: "+" turns a folder into a project, its conversations work in that folder, a general conversation moves into a project when the owner's request is about it, and removing a project deletes its conversations; reads inside a project's folder run without asking, and changes ask unless the settings allow them.
- Approvals that show exactly what will happen, with Allow once, Allow for this conversation, this project or always, Save rule… and Deny, and lists of the permissions that apply, which the owner can revoke.
- Tool calls in the reply as compact rows that the model names in a few words, such as "Reading package.json", with their details and full output on demand; the status line and the conversation list say when a reply waits for an approval.
- Commands that end with every process they started, through Job Objects on Windows and a subreaper on Linux, and Stop that ends a running command within a second.
- A journal of every operation, so that a crash never repeats one and an operation that may have run is shown as such.
- The Tools settings: changes inside the workspace without asking, the command rules, the grants for projects and every conversation, and the environment variables that commands receive.
- A "Use tools" switch for each provider profile, and a Tools check in the provider test.
- Evaluations of the system tools with real models and with a scripted oracle, `task tools:eval` and a weekly workflow.
- Child agents: the conversation agent gives self-contained tasks to explore children, which only read and search, and general children, which can also change files and run commands, on the same computer and in the conversation's workspace, in parallel, in the background, and as follow-ups that keep the child's context; only each child's confirmed report reaches the conversation.
- The card of each task in the reply, with the child's number, title, state and current activity, the child's approval requests to answer in place, Stop for one child, and the instruction and the report on demand.
- Side panels beside the main view: "Open conversation" shows a child's whole conversation read-only, with a tab for each open conversation that has its own Close, a resizable split that works from the keyboard, and Close the side panel in the command palette.
- A "Child agents" button in the conversation's toolbar that says how many child agents work, lists them, and opens a child's conversation in a side panel.
- The Child agents settings: how many child agents work at the same time, from 1 to 16, 4 by default.
- Evaluations of tasks split across child agents.
- Workers: `mannie-worker` runs Mannie's agent runtime on another Windows, Linux or FreeBSD computer as a service, with a configuration that only root or the administrators can change, and takes tasks that the conversation agent delegates to it with `delegate_task`.
- Pairing a worker with a one-time code that `mannie-worker pair` prints on its host, over a link of mutual TLS 1.3 where each side pins the other's key, and removing a worker, which unpairs it.
- Naming a worker in Mannie: an optional name when pairing it, and "Rename…" in its menu and in the Workers settings; Mannie and the conversation agent use that name in place of the one in the worker's configuration, and an empty name goes back to it.
- A ceiling on each worker's host: reads only, asking before every change and command, or changing its allowed folders and running the commands of its rules without asking; a task can narrow it and never widen it, and a worker's approval requests reach the owner in the task's card.
- A worker's tasks in the conversation as cards with the worker's name, Stop, and the task's conversation live in a side panel; a worker task keeps running when Mannie quits, and `wait_for_tasks` reads its report after a restart.
- Leases of the provider's credential: the worker keeps it only in memory, hidden from its logs, for as long as the task runs.
- The "Workers" group in the sidebar, "Pair a worker…" in the command palette, and the Workers settings with each worker's status, version, host and ceiling.
- `mannie-worker status`, `facts`, `self-check --json`, and `update`, which installs a signed release on the host and puts the previous program back when the new one does not start.
- Compatibility with workers one minor version behind, checked by `buf breaking` on the link's contracts and by fixtures of the documents that the link carries (`task lint:proto`).
- Full access: a "Full access" button on each conversation and a setting for new conversations that run every command and change without asking, after a confirmation, with a mark in the conversation list; a worker mode `full_access`; and a conversation in full access answers the requests of its worker tasks unless the worker refuses that.

### Changed

- The help of a form field sits behind a "(?)" button beside its label, in the provider profile dialog, the pairing dialog and the Tools, Child agents and Interface settings.
- The owner's instructions hold up to 32,768 characters, where the limit was 32 KiB, and the counter under them counts characters.

### Fixed

- The provider profile menu in the conversation's toolbar is wide enough for its title and its profiles; it had the width of its button, so its lines wrapped.
- The fields of the provider profile dialog scroll only down; the click area of the "Use tools" switch reached past their edge, so they also scrolled a little to the side.
- The conversation view stops following a reply only when the owner scrolls up; a tool call or an approval request that appeared at once, or a message box that shrank after sending, could end the following and leave the newest activity out of view.
- A long reply no longer stretches the pane around the conversation: the text that only screen readers read escaped the transcript's scrolling, so the wheel could scroll the whole view away into empty space.
- The message box keeps the focus after the first message of a new conversation; the conversation's own pane replaced the new conversation's, and the focus fell to the page.
- A collapsed project expands whenever the open conversation moves into it; a move that came in the same batch of updates as the new conversation, or before the view showed the conversation, left it collapsed.

### Security

- rustls 0.23.46, which fixes the advisories RUSTSEC-2026-0346, RUSTSEC-2026-0347, RUSTSEC-2026-0348 and RUSTSEC-2026-0349.

## 0.1.0 - 2026-10-14

### Added

- Documentation baseline: vision, goals and scope, the feature protocol, architecture, architecture decision records, milestones, engineering standards and agent rules.
- Documentation checks, run with `task docs:check`.
- Decisions on the interface language, closing the window, code signing and release hosting, with the window and tray feature and ADR-0019 for message catalogs.
- The Cargo workspace on a pinned Rust 1.99 toolchain, with the `mannie-types`, `mannie-store`, `mannie-logging` and `mannie-secrets` crates, the desktop app and the worker.
- The desktop shell: Tauri 2 with the security baseline of ADR-0004, a SvelteKit single-page app built from shadcn-svelte components, a sidebar, a command palette (Ctrl+K), the settings view (Ctrl+,), light and dark themes, and one running instance.
- Typed IPC bindings generated from Rust (`task gen`), and channel streams in batches paced by the display's frames.
- The SQLite store, with migrations, a copy of the database before each migration, and refusal of a database from a newer version.
- Credentials in Windows Credential Manager and the Linux Secret Service; without a Secret Service, Mannie stores none.
- Logs in daily rotating files, with secrets redacted, and crash files.
- The worker: a headless service that starts, logs and opens its store, and installs itself as a Windows service, a systemd unit or an rc.d service with `mannie-worker service install`.
- The interface language as a setting, with English message catalogs and a pseudo-locale that the end-to-end tests check.
- Mannie's application icon.
- CI on Windows and Linux, nightly worker tests on FreeBSD, and a release workflow that builds, checks, signs and drafts a release from a version tag.
- The tasks `gen`, `dev`, `build`, `lint`, `test`, `e2e` and `release:prepare`.
- The shadcn-svelte agent skill and the Svelte MCP server for coding agents.
- Chat with OpenAI models and OpenAI-compatible servers, such as vLLM, SGLang, LM Studio, Ollama and OpenRouter, through the Responses API or Chat Completions, with replies that stream into the conversation view.
- Provider profiles in the settings, with credentials in the system's credential store, the known limits of common models filled in, and a provider test of the credential, the server, the model and a short reply.
- Replies that continue in the core while the window reloads or hides: Stop, also with Escape, Retry, automatic retries that wait as long as the provider asks, and replies that a crash or a quit interrupted, which can be retried.
- The conversation list, with rename and delete; a deleted conversation's text is overwritten in the database.
- Model output as Markdown: raw HTML shows as text, images are never loaded, and a link opens only after the owner confirms its address.
- Reasoning shown apart from the answer, and encrypted reasoning sent back only to the profile that wrote it.
- The context budget of each profile, with a meter, automatic compaction into a summary near the limit, and Compact now.
- Instructions from the owner that apply to every conversation.
- A note under a reply when a server, such as Ollama beyond a model's context size, cut the request without an error.
- The choice on the first close between running in the system tray and quitting, the tray menu, and a warning before quitting while replies are being written.
- The tasks `e2e:perf`, `bench`, `provider:live` and `provider:record`, a nightly workflow of live provider tests, and end-to-end tests that keep the app's window off the screen.
- The settings as a page of their own, with a page per section in the settings sidebar and a link back to the main view.
- A provider profile dialog that shows example values in empty fields, says when an API key is saved, and warns before saving replaces or removes it; icon buttons for Send and Retry; and plainer wording across the interface.
