// The core's questions about closing and quitting (docs/features/
// window-and-tray.md, closing and quitting): a question opens its dialog, an
// answer closes it or shows why it failed, and a cleared question closes.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { WindowRequestUpdate } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { WindowRequests } from '../../../../../../apps/desktop/src/lib/state/window-requests.svelte.js';
import { windowSettings } from '../../../../../../apps/desktop/src/lib/state/window-settings.svelte.js';
import type * as Calls from '../../../../../../apps/desktop/src/lib/utils/calls.js';

interface Subscriber {
  readonly onGap: () => void;
  readonly onItems: (items: readonly WindowRequestUpdate[]) => void;
}

const core = vi.hoisted(() => ({
  answerCloseQuestion: vi.fn(),
  confirmQuit: vi.fn(),
  dismissWindowRequest: vi.fn(),
  subscribeWindowRequests: vi.fn(),
  subscribers: [] as Subscriber[],
}));

vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({
  commands: {
    answerCloseQuestion: core.answerCloseQuestion,
    confirmQuit: core.confirmQuit,
    dismissWindowRequest: core.dismissWindowRequest,
    subscribeWindowRequests: core.subscribeWindowRequests,
  },
}));
vi.mock('../../../../../../apps/desktop/src/lib/utils/calls.js', async (importOriginal) => {
  const original = await importOriginal<typeof Calls>();
  return {
    ...original,
    // The channel reaches the core through Tauri; here the test delivers its batches.
    batchChannel: (onItems: Subscriber['onItems'], onGap: Subscriber['onGap']) => {
      core.subscribers.push({ onGap, onItems });
      return {};
    },
  };
});

const OK = { data: null, status: 'ok' };

function started(): WindowRequests {
  const requests = new WindowRequests();
  requests.start();
  return requests;
}

function deliver(...updates: WindowRequestUpdate[]): void {
  core.subscribers.at(-1)?.onItems(updates);
}

beforeEach(() => {
  core.subscribers.length = 0;
  core.answerCloseQuestion.mockReset().mockResolvedValue(OK);
  core.confirmQuit.mockReset().mockResolvedValue(OK);
  core.dismissWindowRequest.mockReset().mockResolvedValue(OK);
  core.subscribeWindowRequests.mockReset().mockResolvedValue(undefined);
  windowSettings.closeAction = 'ask';
});

describe('WindowRequests', () => {
  it('subscribes once and opens the dialog of a question', () => {
    const requests = started();
    requests.start();

    deliver({
      request: { activeRuns: 2, backgroundCommands: 0, kind: 'ask_close_action' },
      requestId: 4,
      type: 'pending',
    });

    expect(core.subscribeWindowRequests).toHaveBeenCalledTimes(1);
    expect(requests.open).toBe(true);
    expect(requests.question).toEqual({
      request: { activeRuns: 2, backgroundCommands: 0, kind: 'ask_close_action' },
      requestId: 4,
    });
  });

  it('saves the first-close choice, closes the dialog and shows the choice in the settings', async () => {
    const requests = started();
    deliver({
      request: { activeRuns: 0, backgroundCommands: 0, kind: 'ask_close_action' },
      requestId: 1,
      type: 'pending',
    });

    await requests.answerClose('tray');

    expect(core.answerCloseQuestion).toHaveBeenCalledWith({ choice: 'tray', requestId: 1 });
    expect(requests.open).toBe(false);
    expect(windowSettings.closeAction).toBe('tray');
  });

  it('keeps the dialog open with the reason when the core refuses an answer', async () => {
    core.answerCloseQuestion.mockResolvedValue({ error: { code: 'storage_failed' }, status: 'error' });
    const requests = started();
    deliver({
      request: { activeRuns: 0, backgroundCommands: 0, kind: 'ask_close_action' },
      requestId: 1,
      type: 'pending',
    });

    await requests.answerClose('quit');

    expect(requests.open).toBe(true);
    expect(requests.failure).toEqual({ error: { code: 'storage_failed' }, kind: 'core' });
    expect(requests.answering).toBe(false);
    expect(windowSettings.closeAction).toBe('ask');
  });

  it('confirms quitting for the question that waits', async () => {
    const requests = started();
    deliver({
      request: { activeRuns: 3, backgroundCommands: 0, kind: 'confirm_quit', source: 'tray' },
      requestId: 9,
      type: 'pending',
    });

    await requests.confirmQuit();

    expect(core.confirmQuit).toHaveBeenCalledWith({ requestId: 9 });
  });

  it('closes the dialog when the owner dismisses it or the core clears the question', async () => {
    const requests = started();
    deliver({
      request: { activeRuns: 1, backgroundCommands: 0, kind: 'confirm_quit', source: 'window' },
      requestId: 2,
      type: 'pending',
    });

    await requests.dismiss();

    expect(requests.open).toBe(false);
    expect(core.dismissWindowRequest).toHaveBeenCalledWith({ requestId: 2 });

    deliver({
      request: { activeRuns: 1, backgroundCommands: 0, kind: 'confirm_quit', source: 'window' },
      requestId: 3,
      type: 'pending',
    });
    deliver({ requestId: 2, type: 'cleared' });
    expect(requests.open).toBe(true);
    deliver({ requestId: 3, type: 'cleared' });
    expect(requests.open).toBe(false);
  });

  it('subscribes again when batches were lost, and ignores the old subscription', () => {
    const requests = started();
    core.subscribers[0]?.onGap();

    core.subscribers[0]?.onItems([
      { request: { activeRuns: 0, backgroundCommands: 0, kind: 'ask_close_action' }, requestId: 1, type: 'pending' },
    ]);
    expect(requests.open).toBe(false);

    deliver({
      request: { activeRuns: 0, backgroundCommands: 0, kind: 'ask_close_action' },
      requestId: 1,
      type: 'pending',
    });
    expect(core.subscribeWindowRequests).toHaveBeenCalledTimes(2);
    expect(requests.open).toBe(true);
  });
});
