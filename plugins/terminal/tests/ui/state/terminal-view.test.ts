// A terminal's view (plugins/terminal/docs/README.md): it opens xterm.js
// once its tab shows and follows the terminal only while its tab and the
// window show, and a view that follows again marks a command only once its
// snapshot came. It hands the owner's keys to the core one write at a time,
// so keys never overtake each other on their way to the shell, with what
// arrives meanwhile in the next write, long input in pieces within the
// core's limit, and nothing more after a write failed. xterm.js is replaced
// by a fake that hands the view what the owner types.
import type * as Ipc from '@mannie/plugin-sdk/ipc';
import type { TerminalViewOptions } from '@mannie/plugin-sdk/terminal';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { BlockView, TerminalSettings, TerminalUpdate } from '../../../ui/generated/bindings.js';
import { TerminalViewState } from '../../../ui/state/terminal-view.svelte.js';

const core = vi.hoisted(() => ({
  acknowledgeOutput: vi.fn(),
  followTerminal: vi.fn(),
  unsubscribe: vi.fn(),
  writeTerminal: vi.fn(),
}));

/** The latest channel's handler of batches, through which a test delivers updates. */
const channel = vi.hoisted(() => ({ deliver: undefined as ((items: unknown[]) => void) | undefined }));

/** How often the view opened xterm.js, the options that it gave, through which a test types, and the lines that it marked. */
const view = vi.hoisted(() => ({
  markLines: vi.fn(),
  opened: 0,
  options: undefined as TerminalViewOptions | undefined,
}));

vi.mock('../../../ui/generated/bindings.js', () => ({
  channels: { followTerminal: core.followTerminal },
  commands: { acknowledgeOutput: core.acknowledgeOutput, writeTerminal: core.writeTerminal },
}));
vi.mock('@mannie/plugin-sdk/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  batchChannel: (deliver: (items: unknown[]) => void) => {
    channel.deliver = deliver;
    return {};
  },
  unsubscribe: core.unsubscribe,
}));
vi.mock('@mannie/plugin-sdk/terminal', () => ({
  createTerminalView: (_element: HTMLElement, options: TerminalViewOptions) => {
    view.opened += 1;
    view.options = options;
    return Promise.resolve({
      buffer: () => ({ baseY: 0, length: 0, rows: 24 }),
      dispose: vi.fn(),
      focus: vi.fn(),
      markLines: view.markLines,
      reset: vi.fn(),
      write: (_data: string, written: () => void) => {
        written();
      },
    });
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

const ADDRESS = { conversationId: 'conversation', terminal: 1 };

beforeEach(() => {
  held = [];
  channel.deliver = undefined;
  view.markLines.mockReset();
  view.opened = 0;
  view.options = undefined;
  core.acknowledgeOutput.mockReset();
  core.acknowledgeOutput.mockResolvedValue({ data: null, status: 'ok' });
  core.followTerminal.mockReset();
  core.unsubscribe.mockReset();
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

/** Lets pending promises run. */
async function settle(): Promise<void> {
  await new Promise((resolve) => {
    setTimeout(resolve, 0);
  });
}

/** Hands the view `updates` through its latest channel. */
function deliver(...updates: TerminalUpdate[]): void {
  channel.deliver?.(updates.map((item) => ({ item, kind: 'item' })));
}

/** A snapshot of the terminal. */
function snapshot(): TerminalUpdate {
  return { data: 'PS> ', firstRow: 0, kind: 'snapshot', seq: 4, topRow: 0 };
}

/** A command that ended on the rows from 1 to 2. */
function block(number: number): BlockView {
  return {
    author: 'owner',
    command: 'echo one',
    durationMs: 5,
    endRow: 3,
    exitCode: 0,
    firstRow: 1,
    key: `block-${String(number)}`,
    number,
    running: false,
    startedAt: 0,
  };
}

/** A view of terminal 1 of a conversation whose tab shows, attached to a fake xterm.js. */
async function attached(): Promise<{ state: TerminalViewState; type: (data: string) => void }> {
  const state = new TerminalViewState(ADDRESS, SETTINGS);
  state.attach({} as HTMLElement);
  state.setTabShown(true);
  await settle();
  const options = view.options;
  if (options === undefined) {
    throw new Error('the view did not open xterm.js');
  }
  return { state, type: (data) => options.onData(data) };
}

describe('TerminalViewState', () => {
  it('opens xterm.js only once its tab shows, and then follows the terminal', async () => {
    const state = new TerminalViewState(ADDRESS, SETTINGS);
    state.attach({} as HTMLElement);
    await settle();
    const openedWhileHidden = view.opened;
    const followedWhileHidden = core.followTerminal.mock.calls.length;

    state.setTabShown(true);
    await settle();

    expect(openedWhileHidden).toBe(0);
    expect(followedWhileHidden).toBe(0);
    expect(view.opened).toBe(1);
    expect(core.followTerminal).toHaveBeenCalledTimes(1);
  });

  it('stops following while its tab hides, and follows again from the start when it shows', async () => {
    const { state } = await attached();

    state.setTabShown(false);
    await settle();
    const unsubscribed = core.unsubscribe.mock.calls.length;
    state.setTabShown(true);
    await settle();

    expect(unsubscribed).toBe(1);
    expect(core.followTerminal).toHaveBeenCalledTimes(2);
    expect(view.opened).toBe(1);
  });

  it('follows only while the window is visible too', async () => {
    const { state } = await attached();

    state.setWindowShown(false);
    await settle();
    const unsubscribed = core.unsubscribe.mock.calls.length;
    state.setTabShown(false);
    state.setWindowShown(true);
    await settle();
    const followedWithTabHidden = core.followTerminal.mock.calls.length;
    state.setTabShown(true);
    await settle();

    expect(unsubscribed).toBe(1);
    expect(followedWithTabHidden).toBe(1);
    expect(core.followTerminal).toHaveBeenCalledTimes(2);
  });

  it('follows once while it hears again that it shows', async () => {
    const { state } = await attached();

    state.setTabShown(true);
    state.setWindowShown(true);
    await settle();

    expect(core.followTerminal).toHaveBeenCalledTimes(1);
    expect(core.unsubscribe).not.toHaveBeenCalled();
  });

  it('marks a command only once the snapshot of a view that follows again came', async () => {
    const { state } = await attached();
    deliver(snapshot(), { blocks: [block(1)], kind: 'blocks' });
    state.setTabShown(false);
    state.setTabShown(true);
    await settle();

    state.reveal({ block: 1, lines: null });
    const markedBeforeSnapshot = view.markLines.mock.calls.length;
    deliver(snapshot());

    expect(markedBeforeSnapshot).toBe(0);
    expect(view.markLines).toHaveBeenCalledTimes(1);
  });

  it('opens nothing after it was disposed while its tab was hidden', async () => {
    const state = new TerminalViewState(ADDRESS, SETTINGS);
    state.attach({} as HTMLElement);
    state.dispose();

    state.setTabShown(true);
    await settle();

    expect(view.opened).toBe(0);
    expect(core.followTerminal).not.toHaveBeenCalled();
  });

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
