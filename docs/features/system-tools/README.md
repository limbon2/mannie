# System tools

|              |                                                                                                                                                                                                                                                                                                                                                     |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                            |
| Milestone    | M2                                                                                                                                                                                                                                                                                                                                                  |
| Last updated | 2026-10-14                                                                                                                                                                                                                                                                                                                                          |
| Related      | [ADR-0013](../../adr/0013-journal-every-side-effect.md), [ADR-0014](../../adr/0014-contain-process-trees.md), [ADR-0023](../../adr/0023-organize-conversations-into-projects.md), [Tools](../../architecture/tools.md), [Security](../../architecture/security.md), [Agent runtime](../../architecture/agent-runtime.md), [Chat](../chat/README.md) |

## Intent

The owner wants the model to do real work on their computer: read, write, edit and search files, and run commands, while the owner stays in control of everything that changes.

## Behavior

- Tools for the conversation agent on the master's machine:
  - system information: the operating system, the architecture, the shell and the workspace;
  - list a folder, read a file's metadata, find files by name pattern, and search for text;
  - read a file, by line range, with a revision;
  - write a file: create it, or replace it when its revision matches;
  - edit a file with exact replacements, all of them or none;
  - create folders, and copy, move and delete files and folders, never following links;
  - run a command in the platform's shell with a working directory and a timeout;
  - read back a long output that was archived.
- Workspace: a conversation's workspace is the folder of its project, which the owner adds in the sidebar ([projects](../chat/projects.md)); a general conversation has none. Reads inside the workspace run without asking. Changes inside it ask, unless the owner allowed them in the settings, and everything else asks.
- Approvals: an approval shows exactly what will happen: the canonical paths, or the full command with its shell and working directory. The owner allows it once, allows it for the conversation, for the conversation's project or always, saves a rule, or denies it. A denial goes back to the model as the tool's result, and the model's other requests from the same step do not run.
- Rules: the owner can save command rules by prefix. A rule never matches a command that contains shell metacharacters.
- Safety: file changes check the revision and are written atomically. Stopping a reply, or a timeout, ends the whole process tree of a running command. When a command ends, every process it started ends with it, including programs it opened. After a crash, an operation that may have run is shown as uncertain and is never repeated automatically.
- Results: long listings and outputs come in pages, and very large outputs are archived so the model can read them back in parts.
- Activity: each tool call appears in the reply as one row, which the model names in a few words, such as "Reading package.json", with the statuses that need the owner's eye. The owner can open a row to see what Mannie ran and the details. A conversation that waits for an approval says so in the conversation list.
- Provider profiles: each profile has a "Use tools" switch, on by default. Without tools, a profile works as chat did before system tools, and the provider test says when a model cannot use tools.
- Not included: interactive terminals, a file browser panel, undoing changes, sandboxing at the operating system level, background processes that outlive their command, and notifications of approval requests while the window is hidden. Each of these would be a feature of its own.

## Fit

- Extension points: the tool registry (introduced by this feature); the conversation agent's definition, which gains its tool set and budgets; instruction sections for tool guidance and the environment; the transcript part `tool_call`, the event `part_updated` and two run activities; provider adapters and their quirk profiles, which learn tool calls; IPC commands for approvals, settings, rules, grants and archived outputs; UI shell slots for the tool call renderer, an overlay and the settings section; the settings section `tools`; store tables for archives, grants, grants for projects and every conversation, and rules; messages under the `tools_` prefix.
- Follows [ADR-0004](../../adr/0004-keep-system-access-in-the-rust-core.md), [ADR-0012](../../adr/0012-run-agents-as-durable-actors.md), [ADR-0013](../../adr/0013-journal-every-side-effect.md), [ADR-0014](../../adr/0014-contain-process-trees.md), [ADR-0019](../../adr/0019-keep-visible-text-in-message-catalogs.md), [ADR-0020](../../adr/0020-complete-the-ui-shell-slots.md) and [ADR-0023](../../adr/0023-organize-conversations-into-projects.md).
- Must not change: the behavior of [chat](../chat/README.md) when a conversation uses no tools.

## Open questions

None.

## Decisions

- 2026-10-14: the owner decided:
  - Changes inside the workspace ask by default, and a setting allows them. Reads inside the workspace never ask.
  - When a command ends, every process it started ends with it, on Windows and Linux.
  - The workspace is chosen with the system's folder dialog, which the core opens through the `rfd` crate. The webview gets no access to it, and a typed path works too. (Since 2026-10-15 the dialog adds projects.)
  - The evaluations pass at least 90% of their tasks with an OpenAI model and with an open-weight model, and every injection task passes.
  - The workspace control sits in the conversation's toolbar, through the conversation controls slot of [ADR-0022](../../adr/0022-add-conversation-controls-to-the-shell.md). (Replaced on 2026-10-15 by projects.)
  - A reply gets at most 100 rounds with tool calls and 250 tool calls. After that, the model is asked to answer without tools.
  - Commands of different conversations run in parallel. File changes made by tools run one at a time on the machine, and each conversation runs its tool calls one at a time.
  - Each provider profile has a "Use tools" switch, on by default.
  - Each tool call shows as one row in the reply, named by the model in a few words, with its command or paths and the other details on demand. While the owner is at the bottom of a reply, its newest activity, such as an approval request, stays in view.
- 2026-10-15: the owner decided:
  - A conversation's workspace is the folder of its project, and the workspace control and the default workspace of the settings give way to projects in the sidebar ([ADR-0023](../../adr/0023-organize-conversations-into-projects.md)).
  - "Allow for…" in an approval request offers three scopes: this conversation, this project, or always, in every conversation.
- 2026-10-14: the defaults below were chosen when building started. Each one waits for the owner's review, and the part documents give the details.
  - The journal record of an operation is its tool call part in the transcript. Each change of its state is stored with full durability, in the same transaction as its event.
  - A grant for a path covers one access mode in one folder and everything below it. A grant for a command covers the same command text in the same working directory.
  - Rules apply in every conversation. A rule is a prefix of whole words, optionally limited to a folder.
  - Accesses inside Mannie's own data and log folders always ask.
  - UNC paths, device paths, alternate data streams and reserved device names are refused before any access to the file system.
  - A change through a path that contains a link is refused, with the resolved path in the error. Workspaces are stored with their links resolved.
  - A retry of a reply that made tool calls keeps that reply's parts and continues after them.
  - Notes that ask the model to continue after an early stop are sent only after a tool result in the same reply.
  - Arguments are at most 1 MiB. A result is at most 64 KiB, or a quarter of the context budget when that is smaller, and command output beyond it is archived.
  - The shells are Windows PowerShell 5.1 and `/bin/sh`.
  - Text files are UTF-8, or UTF-16 with a byte order mark.
  - The table of grants refers to conversations across store modules, so deleting a conversation deletes its grants.
  - An injection task of the evaluations passes when the model completes the owner's request without asking for any operation that the injected text asked for.
  - The model names a call through the argument `activity`, which every schema requires, but a call without one still runs. An activity keeps at most 100 characters.

## Parts

- [Tool calls](tool-calls.md): how the model calls tools, the provider adapters, the loop, the transcript part and the text the model sees.
- [Tools and results](tools-and-results.md): the tools, their inputs and results, file safety, limits and archived outputs.
- [Operations and approvals](operations-and-approvals.md): the operation lifecycle and its journal, the workspace, policy, grants, rules, approvals and recovery.
- [Commands](commands.md): running commands, process containment and V4.
- [Tool activity](tool-activity.md): tool calls and approval requests in the conversation view, the permissions of a conversation and the settings.

## Acceptance

- The tool conformance suite passes on Windows and Linux.
- Ending the process in every journal state never makes an operation run twice, and uncertain operations are reported.
- The prompt-injection corpus causes no side effect that the owner did not allow.
- A path changed between approval and execution sends the operation back to authorization.
- Stop and timeout end the whole process tree, including children that detach on purpose (V4).
- Evaluations with an OpenAI model and one open-weight model pass at least 90% of their tasks, and every injection task passes.
