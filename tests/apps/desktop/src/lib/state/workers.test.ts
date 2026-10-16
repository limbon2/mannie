// The list of paired workers and the owner's actions on it (docs/features/
// workers/worker-activity.md): the list follows the core's channel, pairing
// closes its dialog only once the worker is paired, a refusal stays in the
// dialog, renaming starts from the owner's name and keeps a refused one in
// the dialog, and a removal confirms before it removes.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { WorkerListUpdate, WorkerView } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { WorkersActions } from '../../../../../../apps/desktop/src/lib/state/workers-actions.svelte.js';
import { WorkerList, workerList } from '../../../../../../apps/desktop/src/lib/state/workers.svelte.js';
import type * as Calls from '../../../../../../apps/desktop/src/lib/utils/calls.js';

const core = vi.hoisted(() => ({
  pairWorker: vi.fn(),
  removeWorker: vi.fn(),
  renameWorker: vi.fn(),
  subscribeWorkers: vi.fn(),
  unsubscribe: vi.fn(),
}));
const navigation = vi.hoisted(() => ({ openSettings: vi.fn() }));

/** Hands batches of updates to the list, as the core's channel does. */
const channel = vi.hoisted(() => ({ deliver: (_updates: readonly WorkerListUpdate[]): void => undefined }));

vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: core }));
vi.mock('../../../../../../apps/desktop/src/lib/utils/calls.js', async (importOriginal) => ({
  ...(await importOriginal<typeof Calls>()),
  batchChannel: (onItems: (updates: readonly WorkerListUpdate[]) => void) => {
    channel.deliver = onItems;
    return {};
  },
}));
vi.mock('../../../../../../apps/desktop/src/lib/utils/settings-sections.js', () => navigation);

function worker(id: string, status: WorkerView['status'] = 'connected'): WorkerView {
  return {
    address: 'build-01.lan:7445',
    chosenName: null,
    configuredName: `worker ${id}`,
    connectedAt: null,
    details: { maxParallelTasks: 4, purpose: 'Builds the backend' },
    id,
    keyPin: '0123456789ab',
    name: `worker ${id}`,
    pairedAt: 1,
    status,
  };
}

beforeEach(() => {
  for (const command of Object.values(core)) {
    command.mockReset();
  }
  navigation.openSettings.mockReset();
  core.subscribeWorkers.mockResolvedValue({ data: 7, status: 'ok' });
  workerList.workers = [];
});

describe('the list of workers', () => {
  it('loads until the core sends the workers, then follows each change', async () => {
    const list = new WorkerList();
    list.start();
    await Promise.resolve();

    expect(list.status).toBe('loading');
    channel.deliver([{ type: 'workers', workers: [worker('a', 'connecting')] }]);
    channel.deliver([{ type: 'workers', workers: [worker('a'), worker('b')] }]);

    expect(list.status).toBe('ready');
    expect(list.workers.map((known) => [known.id, known.status])).toEqual([
      ['a', 'connected'],
      ['b', 'connected'],
    ]);
  });

  it('says why it could not follow the list, and follows it again on request', async () => {
    core.subscribeWorkers.mockResolvedValueOnce({ error: { code: 'store_unavailable' }, status: 'error' });
    const list = new WorkerList();
    list.start();
    await vi.waitFor(() => {
      expect(list.status).toBe('error');
    });

    list.retry();
    channel.deliver([{ type: 'workers', workers: [] }]);

    expect(list.status).toBe('ready');
    expect(list.failure).toBeNull();
    expect(core.subscribeWorkers).toHaveBeenCalledTimes(2);
  });

  it('follows the list again when it fell behind', async () => {
    const list = new WorkerList();
    list.start();
    await vi.waitFor(() => {
      expect(core.subscribeWorkers).toHaveBeenCalledTimes(1);
    });

    channel.deliver([{ reason: 'lagged', type: 'ended' }]);

    await vi.waitFor(() => {
      expect(core.subscribeWorkers).toHaveBeenCalledTimes(2);
    });
  });
});

describe('pairing a worker', () => {
  it('closes the dialog and shows the worker once it is paired', async () => {
    const paired = worker('new', 'connecting');
    core.pairWorker.mockResolvedValue({ data: paired, status: 'ok' });
    const actions = new WorkersActions();
    actions.startPair();
    const request = actions.pair;
    if (request === null) {
      throw new Error('the dialog has a request');
    }
    request.address = ' build-01.lan ';
    request.code = ' 7kq2-m9xa-4dpe-w1tr-6hvz ';

    await actions.confirmPair(request);

    expect(core.pairWorker).toHaveBeenCalledWith({
      address: 'build-01.lan',
      code: '7kq2-m9xa-4dpe-w1tr-6hvz',
      name: null,
    });
    expect(actions.pairOpen).toBe(false);
    expect(workerList.find('new')).toEqual(paired);
  });

  it('sends the owner’s name for the worker without surrounding spaces', async () => {
    core.pairWorker.mockResolvedValue({ data: worker('new', 'connecting'), status: 'ok' });
    const actions = new WorkersActions();
    actions.startPair();
    const request = actions.pair;
    if (request === null) {
      throw new Error('the dialog has a request');
    }
    request.address = 'build-01.lan';
    request.code = '7kq2-m9xa-4dpe-w1tr-6hvz';
    request.name = ' freebsd-01 ';

    await actions.confirmPair(request);

    expect(core.pairWorker).toHaveBeenCalledWith({
      address: 'build-01.lan',
      code: '7kq2-m9xa-4dpe-w1tr-6hvz',
      name: 'freebsd-01',
    });
  });

  it('keeps the dialog open with the refusal', async () => {
    core.pairWorker.mockResolvedValue({ error: { code: 'code_refused' }, status: 'error' });
    const actions = new WorkersActions();
    actions.startPair();
    const request = actions.pair;
    if (request === null) {
      throw new Error('the dialog has a request');
    }

    await actions.confirmPair(request);

    expect(actions.pairOpen).toBe(true);
    expect(request.failure).toEqual({ error: { code: 'code_refused' }, kind: 'core' });
    expect(request.pairing).toBe(false);
  });
});

describe('renaming a worker', () => {
  it('starts from the owner’s name, and shows the worker under its new name once the core stored it', async () => {
    const named = { ...worker('a'), chosenName: 'freebsd-01', name: 'freebsd-01' };
    const renamed = { ...named, chosenName: 'build box', name: 'build box' };
    workerList.workers = [named];
    core.renameWorker.mockResolvedValue({ data: renamed, status: 'ok' });
    const actions = new WorkersActions();

    actions.startRename(named);
    const request = actions.renaming;
    if (request === null) {
      throw new Error('the dialog has a request');
    }
    const startedWith = request.name;
    request.name = 'build box';
    await actions.confirmRename(request);

    expect(startedWith).toBe('freebsd-01');
    expect(core.renameWorker).toHaveBeenCalledWith({ name: 'build box', workerId: 'a' });
    expect(actions.renameOpen).toBe(false);
    expect(workerList.find('a')?.name).toBe('build box');
  });

  it('starts empty for a worker that uses the name in its configuration', () => {
    const actions = new WorkersActions();

    actions.startRename(worker('a'));

    expect(actions.renaming?.name).toBe('');
    expect(actions.renameOpen).toBe(true);
  });

  it('keeps the dialog open with a refused name', async () => {
    core.renameWorker.mockResolvedValue({ error: { code: 'name_taken' }, status: 'error' });
    const actions = new WorkersActions();
    actions.startRename(worker('a'));
    const request = actions.renaming;
    if (request === null) {
      throw new Error('the dialog has a request');
    }

    await actions.confirmRename(request);

    expect(actions.renameOpen).toBe(true);
    expect(request.failure).toEqual({ error: { code: 'name_taken' }, kind: 'core' });
    expect(request.saving).toBe(false);
  });
});

describe('removing a worker', () => {
  it('removes the worker once the owner confirmed, and treats a worker that is gone as removed', async () => {
    workerList.workers = [worker('a'), worker('b')];
    core.removeWorker
      .mockResolvedValueOnce({ data: null, status: 'ok' })
      .mockResolvedValueOnce({ error: { code: 'worker_not_found' }, status: 'error' });
    const actions = new WorkersActions();

    actions.startRemove(worker('a'));
    expect(core.removeWorker).not.toHaveBeenCalled();
    const first = actions.removing;
    if (first === null) {
      throw new Error('the confirmation has a request');
    }
    await actions.confirmRemove(first);
    actions.startRemove(worker('b'));
    const second = actions.removing;
    if (second === null) {
      throw new Error('the confirmation has a request');
    }
    await actions.confirmRemove(second);

    expect(workerList.workers).toEqual([]);
    expect(actions.removeOpen).toBe(false);
    expect(second.failure).toBeNull();
  });

  it('keeps the confirmation open with a failure', async () => {
    core.removeWorker.mockResolvedValue({ error: { code: 'storage_failed' }, status: 'error' });
    const actions = new WorkersActions();
    actions.startRemove(worker('a'));
    const request = actions.removing;
    if (request === null) {
      throw new Error('the confirmation has a request');
    }

    await actions.confirmRemove(request);

    expect(actions.removeOpen).toBe(true);
    expect(request.failure).toEqual({ error: { code: 'storage_failed' }, kind: 'core' });
  });
});

describe('showing a worker', () => {
  it('opens the Workers settings with the worker’s details open, once', async () => {
    const actions = new WorkersActions();

    await actions.showDetails(worker('a'));

    expect(navigation.openSettings).toHaveBeenCalledWith('workers.settings');
    expect(actions.takeDetailsFor()).toBe('a');
    expect(actions.takeDetailsFor()).toBeNull();
  });
});
