// How the transcript shows the calls of plugins' tools (ADR-0039): a call
// finds the tool kind that its own plugin registered, the kind reads the
// arguments without the activity, the card's data and the plugin's failure
// code, and a code that no kind words still says that the call failed.
import { describe, expect, it } from 'vitest';

import type { ToolCallView, ToolResult } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import type { ToolKind } from '../../../../../../apps/desktop/src/lib/types/shell-slots.js';
import {
  callToolKind,
  pluginFailure,
  pluginFailureMessage,
  toolKindCall,
} from '../../../../../../apps/desktop/src/lib/utils/tool-kinds.js';

function call(changes: Partial<ToolCallView> = {}): ToolCallView {
  return {
    accesses: [],
    activity: 'Merging feature',
    arguments: JSON.stringify({ activity: 'Merging feature', branch: 'feature' }),
    argumentsCut: false,
    decision: null,
    endedAt: null,
    notRun: null,
    plugin: 'git',
    request: null,
    result: null,
    startedAt: null,
    status: 'pending',
    tool: 'git_merge',
    ...changes,
  };
}

function finished(outcome: ToolResult['outcome']): ToolCallView {
  return call({ result: { archives: [], outcome, output: 'Merged.' }, status: 'completed' });
}

const KIND: ToolKind = {
  failure: (code) => (code === 'merge_conflicts' ? 'The merge stopped at conflicts.' : null),
  icon: () => ({}),
  id: 'git_merge',
  summary: () => 'Merged feature',
};

describe('callToolKind', () => {
  it('asks for the kind of the call’s own plugin, and finds none for the core’s calls', () => {
    const asked: [string, string][] = [];
    const find = (tool: string, plugin: string): ToolKind | undefined => {
      asked.push([tool, plugin]);
      return plugin === 'git' ? KIND : undefined;
    };

    expect(callToolKind(call(), find)).toBe(KIND);
    expect(callToolKind(call({ plugin: null }), find)).toBeUndefined();
    expect(callToolKind(call({ plugin: undefined }), find)).toBeUndefined();
    expect(asked).toEqual([['git_merge', 'git']]);
  });
});

describe('toolKindCall', () => {
  it('reads the arguments without the activity, the card’s data and the failure code', () => {
    const read = toolKindCall(
      finished({ data: '{"commit":"1a2b3c","files":2}', failure: 'merge_conflicts', kind: 'plugin', plugin: 'git' }),
    );

    expect(read).toEqual({
      arguments: { branch: 'feature' },
      data: { commit: '1a2b3c', files: 2 },
      failure: 'merge_conflicts',
      status: 'completed',
    });
  });

  it('reads no data from a call without a result, older data or data that is not an object', () => {
    expect(toolKindCall(call()).data).toBeNull();
    expect(toolKindCall(finished({ kind: 'plugin', plugin: 'git' })).data).toBeNull();
    expect(toolKindCall(finished({ data: '[1,2]', failure: null, kind: 'plugin', plugin: 'git' })).data).toBeNull();
    expect(toolKindCall(finished({ data: '{', failure: null, kind: 'plugin', plugin: 'git' })).data).toBeNull();
    expect(toolKindCall(call({ arguments: '{"branch":', argumentsCut: true })).arguments).toEqual({});
  });
});

describe('pluginFailureMessage', () => {
  it('words a code through the call’s kind, and names a code that no kind words', () => {
    const failed = finished({ data: null, failure: 'push_rejected', kind: 'plugin', plugin: 'git' });

    expect(pluginFailure(failed)).toBe('push_rejected');
    expect(pluginFailure(finished({ failure: { code: 'no_workspace' }, kind: 'failed' }))).toBeNull();
    expect(pluginFailureMessage(KIND, 'merge_conflicts')).toBe('The merge stopped at conflicts.');
    expect(pluginFailureMessage(KIND, 'push_rejected')).toContain('push_rejected');
    expect(pluginFailureMessage(undefined, 'push_rejected')).toContain('push_rejected');
  });
});
