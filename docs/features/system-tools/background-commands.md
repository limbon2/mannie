# Background commands

|              |                                                                                                                                                                                                                                                                                                                                                                                                      |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                                                                             |
| Milestone    | M11                                                                                                                                                                                                                                                                                                                                                                                                  |
| Last updated | 2026-10-21                                                                                                                                                                                                                                                                                                                                                                                           |
| Part of      | [System tools](README.md)                                                                                                                                                                                                                                                                                                                                                                            |
| Related      | [ADR-0041](../../adr/0041-let-agents-run-commands-in-the-background.md), [ADR-0014](../../adr/0014-contain-process-trees.md), [ADR-0013](../../adr/0013-journal-every-side-effect.md), [Commands](commands.md), [Wake-ups](wake-ups.md), [Tools and results](tools-and-results.md), [Operations and approvals](operations-and-approvals.md), [Tools](../../architecture/tools.md#6-running-commands) |

## Design

### Starting a command

- `start_command` takes the command text and an optional working directory, as `run_command` does, and prepares the same access: the command, the shell and the canonical working directory, marked as running in the background ([operations and approvals](operations-and-approvals.md#accesses)). The approval says "Runs in the background until it ends or is stopped".
- The policy decides it as it decides `run_command`, with one difference (D40): a grant or a rule covers a command in the background only when it was given or saved from the approval of a command in the background, and such a grant or rule covers the command in both modes. Grants and rules from before, and those from approvals of `run_command`, keep asking before a command runs in the background. Full access runs it without asking.
- The command runs in the shell of `run_command`, with its environment and its containment ([commands](commands.md)). It has no timeout. It must run in the foreground of its shell: when the shell's command returns, as it does at once for `npm run dev &` or `nohup`, the command ends and everything it started ends with it, so the tool's description tells the model to leave them out.
- The call returns as soon as the shell runs: "Command 3 started in the background", with the shell and the working directory, and how to read its output and stop it. A shell that cannot start fails as `run_command` fails, with `shell_missing` or `spawn_failed`.
- Numbers count from 1 in each conversation and are never reused, also after a command ended.
- The conversation agent and the general child agent get the tools, and so does the worker agent for its tasks. The explore agent never gets them, and a task that a worker's ceiling makes read-only gets none of the three.

### Reading output

- `read_command_output` takes the command's number. Without `from_line`, it returns the lines that the command wrote after the model's last read, and the next read starts after them. With `from_line`, it reads from that line on in pages, as `read_tool_output` pages an archive, and leaves the read position as it is.
- The result starts with the command's number, its text, its working directory or host, and its state: running and for how long, ended with its exit code or its signal, stopped by the agent or the owner, ended with its task, its plugin or its session, or interrupted when Mannie quit. The lines follow, with the range of their numbers.
- New lines that do not fit the result limit keep their start, a quarter of the room, and their end, cut at line breaks as the output of `run_command` is ([commands](commands.md#output)), with a note that names the lines left out and how to read them with `from_line`. Lines that the log no longer holds are named as left out of the log.
- `wait_seconds`, 0 when null and at most 600, waits until the command ends, until a new line matches the regular expression `until`, or until the time passes. Without `until`, the wait ends at the command's end or at the time. Stop of the reply ends the wait and leaves the command running.
- After the command ended, reads come from its archive with the same line numbers, also after Mannie restarted.
- An unknown number fails with `command_not_found`, and a pattern that does not parse with `invalid_pattern`.

### Stopping a command

- `stop_command` takes the number, ends the command, waits until it ended, and returns how it ended, with its exit code when it had one, and how many of its lines are unread.
- It is an operation without accesses, which the policy allows, so its call is journaled like any side effect and becomes uncertain after a crash. A task that a worker's ceiling makes read-only is not offered it. `read_command_output` creates no operation, and is offered only with `start_command`.
- A command that already ended returns how it ended, and nothing happens.
- A local command's tree ends as Stop ends a command of `run_command` ([commands](commands.md)). A remote command's plugin is asked to end it and says how it ended, and the result says that processes that it started may still run on the host.

### How a command ends

Each end is recorded with its cause:

| End                  | Cause                                                                               | Wakes the agent |
| -------------------- | ----------------------------------------------------------------------------------- | --------------- |
| Exited               | The command ended on its own, with its exit code                                    | Yes             |
| Ended by the host    | A remote command's host reported its exit status or a signal, or closed its channel | Yes             |
| Stopped by the agent | `stop_command`                                                                      | No              |
| Stopped by the owner | Stop in the toolbar's menu, the output panel or the call's card                     | No              |
| Task ended           | The run of its task conversation ended, for a child agent's or a worker's task      | No              |
| Plugin off           | Its plugin went off                                                                 | No              |
| Session closed       | Mannie closed the remote session, as when the owner edits or removes the host       | No              |
| Interrupted          | Mannie or the worker quit, or a crash ended them                                    | No              |

A deleted conversation's commands end without a record of their end, since their records go with the conversation. Stopping a reply ends a waiting `read_command_output` and never a command; the owner stops commands from the toolbar, the panel or the call's card.

### The log

- A command's two streams become one log of lines, in the order the lines arrive. Each stream is kept to whole lines, so the lines of the two never mix. CRLF reads as LF, bytes that are not valid UTF-8 are replaced and the result says so, and a line longer than 64 KiB is cut with a mark.
- The log keeps its first 256 KiB and its last 768 KiB of lines, the sizes that `run_command` keeps of a stream, with their numbers from 1, and counts the lines between them.
- When the command ends, its log is archived as the output `output` of its start call, with a line that names the lines left out when it lost any, so `read_tool_output`, the owner's "Show full output" and the conversation export read it.
- A plugin's remote command hands the core its lines, and the plugin SSH replaces the host's password in each line before ([SSH hosts](../../../plugins/ssh/docs/README.md)).

### Limits

| Limit                                | Value                                           |
| ------------------------------------ | ----------------------------------------------- |
| Running commands of one conversation | 8                                               |
| Running commands of one machine      | 32                                              |
| A line                               | 64 KiB, cut beyond                              |
| The log of a command                 | Its first 256 KiB and its last 768 KiB of lines |
| `wait_seconds`                       | 600                                             |
| The command's text                   | 100,000 bytes, as for `run_command`             |

A start beyond a limit fails with `too_many_background_commands`, which names the limit, so the model can stop a command first.

### The record

- A command's record, with its process, is stored with full durability before its start call completes. A crash before it leaves an executing call, which [recovery](operations-and-approvals.md#recovery) makes uncertain and whose tree it ends on Linux when the call's part holds the process, and a crash after it leaves a running record. A record that cannot be stored ends the command's tree before the call fails.
- The record keeps the command, its shell or its host, its working directory, its process ID and start time, its state, how it ended and who ended it, its exit code, its times, its line and byte totals, the model's read position, the lines that the log left out, its archive, and whether the model was told of its end.
- Conversation events report a command's start and its end, never its lines, and a conversation's snapshot lists its commands. The registry writes them beside the conversation's run, through the same hub, so they keep the conversation's order.
- A conversation's next number follows the highest number it ever had, also after a restart.
- A conversation's export record lists its commands with their ends, and the Markdown export names each with the call that started it.

### Recovery

At startup, before any run starts, every record that was running becomes interrupted, whatever its run did, and the next reply tells the model. On Linux and FreeBSD the runtime ends what is left of a local command's tree as it ends an uncertain command's tree ([commands](commands.md#crash-recovery)); on Windows the job ended with Mannie. A remote command's record says that its processes may still run on the host. Nothing starts a command again.

When Mannie or a worker quits, the registry stops taking commands, ends every local tree at the same time and waits for them, about half a second on Unix, before it stores their ends and archives. A shell is never killed before its tree, since on Unix a walk of its descendants made while it lives is what reaches the processes that left its process group. A plugin's commands are asked to end before the plugin closes its sessions.

### Child agents and worker tasks

- A general child agent's commands belong to its task conversation, and they end when its task's run ends, before the task's report reaches the parent. The child's conversation, opened from its task card, shows them in its control "Commands" and in the cards of its calls, and the owner may stop them there.
- A worker's task gets the three tools through the worker agent, and the host's ceiling decides `start_command` as it decides `run_command`. Its commands end when the task's run ends: with its report, Stop from the master, a revoked lease, unpairing, deletion or the worker's stop. The master shows them in the task's conversation with their states, and their output reaches the transcript through the agent's reads. Stopping one of them from the master means stopping its task ([delegation](../../../plugins/workers/docs/delegation.md)).

### Remote commands of plugins

A plugin's tool may start a command on a remote host in the background ([ADR-0041](../../adr/0041-let-agents-run-commands-in-the-background.md), decision 11). The SDK's service `background-commands` takes a ticket that the core put in the call's execution and a handle that the plugin hands the core: a stream of the command's lines that ends with how it ended, and a stop that ends the command and says how it ended. The core uses a ticket once, takes the command's label and text from the call's authorized access, reads the stream into the command's log, and awaits the stop when the agent or the owner stops the command, when the plugin goes off and when Mannie quits. A stream that ends without saying how the command ended ends it as let go. The question before turning a plugin off counts its running commands. The SSH plugin's `start_ssh_command` is the first such tool ([SSH hosts](../../../plugins/ssh/docs/README.md)).

### Interface

- In the reply, a call of `start_command` reads "Started command 3 in the background", and while the conversation is open, its row shows the command's state: running and for how long, ended with its exit code, or stopped. Its details add "Show output" and, while the command runs, "Stop".
- Calls of `read_command_output` read "Read lines 120 to 180 of command 3", and calls of `stop_command` "Stopped command 3".
- The conversation's toolbar gets the control "Commands", after full access, while the conversation has background commands. It shows how many run, and its menu lists the running commands, then the ended ones, each with its number, its command and its state. Choosing one opens its output in a side panel.
- The side panel shows the command, its working directory or host, its state and how long it ran, "Stop" while it runs, and its output, which follows new lines while the owner is at its end and keeps the last 2,000 lines in view. After the end, "Show full output" opens the archive. The output is content in a log region that scrolls from the keyboard.
- A conversation in a side panel, such as a child's or a worker task's, shows the same control above its transcript. A worker task's commands show with their states only: their output and Stop are offered only for this machine's commands.
- A call of a plugin's tool that started a command on a remote host reads "Start {command} on {host} in the background", and its row and details show the command as a call of `start_command` does.
- The approval of a background command says "Runs in the background until it ends or is stopped".
- The Tools settings get the switch "Wake the agent when a background command ends" ([wake-ups](wake-ups.md)).
- Closing or quitting while commands run warns with their number, as it warns about active replies ([window and tray](../window-and-tray.md)).

### Contracts

- Tools: `start_command`, `read_command_output` and `stop_command` ([tools and results](tools-and-results.md#the-tools)), with the outcomes `command_started`, `command_output_read` and `command_stopped`, and the failures `command_not_found` and `too_many_background_commands`.
- The command access and the remote command access gain `background`, false in the foreground and for every access that existed. The digest of an operation leaves the field out while it is false, so a command in the foreground keeps its digest.
- IPC: `stop_background_command` takes the conversation and the number, also of a child's task conversation, and fails with `background_command_not_found`. `subscribe_command_output` takes the conversation, the number and the last line that the view holds, or none for the command's last 2,000 lines, and sends through a channel, in batches of one frame: the command's view first, then its lines in updates of at most 500 lines with the number of their first line, a range of lines where the log let some go, and, once the command ended, its view as it ended, after which the channel ends. A view that subscribes again passes its last line. It fails with `background_command_not_found`.
- Events: `background_command_changed`, with the command's view, when a command starts or ends; the snapshot's `backgroundCommands`; a conversation control receives `backgroundCommands`.
- The scopes of command grants and the rules of commands, here and on remote hosts, gain `background`, false for every grant and rule that existed.
- The export record gains `backgroundCommands`.

### Data

Migration `0032` creates `runtime_background_commands`: the conversation and the number as its key, the start call's part, the source, `local` or a plugin's ID, and a JSON document with the rest of the record. Its foreign keys to `runtime_conversations` and `runtime_parts` delete a record with its conversation. A grant keeps `background` in its JSON scope, absent and so false for every grant that existed. Migration `0033` adds `background` to `tools_rules` and `tools_remote_rules`, false for every row that existed, and a rule saved again from a command in the background widens the rule with the same words to both modes.

### Message keys

The control, the panel, the cards and the setting use keys under `tools_background_`. The quit warning's keys stay the window's.

### Loading, empty and error states

| View               | Loading                      | Empty                     | Error                                              |
| ------------------ | ---------------------------- | ------------------------- | -------------------------------------------------- |
| The output panel   | Skeleton lines               | "No output yet"           | "The output could not be loaded", with "Try again" |
| The toolbar's menu | Not applicable               | The control shows nothing | Not applicable                                     |
| Stop               | The button reads "Stopping…" | Not applicable            | What failed, below the button                      |

### Threat notes

- T-52: a background command keeps running after the owner's attention moved on, such as a server on a port or a watcher that changes files. The approval says that it runs until it ends or is stopped, a grant or a rule from the foreground never starts one, the toolbar lists every running command with Stop, the limits bound them, deleting the conversation ends them, and closing or quitting warns and ends them.
- T-54: a long-lived command's output, such as a log that other users write, can carry instructions at any time. Output reaches the model only as results of `read_command_output`, notes never hold it, and content never grants a permission.
- A command's text shows in notes, in the control and in the panel. It is the model's own text from an approved call, rendered as content.
- Logs at `info` name commands by their conversation and number, with their ends; command texts only at `debug`.

### Platform differences

| Topic                   | Windows                   | Linux                                                 | FreeBSD                       |
| ----------------------- | ------------------------- | ----------------------------------------------------- | ----------------------------- |
| The tree after the call | Its job stays open        | Its shell, a subreaper, keeps it                      | Its shell, a reaper, keeps it |
| The command's end       | The shell exits           | The wrapper's descriptor, read on one blocking thread | As on Linux                   |
| After a crash           | The job ended with Mannie | The next start ends the tree                          | The next start ends the tree  |

### Tests

- `crates/mannie-tools`: unit tests of the log (lines across reads and streams, CRLF, long lines, its start and end with their numbers), of the reads and their cuts, of `until` and of the limits; `tests/background.rs` starts, reads, waits, stops and archives real commands; V4's process tree tests run their shapes as background commands, ended by Stop, by their own end and by the registry's shutdown.
- `crates/mannie-tools`: policy tests of grants and rules in both modes, here and on remote hosts.
- `crates/mannie-store`: the migrations, the upgrade from the runtime and tools modules of 0.4.0, the cascade and recovery.
- `crates/mannie-runtime`: a scripted provider starts, reads and stops commands across replies, a child's task and a deletion end them, shutdown interrupts them, and crashes before and after a command's record is stored leave a tree that the next start ends on Linux. `tests/remote_background.rs` hands the registry a plugin's commands through tickets: one start for each ticket, only by the ticket's plugin, the limit, ends on the host, the plugin going off and quitting.
- The evaluations (`task tools:eval`) start a server in the background, query it and stop it, run tests in the background while the agent reads a file, and keep an injection in a command's output from causing anything.
- The end-to-end test `tests/e2e/specs/background-commands.test.ts`: the control's count, the panel's output as it arrives, Stop, the call's card and a wake-up's row, with axe, and only catalog text in the pseudo-locale.

## Acceptance

- A command started in the background keeps running after its call and after the reply, its new output reads after other tool calls, and `stop_command` ends it with every process that it started, on Windows, Linux and FreeBSD (process tree tests).
- A read with `wait_seconds` returns when the command ends or a line matches `until`, Stop of the reply ends the wait and not the command, and a read after the end comes from the archive (tool and runtime tests).
- A start beyond 8 running commands in a conversation fails and names the limit (tool test).
- A grant for the conversation given from a command in the foreground asks again before the same command runs in the background, and a grant given from the background covers both (policy and runtime tests).
- Deleting the conversation, the end of a child's task and quitting end its commands, and after a crash the next start marks them interrupted and ends what is left of their trees on Linux and FreeBSD (runtime and crash tests).
- The toolbar lists running and ended commands, the panel shows output as it arrives, and Stop in the panel ends the command (end-to-end test).
- Axe finds no WCAG 2.1 AA violation in the control, its menu, the panel and the cards, on WebView2 and WebKitGTK, and the pseudo-locale shows no text outside the catalog.
