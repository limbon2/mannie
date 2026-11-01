// A terminal's view as it hands the owner's keys to the core (plugins/
// terminal/docs/README.md): one write at a time, so keys never overtake
// each other on their way to the shell, what arrives meanwhile in the next
// write, long input in pieces within the core's limit, and nothing more
// after a write failed. xterm.js is replaced by a fake that hands the view
// what the owner types.
import type * as Ipc from '@mannie/plugin-sdk/ipc';
import type { TerminalViewOptions } from '@mannie/plugin-sdk/terminal';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { TerminalSettings } from '../../../ui/generated/bindings.js';
import { TerminalViewState } from '../../../ui/state/terminal-view.svelte.js';

const core = vi.hoisted(() => ({
  followTerminal: vi.fn(),
  unsubscribe: vi.fn(),
  writeTerminal: vi.fn(),
}));

/** The options that the view gave xterm.js, through which a test types. */
const view = vi.hoisted(() => ({ options: undefined as TerminalViewOptions | undefined }));

vi.mock('../../../ui/generated/bindings.js', () => ({
  channels: { followTerminal: core.followTerminal },
  commands: { writeTerminal: core.writeTerminal },
}));
vi.mock('@mannie/plugin-sdk/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  batchChannel: () => ({}),
  unsubscribe: core.unsubscribe,
}));
vi.mock('@mannie/plugin-sdk/terminal', () => ({
  createTerminalView: (_element: HTMLElement, options: TerminalViewOptions) => {
    view.options = options;
    return Promise.resolve({ dispose: vi.fn(), focus: vi.fn(), write: vi.fn() });
  },
}));

const SETTINGS: TerminalSettings = {
  cursorBlink: false,
  defaultShell: null,
  fontSize: 13,
  renderer: 'compatible',
  scrollback: 2000,
  screenReader: false,
};

/** A write that the test lets finish when it wants. */
interface HeldWrite {
  readonly data: string;
  readonly finish: (ok: boolean) => void;
}

let held: HeldWrite[];

beforeEach(() => {
  held = [];
  core.writeTerminal.mockReset();
  core.writeTerminal.mockImplementation(
    (input: { data: string }) =>
      new Promise((resolve) => {
        held.push({
          data: input.data,
          finish: (ok) => {
            resolve(
              ok
                ? { data: null, status: 'ok' }
                : { error: { code: 'terminal.terminal_ended', kind: 'plugin' }, status: 'error' },
            );
          },
        });
      }),
  );
  core.followTerminal.mockResolvedValue({ data: 'subscription', status: 'ok' });
});

/** A view that shows terminal 1 of a conversation, attached to a fake xterm.js. */
async function attached(): Promise<{ state: TerminalViewState; type: (data: string) => void }> {
  const state = new TerminalViewState({ conversationId: 'conversation', terminal: 1 });
  await state.attach({} as HTMLElement, SETTINGS);
  const options = view.options;
  if (options === undefined) {
    throw new Error('the view did not open xterm.js');
  }
  return { state, type: (data) => options.onData(data) };
}

/** Lets pending promises run. */
async function settle(): Promise<void> {
  await new Promise((resolve) => {
    setTimeout(resolve, 0);
  });
}

describe('TerminalViewState', () => {
  it('writes one piece at a time, in order, with what was typed meanwhile in the next write', async () => {
    const { type } = await attached();

    type('e');
    type('c');
    type('h');
    type('o');
    await settle();
    const first = held.map((write) => write.data);
    held[0]?.finish(true);
    await settle();
    const second = held.map((write) => write.data);
    held[1]?.finish(true);
    await settle();

    expect(first).toEqual(['e']);
    expect(second).toEqual(['e', 'cho']);
    expect(held).toHaveLength(2);
  });

  it('cuts long input into pieces within the limit, without splitting a character of two code units', async () => {
    const { type } = await attached();
    const emoji = '\u{1F642}';

    type(`${'x'.repeat(16 * 1024 - 1)}${emoji}tail`);
    await settle();
    held[0]?.finish(true);
    await settle();

    expect(held[0]?.data).toBe('x'.repeat(16 * 1024 - 1));
    expect(held[1]?.data).toBe(`${emoji}tail`);
  });

  it('drops what waited after a write failed, and says why', async () => {
    const { state, type } = await attached();

    type('a');
    type('b');
    await settle();
    held[0]?.finish(false);
    await settle();

    expect(held.map((write) => write.data)).toEqual(['a']);
    expect(state.failure).not.toBeNull();
  });
});
