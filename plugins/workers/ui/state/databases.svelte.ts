// The registered databases (plugins/workers/docs/databases.md, the view):
// the list from the core first, then the whole list again after every
// change. The window follows it while Workers and its feature Databases are
// on, since the host ends the feature's channel when the feature stops and
// refuses a new one while it is off (ADR-0055, decision 6).
import type { FeatureContext } from '@mannie/plugin-sdk';
import { batchChannel, call, type ChannelMessage, type SubscriptionId, unsubscribe } from '@mannie/plugin-sdk/ipc';

import { channels, commands, type DatabaseListUpdate, type DatabaseView } from '../generated/bindings.js';
import type { DatabasesFailure } from '../types/databases.js';

/** The window's list of registered databases. */
export class DatabaseList {
  /** The databases, by name. */
  databases: DatabaseView[] = $state([]);
  /** Why the list could not be read, if it could not. */
  failure: DatabasesFailure | null = $state(null);
  status: 'error' | 'loading' | 'ready' = $state('loading');
  #following = false;
  /** Counts subscriptions, so that the updates of an ended one change nothing. */
  #generation = 0;
  #subscription: SubscriptionId | null = null;

  /** The database with this key, if the list has it. */
  find(key: string): DatabaseView | undefined {
    return this.databases.find((database) => database.key === key);
  }

  /** Follows the list while the feature's context reports Databases as on. Later calls change nothing. */
  follow(feature: Pick<FeatureContext, 'whileOn'>): void {
    if (this.#following) {
      return;
    }
    this.#following = true;
    feature.whileOn(
      () => void this.#subscribe(),
      () => {
        this.#end();
      },
    );
  }

  /** Looks at each registered file again, such as when the view opens; the channel brings the new states. */
  async refresh(): Promise<DatabasesFailure | null> {
    const outcome = await call(() => commands.refreshDatabases());
    return outcome.ok ? null : outcome.failure;
  }

  /** Follows the list again after a failure. */
  retry(): void {
    void this.#subscribe();
  }

  #apply(message: ChannelMessage<DatabaseListUpdate>): void {
    switch (message.kind) {
      case 'ended': {
        // A list that fell behind follows the list again. When the feature
        // stops, the list ends as its state leaves "on".
        if (message.reason === 'lagged') {
          void this.#subscribe();
        }
        return;
      }
      case 'item': {
        this.databases = message.item.databases;
        this.status = 'ready';
        this.failure = null;
      }
    }
  }

  /** Ends the subscription and forgets the databases, as the feature stops. */
  #end(): void {
    this.#generation += 1;
    const previous = this.#subscription;
    this.#subscription = null;
    if (previous !== null) {
      void unsubscribe(previous);
    }
    this.databases = [];
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
    if (this.databases.length === 0) {
      this.status = 'loading';
    }
    const channel = batchChannel<ChannelMessage<DatabaseListUpdate>>(
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
    const outcome = await call(() => channels.followDatabases(channel));
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

/** The window's list of registered databases. */
export const databaseList = new DatabaseList();
