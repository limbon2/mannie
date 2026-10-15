// The conversation view's transcript (docs/features/chat/conversations.md,
// transcript and events): a snapshot, then events in sequence order; events
// at or below the cursor change nothing.
import { describe, expect, it } from 'vitest';

import type {
  ConversationEvent,
  ConversationSnapshot,
  ConversationUpdate,
  PartView,
  RunView,
  ToolCallStatus,
  ToolCallView,
  TurnView,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import {
  activeRun,
  applyUpdate,
  canRetry,
  emptyTranscript,
  endedRun,
  hasActiveRun,
  prependTurns,
} from '../../../../../../apps/desktop/src/lib/utils/conversation-reducer.js';

const CONVERSATION = '0199e0f2-0000-7000-8000-000000000001';

function run(id: string, turnId: string, state: RunView['state']): RunView {
  return {
    activity: null,
    createdAt: 0,
    endedAt: null,
    id,
    outcome: null,
    profileId: '0199e0f2-0000-7000-8000-0000000000ff',
    state,
    turnId,
  };
}

function userPart(id: string, turnId: string, text: string): PartView {
  return {
    content: { kind: 'user_text', text },
    createdAt: 0,
    id,
    position: 0,
    runId: null,
    state: 'complete',
    turnId,
  };
}

function textPart(id: string, turnId: string, runId: string, position: number, text: string): PartView {
  return { content: { kind: 'text', text }, createdAt: 0, id, position, runId, state: 'streaming', turnId };
}

function toolPart(id: string, turnId: string, runId: string, position: number, status: ToolCallStatus): PartView {
  const call: ToolCallView = {
    accesses: [],
    activity: null,
    arguments: '{"path":"notes.md"}',
    argumentsCut: false,
    decision: null,
    endedAt: null,
    notRun: null,
    request: null,
    result: null,
    startedAt: null,
    status,
    tool: 'read_file',
  };
  return { content: { call, kind: 'tool_call' }, createdAt: 0, id, position, runId, state: 'streaming', turnId };
}

function turn(position: number, runState: RunView['state'] = 'running'): TurnView {
  const id = `turn-${String(position)}`;
  return {
    createdAt: 0,
    id,
    parts: [userPart(`user-${String(position)}`, id, `Question ${String(position)}`)],
    position,
    run: run(`run-${String(position)}`, id, runState),
  };
}

function snapshot(turns: TurnView[], throughSeq: number): ConversationUpdate {
  const value: ConversationSnapshot = {
    checkpoint: null,
    context: null,
    conversation: {
      awaitingApproval: false,
      createdAt: 0,
      hasActiveRun: false,
      id: CONVERSATION,
      profileId: null,
      title: 'Title',
      titleSource: 'first_message',
      updatedAt: 0,
      projectId: null,
      workspace: null,
    },
    hasEarlierTurns: false,
    throughSeq,
    turns,
  };
  return { snapshot: value, type: 'snapshot' };
}

function event(seq: number, value: ConversationEvent): ConversationUpdate {
  return { event: value, seq, type: 'event' };
}

describe('applyUpdate', () => {
  it('starts from a snapshot and its sequence number', () => {
    const state = emptyTranscript();

    expect(applyUpdate(state, snapshot([turn(1), turn(0)], 7))).toBe('applied');

    expect(state.turns.map((entry) => entry.position)).toEqual([0, 1]);
    expect(state.cursor).toBe(7);
  });

  it('streams a part from its start through its deltas to its final text', () => {
    const state = emptyTranscript();
    applyUpdate(state, snapshot([turn(0)], 3));
    const started = textPart('part-1', 'turn-0', 'run-0', 1, '');

    applyUpdate(state, event(4, { kind: 'part_started', part: started }));
    applyUpdate(state, event(5, { kind: 'part_delta', partId: 'part-1', text: 'Hel' }));
    applyUpdate(state, event(6, { kind: 'part_delta', partId: 'part-1', text: 'lo' }));
    const partial = state.turns[0]?.parts[1]?.content;
    applyUpdate(
      state,
      event(7, {
        kind: 'part_finished',
        part: { ...started, content: { kind: 'text', text: 'Hello!' }, state: 'complete' },
      }),
    );

    expect(partial).toEqual({ kind: 'text', text: 'Hello' });
    expect(state.turns[0]?.parts[1]).toMatchObject({ content: { kind: 'text', text: 'Hello!' }, state: 'complete' });
    expect(state.cursor).toBe(7);
  });

  it('skips events at or below the cursor', () => {
    const state = emptyTranscript();
    applyUpdate(state, snapshot([turn(0)], 10));

    const outcome = applyUpdate(state, event(9, { kind: 'turn_started', turn: turn(1) }));

    expect(outcome).toBe('skipped');
    expect(state.turns).toHaveLength(1);
  });

  it('removes the parts of a discarded attempt', () => {
    const state = emptyTranscript();
    applyUpdate(state, snapshot([turn(0)], 1));
    applyUpdate(state, event(2, { kind: 'part_started', part: textPart('lost', 'turn-0', 'run-0', 1, 'partial') }));

    applyUpdate(state, event(3, { kind: 'attempt_discarded', partIds: ['lost'], runId: 'run-0' }));

    expect(state.turns[0]?.parts.map((part) => part.id)).toEqual(['user-0']);
  });

  it('replaces the reply of a retried turn and keeps the owner message', () => {
    const state = emptyTranscript();
    const failed = turn(0, 'failed');
    failed.parts.push(textPart('old', 'turn-0', 'run-0', 1, 'old reply'));
    applyUpdate(state, snapshot([failed], 5));

    applyUpdate(state, event(6, { kind: 'run_started', run: run('run-retry', 'turn-0', 'queued') }));
    applyUpdate(state, event(7, { kind: 'part_started', part: textPart('new', 'turn-0', 'run-retry', 2, '') }));

    expect(state.turns[0]?.run?.id).toBe('run-retry');
    expect(state.turns[0]?.parts.map((part) => part.id)).toEqual(['user-0', 'new']);
  });

  it('replaces a tool call with each update, and ignores text for it', () => {
    const state = emptyTranscript();
    applyUpdate(state, snapshot([turn(0)], 1));
    applyUpdate(state, event(2, { kind: 'part_started', part: toolPart('call', 'turn-0', 'run-0', 1, 'receiving') }));

    applyUpdate(state, event(3, { kind: 'part_delta', partId: 'call', text: '{"path":' }));
    applyUpdate(
      state,
      event(4, { kind: 'part_updated', part: toolPart('call', 'turn-0', 'run-0', 1, 'awaiting_approval') }),
    );

    const content = state.turns.at(0)?.parts.at(1)?.content;
    expect(content?.kind === 'tool_call' ? content.call.status : null).toBe('awaiting_approval');
    expect(content?.kind === 'tool_call' ? content.call.arguments : null).toBe('{"path":"notes.md"}');
    expect(state.turns[0]?.parts).toHaveLength(2);
  });

  it('keeps the parts of the runs a retried turn keeps', () => {
    const state = emptyTranscript();
    const stopped = turn(0, 'stopped');
    stopped.parts.push(toolPart('kept-call', 'turn-0', 'run-0', 1, 'completed'));
    stopped.parts.push(textPart('dropped', 'turn-0', 'run-other', 2, 'text of a run that is not kept'));
    applyUpdate(state, snapshot([stopped], 5));

    applyUpdate(
      state,
      event(6, { keptRunIds: ['run-0'], kind: 'run_started', run: run('run-retry', 'turn-0', 'queued') }),
    );

    expect(state.turns[0]?.run?.id).toBe('run-retry');
    expect(state.turns[0]?.parts.map((part) => part.id)).toEqual(['user-0', 'kept-call']);
  });

  it('reads a retry stored before runs were kept as keeping none', () => {
    const state = emptyTranscript();
    const failed = turn(0, 'failed');
    failed.parts.push(toolPart('old-call', 'turn-0', 'run-0', 1, 'completed'));
    applyUpdate(state, snapshot([failed], 5));

    applyUpdate(state, event(6, { kind: 'run_started', run: run('run-retry', 'turn-0', 'queued') }));

    expect(state.turns[0]?.parts.map((part) => part.id)).toEqual(['user-0']);
  });

  it('follows the run of the latest turn', () => {
    const state = emptyTranscript();
    applyUpdate(state, snapshot([turn(0, 'completed'), turn(1, 'running')], 2));
    expect(hasActiveRun(state)).toBe(true);
    expect(canRetry(state)).toBe(false);

    applyUpdate(state, event(3, { kind: 'run_updated', run: run('run-1', 'turn-1', 'stopped') }));

    expect(hasActiveRun(state)).toBe(false);
    expect(canRetry(state)).toBe(true);
  });

  it('finds a reply that was being written once it has ended', () => {
    const state = emptyTranscript();
    applyUpdate(state, snapshot([turn(0, 'running')], 1));
    const running = activeRun(state)?.id ?? '';
    expect(endedRun(state, running)).toBeNull();

    applyUpdate(state, event(2, { kind: 'run_updated', run: run('run-0', 'turn-0', 'completed') }));

    expect(activeRun(state)).toBeNull();
    expect(endedRun(state, running)?.state).toBe('completed');
    expect(endedRun(state, 'run-unknown')).toBeNull();
  });

  it('tells the subscriber what an end of the subscription asks of it', () => {
    const state = emptyTranscript();

    expect(applyUpdate(state, { reason: 'lagged', type: 'ended' })).toBe('resubscribe');
    expect(applyUpdate(state, { reason: 'deleted', type: 'ended' })).toBe('deleted');
    expect(applyUpdate(state, { reason: 'shutdown', type: 'ended' })).toBe('shutdown');
  });

  it('keeps the context, the checkpoint and the conversation current', () => {
    const state = emptyTranscript();
    applyUpdate(state, snapshot([turn(0)], 1));

    applyUpdate(
      state,
      event(2, { kind: 'context_usage_changed', usage: { budgetTokens: 100, estimated: false, usedTokens: 40 } }),
    );
    applyUpdate(
      state,
      event(3, {
        checkpoint: { coversThroughTurn: 0, createdAt: 0, id: 'checkpoint', tokensAfter: 10, tokensBefore: 90 },
        kind: 'checkpoint_created',
      }),
    );
    applyUpdate(state, event(4, { kind: 'unknown', name: 'tool_progress' }));

    expect(state.context?.usedTokens).toBe(40);
    expect(state.checkpoint?.id).toBe('checkpoint');
    expect(state.cursor).toBe(4);
  });
});

describe('prependTurns', () => {
  it('adds older turns in front and ignores turns already loaded', () => {
    const state = emptyTranscript();
    applyUpdate(state, snapshot([turn(5), turn(6)], 1));

    prependTurns(state, [turn(3), turn(4), turn(5)], true);

    expect(state.turns.map((entry) => entry.position)).toEqual([3, 4, 5, 6]);
    expect(state.hasEarlierTurns).toBe(true);
  });
});
