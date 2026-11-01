# Terminals

|              |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Plugin       | `terminal`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Milestone    | M14                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| Last updated | 2026-10-30                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Related      | [ADR-0053](../../../docs/adr/0053-let-plugins-offer-terminals.md), [ADR-0054](../../../docs/adr/0054-let-tool-results-keep-what-they-cite-and-let-plugin-views-add-mentions.md), [mark commands with shell integration](adr/0001-mark-commands-with-shell-integration.md), [keep a headless emulator](adr/0002-keep-a-headless-emulator-of-each-terminal-in-rust.md), [SSH hosts](../../ssh/docs/README.md), [Plugins](../../../docs/features/plugins.md), [Operations and approvals](../../../docs/features/system-tools/operations-and-approvals.md), [Mentions](../../../docs/features/chat/mentions.md), [Goals and scope](../../../docs/product/goals-and-scope.md#41-performance), [M14 validation results](../../../docs/roadmap/m14-validation.md) |

## Intent

The owner wants terminals in Mannie, as a plugin. In the owner's words: "A terminal that can be opened in a sidebar, can be integrated with workers and ssh plugins (so that i can execute commands on a worker machine or via ssh host directly)". Agents run commands in those terminals and may point the owner at their output, as when the owner asks "Find occupied ports" and the reply points "to the evidence from a command it might have run in the terminal". The owner can also "point to some command and output directly". Later the owner asked that terminals cause no lag: "i remember implementing such things causing performance issues on low end PCs and laptops (especially laptops)" ([D45](../../../docs/roadmap/milestones.md#5-decisions)).

Version 1 opens terminals on this machine and on the hosts of the SSH plugin, as tabs of the side panel area beside the conversation. Terminals on a worker's host are an [idea](worker-terminals.md) for later (D45).

## Behavior

### Turning it on

- Terminal is a plugin on the Plugins page, with a switch. It starts off on a new installation, like every plugin ([D28](../../../docs/roadmap/milestones.md#5-decisions)).
- SSH hosts show in Terminal only while SSH is on.
- Turning Terminal off ends every terminal, and asks first while a terminal runs a command.

### Opening a terminal

- A Terminal button in the conversation's toolbar opens a menu. It lists the conversation's terminals with their state, and has "New terminal" with "This machine", with its shells, and the SSH hosts that are ready. A host that is unavailable shows disabled, with the reason.
- `` Ctrl+` `` and the palette command "Open a terminal" open the conversation's last terminal, or a new one on this machine, and move the focus into it. A key that the panel names moves the focus out of the terminal.
- Each terminal is a tab of the side panel area, labeled with its number, its shell and its host.
- A terminal on this machine runs one of its shells, and starts in the conversation's project folder, else in the home folder:
  - On Windows: PowerShell 7 when it is installed, Windows PowerShell, Command Prompt, and Git Bash when it is installed. The default is PowerShell 7, else Windows PowerShell.
  - On Linux: the account's login shell and the shells that `/etc/shells` lists. The default is the login shell.
- A terminal on an SSH host runs the account's login shell there.
- A conversation holds at most 8 terminals, and this machine at most 32, those on SSH hosts included.

### The panel

- The panel offers "Find", the command list, and "End", which asks first while a command runs.
- The panel shows when the shell is plain, so that agents cannot run commands in it, and when the terminal has ended, with "Start again".
- Selecting text in the terminal offers "Mention selection".
- Closing the tab keeps the terminal running. The panel keeps its terminal view while the conversation is open, and starts from a snapshot of the terminal when the conversation opens again or the window reloads.
- A terminal ends when the owner ends it, its shell exits, its conversation is deleted, Terminal turns off or Mannie quits. Terminals live in memory and do not come back after a restart.
- The warning before closing or quitting counts the terminals that run a command.

### The command list

- The command list is accessible, works from the keyboard and serves the screen reader.
- Each command shows who ran it, the owner or the agent, its exit code and its time.
- Its actions are "Show", "Mention in message", "Copy command", "Copy output" and "Show output as text", which opens a dialog.

### Agents' commands

- The conversation agent gets two tools. Child agents and worker tasks get none.
  - `list_terminals` lists the conversation's terminals: their numbers, hosts, shells and states, and whether they mark commands. It never shows the text of the owner's commands.
  - `run_in_terminal` takes `command`, then `terminal`, or `host` and `shell`, and `timeout_seconds`. It returns the terminal, the block, the exit code, the duration and the folder, the block's cite link, and the output with numbered lines, cut to the result's limit with the rest in archives. It also returns the block as a cited item ([ADR-0054](../../../docs/adr/0054-let-tool-results-keep-what-they-cite-and-let-plugin-views-add-mentions.md)).
- An agent types one line of at most 16 KiB, without control characters, and only at an idle prompt: never while the owner types or a command runs.
- An agent reuses its own idle terminal on the host, or opens one as part of its command's approval. The terminals that agents open show in the toolbar's menu, and open from the tool card or a citation. No panel opens by itself.
- Each command asks the owner, as a command of `run_command` does. The request names the terminal, the host, the user and the folder, and says that the command runs as typed, in a shell that earlier commands set up. The owner can:
  - allow it once;
  - allow it for this conversation, this project or every conversation, which covers the same command on the same host, and on this machine in the same shell;
  - save a rule of its leading words, which applies only on that host;
  - deny it.
- A command with shell operators offers only "Allow once" and "Deny". Full access runs commands in terminals without asking.
- Grants and rules given from a terminal command's approval cover only terminal commands, and those of `run_command` and `run_ssh_command` never cover one.
- A command does not run when the terminal's user or folder changed after the owner allowed it.
- A command waits 60 seconds unless the call names up to 600. At the timeout, and when the owner stops the reply, Mannie sends Ctrl+C. When the command has not ended 5 seconds later, the result says that it still runs.
- A tool card shows "Ran `…` in terminal 2 · exit 0", with "Show in terminal".

### What the model knows

- In runs that have the tools, the instruction section lists the hosts and says three things: terminals are for commands that the owner should see or that need a terminal's state; the agent sees only its own commands and the owner's mentions; and it cites output by copying its link, adding `#L<a>-L<b>` for lines.
- An agent reads nothing else of a terminal: neither what the owner types nor the output of the owner's commands, unless the owner mentions them (D45).

### Evidence

- A citation in a reply shows the cited lines in its popover. "Show in terminal" opens the panel, scrolls to the block and marks the lines.
- After the terminal ended, and after a restart, the popover still shows the stored lines.

### Pointing an agent at output

- "Mention in message", "Mention selection" and the "@" group Terminal, which lists the conversation's recent commands, add a mention to the message box.
- The model reads the terminal, the host, the command, its exit code and its output, or the selection, with the block's cite link.

### Settings

- The Terminal settings set the default shell of this machine, the lines of scrollback, the font size, whether the cursor blinks, the renderer (automatic, GPU or compatible) and screen reader mode.
- Scrollback holds 2,000 lines unless the owner chooses 500 to 20,000, so that a low-end laptop stays within the memory budget.
- The cursor does not blink, the renderer is chosen automatically, and screen reader mode is off, until the owner changes them.

### SSH hosts

- The SSH plugin offers its hosts: a pseudo terminal and the account's login shell on the host's pooled connection, with the login shell read when the terminal opens and the host's password replaced in the stream ([SSH hosts](../../ssh/docs/README.md)).
- Opening a terminal on a host logs in, as testing the login does. A host whose key changed refuses to open.

### Performance

- Terminals stay within the budgets NFR-PERF-06 to 10 on the reference machine and on a low-end laptop on battery ([goals and scope](../../../docs/product/goals-and-scope.md#41-performance)).
- The page parses only what the owner can see: a hidden view pauses, and a flood becomes a jump to the terminal's current state ([keep a headless emulator](adr/0002-keep-a-headless-emulator-of-each-terminal-in-rust.md)).
- Idle terminals cost nothing: nothing polls, and a hidden or minimized window pauses every view.
- xterm.js loads with the first terminal, and the plugin's Rust side starts nothing until a terminal opens.

### What it does not do

- It opens no terminal on a worker's host, restores no terminal after a restart, leaves no agent's command running after its call, and offers neither WSL nor shells beyond those above. These are [ideas](#parts) for later.
- It stores nothing of a terminal except what reaches the model: tool results, the items they cite, and mention parts. What the owner types is never logged or stored.

## Fit

- Builds on the extension points of [ADR-0053](../../../docs/adr/0053-let-plugins-offer-terminals.md): the service `terminals`, which starts terminals of this machine in the core's supervisor and opens terminals on the hosts that the point `terminal-hosts` offers; the terminal-command access, which the core's policy decides and its journal records; xterm.js through `@mannie/plugin-sdk/terminal`; and a key for a command of the palette.
- Builds on [ADR-0054](../../../docs/adr/0054-let-tool-results-keep-what-they-cite-and-let-plugin-views-add-mentions.md): cited items in its tool's results, lines in cite keys, a context kind without a context provider, and mentions from its panel through `@mannie/plugin-sdk/message-box`.
- Adds a conversation control, a kind of side panel, two tools with their tool kinds, an instruction section, a mention provider with its kind, a command of the palette and a settings section.
- The SSH plugin offers its hosts through the point `terminal-hosts` and opens their terminals, so this plugin never reads another plugin's hosts or secrets.
- Follows [Plugins](../../../docs/features/plugins.md): a switch, a start and a stop that take effect at once, the plugin's code in `plugins/terminal/`, its UI files named `terminal` and its messages under `terminal_`.
- Changes the [window and tray](../../../docs/features/window-and-tray.md) feature: the warning before closing or quitting counts the terminals that run a command.
- Must not change: `run_command`, `run_ssh_command` and their grants and rules, which never cover a terminal command, and the other plugins beyond the SSH plugin's source of hosts.

## Open questions

None.

## Decisions

- 2026-10-30: the owner decided ([D45](../../../docs/roadmap/milestones.md#5-decisions)):
  - Version 1 opens terminals on this machine and on SSH hosts. Terminals on a worker's host become an [idea](worker-terminals.md): they need an ADR that supersedes ADR-0015 d2 in part, since the master never runs tools on a worker's host (D5).
  - Terminals open as tabs of the side panel area beside the conversation. Each terminal belongs to the conversation that opened it (D18), and only that conversation's agent uses it.
  - An agent reads only two things from a terminal: the output of the commands that it ran, and what the owner mentions.
  - Terminals must not lag on low-end PCs and laptops, so they get budgets of their own on a low-end laptop profile, NFR-PERF-06 to 10.
  - Approving the plan accepts ADR-0053, ADR-0054 and the plugin's first two ADRs, [mark commands with shell integration](adr/0001-mark-commands-with-shell-integration.md) and [keep a headless emulator of each terminal in Rust](adr/0002-keep-a-headless-emulator-of-each-terminal-in-rust.md).
- 2026-10-30: the defaults below were chosen when shaping, and wait for the owner's review:
  - The plugin starts off, like every plugin, and SSH hosts show in it only while SSH is on.
  - A conversation holds at most 8 terminals, and this machine at most 32, those on SSH hosts included.
  - Terminals live in memory. Closing a tab keeps a terminal running, a terminal ends when the owner ends it, its shell exits, its conversation is deleted, the plugin turns off or Mannie quits, and no terminal comes back after a restart.
  - The shells of this machine, their defaults, and a start in the conversation's project folder, else in the home folder.
  - Only the conversation agent gets the tools. Child agents and worker tasks do not.
  - An agent types one line of at most 16 KiB without control characters, only at an idle prompt, and waits 60 seconds by default and 600 at most. At the timeout, or on Stop, it sends Ctrl+C.
  - An agent reuses its own idle terminal on the host, or opens one as part of its command's approval. Terminals that agents open show in the toolbar's menu and open from the tool card or a citation, and no panel opens by itself.
  - Scrollback holds 2,000 lines, and the settings allow 500 to 20,000.
  - The cursor does not blink, the renderer is chosen automatically, and screen reader mode is a setting that is off.
  - Nothing of a terminal is stored except what reaches the model: tool results, the items they cite, and mention parts. The owner's input is never logged or stored.
  - `` Ctrl+` `` opens the conversation's last terminal, or a new one on this machine, and moves the focus into it. A key that the panel names moves the focus out.
- 2026-10-30: the choices below were made when building, and wait for the owner's review:
  - A view that hides, because its tab, its conversation or the window hides, closes its channel, and one that shows again opens it and starts from a full snapshot. So no command pauses a view, and "Show in terminal" scrolls and marks within the view.
  - "Mention in message" of a command goes through the command `mention_block`, which checks the block before the panel puts the mention into the message box.
  - Counts of bytes cross IPC as numbers up to 2^53, since the SDK's bindings refuse 64-bit integers.
  - A remote host's setup is chosen when it is typed, after the first prompt, from the login shell that the host's plugin lists by then, since the SSH plugin reads it as the first terminal there opens. A host whose shell is not known within 2 seconds of the first prompt gets a plain terminal.
  - The fakes that the plugin's tests use live in the SDK's testing module, as the other services' fakes do: one stands in for the core's terminals, and a source of hosts whose shells speak the markers.
  - A panel moves the focus into its terminal once the terminal first shows, whether `` Ctrl+` ``, the control, a citation or a card opened it.
  - Besides the DOM renderer's style elements, the patches of xterm.js cover the style element of its scrollbar, and define `toString` in VS Code's `KeyCodeUtils`, which xterm.js and its WebGL addon carry, since Tauri's `freezePrototype` refuses the assignment (M14 validation results, section 7.1).

## Parts

- [Worker terminals](worker-terminals.md): terminals on a worker's host, offered by the Workers plugin. An idea.
- [Restored terminals](restored-terminals.md): terminals that come back after Mannie restarts. An idea.
- [Commands left running](commands-left-running.md): agents' commands that keep running in a terminal after their call. An idea.
- [More local shells](more-local-shells.md): WSL and more shells of this machine. An idea.

## Design

### The plugin

`plugins/terminal/crates/mannie-terminal` defines the plugin `terminal`:

- Its manifest, `plugins/terminal/plugin.toml`, declares the services `store` and `terminals`, and the crate `vt100`, which only it may use. It has no `[native]` grant, since the core's supervisor and the SSH plugin open the pseudo terminals. Its default is off.
- Its start contributes the tools, the instruction section and the mention provider, follows the core's list of hosts, and starts nothing else until a terminal opens. `Instance::work` lists the terminals that run a command, and `Instance::stop` ends them.
- Its tests use the SDK's testing module: `FakeTerminals` stands in for the core's terminals, and `ScriptedHosts` offers hosts whose scripted shells speak the markers through the point `terminal-hosts`, so an agent's command passes the core's registry and policy.

### Contracts

- Commands, which the host's dispatch answers with `plugin_off` while Terminal is off ([ADR-0034](../../../docs/adr/0034-build-plugins-on-a-plugin-sdk.md), decision 8): `open_terminal`, `write_terminal`, `resize_terminal` and `end_terminal` for a terminal of the conversation; `acknowledge_output` for a view's flow ([keep a headless emulator](adr/0002-keep-a-headless-emulator-of-each-terminal-in-rust.md)); `block_text`, `mention_block` and `mention_selection`, which serve the output of a block as text, "Mention in message" and "Mention selection"; and `get_settings` and `update_settings`. `write_terminal` takes at most 64 KiB, and counts of bytes cross as numbers up to 2^53.
- Channels: `follow_hosts` sends this machine with its shells and the hosts of the sources, `follow_terminals` a conversation's terminals with their states, and `follow_terminal` a full snapshot first, then output text, marks and later snapshots in order. A view that hides closes its `follow_terminal`, and one that shows again opens it.
- The tools `list_terminals` and `run_in_terminal`, offered to the conversation agent. The preparation of `run_in_terminal` returns one terminal-command access: `ToolAccess::TerminalCommand`, which the core maps onto `Access::TerminalCommand`, with the reason `terminal_command`, grants of `GrantScope::TerminalCommand`, rules in `tools_terminal_rules`, and the refusal `no_terminals` on a worker ([ADR-0053](../../../docs/adr/0053-let-plugins-offer-terminals.md), decision 5). Its result carries the block in `cited`.
- The mention provider `terminal` lists the conversation's recent commands, and its kind words the group Terminal. The context kind `terminal` opens a cite key in its panel, and no context provider has its ID ([ADR-0054](../../../docs/adr/0054-let-tool-results-keep-what-they-cite-and-let-plugin-views-add-mentions.md), decision 3).
- The side panel kind `terminal.session` takes the argument `<conversation>.<number>`.
- The palette command "Open a terminal" names its key, `Backquote` with Ctrl, in `key`.
- The toolbar's control and the palette command read the open conversation with `openConversationId`, and a terminal on this machine starts in the folder that `projectFolder` gives for the conversation.
- The panel puts mentions into the message box with `mention` of `@mannie/plugin-sdk/message-box`.

### Data

- The plugin's migration 1 creates `terminal_settings`, which holds the settings above. The plugin stores nothing else; the core keeps the tool results, their cited items and the mention parts with their conversations.

### Interface

- The conversation control Terminal, the side panel `terminal.session` with its command list and dialog, the tool kinds of the two tools, the context kind and the mention kind `terminal`, the palette command and the settings section Terminal.
- Messages live in `plugins/terminal/messages/en.json` under `terminal_`.

### Threat notes

- Script in a compromised webview can type into a terminal or open one. It could already turn on full access and send a message, and the plugin starts off (T-68).
- Every marker carries the terminal's nonce, so a marker that output forges stays output (T-69).
- A granted command runs in a shell that earlier commands set up, so terminal grants and rules stay apart from all others, an approval checks the terminal's user and folder, and shell operators get "Allow once" only (T-70).
- Agents read only their own commands' output and the owner's mentions, the SSH plugin replaces the host's password in the stream, and the owner's input is never kept (T-71).
- The toolbar's count, the quit warning and the ending rules bound commands that keep running (T-72). Output that asks for more commands is content and grants nothing (T-73).

### Platform differences

- On Windows, terminals run on ConPTY, which needs Windows 10 1809 or later. cmd marks only its prompts and gives no exit code.
- On Linux, terminals run on a pseudo terminal of the system. sh marks only its prompts.
- On an SSH host, the plugin types the setup after the first prompt, for the login shell that the SSH plugin lists by then, which it reads as the first terminal on the host opens ([mark commands with shell integration](adr/0001-mark-commands-with-shell-integration.md)). A host whose login shell no setup reaches, such as `cmd.exe` on OpenSSH for Windows, gives a plain terminal.

## Acceptance

- The owner opens terminals from the control and with `` Ctrl+` ``, on this machine (PowerShell, cmd and Git Bash on Windows; bash, zsh, fish and sh on Linux) and on an SSH host, and types, resizes, finds and ends them. Closing a tab keeps a terminal, and deleting its conversation, turning the plugin off and quitting each end it. Nothing that a terminal started stays alive on Windows and Linux.
- An agent runs a command in a new terminal after the owner allows it, and the owner watches it run. A grant for the conversation runs it again without asking, and the same command through `run_command`, or on another host, still asks. An agent never types while the owner types or a command runs, nor after the approved terminal's user or folder changed. Stop and the timeout interrupt the command, and turning the plugin off during a call leaves it uncertain within 6 seconds.
- A reply cites a command's output by a link, its popover shows the cited lines, and "Show in terminal" scrolls to them and marks them. The popover still shows the lines after the terminal ended and after a restart.
- The owner mentions a command, a selection and an "@" entry, and the model reads them. Nothing else of the owner's commands reaches the model.
- A forged marker changes no block. The SSH host's password appears in no terminal output, log, table or tool result. A host whose key changed refuses to open.
- Axe finds no WCAG 2.1 AA violation in the control, the panel, the command list, the dialogs, the settings and the cards, with screen reader mode on and off, on WebView2 and WebKitGTK. The focus leaves the terminal with the documented key.
- NFR-PERF-06 to 10 are met on the reference machine and under the emulated low-end profile on WebView2 and WebKitGTK.
- The checks that show it:
  - The plugin's tests in `plugins/terminal/crates/mannie-terminal/tests/`, against real shells of each platform and the scripted shell: blocks, absolute rows and the text log, forged markers, busy and typing detection, changes of state, timeout and Stop, snapshots, paused views and the watermark, the caps of memory, mentions and selections, cite keys, and the SDK's conformance check.
  - Runtime tests with a scripted provider: Allow once, a grant, a busy terminal, a changed state, a timeout, Stop, the plugin turned off during a call, and a cited reply.
  - The SSH plugin's tests against its server in the test's process: a pseudo terminal, resize, the login shell, the password replaced when chunks split it, a long typed line, a changed key, an approval whose endpoint changed, and the end of a terminal whose session closed.
  - The plugin's UI passes its Vitest tests in `plugins/terminal/tests/ui/`.
  - In the app (`plugins/terminal/tests/e2e/terminal.test.ts`), with axe in both themes: turn Terminal on, open a terminal with `` Ctrl+` ``, type `echo` and see its output; a scripted model calls `run_in_terminal`, the approval shows the terminal access, and the owner allows it once; the card shows the command, the reply's citation opens its popover, and "Show in terminal" marks the lines; mention a command and a selection; end the terminal.
  - `task e2e:perf` with `tests/e2e/perf/terminal.test.ts` on the reference machine and under `MANNIE_PERF_PROFILE=low-end`, the Linux virtual machine with 2 vCPUs, and `task bench` for the emulator.
- The owner's checks:
  - On a low-end laptop on battery: typing in a terminal, a flood of `yes`, and 8 open terminals while chatting show no lag.
  - "Find occupied ports" on Windows in PowerShell and on a Linux SSH host, with the cited lines.
  - A FreeBSD SSH host with sh.
  - A `sudo` prompt that the owner answers during an agent's command.
  - "Why did this fail?" with a mentioned command.
