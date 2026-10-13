# Child agents

|              |                                                                                                                                                                                    |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Accepted                                                                                                                                                                           |
| Milestone    | M3                                                                                                                                                                                 |
| Last updated | 2026-10-13                                                                                                                                                                         |
| Related      | [ADR-0015](../adr/0015-run-an-agent-runtime-on-every-machine.md), [Orchestration](../architecture/orchestration.md), [System tools](system-tools.md), [Workers](workers/README.md) |

## Intent

The owner wants the conversation agent to split larger work into focused tasks for child agents on the same machine. Each child works in its own context, in parallel when that helps, and reports back. This is the local half of the master/worker system; [workers](workers/README.md) are the remote half.

## Behavior

- Kinds: explore children use read-only tools. General children can also change files and run commands.
- Tasks: the conversation agent writes a self-contained task for a child. The child works in its own conversation and context window, on the same machine, under the parent conversation's workspace, grants and rules.
- Approvals: approval requests from a child appear in the parent conversation, labeled with the child's task title.
- Parallel work: up to 4 children run at once by default, and the owner can set the limit from 1 to 16. Further tasks wait for a free slot.
- Background tasks: the parent can let a child work in the background and continue. It can wait for the report later, and its own reply does not finish while background tasks are still running.
- Follow-ups: the parent can send a follow-up task to a finished child, which keeps its context.
- Reports: before a child reports, it confirms that it has finished. Only the report enters the parent's context.
- Visibility: each child appears in the parent's reply as a card with its status, and the owner can open the child's whole conversation.
- Stopping: stopping the parent's reply stops its children, and the owner can also stop a single child.
- Limits: children cannot start children of their own, and per-turn budgets limit how many tasks a reply can start.
- Not included: child agents on workers, and agent definitions written by the owner. Each would be a feature of its own.

## Fit

- Extension points: agent definitions (explore and general), the tool registry (the task tools, and tool subsets per definition), transcript parts for task events and reports, run events, UI part renderers for the child agent card and the conversation view of a child, and the settings section for the number of parallel slots.
- Follows [ADR-0012](../adr/0012-run-agents-as-durable-actors.md), [ADR-0013](../adr/0013-journal-every-side-effect.md) and [ADR-0015](../adr/0015-run-an-agent-runtime-on-every-machine.md). The task and report contract in [orchestration](../architecture/orchestration.md#2-tasks-and-reports) is shared with workers.
- Must not change: [system tools](system-tools.md) behave the same in a child as in the conversation agent, within the child's tool set.

## Open questions

None yet.

## Decisions

None yet.

## Design

Written when M3 starts.

## Acceptance

- Explore and general children run from the same runtime as the conversation agent, and their tool sets are enforced.
- Parallel slots, background tasks, waiting, and stopping a single child all work.
- The parent receives only confirmed reports, and a follow-up keeps the child's context.
- Retrying the parent never repeats an operation that a child completed.
- An evaluation of a multi-step task split across children passes.
