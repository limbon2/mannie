# Domain model

|              |                                                                                                                                                                                                                                                                                   |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                                                                                                                                                  |
| Last updated | 2026-10-20                                                                                                                                                                                                                                                                        |
| Related      | [Glossary](../glossary.md), [Agent runtime](agent-runtime.md), [Tools](tools.md), [Orchestration](orchestration.md), [Overview](overview.md), [ADR-0030](../adr/0030-split-mannie-into-a-core-and-plugins.md), [Workers architecture](../../plugins/workers/docs/architecture.md) |

This document defines the core concepts, their lifecycles and the invariants every contract follows. The [glossary](../glossary.md) gives short definitions; this document shows how the concepts relate.

## 1. Concepts

```mermaid
classDiagram
  class Machine
  class Master
  class Worker
  class Project
  class Conversation
  class Turn
  class Step
  class Part
  class Run
  class Operation
  class Task
  class Report
  class ProviderProfile
  class Plugin
  class TaskExecutor
  Machine <|-- Master
  Machine <|-- Worker
  Machine "1" --> "*" Conversation : stores
  Machine "1" --> "*" Project : holds
  Conversation "*" --> "0..1" Project : belongs to
  Conversation "1" --> "*" Turn
  Turn "1" --> "*" Step
  Turn "1" --> "*" Part
  Turn "1" --> "0..1" Run : executed by
  Step "1" --> "*" Operation : tool calls with side effects
  Task "1" --> "1" Conversation : runs in
  Task "1" --> "0..1" Report
  Task "*" --> "0..1" TaskExecutor : runs elsewhere through
  Plugin "1" --> "*" TaskExecutor : registers
  Conversation "*" --> "1" ProviderProfile : uses
```

## 2. Machines

- A machine is a computer Mannie acts on. There is one master, the desktop app, and, with the Workers plugin on, any number of workers. Each worker runs on its own host.
- The master holds the owner's conversations, the provider profiles, the owner's choices of plugins and the owner's approvals, and each plugin's data in the plugin's own tables, such as the worker registry.
- A worker holds its host facts, its host instructions, its policy ceiling and the conversations of the tasks it runs. It is paired with exactly one master, which records its name, address, pinned key and last known host facts ([workers architecture](../../plugins/workers/docs/architecture.md)).

## 3. Conversations, turns, steps and parts

- A conversation is an ordered list of turns between a requester and an agent. The requester is the owner for conversations on the master, and an agent for task conversations.
- A turn is one request and the agent's complete response. A turn is made of parts, the typed elements of the transcript: text, reasoning, tool call, tool result, task events, errors and the part types that features add.
- A step is one model call within a turn, together with the tool calls it produced. A turn with tool use has several steps.
- Parts are appended. A part's text and document can change while it is open, as streamed text does or a tool call that waits for an approval, and they are final once it finishes. A correction or a retry adds parts; it does not rewrite finished ones.
- Every change to a conversation produces an event with a sequence number that is unique and increasing within that conversation.
- A project is a folder on the master that the owner added, with the conversations that work in it ([ADR-0023](../adr/0023-organize-conversations-into-projects.md)). A conversation belongs to at most one project, from its start or from the moment it moves into one, and its workspace is the project's folder. A general conversation belongs to none.

## 4. Runs

A run executes one turn ([ADR-0012](../adr/0012-run-agents-as-durable-actors.md)).

```mermaid
stateDiagram-v2
  [*] --> Queued
  Queued --> Running
  Running --> Completed
  Running --> Failed: permanent error
  Running --> Stopped: the owner stops it
  Running --> Interrupted: process ended
  Completed --> [*]
  Failed --> [*]
  Stopped --> [*]
  Interrupted --> [*]
```

- A run that waits for the owner's approval of an operation stays running. Its activity says that it waits, and it holds no model slot meanwhile.
- A retry of a failed, stopped or interrupted run starts a new run for the same turn. It reuses the results of operations that completed and never repeats an operation that may have run. When the earlier run made tool calls, its parts stay in the turn and the new run continues after them.
- A run's state is stored before it is published, like every other event.

## 5. Operations

An operation is one side effect of a tool, recorded in the journal ([ADR-0013](../adr/0013-journal-every-side-effect.md)).

```mermaid
stateDiagram-v2
  [*] --> Prepared
  Prepared --> AwaitingApproval: policy asks the owner
  Prepared --> Executing: policy allows
  Prepared --> Denied: policy denies
  AwaitingApproval --> Executing: owner allows
  AwaitingApproval --> Denied: owner denies
  AwaitingApproval --> Canceled: run stopped
  Executing --> Completed
  Executing --> Uncertain: process ended during execution
  Executing --> Canceled: stopped and confirmed not applied
  Completed --> [*]
  Denied --> [*]
  Canceled --> [*]
  Uncertain --> [*]
```

- An operation records its canonical accesses: each path with its access mode, and each command with its shell and working directory.
- The journal record of an operation is its tool call part, so each change of its state is an event of the conversation.
- Executing is committed durably before the side effect starts.
- An uncertain operation is reported to the model and to the owner. Nothing repeats it automatically.

## 6. Tasks and reports

A task is work that one agent assigns to another: a child agent on the same machine, or the agent that a plugin's executor runs it with, such as a worker agent ([orchestration](orchestration.md)).

```mermaid
stateDiagram-v2
  [*] --> Created
  Created --> Accepted: the executing machine stored it
  Accepted --> Running
  Running --> Reported: confirmed report
  Running --> Failed
  Running --> Canceled
  Running --> Interrupted: process ended
  Interrupted --> Running: resumed
  Reported --> [*]
  Failed --> [*]
  Canceled --> [*]
```

- A task has an ID that the assigning side chooses. Submitting the same ID twice creates one task.
- A task carries an instruction, its constraints (budgets, deadline, a narrower policy) and a provider profile. It may continue an earlier task's conversation.
- A task runs in a conversation on the executing machine. The assigning agent receives only the report; the owner can open the whole task conversation.
- A report is confirmed: the executing agent checked that it finished before it answered.
- A child agent's task runs in a task conversation on the master: a conversation with a parent conversation, a number that is unique within the parent, and the child's agent definition ([tasks](../features/child-agents/tasks.md#task-conversations)). A task conversation belongs to no project and works under its parent's workspace, project, grants and rules. Deleting the parent deletes its task conversations.
- A task conversation has at most one task that is accepted or running. A follow-up is a new task in the same conversation, with the same number and agent. Nothing resumes an interrupted task of a child by itself; the parent's model decides whether to follow it up.
- The conversation of a task of a plugin's executor lives on the executor's target, such as a worker, which runs the task without a parent of its own. The master keeps a row for it with the parent, the number and the executor, with its kind and target, and reads the conversation through the executor ([ADR-0031](../adr/0031-move-workers-into-the-first-plugin.md)). Quitting the master, or turning the plugin off, lets the task go while it runs on, and the executor learns its end when it follows the task again.

## 7. Plugins

A plugin is an optional feature, or a group of features, that the owner turns on and off ([ADR-0030](../adr/0030-split-mannie-into-a-core-and-plugins.md)).

```mermaid
stateDiagram-v2
  [*] --> Off
  Off --> Starting: the owner turns it on
  Starting --> On: its start returned its contributions
  Starting --> Failed: its start failed
  Failed --> Starting: Try again
  On --> Stopping: the owner turns it off, or Mannie quits
  Failed --> Off: the owner turns it off
  Stopping --> Off
```

- The owner's choice for each plugin is a setting of the core. At startup, after the core's recovery, the host starts each plugin that the owner turned on. A plugin without a choice follows its default once, and that outcome becomes the choice.
- While a plugin is on, its contributions are in the registries and its entries in the UI. Turning it off drops them first and then stops the plugin, which ends its background work as quitting does.
- A plugin's data stays while it is off, and the plugin continues with it when it starts again. What it added to conversations stays readable from what the core stored.
- The workers of the Workers plugin have a lifecycle of their own, from pairing to removal ([workers architecture](../../plugins/workers/docs/architecture.md#the-link)).

## 8. Identifiers and time

- Entity IDs are UUIDv7 strings, created by the side that creates the entity.
- Event sequence numbers are integers, increasing within a conversation.
- Timestamps are UTC, stored as milliseconds since the Unix epoch. The UI shows local time.

## 9. Invariants

1. The transcript is append-only. Compaction changes what the model sees, never what is stored.
2. An event is stored before it is published.
3. An operation executes at most once. An uncertain operation is never repeated automatically.
4. A side effect needs an allow decision from policy or from the owner. Content that the model reads never grants a permission.
5. An agent's tools act on the machine where the agent runs, and on a remote host only through a remote command that the master's policy authorized ([ADR-0036](../adr/0036-let-plugins-run-commands-on-remote-hosts.md)). The master never runs tools on a worker's host.
6. A machine's policy ceiling, such as a worker's, can be narrowed for a task and never widened.
7. Provider credentials are at rest only in the master's operating system credential store.
8. Contracts and stored data change only by addition, with deprecation before removal.
9. Every task ID identifies one task, however many times it is submitted.
10. A conversation in a project works in the project's folder and never leaves the project.
11. A child agent works under its parent conversation's workspace, grants and rules, and starts no task of its own. Its parent's run ends only after the child's run ended.
12. The core never depends on a plugin. Turning a plugin off or on repeats no operation and loses none, and a run keeps the tools it started with.
