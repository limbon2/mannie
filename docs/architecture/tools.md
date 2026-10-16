# Tools

|              |                                                                                                                                                                                           |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                                                          |
| Last updated | 2026-10-15                                                                                                                                                                                |
| Related      | [ADR-0013](../adr/0013-journal-every-side-effect.md), [ADR-0014](../adr/0014-contain-process-trees.md), [Extension points](extension-points.md#31-tool-registry), [Security](security.md) |

This document describes the rules that every tool follows. It does not list the tools: each feature's document lists the tools it adds.

## 1. Principles

- A tool acts only on the machine where its agent runs ([ADR-0015](../adr/0015-run-an-agent-runtime-on-every-machine.md)).
- Every side effect is an operation with a journal record ([ADR-0013](../adr/0013-journal-every-side-effect.md)).
- Results are typed and bounded in size. The model sees a rendering of them, and the transcript stores them once.
- Content that a tool returns never grants a permission.

## 2. The tool contract

A tool implements the `Tool` trait in `mannie-tools` and registers with the [tool registry](extension-points.md#31-tool-registry). It declares:

| Item             | Rule                                                                                                                                                                                                      |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Name             | `snake_case`, unique in the registry, never reused for a different meaning.                                                                                                                               |
| Description      | Written for the model and reviewed like code, because prompts and tool descriptions are a contract with the model.                                                                                        |
| Input schema     | JSON Schema generated with schemars from a Rust type. Arguments are validated against the type before anything else happens.                                                                              |
| Availability     | The agent definitions and machine roles that may use the tool, and whether a model call is offered it: a tool can ask for the conversation's project and the owner's projects, as `move_to_project` does. |
| Permission class | Read-only, or with side effects. Only tools with side effects create operations.                                                                                                                          |
| Prepare          | Computes the canonical accesses without side effects.                                                                                                                                                     |
| Execute          | Performs the work after authorization and returns a typed result.                                                                                                                                         |

A call's surroundings, which the runtime reads again for every call, hold the conversation, its workspace, the store, the limits, the environment of commands, the conversation's project and the owner's projects, and the capability to move a general conversation into a project, which the runtime provides because a move changes the conversation ([ADR-0023](../adr/0023-organize-conversations-into-projects.md)). For a child agent, the runtime reads the workspace, the project and the grants from the parent conversation, and the conversation stays the child's own, where its archives live.

The task tools of child agents are not in the registry. They act on runs rather than on the machine: they start, continue and wait for the runs of child agents, they have no operations of their own, and every side effect of a child is an operation of the child's own run. The runtime defines them, offers them only to runs that are not tasks themselves, and renders their results ([ADR-0024](../adr/0024-run-child-agent-tasks-in-the-parent-run.md)).

Every call also carries an activity: a few words in which the model says what the call does, for the owner to read. The runtime adds the argument `activity` to each schema that a model receives and takes it out of the arguments before the tool reads them, so a tool cannot have an argument of that name. The activity is the model's claim, and nothing that authorizes a call reads it ([tool calls](../features/system-tools/tool-calls.md#the-activity-of-a-call)).

## 3. Operations

```mermaid
flowchart LR
  P[Prepare] --> A{Authorize}
  A -- allowed --> R[Prepare again and compare]
  A -- ask --> W[Await the owner] --> A
  A -- denied --> D[Denied]
  R -- same accesses --> X[Execute]
  R -- different --> A
  X --> C[Completed]
```

- Prepare resolves each path to its canonical absolute form and records it with its access mode: read, write, create or delete. For a command it records the command line, the shell and the working directory.
- Authorization happens against those canonical accesses, never against the raw arguments.
- Right before executing, the tool prepares again. If any access changed, for example because a symlink now points elsewhere, the operation goes back to authorization.
- The journal commits the executing state durably before the side effect starts. After a crash, an executing operation becomes uncertain, and the model and the owner are told. Nothing repeats it automatically.
- The journal record is the tool call's part in the transcript, so a change of state and its event are stored in one transaction ([ADR-0013](../adr/0013-journal-every-side-effect.md)).

## 4. Authorization

Policy decides first, and the owner decides what policy leaves open.

| Source         | Effect                                                                                                                                                                                                         |
| -------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Workspace      | Reads inside the conversation's workspace are allowed. Whether writes inside it are allowed without asking is a setting.                                                                                       |
| Grant          | An approval "for" this conversation, its project or every conversation allows the same kind of access there, for example writes in one folder.                                                                 |
| Rule           | A saved pattern, such as a command prefix, allows matching operations in every conversation it covers.                                                                                                         |
| Policy ceiling | On a worker, the ceiling set on the host limits everything above and can refuse an operation outright. A task can narrow it further ([host and ceiling](../features/workers/host-and-ceiling.md#the-ceiling)). |

- An approval request shows the operation exactly: the canonical paths, or the full command with its shell and working directory, and on a worker also the host. A worker's request offers only Allow once and Deny.
- The owner can allow the operation once, allow it for the conversation, its project or every conversation, save a rule, or deny it. A denial goes back to the model as the tool result.
- A child agent's request shows in its parent's conversation, inside the card of its task, with the task's number and title. The decision names the child's conversation, and a grant "for this conversation" is saved for the parent, so it covers the parent and all its children.
- A move into a project runs without a question only when the owner's message of the turn names the project and no other tool call of the turn came before it, so nothing that the model read can stand in for the owner's words; no grant or rule covers a move.
- A command rule never matches a command that contains shell metacharacters (`;`, `&`, `|`, `$`, backticks, parentheses, braces or redirections). Such commands always ask.
- Text that the model read can never create a grant or a rule, and neither can a report from another agent.

## 5. File safety

- Paths are resolved to canonical form, with case-insensitive comparison on Windows. Operations that change files refuse to follow symlinks and junctions at any level of the path.
- Paths that could reach the network or a device, such as UNC paths, device paths and alternate data streams, are refused before any access to the file system.
- Every read returns a revision, the SHA-256 of the content. Writes and edits to an existing file must name the revision they expect, so a file that changed in the meantime is never overwritten by accident.
- An edit replaces exact text. Each replaced span must match exactly once, and an edit with several replacements applies all of them or none.
- Writes are atomic: the new content goes to a temporary file in the same folder, is flushed to disk, and replaces the original by rename. Creating a file never overwrites an existing one.
- On one machine, operations that change files run one at a time.
- Text is read and written as UTF-8, or as UTF-16 when the file starts with a byte order mark. Byte order marks and line endings are preserved.

## 6. Running commands

- Commands run through one process supervisor that contains the whole process tree on every platform ([ADR-0014](../adr/0014-contain-process-trees.md)).
- The shell is Windows PowerShell on Windows and `/bin/sh` on Linux and FreeBSD. On Windows, the command text reaches PowerShell through standard input, never as an encoded command.
- When a command ends, every process it started ends with it.
- A command has a timeout of 60 seconds by default and 600 seconds at most. Stop and timeout end the whole tree.
- Standard output and standard error are captured separately, each capped in size, and a cut is marked as truncated.
- A child process gets an environment built from an allow list. Credentials never reach it.

## 7. Bounded results

- Every result has a size limit. Listings and searches return pages with an offset for the next page. File reads return line ranges and say where the next range starts.
- A result that is too large for the transcript is archived. The model gets an excerpt and a reference that a read-back tool follows ([agent runtime](agent-runtime.md#7-context-management)).

## 8. Testing tools

- A conformance suite runs every tool on each supported platform: Windows and Linux on the master, plus FreeBSD on workers.
- Crash tests end the process in every journal state and check that nothing repeats.
- A prompt-injection corpus feeds hostile instructions through files, command output and reports, and must cause no side effect the owner did not allow.
- A test changes a path between the approval and the execution and expects the operation to return to authorization.
