// The parts of task tools (docs/features/child-agents/child-agent-activity.md):
// a task's progress, the part of a child's call that waits for the owner,
// which a card renders as a part of the child's conversation, and the child
// agents that a conversation's toolbar lists.
import { describe, expect, it } from 'vitest';

import type {
  ApprovalRequest,
  PartView,
  TaskState,
  TaskView,
  ToolCallView,
  TurnView,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import {
  approvalPart,
  childAgentsOf,
  isCoreTaskTool,
  isTaskActive,
  isTaskTroubled,
} from '../../../../../../apps/desktop/src/lib/utils/task-parts.js';

const CALL: ToolCallView = {
  accesses: [],
  activity: 'Writing the notes',
  arguments: '{"path":"notes.md"}',
  argumentsCut: false,
  decision: null,
  endedAt: null,
  notRun: null,
  request: null,
  result: null,
  startedAt: null,
  status: 'awaiting_approval',
  tool: 'write_file',
};

function task(approval: TaskView['approval']): TaskView {
  return {
    activity: null,
    agent: 'general',
    approval,
    background: false,
    conversationId: 'child-conversation',
    detached: false,
    endedAt: null,
    executor: null,
    failure: null,
    followUp: false,
    instruction: '',
    number: 2,
    report: null,
    startedAt: 5,
    state: 'running',
    taskId: 'task',
    title: 'Write the notes',
    toolCalls: 1,
    worker: null,
  };
}

/** The target of a task of the Workers plugin's executor. */
const WORKER_TARGET = { kind: 'workers', noun: 'the worker', targetId: 'worker', targetName: 'freebsd-01' };

describe('isCoreTaskTool', () => {
  it('knows the core’s task tools, and takes any other tool for a plugin’s executor’s', () => {
    expect(['start_task', 'follow_up_task', 'wait_for_tasks'].every((tool) => isCoreTaskTool(tool))).toBe(true);
    expect(isCoreTaskTool('delegate_task')).toBe(false);
  });
});

describe('task states', () => {
  it('counts a task as active until it ends, and its trouble once it failed or was interrupted', () => {
    const states: TaskState[] = ['accepted', 'running', 'reported', 'failed', 'canceled', 'interrupted'];

    expect(states.filter((state) => isTaskActive(state))).toEqual(['accepted', 'running']);
    expect(states.filter((state) => isTaskTroubled(state))).toEqual(['failed', 'interrupted']);
  });
});

describe('approvalPart', () => {
  it('builds the child’s waiting call as a streaming tool call part of the child’s conversation', () => {
    const part = approvalPart(
      task({ call: CALL, createdAt: 7, partId: 'child-part', position: 3, runId: 'child-run', turnId: 'child-turn' }),
    );

    expect(part).toEqual({
      content: { call: CALL, kind: 'tool_call' },
      createdAt: 7,
      id: 'child-part',
      position: 3,
      runId: 'child-run',
      state: 'streaming',
      turnId: 'child-turn',
    });
  });

  it('has no part while no call of the child waits', () => {
    expect(approvalPart(task(null))).toBeNull();
  });

  it('names the host of a call of an executor’s task by the name that the card shows for the target', () => {
    const request: ApprovalRequest = { digest: 'digest', grants: [], host: 'freebsd.local', reasons: [], rule: null };
    const waiting = { call: { ...CALL, request }, createdAt: 7, partId: 'p', position: 3, runId: 'r', turnId: 't' };
    const onWorker: TaskView = { ...task(waiting), executor: WORKER_TARGET };

    const shown = approvalPart(onWorker)?.content;
    const ofChild = approvalPart(task(waiting))?.content;

    expect(shown).toEqual({ call: { ...CALL, request: { ...request, host: 'freebsd-01' } }, kind: 'tool_call' });
    expect(ofChild).toEqual({ call: { ...CALL, request }, kind: 'tool_call' });
  });
});

/** A part of a task tool's call that assigned `assigned`, or none, as a wait does. */
function taskPart(id: string, assigned: TaskView | null): PartView {
  return {
    content: {
      call: {
        activity: null,
        awaited: [],
        endedAt: null,
        notRun: null,
        refusal: null,
        startedAt: null,
        status: 'completed',
        task: assigned,
        tool: assigned === null ? 'wait_for_tasks' : 'start_task',
      },
      kind: 'task',
    },
    createdAt: 1,
    id,
    position: 1,
    runId: 'run',
    state: 'complete',
    turnId: 'turn',
  };
}

function child(number: number, state: TaskState, conversationId = `child-${String(number)}`): TaskView {
  return { ...task(null), conversationId, number, state, title: `Child ${String(number)}` };
}

function turn(position: number, parts: PartView[]): TurnView {
  return { createdAt: position, id: `turn-${String(position)}`, parts, position, run: null };
}

describe('childAgentsOf', () => {
  it('lists each child once, with its latest task, the ones that work first', () => {
    const turns = [
      turn(0, [taskPart('a', child(1, 'reported')), taskPart('b', child(2, 'reported'))]),
      turn(1, [taskPart('c', child(3, 'running')), taskPart('d', child(1, 'running')), taskPart('e', null)]),
    ];

    const children = childAgentsOf(turns);

    expect(children.map((agent) => [agent.number, agent.state])).toEqual([
      [1, 'running'],
      [3, 'running'],
      [2, 'reported'],
    ]);
  });

  it('lists no child in turns without tasks', () => {
    expect(childAgentsOf([turn(0, [])])).toEqual([]);
  });

  it('leaves out the tasks of plugins’ executors, which are no child agents', () => {
    const onWorker = { ...child(2, 'running'), executor: WORKER_TARGET };
    const turns = [turn(0, [taskPart('a', child(1, 'running')), taskPart('b', onWorker)])];

    expect(childAgentsOf(turns).map((agent) => agent.number)).toEqual([1]);
  });
});
