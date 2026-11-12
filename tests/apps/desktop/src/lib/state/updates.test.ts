// Updates as the window follows them (docs/features/updates.md; ADR-0063):
// the status follows the core's channel, a ready version stays through a
// check, each notice closes for the owner, one action waits for the core at a
// time, a restart that would end running work asks first, and a failed action
// says why until the next one starts.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  UpdateOutcome,
  UpdateState,
  UpdateStatus,
  UpdateStatusUpdate,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { Updates } from '../../../../../../apps/desktop/src/lib/state/updates.svelte.js';
import type * as Calls from '../../../../../../apps/desktop/src/lib/utils/calls.js';

const core = vi.hoisted(() => ({
  checkForUpdates: vi.fn(),
  dismissUpdateOutcome: vi.fn(),
  getUpdatesSettings: vi.fn(),
  openReleasePage: vi.fn(),
  restartToUpdate: vi.fn(),
  subscribeUpdates: vi.fn(),
  unsubscribe: vi.fn(),
  updateUpdatesSettings: vi.fn(),
}));

/** Hands batches of updates to the state and reports lost batches, as the core's channel does. */
const channel = vi.hoisted(() => ({
  deliver: (_updates: readonly UpdateStatusUpdate[]): void => undefined,
  lose: (): void => undefined,
}));

vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: core }));
vi.mock('../../../../../../apps/desktop/src/lib/utils/calls.js', async (importOriginal) => ({
  ...(await importOriginal<typeof Calls>()),
  batchChannel: (onItems: (updates: readonly UpdateStatusUpdate[]) => void, onGap: () => void) => {
    channel.deliver = onItems;
    channel.lose = onGap;
    return {};
  },
}));

const OFF: UpdateState = { kind: 'off', reason: 'development_build' };
const IDLE: UpdateState = { checkedAt: 1_000, kind: 'idle' };
const CHECKING: UpdateState = { kind: 'checking' };
const DOWNLOADING: UpdateState = { kind: 'downloading', receivedBytes: 10, totalBytes: 40, version: '0.8.0' };
const FAILED: UpdateState = { checkedAt: 2_000, code: 'unreachable', kind: 'failed' };

/** The answer of the core to a restart that would end running work. */
const CONFIRM_FIRST = {
  data: { activeRuns: 1, backgroundCommands: 0, kind: 'confirm_first', terminals: 2 },
  status: 'ok',
};

function ready(version: string): UpdateState {
  return { kind: 'ready', version };
}

function status(state: UpdateState, changes: Partial<UpdateStatus> = {}): UpdateStatus {
  return { currentVersion: '0.7.0', lastOutcome: null, needsSystemPassword: false, state, ...changes };
}

/** Delivers the status as the core reports it. */
function deliver(next: UpdateStatus): void {
  channel.deliver([{ status: next, type: 'status' }]);
}

/** Starts following the status, once the core accepted the subscription. */
async function started(): Promise<Updates> {
  const updates = new Updates();
  updates.start();
  await vi.waitFor(() => {
    expect(core.subscribeUpdates).toHaveBeenCalled();
  });
  return updates;
}

beforeEach(() => {
  for (const command of Object.values(core)) {
    command.mockReset();
  }
  core.subscribeUpdates.mockResolvedValue({ data: 7, status: 'ok' });
  core.unsubscribe.mockResolvedValue({ data: null, status: 'ok' });
  core.dismissUpdateOutcome.mockResolvedValue(undefined);
  for (const command of [core.checkForUpdates, core.openReleasePage]) {
    command.mockResolvedValue({ data: null, status: 'ok' });
  }
});

describe('the status of updates', () => {
  it('shows once the core reports it', async () => {
    const updates = await started();
    const phaseBefore = updates.phase;

    deliver(status(IDLE));

    expect(phaseBefore).toBe('loading');
    expect(updates.phase).toBe('ready');
    expect(updates.status).toEqual(status(IDLE));
    expect(updates.failure).toBeNull();
  });

  it('is followed once, however often the window starts it', async () => {
    const updates = await started();

    updates.start();
    updates.start();

    expect(core.subscribeUpdates).toHaveBeenCalledTimes(1);
  });

  it('says why it could not be followed, and is followed again on a retry', async () => {
    core.subscribeUpdates.mockResolvedValueOnce({ error: { code: 'store_unavailable' }, status: 'error' });
    const updates = await started();
    await vi.waitFor(() => {
      expect(updates.phase).toBe('error');
    });
    const failure = updates.failure;

    updates.retry();
    await vi.waitFor(() => {
      expect(core.subscribeUpdates).toHaveBeenCalledTimes(2);
    });
    deliver(status(IDLE));

    expect(failure).toEqual({ error: { code: 'store_unavailable' }, kind: 'core' });
    expect(updates.phase).toBe('ready');
    expect(updates.failure).toBeNull();
  });

  it('counts a core that does not answer as a failure', async () => {
    core.subscribeUpdates.mockRejectedValueOnce(new Error('the window lost its core'));
    const updates = await started();

    await vi.waitFor(() => {
      expect(updates.phase).toBe('error');
    });

    expect(updates.failure).toEqual({ kind: 'ipc' });
  });

  it('is followed again when its channel falls behind, and lets the old subscription go', async () => {
    const updates = await started();
    deliver(status(IDLE));

    channel.deliver([{ reason: 'lagged', type: 'ended' }]);

    await vi.waitFor(() => {
      expect(core.subscribeUpdates).toHaveBeenCalledTimes(2);
    });
    expect(core.unsubscribe).toHaveBeenCalledWith({ subscriptionId: 7 });
    expect(updates.phase).toBe('ready');
    expect(updates.status).toEqual(status(IDLE));
  });

  it('is followed again when batches of its channel were lost', async () => {
    const updates = await started();
    deliver(status(IDLE));

    channel.lose();

    await vi.waitFor(() => {
      expect(core.subscribeUpdates).toHaveBeenCalledTimes(2);
    });
    expect(updates.status).toEqual(status(IDLE));
  });

  it('is not followed again when Mannie is quitting', async () => {
    const updates = await started();
    deliver(status(IDLE));

    channel.deliver([{ reason: 'shutdown', type: 'ended' }]);

    expect(core.subscribeUpdates).toHaveBeenCalledTimes(1);
    expect(updates.status).toEqual(status(IDLE));
  });

  it('ignores what a subscription reports after another replaced it', async () => {
    const updates = await started();
    const replaced = channel.deliver;
    updates.retry();
    await vi.waitFor(() => {
      expect(core.subscribeUpdates).toHaveBeenCalledTimes(2);
    });

    replaced([{ status: status(ready('0.8.0')), type: 'status' }]);

    expect(updates.status).toBeNull();
    expect(updates.readyVersion).toBeNull();
  });
});

describe('a ready version', () => {
  it('stays through a check, and goes with a state that has no installer', async () => {
    const updates = await started();

    deliver(status(ready('0.8.0')));
    deliver(status(CHECKING));
    const duringCheck = updates.readyVersion;
    deliver(status(IDLE));

    expect(duringCheck).toBe('0.8.0');
    expect(updates.readyVersion).toBeNull();
  });

  it('shows its notice until the owner closes it, and again for another version', async () => {
    const updates = await started();

    deliver(status(ready('0.8.0')));
    const shown = updates.noticeVersion;
    updates.closeReadyNotice();
    const afterClosing = updates.noticeVersion;
    deliver(status(CHECKING));
    deliver(status(ready('0.8.0')));
    const afterCheck = updates.noticeVersion;
    deliver(status(ready('0.9.0')));

    expect(shown).toBe('0.8.0');
    expect(afterClosing).toBeNull();
    expect(afterCheck).toBeNull();
    expect(updates.noticeVersion).toBe('0.9.0');
  });

  it('can still be installed after the owner closed its notice', async () => {
    const updates = await started();
    deliver(status(ready('0.8.0')));

    updates.closeReadyNotice();

    expect(updates.readyVersion).toBe('0.8.0');
    expect(updates.canRestart).toBe(true);
  });
});

describe('the result of the last install', () => {
  const outcome: UpdateOutcome = { fromVersion: '0.7.0', result: 'completed', toVersion: '0.8.0' };

  it('shows until the owner closes it, which the core hears of', async () => {
    const updates = await started();
    deliver(status(IDLE, { lastOutcome: outcome }));
    const shown = updates.outcome;

    await updates.closeOutcome();

    expect(shown).toEqual(outcome);
    expect(updates.outcome).toBeNull();
    expect(core.dismissUpdateOutcome).toHaveBeenCalledTimes(1);
  });

  it('stays closed in this window when the core does not hear of it', async () => {
    core.dismissUpdateOutcome.mockRejectedValueOnce(new Error('the window lost its core'));
    const updates = await started();
    deliver(status(IDLE, { lastOutcome: outcome }));

    await updates.closeOutcome();
    deliver(status(IDLE, { lastOutcome: outcome }));

    expect(updates.outcome).toBeNull();
  });

  it('is absent when Mannie installed nothing', async () => {
    const updates = await started();

    deliver(status(IDLE));

    expect(updates.outcome).toBeNull();
  });
});

describe('a check for updates', () => {
  it('starts in the core, which reports what it finds with the status', async () => {
    const updates = await started();
    deliver(status(IDLE));

    await updates.check();

    expect(core.checkForUpdates).toHaveBeenCalledTimes(1);
    expect(updates.actionFailure).toBeNull();
    expect(updates.pending).toBeNull();
  });

  it('says why it did not start', async () => {
    core.checkForUpdates.mockResolvedValueOnce({ error: { code: 'store_unavailable' }, status: 'error' });
    const updates = await started();
    deliver(status(IDLE));

    await updates.check();

    expect(updates.actionFailure).toEqual({
      action: 'check',
      failure: { error: { code: 'store_unavailable' }, kind: 'core' },
    });
    expect(updates.pending).toBeNull();
  });

  it('forgets why the one before did not start', async () => {
    core.checkForUpdates.mockRejectedValueOnce(new Error('the window lost its core'));
    const updates = await started();
    deliver(status(IDLE));

    await updates.check();
    const failed = updates.actionFailure;
    await updates.check();

    expect(failed).toEqual({ action: 'check', failure: { kind: 'ipc' } });
    expect(updates.actionFailure).toBeNull();
  });

  it('can start only in a state where no check or download runs', async () => {
    const updates = await started();
    const beforeStatus = updates.canCheck;
    const allowed: boolean[] = [];

    for (const state of [OFF, IDLE, CHECKING, DOWNLOADING, ready('0.8.0'), FAILED]) {
      deliver(status(state));
      allowed.push(updates.canCheck);
    }

    expect(beforeStatus).toBe(false);
    expect(allowed).toEqual([false, true, false, false, true, true]);
  });

  it('keeps the actions off while it waits for the core', async () => {
    let answer: (result: { data: null; status: 'ok' }) => void = () => undefined;
    core.checkForUpdates.mockReturnValueOnce(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );
    const updates = await started();
    deliver(status(ready('0.8.0')));

    const checking = updates.check();
    const pending = updates.pending;
    const canCheck = updates.canCheck;
    const canRestart = updates.canRestart;
    await updates.check();
    await updates.restart();
    answer({ data: null, status: 'ok' });
    await checking;

    expect(pending).toBe('check');
    expect(canCheck).toBe(false);
    expect(canRestart).toBe(false);
    expect(core.checkForUpdates).toHaveBeenCalledTimes(1);
    expect(core.restartToUpdate).not.toHaveBeenCalled();
    expect(updates.canCheck).toBe(true);
    expect(updates.canRestart).toBe(true);
  });
});

describe('a restart to update', () => {
  it('starts at once when it ends no running work', async () => {
    core.restartToUpdate.mockResolvedValueOnce({ data: { kind: 'restarting' }, status: 'ok' });
    const updates = await started();
    deliver(status(ready('0.8.0')));

    await updates.restart();

    expect(core.restartToUpdate).toHaveBeenCalledExactlyOnceWith({ confirmed: false, version: '0.8.0' });
    expect(updates.question).toBeNull();
    expect(updates.restarting).toBe(true);
  });

  it('asks first when it would end running work, and starts once the owner confirms', async () => {
    core.restartToUpdate
      .mockResolvedValueOnce(CONFIRM_FIRST)
      .mockResolvedValueOnce({ data: { kind: 'restarting' }, status: 'ok' });
    const updates = await started();
    deliver(status(ready('0.8.0')));

    await updates.restart();
    const question = updates.question;
    const restartingBeforeAnswer = updates.restarting;
    await updates.confirmRestart();

    expect(question).toEqual({ activeRuns: 1, backgroundCommands: 0, terminals: 2, version: '0.8.0' });
    expect(restartingBeforeAnswer).toBe(false);
    expect(core.restartToUpdate).toHaveBeenNthCalledWith(1, { confirmed: false, version: '0.8.0' });
    expect(core.restartToUpdate).toHaveBeenNthCalledWith(2, { confirmed: true, version: '0.8.0' });
    expect(updates.question).toBeNull();
    expect(updates.restarting).toBe(true);
  });

  it('keeps every action off once Mannie is quitting', async () => {
    core.restartToUpdate.mockResolvedValueOnce({ data: { kind: 'restarting' }, status: 'ok' });
    const updates = await started();
    deliver(status(ready('0.8.0')));

    await updates.restart();
    await updates.restart();
    await updates.check();

    expect(updates.canCheck).toBe(false);
    expect(updates.canRestart).toBe(false);
    expect(core.restartToUpdate).toHaveBeenCalledTimes(1);
    expect(core.checkForUpdates).not.toHaveBeenCalled();
  });

  it('changes nothing when the owner cancels the question', async () => {
    core.restartToUpdate.mockResolvedValueOnce(CONFIRM_FIRST);
    const updates = await started();
    deliver(status(ready('0.8.0')));
    await updates.restart();

    updates.cancelRestart();
    await updates.confirmRestart();

    expect(updates.question).toBeNull();
    expect(updates.restarting).toBe(false);
    expect(core.restartToUpdate).toHaveBeenCalledTimes(1);
    expect(updates.canRestart).toBe(true);
  });

  it('says why it failed, in the notice that the owner closed', async () => {
    core.restartToUpdate
      .mockResolvedValueOnce(CONFIRM_FIRST)
      .mockResolvedValueOnce({ error: { code: 'version_not_ready' }, status: 'error' });
    const updates = await started();
    deliver(status(ready('0.8.0')));
    updates.closeReadyNotice();

    await updates.restart();
    await updates.confirmRestart();

    expect(updates.question).toBeNull();
    expect(updates.actionFailure).toEqual({
      action: 'restart',
      failure: { error: { code: 'version_not_ready' }, kind: 'core' },
    });
    expect(updates.noticeVersion).toBe('0.8.0');
    expect(updates.restarting).toBe(false);
  });

  it('keeps its question through a check that finds the same version', async () => {
    core.restartToUpdate.mockResolvedValueOnce(CONFIRM_FIRST);
    const updates = await started();
    deliver(status(ready('0.8.0')));
    await updates.restart();

    deliver(status(CHECKING));
    deliver(status(ready('0.8.0')));

    expect(updates.question).toEqual({ activeRuns: 1, backgroundCommands: 0, terminals: 2, version: '0.8.0' });
  });

  it('drops its question when another version becomes ready', async () => {
    core.restartToUpdate.mockResolvedValueOnce(CONFIRM_FIRST);
    const updates = await started();
    deliver(status(ready('0.8.0')));
    await updates.restart();

    deliver(status(ready('0.9.0')));

    expect(updates.question).toBeNull();
  });

  it('drops why it failed when its version is no longer ready', async () => {
    core.restartToUpdate.mockResolvedValueOnce({ error: { code: 'storage_failed' }, status: 'error' });
    const updates = await started();
    deliver(status(ready('0.8.0')));
    await updates.restart();
    const failed = updates.actionFailure;

    deliver(status(IDLE));

    expect(failed).toEqual({ action: 'restart', failure: { error: { code: 'storage_failed' }, kind: 'core' } });
    expect(updates.actionFailure).toBeNull();
  });

  it('does not start without a ready version or without a question to confirm', async () => {
    const updates = await started();
    deliver(status(IDLE));

    await updates.restart();
    await updates.confirmRestart();

    expect(core.restartToUpdate).not.toHaveBeenCalled();
    expect(updates.canRestart).toBe(false);
  });
});

describe('the setting for automatic checks', () => {
  it('is unknown until the core reports it', () => {
    const updates = new Updates();

    expect(updates.checkAutomatically).toBeNull();
  });

  it('is on when the stored section does not name it', async () => {
    core.getUpdatesSettings.mockResolvedValueOnce({ data: {}, status: 'ok' });
    const updates = new Updates();

    await updates.loadSettings();

    expect(updates.checkAutomatically).toBe(true);
    expect(updates.settingsFailure).toBeNull();
  });

  it('is off when the owner turned it off', async () => {
    core.getUpdatesSettings.mockResolvedValueOnce({ data: { checkAutomatically: false }, status: 'ok' });
    const updates = new Updates();

    await updates.loadSettings();

    expect(updates.checkAutomatically).toBe(false);
  });

  it('says why it could not be read, until a read succeeds', async () => {
    core.getUpdatesSettings
      .mockResolvedValueOnce({ error: { code: 'storage_failed' }, status: 'error' })
      .mockResolvedValueOnce({ data: { checkAutomatically: true }, status: 'ok' });
    const updates = new Updates();

    await updates.loadSettings();
    const failure = updates.settingsFailure;
    const valueAfterFailure = updates.checkAutomatically;
    await updates.loadSettings();

    expect(failure).toEqual({ error: { code: 'storage_failed' }, kind: 'core' });
    expect(valueAfterFailure).toBeNull();
    expect(updates.settingsFailure).toBeNull();
    expect(updates.checkAutomatically).toBe(true);
  });

  it('changes to what the core stored', async () => {
    core.updateUpdatesSettings.mockResolvedValueOnce({ data: { checkAutomatically: false }, status: 'ok' });
    const updates = new Updates();

    await updates.setCheckAutomatically(false);

    expect(core.updateUpdatesSettings).toHaveBeenCalledExactlyOnceWith({ checkAutomatically: false });
    expect(updates.checkAutomatically).toBe(false);
    expect(updates.saving).toBe(false);
    expect(updates.saveFailure).toBeNull();
  });

  it('stays as it was when a change is not saved, and says why until a change is saved', async () => {
    core.getUpdatesSettings.mockResolvedValueOnce({ data: { checkAutomatically: true }, status: 'ok' });
    core.updateUpdatesSettings
      .mockResolvedValueOnce({ error: { code: 'written_by_newer_version' }, status: 'error' })
      .mockResolvedValueOnce({ data: { checkAutomatically: false }, status: 'ok' });
    const updates = new Updates();
    await updates.loadSettings();

    await updates.setCheckAutomatically(false);
    const failure = updates.saveFailure;
    const valueAfterFailure = updates.checkAutomatically;
    await updates.setCheckAutomatically(false);

    expect(failure).toEqual({ error: { code: 'written_by_newer_version' }, kind: 'core' });
    expect(valueAfterFailure).toBe(true);
    expect(updates.saveFailure).toBeNull();
    expect(updates.checkAutomatically).toBe(false);
  });

  it('saves one change at a time', async () => {
    let answer: (result: { data: { checkAutomatically: boolean }; status: 'ok' }) => void = () => undefined;
    core.updateUpdatesSettings.mockReturnValueOnce(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );
    const updates = new Updates();

    const saving = updates.setCheckAutomatically(false);
    const savingWhileWaiting = updates.saving;
    await updates.setCheckAutomatically(true);
    answer({ data: { checkAutomatically: false }, status: 'ok' });
    await saving;

    expect(savingWhileWaiting).toBe(true);
    expect(core.updateUpdatesSettings).toHaveBeenCalledExactlyOnceWith({ checkAutomatically: false });
    expect(updates.saving).toBe(false);
    expect(updates.checkAutomatically).toBe(false);
  });
});

describe('a page of the releases', () => {
  it('opens through the core, which builds its address', async () => {
    const updates = new Updates();

    await updates.openReleasePage('latest');

    expect(core.openReleasePage).toHaveBeenCalledExactlyOnceWith({ page: 'latest' });
    expect(updates.actionFailure).toBeNull();
  });

  it('records which page did not open, and why', async () => {
    core.openReleasePage.mockResolvedValueOnce({ error: { code: 'browser_unavailable' }, status: 'error' });
    const updates = new Updates();

    await updates.openReleasePage('running');

    expect(updates.actionFailure).toEqual({
      action: 'page',
      failure: { error: { code: 'browser_unavailable' }, kind: 'core' },
      page: 'running',
    });
  });

  it('opens while a check waits for the core', async () => {
    let answer: (result: { data: null; status: 'ok' }) => void = () => undefined;
    core.checkForUpdates.mockReturnValueOnce(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );
    const updates = await started();
    deliver(status(ready('0.8.0')));

    const checking = updates.check();
    await updates.openReleasePage('ready');
    answer({ data: null, status: 'ok' });
    await checking;

    expect(core.openReleasePage).toHaveBeenCalledExactlyOnceWith({ page: 'ready' });
  });
});
