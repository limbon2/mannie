# Operations and approvals

|              |                                                                                                                                                                                                                                                                        |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                               |
| Milestone    | M2                                                                                                                                                                                                                                                                     |
| Last updated | 2026-10-15                                                                                                                                                                                                                                                             |
| Part of      | [System tools](README.md)                                                                                                                                                                                                                                              |
| Related      | [ADR-0013](../../adr/0013-journal-every-side-effect.md), [ADR-0012](../../adr/0012-run-agents-as-durable-actors.md), [Tools](../../architecture/tools.md), [Security](../../architecture/security.md), [Domain model](../../architecture/domain-model.md#5-operations) |

## Design

### The lifecycle

The runtime runs each tool call through the same steps ([ADR-0013](../../adr/0013-journal-every-side-effect.md)):

```mermaid
flowchart TD
  C[The call is complete] --> V{Valid arguments?}
  V -- no --> N[Not run, with the reason]
  V -- yes --> P[Prepare: the accesses]
  P --> D{Policy}
  D -- allows --> R[Prepare again]
  D -- asks --> A[Await the owner]
  A -- denied --> X[Denied]
  A -- allowed --> R
  R -- same accesses --> E[Executing, stored with full durability]
  R -- different accesses --> D
  E --> F[Completed]
```

- Preparing has no side effects. It resolves paths, reads metadata and counts the entries a delete would remove, and it runs on a blocking thread.
- Right before the execution, the tool prepares again. When the accesses differ, the operation goes back to the policy, and an earlier approval no longer counts. When the second preparation fails, for example because a folder of the path became a link, the call completes with that error and nothing runs.
- A file change compares its accesses once more under the machine's file lock, right before it writes, and fails with `path_changed` when they moved again ([tools and results](tools-and-results.md#file-safety)).
- Read-only tools go through the same policy, but they are not operations: their status while they run is `running`, and they never become uncertain.

### Accesses

- A path access names the canonical path, its mode (read, write, create or delete), whether the target exists, and the identity of the target or of its nearest existing folder: the volume serial number and file index on Windows, the device and inode on Linux. A recursive delete adds the number of files and folders it removes.
- A command access names the command's text, the shell's path and the canonical working directory.
- The digest of an operation is the SHA-256 of its accesses in canonical JSON. A replaced file, a swapped folder or a changed folder content therefore changes the digest.

### The journal

The journal record of an operation is its tool call part ([tool calls](tool-calls.md#the-transcript-part)). Each change of its status is stored in the same transaction as its event, with full durability, and the `executing` status is stored before the side effect starts. The record keeps the accesses, the decision with its source (the workspace, a grant of the conversation, of its project or of every conversation, a rule or the owner), the result, and for a command the shell's process ID and start time.

### Policy

The policy is a function of the accesses, the conversation's workspace and project, the `tools` settings, the grants that apply to the conversation, the rules and, for a move into a project, the owner's message of the turn. It is read again for every call, so a change applies from the next call.

| Order | Source                        | Effect                                                                                                               |
| ----- | ----------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| 1     | Mannie's data and log folders | Every access inside them asks, whatever the workspace, grants and rules say                                          |
| 2     | Workspace, reads              | Reads inside the workspace are allowed                                                                               |
| 3     | Workspace, changes            | Creating, writing and deleting inside the workspace are allowed only when the settings allow it                      |
| 4     | Grants                        | A grant of the conversation, of its project or of every conversation allows what it covers                           |
| 5     | Rules                         | A rule allows a command it matches                                                                                   |
| 6     | The owner's message           | A move into a project that the owner's message names is allowed, while no other tool call of the turn came before it |
| 7     | The owner                     | Everything else asks                                                                                                 |

An operation with several accesses is allowed only when every access is. Commands are never allowed by the workspace, and no grant or rule moves a conversation ([projects](../chat/projects.md#moving-a-conversation-into-a-project)). The decision names the source that reaches furthest: a rule, then a grant for every conversation, of the project and of the conversation, then the owner's message and the workspace.

### The workspace

- A conversation's workspace is the folder of its project ([projects](../chat/projects.md)), stored on the conversation with its links resolved. A general conversation has none, and so do conversations from before system tools that used no folder.
- A conversation gets its workspace when it starts in a project or moves into one. The move is an event of the conversation, so every window shows it, and it applies from the next tool call. A conversation never leaves its project, so its workspace never changes again.
- Without a workspace, a relative path and a command without a working directory are refused with `no_workspace`.

### Grants

- "Allow for…" offers three scopes. "This conversation" creates grants that last until the conversation is deleted or the owner revokes them. "This project", offered in a project's conversation, creates grants that every conversation of the project uses until the project is removed or the owner revokes them. "Every conversation" creates grants that every conversation uses until the owner revokes them.
- A path grant covers one mode in one folder and everything below it: the folder of a file, or the folder itself for a folder.
- A command grant covers the same command text in the same working directory.
- The core computes a grant from the operation's accesses, and the approval shows what it would cover. When one operation's accesses would give several grants and one covers another, the request offers only the wider one. The owner sees and revokes the grants that apply to a conversation in a dialog, and the grants for projects and for every conversation in the Tools settings.
- An approval that touches Mannie's data or log folders offers no grant, so those folders keep asking.

### Rules

- A rule allows commands that start with its words, in every conversation. It may be limited to a folder, which the working directory must be in.
- Words are split at unquoted spaces. A command that contains `;`, `&`, `|`, `$`, a backtick, parentheses, braces, `<`, `>`, a line break or an unbalanced quote never matches a rule or a grant, and its approval offers only "Allow once" and "Deny".
- The owner saves a rule from an approval. The dialog shows the command's words, the owner chooses how many of the leading words the rule keeps and whether it is limited to the working directory, and the core builds the rule from the command it holds. The settings list the rules, and the owner deletes them there.

### Approvals

- An approval request shows the tool, the accesses exactly (each canonical path with its mode and, for a delete, the number of files and folders; or the command with its shell and working directory), why Mannie asks, and what "Allow for…" would cover.
- The options are "Allow once", "Allow for…" with "This conversation", "This project" in a project's conversation and "Every conversation", "Save rule" for commands that a rule can cover, and "Deny". The request names the conversation's project, and the core refuses a scope that the request does not offer: "This project" outside a project (`not_in_a_project`), and any "Allow for…" for an operation that can only be allowed once (`grant_not_possible`).
- The request carries the operation's digest. `decide_approval` names the conversation, the tool call and the digest, and the core refuses a decision for a request that no longer waits (`approval_not_pending`) or whose accesses changed (`approval_changed`).
- The run waits through a pending approval, a one-shot answer that the runtime registers for the request. Stop and Shutdown still reach the run through its mailbox. A pending approval holds no model slot.
- A denial goes back to the model, and the other calls of the same step are not run.
- Nothing that the model reads can answer an approval, create a grant or save a rule. Only the owner's actions in the UI can.

### Recovery

At startup, before any command is accepted, the runtime finishes every tool call that was open:

| Status when Mannie ended                    | Status after the start | What the model sees on a retry |
| ------------------------------------------- | ---------------------- | ------------------------------ |
| `receiving`, `pending`, `awaiting_approval` | `canceled`             | Not run                        |
| `running`                                   | `canceled`             | Not run                        |
| `executing`                                 | `uncertain`            | It may have taken effect       |

The statuses are stored before the runs end as interrupted, so a crash during the recovery never leads to an operation running again. An uncertain operation is reported to the model and to the owner and never repeated automatically. On Linux, the recovery then ends what is left of an uncertain command's process tree ([commands](commands.md#crash-recovery)).

### Contracts

- Commands (`apps/desktop/src-tauri/src/commands/tools.rs`): `decide_approval`, `get_tools_settings`, `update_tools_settings`, `list_command_rules`, `delete_command_rule`, `list_conversation_grants`, `revoke_conversation_grant`, `list_shared_grants`, `revoke_shared_grant` and `read_tool_output`. `list_conversation_grants` returns the grants of one conversation, and `list_shared_grants` the grants for a project, which name it, and those for every conversation.
- Errors use `ToolsError`: `conversation_not_found`, `approval_not_pending`, `approval_changed`, `rule_not_possible`, `rule_prefix_invalid`, `not_in_a_project`, `grant_not_possible`, `rule_not_found`, `grant_not_found`, `output_not_found`, `environment_name_invalid`, `environment_name_secret`, `too_many_environment_names`, `written_by_newer_version`, `storage_failed`, `store_unavailable` and `shutting_down`.
- The conversation summary carries the workspace, the project and whether the conversation waits for an approval. The conversation list receives a new summary when a reply starts or stops waiting, so the list marks it at once.

### Settings

The settings section `tools` holds:

| Field                        | Rule                                                                                                                                                    |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `allow_changes_in_workspace` | False by default                                                                                                                                        |
| `command_environment`        | At most 50 variable names passed to commands besides the built-in ones. Names that look like secrets are refused ([commands](commands.md#environment)). |

### Data

- Migration `0008` adds `workspace` to `runtime_conversations`.
- Migration `0010` creates `tools_grants`: the conversation, the kind (path or command), the mode and folder of a path grant, the command and working directory of a command grant, and the time. The conversation is a foreign key to `runtime_conversations` with `ON DELETE CASCADE`, the store's first foreign key across modules, so deleting a conversation deletes its grants with `secure_delete`.
- Migration `0011` creates `tools_rules`: the words, the optional folder, and the time.
- Migration `0014` creates `tools_shared_grants`: the project, or none for every conversation, the scope and the time. The project is a foreign key to `runtime_projects` with `ON DELETE CASCADE`, so removing a project deletes its grants.

### Threat notes

- T-1 and T-2: operations are authorized against their canonical accesses, the second preparation catches changes, and commands with metacharacters always ask.
- A broad grant could let the model change Mannie's database and with it the rules. Accesses inside Mannie's data and log folders therefore always ask.
- A grant for a project or for every conversation reaches conversations that the owner is not looking at. The owner chooses the scope in each request, the request shows what the grant covers, and the Tools settings list every such grant with "Revoke".
- T-5: a decision must name the digest of the request it answers, so a stale or forged decision changes nothing. A compromised webview could still answer a pending request, which stays a residual risk ([security](../../architecture/security.md#8-if-the-webview-is-compromised)).
- Paths, commands and approvals are content. Logs at `info` level record operations with their IDs, tools, statuses and codes, and paths and commands only at `debug`.

## Acceptance

- Ending the process in every journal state never makes an operation run twice, the recovered statuses match the table, and a retry reports uncertain operations without repeating them (crash tests in `mannie-runtime`, and an end-to-end test that kills the app during a command).
- The policy decides as the table says, for every source and combination (unit tests of the policy).
- An approval runs the operation only for the request it answers; a stale decision, a changed digest and a decision for another conversation are refused (runtime tests).
- A path changed between the approval and the execution goes back to the policy (runtime test).
- The prompt-injection corpus causes no side effect that the owner did not allow, and creates no grant and no rule (runtime test with a scripted model).
- A rule never matches a command with metacharacters, and grants and rules cover exactly what the approval showed (unit tests).
- Deleting a conversation deletes its grants and archives, with a planted string in them (store test).
- "For this project" lets another conversation of the project run what the grant covers, "Always" lets a general conversation run it, and revoking them asks again (runtime tests and end-to-end test).
