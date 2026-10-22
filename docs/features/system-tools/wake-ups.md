# Wake-ups

|              |                                                                                                                                                                                                                                                                                                                                              |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                     |
| Milestone    | M11                                                                                                                                                                                                                                                                                                                                          |
| Last updated | 2026-10-21                                                                                                                                                                                                                                                                                                                                   |
| Part of      | [System tools](README.md)                                                                                                                                                                                                                                                                                                                    |
| Related      | [ADR-0041](../../adr/0041-let-agents-run-commands-in-the-background.md), [ADR-0012](../../adr/0012-run-agents-as-durable-actors.md), [ADR-0024](../../adr/0024-run-child-agent-tasks-in-the-parent-run.md), [Background commands](background-commands.md), [Tool calls](tool-calls.md), [Agent runtime](../../architecture/agent-runtime.md) |

## Design

### Notes

- For each conversation, Mannie keeps the ends of background commands that the model has not been told. An end by the agent's own `stop_command` is told by that call's result and needs no note.
- During a reply, after a step's tool calls ran, the run adds one note of Mannie's own with the ends that are untold, so the model reads it in its next call. The note does not count against the reply's continuation notes ([tool calls](tool-calls.md#the-loop)).
- A reply that would end while ends are untold continues once with the note, as a reply with unread reports of its tasks does, unless its tool calls are used up. An end that arrives after that check wakes the agent once the reply ended.
- A reply that the owner's message starts begins with a note when the conversation has commands that run or untold ends. The note is a continuation note stored with the owner's message, for the model only, and it never counts as the owner's words, so it cannot name a project for `move_to_project`.
- An end is told once: the transaction that stores a note marks the ends that it tells.
- A note names each command by its number and its text, cut at 200 characters, says how it ended or how long it has run, and how many of its lines are unread. It never holds a command's output.

### Waking the agent

- A conversation wakes when one of its commands ends on its own (it exited, or its host reported its end or closed its channel), the model has not been told of the end, the conversation is one of the owner's conversations on the master, and no reply runs in it. A task conversation never wakes.
- Mannie checks when it records the end, and again whenever the conversation becomes free: after a reply ends, after a message or a retry that failed before its reply started, and after a compaction. The check reserves the conversation as the owner's message does, so a wake-up and the owner's message never both start a reply, and the ends stay untold until the turn of the wake-up is stored.
- Ends within 2 seconds of each other share one wake-up.
- A wake-up starts a reply of the conversation agent with the conversation's provider profile. Its request is a part `command_note`: the model reads Mannie's note with the ends, and the owner sees a row in place of a message.
- A wake-up's reply is an ordinary reply. It has the agent's budgets and tools, asks for approvals and waits for the owner, runs without asking in a conversation with full access, can be stopped and retried, and shows in the conversation list and in the quit warning as a reply does.
- Limits:
  - at most 6 wake-ups in an hour for each conversation; an end beyond them stays untold for the next reply;
  - after the owner stops a reply, the conversation does not wake until the owner's next message;
  - the Tools setting "Wake the agent when a background command ends", on by default, turns wake-ups off, and the ends stay untold for the next reply;
  - nothing wakes while Mannie quits, or at startup, when every command that ran is interrupted.

### The part `command_note`

A part of the kind `command_note`, with the audience both. It is the request of a wake-up's turn, stored before the reply runs, as the owner's message is. Its text is what the model reads, and its document lists the ends: each command's number, text, end, exit code and unread lines. The UI shows it as a row, "Command 3 ended, and the agent continued", built from the document, with each command and its end. A reader that does not know the kind shows the generic card, and the conversation export writes it as one line.

### The text the model sees

The texts live in `crates/mannie-runtime/src/texts.rs` and are reviewed like code. "{n}" is a command's number and "{command}" its text.

| Case                             | Wording                                                                                                                                                                                                                  |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| The note                         | "Background commands of this conversation:", one line for each command, then "Read their output with read_command_output when you need it."                                                                              |
| A command that exited            | "Command {n} ({command}) ended with exit code {code} after {duration}; {unread} of its lines are unread."                                                                                                                |
| A command that the host ended    | "Command {n} ({command}) was ended on its host after {duration}", with the exit code or the signal when the host reported one, and the unread lines                                                                      |
| A command that the owner stopped | "Command {n} ({command}) was stopped by the owner after {duration}", with the unread lines                                                                                                                               |
| A command that ended otherwise   | "Command {n} ({command}) ended" with its task, its plugin or its session, or "when Mannie quit", with the unread lines, and for a remote command that its processes may still run on the host                            |
| A command that runs              | "Command {n} ({command}) has run for {duration}; {unread} new lines are unread."                                                                                                                                         |
| A wake-up                        | The note, then: "No message from the owner started this reply: a background command ended. If its end matters to what the owner asked last, continue with that; otherwise say in one sentence that nothing needs doing." |

### Settings

The settings section `tools` gains `wake_on_command_end`, true by default, also for settings that were stored before it existed.

### Contracts

- The part kind `command_note`, with its document, and the change that starts a turn with it as the request and marks its ends as told. The UI's reducer, compaction and the Markdown export know the kind.
- A turn that the owner's message starts may hold a continuation note with its request.
- `ToolsSettings` and `UpdateToolsSettings` gain `wake_on_command_end`.

### Message keys

The row and the setting use keys under `tools_background_`.

### Threat notes

- T-53: a wake-up starts a reply without the owner's message, so a reply can run tools while the owner is away, and in a conversation with full access its operations run without asking (T-29). Only a command's own end wakes, the note names commands and never holds their output, at most 6 wake-ups run in an hour, Stop pauses them until the owner's next message, the setting turns them off, and approvals still wait for the owner.
- T-54: a note never holds output, so a command's output reaches the model only as a tool result, never as a message of the user.

### Tests

- `crates/mannie-runtime/tests/background_commands.rs`, with a scripted provider and a manual clock: the note after a step, the continuation of a reply that would end with ends untold, the note at the start of a reply and its audience, a wake-up's reply with its part and profile, ends that share a wake-up, the hourly limit, the pause after Stop, the setting, the checks after a reply ends and after a failed send, a wake-up that races the owner's message, no wake-up for a task conversation or while Mannie quits, and an end by `stop_command` that is never noted.
- The end-to-end test shows a wake-up's row and its reply.

## Acceptance

- A command that ends on its own while the conversation is idle starts one reply whose request is the row, and the model reads the note without the command's output (runtime test and end-to-end test).
- Two ends within 2 seconds start one reply, a seventh end within an hour starts none and is told in the next reply, and after Stop no end wakes until the owner's next message (runtime tests with a manual clock).
- With the setting off, no end wakes, and the next reply's opening note tells the ends (runtime test).
- An end during a reply reaches the model as a note before its next call, a reply that would end with an end untold continues once, and an end that arrives as the reply ends wakes the agent after it (runtime tests).
- A child's or a worker's task conversation never wakes (runtime tests).
