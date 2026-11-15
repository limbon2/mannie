import { describe, expect, it } from 'vitest';

import type { ToolCallView, ToolResult } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { changedFileName, savedFileChange } from '../../../../../../apps/desktop/src/lib/utils/tools-file-changes.js';

function call(): ToolCallView & { result: ToolResult } {
  return {
    accesses: [],
    activity: 'Updating the parser',
    arguments: '{}',
    argumentsCut: false,
    decision: null,
    endedAt: null,
    interruption: null,
    notRun: null,
    plugin: null,
    request: null,
    startedAt: null,
    status: 'completed',
    tool: 'edit_file',
    result: {
      archives: [],
      images: [],
      output: 'Edited main.rs.',
      outcome: {
        kind: 'edited',
        path: '/work/main.rs',
        replacements: 2,
        change: { added: 1, removed: 1, hunks: [], truncated: true },
      },
    },
  };
}

describe('savedFileChange', () => {
  it('uses the outcome, including when the arguments are unavailable', () => {
    const current = call();
    current.arguments = '{';
    current.argumentsCut = true;
    expect(savedFileChange(current)).toMatchObject({ path: '/work/main.rs', change: { added: 1, removed: 1 } });
    current.tool = 'write_file';
    current.result.outcome = {
      kind: 'written',
      path: '/work/new.txt',
      bytes: 0,
      created: true,
      change: { added: 0, removed: 0, hunks: [], truncated: false },
    };
    expect(savedFileChange(current)?.change.added).toBe(0);
  });

  it('preserves the generic presentation for old, unfinished, failed and plugin calls', () => {
    expect(savedFileChange(null)).toBeNull();
    for (const status of ['executing', 'denied', 'canceled', 'uncertain'] as const) {
      expect(savedFileChange({ ...call(), status })).toBeNull();
    }
    expect(savedFileChange({ ...call(), plugin: 'example' })).toBeNull();
    expect(savedFileChange({ ...call(), tool: 'run_command' })).toBeNull();
    const historical = call();
    historical.result.outcome = { kind: 'edited', path: '/work/main.rs', replacements: 1 };
    expect(savedFileChange(historical)).toBeNull();
    historical.result.outcome = { kind: 'failed', failure: { code: 'revision_mismatch', current: 'sha256:old' } };
    expect(savedFileChange(historical)).toBeNull();
  });
});

describe('changedFileName', () => {
  it('handles paths from each host without interpreting content as markup', () => {
    expect(changedFileName('C:\\work\\main.rs')).toBe('main.rs');
    expect(changedFileName('/work/main.rs')).toBe('main.rs');
    expect(changedFileName('notes.txt')).toBe('notes.txt');
    expect(changedFileName('/work/<img>.txt')).toBe('<img>.txt');
  });
});
