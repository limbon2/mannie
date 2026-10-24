# Changelog

All notable changes to Mannie are recorded in this file. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/). Each change adds one line under Unreleased, and a release turns that section into its notes. A change to a plugin goes into the plugin's own changelog, `plugins/<id>/CHANGELOG.md`, which a release moves into its section here under the plugin's name.

## Unreleased

### Added

- Plugins publish tools while they run, through the point `tool-sources`: each set replaces the source's tools, a run keeps the set that it started with, and a published tool gives way to any tool of the same name (ADR-0043).
- A plugin's tool may call a tool server's tool as an external-tool access, which the owner allows once, for the conversation, for the project or for every conversation, covering the same tool of the same server at the same endpoint with any arguments; full access runs every call, a worker's ceiling refuses them, and the approval shows the server's hints as its claims (ADR-0043).
- Plugins start the programs of local tool servers through the service `local-servers`, which finds the program on Mannie's `PATH`, runs it in the supervisor with the environment of commands and the server's own variables and secrets, and ends it when the plugin stops or Mannie quits (ADR-0043).
- The images of tool results reach the model, PNG and JPEG without their metadata, at most 4 of a result, through the Responses API's tool outputs and a message after the tool results on Chat Completions; the call's details show them and the export keeps them (ADR-0044).
- A plugin's tool may have a schema that is not strict, which the providers' strict mode leaves as it is, and a plugin's UI opens a link through the core's checks with `openLink` and describes every tool that it publishes with one tool kind (ADR-0043).
- The evaluations count, for each report of a child agent, the parent's reads and searches that repeat what a child read or searched, apart from its checks of files that a child changed. A new task fails when the parent repeats more than two of them for its report, and `MANNIE_EVAL_RUNS` runs each task several times.

### Changed

- The "@" list shows each kind of mention as soon as it answers, and while more than one kind shows entries, each shows its first 5 with "Show N more", so Git's and SSH's entries no longer sit below 50 files. A space no longer ends the query while an entry that the list shows holds it, so a name with spaces, such as a commit's subject or a path with a space, can be typed out.
- Child agents write their reports in sections: status, answer, established findings with their evidence, what they did not verify, changes, what is left open and their assumptions. A report's result says that its evidence comes from the child's own tool calls, and the guidance on tasks treats what a report backs with evidence as established, with at most two tool calls to check one report, on the files that a task changed and the claims that it did not verify.
- Checkpoints are written under fixed headings: the goal, what is done with what shows it, established facts with their sources, decisions, what nobody verified, open questions and the next steps.

### Fixed

- The "@" list keeps up in large projects: queries that arrive while the project's folder is read share that one reading instead of each reading the folder again, an index older than 15 seconds answers at once while the folder is read again in the background, and a query no longer sorts every match.
- A summary that reaches the output limit no longer becomes a checkpoint: the compaction fails with `summary_cut`, which asks the owner to raise the profile's output limit.
- The styles that only a plugin's interface uses reach the app, such as the row of a host's name and port in SSH's host dialog: Tailwind read no file of the plugins' folders, because their source pattern named the folders instead of their files.

### Security

- ctutils 0.4.5, which fixes the advisory RUSTSEC-2026-0385 in the constant-time masks and `u128` comparisons that the Secret Service credential store reaches through its hashes.

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
