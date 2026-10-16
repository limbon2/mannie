// The paired workers (docs/features/workers/worker-activity.md): the list
// from the core first, then the whole list again after every change of a
// worker or its status. The window follows it while Workers is on, since the
// core ends the list's subscription when the plugin stops and refuses a new
// one while it is off (docs/features/workers/plugin.md).
import {
  commands,
  type SubscriptionId,
  type WorkerListUpdate,
  type WorkersError,
  type WorkerView,
} from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';
import { batchChannel, call } from '#lib/utils/calls.js';

import { type PluginStates, pluginStates } from './plugin-states.svelte.js';

/** The Workers plugin's ID, which is also its executor's kind. */
export const WORKERS_PLUGIN = 'workers';

/** The window's list of paired workers. */
export class WorkerList {
  /** Why the list could not be read, if it could not. */
  failure: Failure<WorkersError> | null = $state(null);
  status: 'error' | 'loading' | 'ready' = $state('loading');
  /** The paired workers, in the order of pairing. */
  workers: WorkerView[] = $state([]);
  /** Counts subscriptions, so that the updates of an ended one change nothing. */
  #generation = 0;
  readonly #plugins: Pick<PluginStates, 'whileOn'>;
  #started = false;
  #subscription: SubscriptionId | null = null;

  /** A list that follows the workers while `plugins` reports Workers as on. */
  constructor(plugins: Pick<PluginStates, 'whileOn'> = pluginStates) {
    this.#plugins = plugins;
  }

  /** The worker with this ID, if the list has it. */
  find(id: string): WorkerView | undefined {
    return this.workers.find((worker) => worker.id === id);
  }

  /** Follows the list again after a failure. */
  retry(): void {
    void this.#subscribe();
  }

  /** Starts following the list while Workers is on. Later calls change nothing. */
  start(): void {
    if (this.#started) {
      return;
    }
    this.#started = true;
    this.#plugins.whileOn(
      WORKERS_PLUGIN,
      () => void this.#subscribe(),
      () => {
        this.#end();
      },
    );
  }

  /** Adds or replaces a worker that a command returned, before its update arrives through the channel. */
  upsert(worker: WorkerView): void {
    const index = this.workers.findIndex((known) => known.id === worker.id);
    this.workers = index === -1 ? [...this.workers, worker] : this.workers.with(index, worker);
  }

  /** Forgets a worker that a command removed, before its update arrives through the channel. */
  forget(id: string): void {
    this.workers = this.workers.filter((worker) => worker.id !== id);
  }

  #apply(update: WorkerListUpdate): void {
    switch (update.type) {
      case 'ended': {
        // A list that fell behind follows the list again. When Workers
        // stops, the list ends as the plugin's state leaves "on"; at
        // shutdown the window is about to close.
        if (update.reason === 'lagged') {
          void this.#subscribe();
        }
        return;
      }
      case 'workers': {
        this.workers = update.workers;
        this.status = 'ready';
        this.failure = null;
      }
    }
  }

  /** Ends the subscription and forgets the workers, as Workers stops. */
  #end(): void {
    this.#generation += 1;
    const previous = this.#subscription;
    this.#subscription = null;
    if (previous !== null) {
      void commands.unsubscribe({ subscriptionId: previous });
    }
    this.workers = [];
    this.failure = null;
    this.status = 'loading';
  }

  async #subscribe(): Promise<void> {
    this.#generation += 1;
    const generation = this.#generation;
    const previous = this.#subscription;
    this.#subscription = null;
    if (previous !== null) {
      void commands.unsubscribe({ subscriptionId: previous });
    }
    if (this.workers.length === 0) {
      this.status = 'loading';
    }
    const channel = batchChannel<WorkerListUpdate>(
      (items) => {
        if (generation !== this.#generation) {
          return;
        }
        for (const item of items) {
          this.#apply(item);
        }
      },
      () => {
        if (generation === this.#generation) {
          void this.#subscribe();
        }
      },
    );
    const outcome = await call(() => commands.subscribeWorkers(channel));
    if (generation !== this.#generation) {
      // A newer subscription replaced this one while it started.
      if (outcome.ok) {
        void commands.unsubscribe({ subscriptionId: outcome.data });
      }
      return;
    }
    if (outcome.ok) {
      this.#subscription = outcome.data;
      return;
    }
    this.failure = outcome.failure;
    this.status = 'error';
  }
}

/** The window's list of paired workers. */
export const workerList = new WorkerList();
