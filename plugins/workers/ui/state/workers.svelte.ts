// The paired workers (plugins/workers/docs/worker-activity.md): the list
// from the core first, then the whole list again after every change of a
// worker or its status. The window follows it while Workers is on, since the
// host ends the plugin's channel when the plugin stops and refuses a new one
// while it is off (plugins/workers/docs/plugin.md; ADR-0034, decision 8).
import type { PluginContext } from '@mannie/plugin-sdk';
import {
  batchChannel,
  call,
  type ChannelMessage,
  type PluginCallFailure,
  type SubscriptionId,
  unsubscribe,
} from '@mannie/plugin-sdk/ipc';

import { channels, type WorkerListUpdate, type WorkersError, type WorkerView } from '../generated/bindings.js';

/** The window's list of paired workers. */
export class WorkerList {
  /** Why the list could not be read, if it could not. */
  failure: PluginCallFailure<WorkersError> | null = $state(null);
  status: 'error' | 'loading' | 'ready' = $state('loading');
  /** The paired workers, in the order of pairing. */
  workers: WorkerView[] = $state([]);
  #following = false;
  /** Counts subscriptions, so that the updates of an ended one change nothing. */
  #generation = 0;
  #subscription: SubscriptionId | null = null;

  /** The worker with this ID, if the list has it. */
  find(id: string): WorkerView | undefined {
    return this.workers.find((worker) => worker.id === id);
  }

  /** Follows the list while the plugin's context reports Workers as on. Later calls change nothing. */
  follow(plugin: Pick<PluginContext, 'whileOn'>): void {
    if (this.#following) {
      return;
    }
    this.#following = true;
    plugin.whileOn(
      () => void this.#subscribe(),
      () => {
        this.#end();
      },
    );
  }

  /** Follows the list again after a failure. */
  retry(): void {
    void this.#subscribe();
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

  #apply(message: ChannelMessage<WorkerListUpdate>): void {
    switch (message.kind) {
      case 'ended': {
        // A list that fell behind follows the list again. When Workers
        // stops, the list ends as the plugin's state leaves "on"; at
        // shutdown the window is about to close.
        if (message.reason === 'lagged') {
          void this.#subscribe();
        }
        return;
      }
      case 'item': {
        this.workers = message.item.workers;
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
      void unsubscribe(previous);
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
      void unsubscribe(previous);
    }
    if (this.workers.length === 0) {
      this.status = 'loading';
    }
    const channel = batchChannel<ChannelMessage<WorkerListUpdate>>(
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
    const outcome = await call(() => channels.subscribeWorkers(channel));
    if (generation !== this.#generation) {
      // A newer subscription replaced this one while it started.
      if (outcome.ok) {
        void unsubscribe(outcome.data);
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
