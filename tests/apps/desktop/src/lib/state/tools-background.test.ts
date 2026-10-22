// The output that the panel of a background command holds (docs/features/
// system-tools/background-commands.md, interface): lines in order, the lines
// that the log let go where they would be, and only the last lines.
import { describe, expect, it } from 'vitest';

import {
  lastLine,
  MAX_SHOWN_LINES,
  type OutputEntry,
  withUpdate,
} from '../../../../../../apps/desktop/src/lib/state/tools-background.svelte.js';

describe('withUpdate', () => {
  it('numbers the lines of an update from its first line', () => {
    const entries = withUpdate([], { firstLine: 4, kind: 'lines', lines: ['a', 'b'] });

    expect(entries).toEqual([
      { kind: 'line', number: 4, text: 'a' },
      { kind: 'line', number: 5, text: 'b' },
    ]);
    expect(lastLine(entries)).toBe(5);
  });

  it('keeps the lines that the log let go as one entry', () => {
    const entries = withUpdate([{ kind: 'line', number: 1, text: 'first' }], {
      firstLine: 2,
      kind: 'left_out',
      lastLine: 900,
    });

    expect(entries.at(-1)).toEqual({ first: 2, kind: 'left_out', last: 900 });
    expect(lastLine(entries)).toBe(900);
  });

  it('holds only the last lines', () => {
    const lines = Array.from({ length: MAX_SHOWN_LINES + 10 }, (_line, index) => `line ${String(index + 1)}`);

    const entries: OutputEntry[] = withUpdate([], { firstLine: 1, kind: 'lines', lines });

    expect(entries).toHaveLength(MAX_SHOWN_LINES);
    expect(entries[0]).toEqual({ kind: 'line', number: 11, text: 'line 11' });
  });

  it('changes nothing for the command and its end', () => {
    const entries: OutputEntry[] = [{ kind: 'line', number: 1, text: 'a' }];
    const command = {
      activity: null,
      archiveId: null,
      bytes: 0,
      command: 'make',
      endedAt: null,
      ending: null,
      exitCode: null,
      lines: 0,
      number: 1,
      onWorker: false,
      partId: '0199e0f2-0000-7000-8000-000000000001',
      signal: null,
      source: { kind: 'local' as const, shell: '/bin/sh', workingDirectory: '/work' },
      startedAt: 0,
    };

    expect(withUpdate(entries, { command, kind: 'command' })).toEqual(entries);
    expect(withUpdate(entries, { command, kind: 'ended' })).toEqual(entries);
    expect(lastLine([])).toBeNull();
  });
});
