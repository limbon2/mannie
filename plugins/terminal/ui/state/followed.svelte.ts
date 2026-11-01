// A channel of the plugin that the window follows (ADR-0034, decision 8):
// the items in order, a new subscription when batches were lost or the
// channel fell behind, and nothing after it stopped. The host ends the
// plugin's channels when the plugin stops and refuses new ones while it is
// off, so a follower starts and stops with the plugin or with the component
// that needs it.
import {
  batchChannel,
  call,
  type Channel,
  type ChannelBatch,
  type ChannelMessage,
  type CommandError,
  type PluginFailure,
  type Result,
  type SubscriptionId,
  unsubscribe,
} from '@mannie/plugin-sdk/ipc';

import type { TerminalError } from '../generated/bindings.js';
import type { TerminalFailure } from '../types/terminal.js';

/** How a follower subscribes to its channel. */
export type Subscribe<T> = (
  channel: Channel<ChannelBatch<ChannelMessage<T>>>,
) => Promise<Result<SubscriptionId, CommandError<PluginFailure<TerminalError>>>>;

/** A channel that the window follows, and what it heard. */
export class Followed<T> {
  /** Why the channel could not be followed, if it could not. */
  failure: TerminalFailure | null = $state(null);
  /** Whether the channel ended for good, as when the terminal it follows is gone. */
  ended = $state(false);
  /** Counts subscriptions, so that the items of an ended one change nothing. */
  #generation = 0;
  readonly #onItem: (item: T) => void;
  readonly #subscribe: Subscribe<T>;
  #subscription: SubscriptionId | null = null;

  /** A follower of the channel that `subscribe` opens, which hands each item to `onItem`. */
  constructor(subscribe: Subscribe<T>, onItem: (item: T) => void) {
    this.#subscribe = subscribe;
    this.#onItem = onItem;
  }

  /** Follows the channel, or follows it again from its start. */
  start(): void {
    void this.#open();
  }

  /** Stops following the channel. */
  stop(): void {
    this.#generation += 1;
    const previous = this.#subscription;
    this.#subscription = null;
    if (previous !== null) {
      void unsubscribe(previous);
    }
  }

  #apply(message: ChannelMessage<T>): void {
    switch (message.kind) {
      case 'ended': {
        // A channel that fell behind starts again; one that the host ended
        // with the plugin stays ended until the plugin starts again.
        if (message.reason === 'lagged') {
          void this.#open();
        } else {
          this.ended = true;
        }
        return;
      }
      case 'item': {
        this.failure = null;
        this.#onItem(message.item);
      }
    }
  }

  async #open(): Promise<void> {
    this.stop();
    const generation = this.#generation;
    this.ended = false;
    const channel = batchChannel<ChannelMessage<T>>(
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
          void this.#open();
        }
      },
    );
    const outcome = await call(() => this.#subscribe(channel));
    if (generation !== this.#generation) {
      // A newer subscription, or a stop, replaced this one while it started.
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
  }
}
