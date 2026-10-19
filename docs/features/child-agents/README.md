# Child agents

|              |                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Milestone    | M3                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Last updated | 2026-10-15                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Related      | [ADR-0015](../../adr/0015-run-an-agent-runtime-on-every-machine.md), [ADR-0024](../../adr/0024-run-child-agent-tasks-in-the-parent-run.md), [ADR-0025](../../adr/0025-add-side-panels-to-the-shell.md), [Orchestration](../../architecture/orchestration.md), [Agent runtime](../../architecture/agent-runtime.md), [System tools](../system-tools/README.md), [Chat](../chat/README.md), [Workers](../../../plugins/workers/docs/README.md) |

## Intent

The owner wants the conversation agent to split larger work into focused tasks for child agents on the same machine. Each child works in its own context, in parallel when that helps, and reports back. This is the local half of the master/worker system; [workers](../../../plugins/workers/docs/README.md) are the remote half.

## Behavior

- Kinds: explore children use read-only tools. General children can also change files and run commands.
- Tasks: the conversation agent writes a self-contained task for a child. The child works in its own conversation and context window, on the same machine, under the parent conversation's workspace, grants and rules, with the provider profile of the reply that started it.
- Numbers: each child of a conversation has a number, 1, 2 and so on, by which the conversation agent and the owner refer to it.
- Approvals: approval requests from a child appear in the parent conversation, inside the child's card, labeled with the child's number and title. The owner answers them there as for the parent's own calls, and a permission given "for this conversation" applies to the parent conversation and all its children.
- Parallel work: up to 4 children run at once by default, and the owner can set the limit from 1 to 16. Further tasks wait for a free slot.
- Background tasks: the parent can let a child work in the background and continue. It reads the report later, and its own reply does not finish while background tasks are still running.
- Follow-ups: the parent can send a follow-up task to a child whose task ended. The child keeps its context.
- Reports: before a child reports, it confirms that it has finished. Only the report enters the parent's context.
- Visibility: each child appears in the parent's reply as a card with its state and what it is doing. "Open conversation" shows the child's whole conversation in a side panel beside the parent's conversation. A button in the conversation's toolbar lists its child agents, says how many work, and opens a child's conversation the same way. Several open conversations show as tabs, and they close when the owner opens another conversation.
- Stopping: stopping the parent's reply stops its children, and the owner can also stop a single child from its card or its panel.
- Limits: children cannot start children of their own, and per-reply budgets limit how many tasks a reply can start.
- Not included: child agents on workers, agent definitions written by the owner, a provider profile chosen for children, and messages from the owner to a child. Each would be a feature of its own.

## Fit

- Extension points: agent definitions, which gain who the agent answers to and a task budget, with the explore and general definitions; the task tools, which belong to the runtime ([ADR-0024](../../adr/0024-run-child-agent-tasks-in-the-parent-run.md)); the transcript part `task` and the run activity `waiting_for_tasks`; instruction sections for the guidance on tasks and the sections of child agents; store tables and migrations for task conversations and tasks; the settings section `child_agents`; IPC commands for that section; UI shell slots for the part renderer `task`, the settings section and the child agents' button among the conversation controls of [ADR-0026](../../adr/0026-add-conversation-controls-back-for-child-agents.md); the side panel slot of [ADR-0025](../../adr/0025-add-side-panels-to-the-shell.md), into which chat registers the conversation panel that shows a child; messages under the `child_agents_` prefix.
- Changes in chat, for states of the runtime, as [system tools](../system-tools/README.md#fit) made: the conversation list leaves task conversations out; the owner's commands refuse a task conversation; the status line names `waiting_for_tasks`; a conversation can show read-only, without the message box or Retry, in the side panel and in the conversation view; "Jump to the approval request" scrolls to the part that waits.
- Follows [ADR-0012](../../adr/0012-run-agents-as-durable-actors.md), [ADR-0013](../../adr/0013-journal-every-side-effect.md), [ADR-0015](../../adr/0015-run-an-agent-runtime-on-every-machine.md), [ADR-0019](../../adr/0019-keep-visible-text-in-message-catalogs.md), [ADR-0020](../../adr/0020-complete-the-ui-shell-slots.md), [ADR-0023](../../adr/0023-organize-conversations-into-projects.md), [ADR-0024](../../adr/0024-run-child-agent-tasks-in-the-parent-run.md) and [ADR-0025](../../adr/0025-add-side-panels-to-the-shell.md). The task and report contract in [orchestration](../../architecture/orchestration.md#2-tasks-and-reports) is shared with workers.
- Must not change: [system tools](../system-tools/README.md) behave the same in a child as in the conversation agent, within the child's tool set, and a conversation that starts no task behaves as it did before.

## Open questions

None.

## Decisions

- 2026-10-15: the owner decided:
  - A child's conversation opens in a side panel docked beside the parent's conversation, and side panels become an extension point of the shell that later features use too, as in the Claude app and Codex ([D18](../../roadmap/milestones.md#5-decisions), [ADR-0025](../../adr/0025-add-side-panels-to-the-shell.md)).
  - Open side panels show as tabs, and they belong to the conversation: opening another conversation leaves them behind.
  - Children use the provider profile of the reply that started them. There is no setting for a profile of children.
  - After reviewing the interface, a button in the conversation's toolbar lists the conversation's child agents and opens their conversations, through a slot for conversation controls that the shell gets again ([D19](../../roadmap/milestones.md#5-decisions), [ADR-0026](../../adr/0026-add-conversation-controls-back-for-child-agents.md)).
- 2026-10-15: the defaults below were chosen when building started. Each one waits for the owner's review, and the part documents give the details.
  - The task tools belong to the runtime and write parts of their own kind ([ADR-0024](../../adr/0024-run-child-agent-tasks-in-the-parent-run.md)).
  - A reply can start or follow up 20 tasks.
  - Explore children have the read-only system tools, without `run_command`, with 50 rounds and 150 calls of tools per task. General children have every system tool but `move_to_project`, with 100 rounds and 250 calls. Both get two continuation notes and one confirmation.
  - Children are numbered within their parent conversation, and the model refers to them by number.
  - A title has at most 80 characters, cleaned like the activity of a call. An instruction has at most 64 KiB.
  - Reports reach the parent's model only as results of the task tools. When a reply wants to end while reports are unread, a note of Mannie's own, without their text, asks the model to read them.
  - A child keeps its slot while it waits for an approval.
  - At most 8 side panels are open at once; opening another closes the oldest.

## Parts

- [Tasks](tasks.md): the task and report contract, the task tools and the text the model sees, child runs, lanes, slots, background tasks, follow-ups, reports, stopping, budgets, recovery and data.
- [Child agent activity](child-agent-activity.md): the card of a task, a child's approvals in the parent, the child's conversation in the side panel, the settings and the messages.

## Acceptance

- Explore and general children run from the same runtime as the conversation agent, and their tool sets are enforced.
- Parallel slots, background tasks, waiting, and stopping a single child all work.
- The parent receives only confirmed reports, and a follow-up keeps the child's context.
- Retrying the parent never repeats an operation that a child completed.
- An evaluation of a multi-step task split across children passes.
