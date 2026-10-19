// What the owner does with workers (plugins/workers/docs/
// worker-activity.md): pairing one with the code that its host printed,
// naming one, removing one, and showing one in the Workers settings. The
// dialogs these actions open are mounted once, as the feature's overlay.
// The plugin's commands go through its generated bindings (ADR-0034,
// decision 9).
import { call } from '@mannie/plugin-sdk/ipc';
import { openSettings } from '@mannie/plugin-sdk/navigation';

import { commands, type WorkerView } from '../generated/bindings.js';
import type { WorkersFailure } from '../types/workers.js';

import { workerList } from './workers.svelte.js';

/** The ID of the Workers section of the settings. */
export const WORKERS_SETTINGS_SECTION = 'workers.settings';

/** One opening of the pairing dialog, whose fields start empty. */
export class PairRequest {
  address = $state('');
  code = $state('');
  /** Why the latest attempt failed. */
  failure: WorkersFailure | null = $state(null);
  /** The owner's name for the worker; empty keeps the name in its configuration. */
  name = $state('');
  /** Whether the core is pairing. */
  pairing = $state(false);
}

/** One opening of the rename dialog, which starts with the owner's current name for the worker. */
export class RenameWorkerRequest {
  /** Why the latest attempt failed. */
  failure: WorkersFailure | null = $state(null);
  name = $state('');
  saving = $state(false);
  readonly worker: WorkerView;

  constructor(worker: WorkerView) {
    this.worker = worker;
    this.name = worker.chosenName ?? '';
  }
}

/** One opening of the removal confirmation. */
export class RemoveWorkerRequest {
  /** Why the latest attempt failed. */
  failure: WorkersFailure | null = $state(null);
  removing = $state(false);
  readonly worker: WorkerView;

  constructor(worker: WorkerView) {
    this.worker = worker;
  }
}

/** The owner's actions on workers, and the dialogs they open. */
export class WorkersActions {
  /** The worker whose details the Workers settings show open, once. */
  detailsFor: string | null = $state(null);
  /** The latest pairing dialog, which stays while it closes. */
  pair: PairRequest | null = $state(null);
  pairOpen = $state(false);
  /** The latest removal confirmation, which stays while it closes. */
  removing: RemoveWorkerRequest | null = $state(null);
  removeOpen = $state(false);
  /** The latest rename dialog, which stays while it closes. */
  renaming: RenameWorkerRequest | null = $state(null);
  renameOpen = $state(false);

  /** Closes every dialog, as Workers turns off. */
  closeAll(): void {
    this.pairOpen = false;
    this.renameOpen = false;
    this.removeOpen = false;
    this.detailsFor = null;
  }

  /** Pairs with the worker of a pairing dialog, which closes once the worker is paired. */
  async confirmPair(request: PairRequest): Promise<void> {
    request.pairing = true;
    request.failure = null;
    const name = request.name.trim();
    const outcome = await call(() =>
      commands.pairWorker({
        address: request.address.trim(),
        code: request.code.trim(),
        name: name === '' ? null : name,
      }),
    );
    request.pairing = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }
    workerList.upsert(outcome.data);
    if (this.pair === request) {
      this.pairOpen = false;
    }
  }

  /** Removes the worker of a removal confirmation, which closes once the core removed it. */
  async confirmRemove(request: RemoveWorkerRequest): Promise<void> {
    request.removing = true;
    request.failure = null;
    const workerId = request.worker.id;
    const outcome = await call(() => commands.removeWorker({ workerId }));
    request.removing = false;
    const gone = !outcome.ok && outcome.failure.kind === 'core' && outcome.failure.error.code === 'worker_not_found';
    if (!outcome.ok && !gone) {
      request.failure = outcome.failure;
      return;
    }
    workerList.forget(workerId);
    if (this.removing === request) {
      this.removeOpen = false;
    }
  }

  /** Renames the worker of a rename dialog, which closes once the core stored the name. */
  async confirmRename(request: RenameWorkerRequest): Promise<void> {
    request.saving = true;
    request.failure = null;
    const outcome = await call(() => commands.renameWorker({ name: request.name, workerId: request.worker.id }));
    request.saving = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }
    workerList.upsert(outcome.data);
    if (this.renaming === request) {
      this.renameOpen = false;
    }
  }

  /** Opens the Workers settings with a worker's details open. */
  async showDetails(worker: WorkerView): Promise<void> {
    this.detailsFor = worker.id;
    await openSettings(WORKERS_SETTINGS_SECTION);
  }

  /** Opens the pairing dialog with empty fields. */
  startPair(): void {
    this.pair = new PairRequest();
    this.pairOpen = true;
  }

  /** Opens the removal confirmation for a worker. */
  startRemove(worker: WorkerView): void {
    this.removing = new RemoveWorkerRequest(worker);
    this.removeOpen = true;
  }

  /** Opens the rename dialog for a worker. */
  startRename(worker: WorkerView): void {
    this.renaming = new RenameWorkerRequest(worker);
    this.renameOpen = true;
  }

  /** The worker whose details the settings open, once; later calls return nothing. */
  takeDetailsFor(): string | null {
    const id = this.detailsFor;
    this.detailsFor = null;
    return id;
  }
}

/** The owner's actions on workers. */
export const workersActions = new WorkersActions();
