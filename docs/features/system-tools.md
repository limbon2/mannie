# System tools

|              |                                                                                                                                                                                                            |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Accepted                                                                                                                                                                                                   |
| Milestone    | M2                                                                                                                                                                                                         |
| Last updated | 2026-10-13                                                                                                                                                                                                 |
| Related      | [ADR-0013](../adr/0013-journal-every-side-effect.md), [ADR-0014](../adr/0014-contain-process-trees.md), [Tools](../architecture/tools.md), [Security](../architecture/security.md), [Chat](chat/README.md) |

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
  - run a command in the platform's shell with a working directory and a timeout.
- Workspace: a conversation can have a workspace folder. Reads inside it run without asking. Whether writes inside it run without asking is a setting, and everything else asks.
- Approvals: an approval shows exactly what will happen: the canonical paths, or the full command with its shell and working directory. The owner allows it once, allows it for the conversation, saves a rule, or denies it. A denial goes back to the model as the tool's result.
- Rules: the owner can save command rules by prefix. A rule never matches a command that contains shell metacharacters.
- Safety: file changes check the revision and are written atomically. Stopping a reply, or a timeout, ends the whole process tree of a running command. After a crash, an operation that may have run is shown as uncertain and is never repeated automatically.
- Results: long listings and outputs come in pages, and very large outputs are archived so the model can read them back in parts.
- Activity: each tool call appears in the reply with its status and a summary of its result, and the owner can expand it to see the details.
- Not included: interactive terminals, a file browser panel, undoing changes, and sandboxing at the operating system level. Each of these would be a feature of its own.

## Fit

- Extension points: the tool registry (introduced by this feature), the conversation agent's definition, transcript parts for tool calls, tool results and operation states, run events, IPC commands for approval decisions and rules, UI part renderers for tool activity and approval requests, the settings section for workspace defaults and rules, and store tables for the operation journal, grants and rules.
- Follows [ADR-0004](../adr/0004-keep-system-access-in-the-rust-core.md), [ADR-0013](../adr/0013-journal-every-side-effect.md) and [ADR-0014](../adr/0014-contain-process-trees.md).
- Must not change: the behavior of [chat](chat/README.md) when a conversation uses no tools.

## Open questions

- The default for writes inside the workspace: allowed without asking, or asking.

## Decisions

None yet.

## Design

Written when M2 starts.

## Acceptance

- The tool conformance suite passes on Windows and Linux.
- Ending the process in every journal state never makes an operation run twice, and uncertain operations are reported.
- The prompt-injection corpus causes no side effect that the owner did not allow.
- A path changed between approval and execution sends the operation back to authorization.
- Stop and timeout end the whole process tree, including children that detach on purpose (V4).
- Evaluations with an OpenAI model and one open-weight model reach the agreed pass rate.
