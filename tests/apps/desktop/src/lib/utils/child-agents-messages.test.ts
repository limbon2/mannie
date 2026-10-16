// Every code and state of the child agents feature has a message of its own
// (ADR-0019), and the messages name the tasks they are about. A refusal of a
// plugin's executor shows in its kind's words while the plugin is on, and
// names its target otherwise (ADR-0031, decision 7).
import { afterEach, describe, expect, it } from 'vitest';

import type {
  ChildAgentsError,
  TaskRefusal,
  TaskState,
  TaskView,
  ToolCallView,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { setPluginCatalog } from '../../../../../../apps/desktop/src/lib/state/plugin-catalog.js';
import type { TaskKind } from '../../../../../../apps/desktop/src/lib/types/shell-slots.js';
import {
  agentLabel,
  childAgentsFailureMessage,
  taskRefusalMessage,
  taskStateLabel,
  taskStateName,
} from '../../../../../../apps/desktop/src/lib/utils/child-agents-messages.js';

const STATES: TaskState[] = ['accepted', 'running', 'reported', 'failed', 'canceled', 'interrupted'];

function task(state: TaskState, waits: boolean): TaskView {
  const call: ToolCallView = {
    accesses: [],
    activity: null,
    arguments: '{}',
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
  return {
    activity: null,
    agent: 'general',
    approval: waits ? { call, createdAt: 1, partId: 'part', position: 2, runId: 'run', turnId: 'turn' } : null,
    background: false,
    conversationId: 'child',
    detached: false,
    endedAt: null,
    executor: null,
    failure: null,
    followUp: false,
    instruction: 'Write it.',
    number: 1,
    report: null,
    startedAt: null,
    state,
    taskId: 'task',
    title: 'Write the notes',
    toolCalls: 0,
    worker: null,
  };
}

describe('childAgentsFailureMessage', () => {
  it('has a message for every error, and the range for an invalid limit', () => {
    const errors: ChildAgentsError[] = [
      { code: 'parallel_children_invalid', max: 16, min: 1 },
      { code: 'written_by_newer_version' },
      { code: 'storage_failed' },
      { code: 'store_unavailable' },
    ];

    const messages = errors.map((error) => childAgentsFailureMessage({ error, kind: 'core' }));

    expect(new Set(messages).size).toBe(errors.length);
    expect(messages[0]).toBe('Choose a number from 1 to 16.');
    expect(childAgentsFailureMessage({ kind: 'ipc' })).toMatch(/core/u);
  });
});

describe('task states', () => {
  it('names every state differently', () => {
    const names = STATES.map((state) => taskStateName(state));

    expect(new Set(names).size).toBe(STATES.length);
    expect(taskStateName('accepted')).toBe('Waiting for a free slot');
  });

  it('says that a working child waits for the owner while its call waits', () => {
    expect(taskStateLabel(task('running', false))).toBe('Working');
    expect(taskStateLabel(task('running', true))).toBe('Waiting for your approval');
    expect(taskStateLabel(task('reported', false))).toBe('Reported');
  });
});

describe('agentLabel', () => {
  it('names the built-in agents and shows another by its name', () => {
    expect(agentLabel('explore')).toBe('Explore');
    expect(agentLabel('general')).toBe('General');
    expect(agentLabel('reviewer')).toBe('reviewer');
  });
});

/** The Workers plugin's kind of task, which words one refusal code. */
const WORKERS_KIND: TaskKind = {
  detached: () => 'It may still run on its worker.',
  icon: () => ({}),
  id: 'workers',
  readOnly: () => 'A worker runs this task.',
  refusal: (code, target) => (code === 'worker_offline' ? `${target ?? ''} is not connected.` : null),
  targetPrefix: () => 'Worker:',
  tool: 'delegate_task',
  untitledCall: () => 'Delegating a task',
  waiting: () => 'Waiting for workers',
};

function kindOf(kind: string): TaskKind | undefined {
  return kind === 'workers' ? WORKERS_KIND : undefined;
}

afterEach(() => {
  setPluginCatalog([]);
});

describe('taskRefusalMessage', () => {
  it('says why each call of the core’s task tools assigned no task, with the number it named', () => {
    const refusals: TaskRefusal[] = [
      { number: 3, reason: 'unknown_task' },
      { number: 2, reason: 'still_running' },
      { reason: 'budget_used' },
      { reason: 'not_started' },
    ];

    const messages = refusals.map((refusal) => taskRefusalMessage(refusal));

    expect(new Set(messages).size).toBe(refusals.length);
    expect(messages[0]).toContain('task 3');
    expect(messages[1]).toContain('task 2');
  });

  it('words an executor’s refusal as its kind does while the plugin is on, and names the target otherwise', () => {
    const offline: TaskRefusal = { code: 'worker_offline', kind: 'workers', reason: 'executor', target: 'build-01' };

    expect(taskRefusalMessage(offline, kindOf)).toBe('build-01 is not connected.');
    expect(taskRefusalMessage(offline)).toBe('No task started for build-01.');
    expect(taskRefusalMessage({ ...offline, code: 'newer_code' }, kindOf)).toBe('No task started for build-01.');
    expect(taskRefusalMessage({ code: 'newer_code', kind: 'workers', reason: 'executor' }, kindOf)).toBe(
      'No task started.',
    );
  });

  it('says that the plugin of an executor is off, by the plugin’s name', () => {
    setPluginCatalog([
      { description: () => '', failure: () => null, id: 'workers', name: () => 'Workers', offQuestion: () => null },
    ]);

    expect(taskRefusalMessage({ code: 'plugin_off', kind: 'workers', reason: 'executor' }, kindOf)).toBe(
      'No task started: the Workers plugin is off.',
    );
    expect(taskRefusalMessage({ code: 'plugin_off', kind: 'removed', reason: 'executor' })).toBe(
      'No task started: the removed plugin is off.',
    );
  });

  it('names the worker of a refusal that 0.2.0 stored', () => {
    const refusals: TaskRefusal[] = [
      { reason: 'worker_offline', worker: 'build-01' },
      { reason: 'worker_unknown', worker: 'build-01' },
      { reason: 'worker_incompatible', worker: 'build-01' },
      { reason: 'profile_not_reachable', worker: 'build-01' },
    ];

    const messages = refusals.map((refusal) => taskRefusalMessage(refusal, kindOf));

    expect(new Set(messages)).toEqual(new Set(['No task started for build-01.']));
  });
});
