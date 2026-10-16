// What the parts of task tools show (docs/features/child-agents/
// child-agent-activity.md): the tools, a task's progress, the part of a
// child's call that waits for the owner, which a card renders through the
// shell's part outlet as a part of the child's conversation, and the child
// agents that a conversation's toolbar lists.
import type { PartView, TaskState, TaskView, ToolCallView, TurnView } from '#lib/generated/bindings.js';

/** The task tool that waits for reports; the other two start and follow up tasks. */
export const WAIT_FOR_TASKS = 'wait_for_tasks';

/** The core's task tools; every other tool of a task part is a plugin's executor's (ADR-0031). */
const CORE_TASK_TOOLS: readonly string[] = ['start_task', 'follow_up_task', WAIT_FOR_TASKS];

/** Whether a task part's tool is one of the core's. */
export function isCoreTaskTool(tool: string): boolean {
  return CORE_TASK_TOOLS.includes(tool);
}

/** Whether a task's child still works on it, or waits for a slot to start. */
export function isTaskActive(state: TaskState): boolean {
  const waitsForSlot = state === 'accepted';
  const runs = state === 'running';
  return waitsForSlot || runs;
}

/** Whether a task ended in a way that the owner should notice: it failed or was interrupted. */
export function isTaskTroubled(state: TaskState): boolean {
  const failed = state === 'failed';
  const interrupted = state === 'interrupted';
  return failed || interrupted;
}

/** The ones that work first, then each by its number. */
function workingFirst(left: TaskView, right: TaskView): number {
  const leftWorks = isTaskActive(left.state);
  const rightWorks = isTaskActive(right.state);
  if (leftWorks !== rightWorks) {
    return leftWorks ? -1 : 1;
  }
  return left.number - right.number;
}

/**
 * The child agents in a conversation's turns: one for each child
 * conversation, with the view of its latest task, since a follow-up shows
 * the child's latest state. The ones that work come first. The tasks of
 * plugins' executors are no child agents and stay out (ADR-0031, decision 7).
 */
export function childAgentsOf(turns: readonly TurnView[]): TaskView[] {
  const latest = new Map<string, TaskView>();
  for (const turn of turns) {
    for (const part of turn.parts) {
      const task = part.content.kind === 'task' ? part.content.call.task : null;
      // A task without an executor is a child agent's.
      if (task?.executor === null) {
        latest.set(task.conversationId, task);
      }
    }
  }
  return [...latest.values()].sort(workingFirst);
}

/**
 * The call of a task of a plugin's executor as the card shows it: the
 * request names the computer that runs it by the name that the card shows
 * for the target, which can be the owner's, where the target wrote another
 * one. Only the shown name changes; the decision still names the request by
 * its digest.
 */
function shownCall(task: TaskView, call: ToolCallView): ToolCallView {
  const target = task.executor ?? null;
  const request = call.request;
  if (target === null) {
    return call;
  }
  if (request === null) {
    return call;
  }
  const namesHost = (request.host ?? null) !== null;
  if (!namesHost) {
    return call;
  }
  return { ...call, request: { ...request, host: target.targetName } };
}

/**
 * The part of the child's conversation that waits for the owner, built from
 * what the parent's part mirrors, or `null` while none waits. It renders as
 * the child's own tool call, with its request and the owner's choices.
 */
export function approvalPart(task: TaskView): PartView | null {
  const approval = task.approval;
  if (approval === null) {
    return null;
  }
  return {
    content: { call: shownCall(task, approval.call), kind: 'tool_call' },
    createdAt: approval.createdAt,
    id: approval.partId,
    position: approval.position,
    runId: approval.runId,
    state: 'streaming',
    turnId: approval.turnId,
  };
}
