// Every code and state of the child agents feature has a message of its own
// (ADR-0019), and the messages name the tasks they are about.
import { describe, expect, it } from 'vitest';

import type {
  ChildAgentsError,
  TaskRefusal,
  TaskState,
  TaskView,
  ToolCallView,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
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

describe('taskRefusalMessage', () => {
  it('says why each call assigned no task, with the number it named', () => {
    const refusals: TaskRefusal[] = [
      { number: 3, reason: 'unknown_task' },
      { number: 2, reason: 'still_running' },
      { reason: 'budget_used' },
      { reason: 'not_started' },
      { reason: 'worker_offline', worker: 'build-01' },
      { reason: 'worker_unknown', worker: 'build-02' },
      { reason: 'worker_incompatible', worker: 'build-03' },
      { reason: 'profile_not_reachable', worker: 'build-04' },
    ];

    const messages = refusals.map((refusal) => taskRefusalMessage(refusal));

    expect(new Set(messages).size).toBe(refusals.length);
    expect(messages[0]).toContain('task 3');
    expect(messages[1]).toContain('task 2');
    expect(messages.slice(4).map((message, index) => message.includes(`build-0${String(index + 1)}`))).toEqual([
      true,
      true,
      true,
      true,
    ]);
  });
});
