// A conversation's browser session as the pane sees it: the status that the
// core sends through `browser_follow`, and the owner's controls. The
// subscription is the lease of the session's native pages: while no pane
// follows the conversation, its pages hide (ADR-0064, decision 5).
import { batchChannel, call, type ChannelMessage, type SubscriptionId, unsubscribe } from '@mannie/plugin-sdk/ipc';

import {
  type BrowserControl,
  type BrowserStatus_Serialize as BrowserStatus,
  channels,
  commands,
} from '../generated/bindings.js';
import { browserFailureMessage } from '../utils/browser-messages.js';

/** One conversation's browser, followed by one pane. */
export class BrowserSession {
  /** Whether an owner control is running. */
  busy = $state(false);
  /** Why the last control or the following failed, in catalog words. */
  failure = $state<string | null>(null);
  /** Whether the first status is still on its way. */
  loading = $state(true);
  /** The latest status, complete. */
  status = $state.raw<BrowserStatus | null>(null);
  /** Counts subscriptions, so that the items of an ended one change nothing. */
  #generation = 0;
  #subscription: SubscriptionId | null = null;

  constructor(readonly conversationId: string) {}

  /** Follows the conversation's status, or follows it again. */
  follow(): void {
    void this.#open();
  }

  /** Stops following, which lets the core hide the pages when no pane follows. */
  stop(): void {
    this.#generation += 1;
    const previous = this.#subscription;
    this.#subscription = null;
    if (previous !== null) {
      void unsubscribe(previous);
    }
  }

  async control(action: BrowserControl, tabId: string | null): Promise<boolean> {
    if (this.busy) {
      return false;
    }
    this.busy = true;
    this.failure = null;
    try {
      const result = await call(() => commands.browserControl({ action, conversationId: this.conversationId, tabId }));
      if (!result.ok) {
        // The status stream reports certificate rejection in the affected tab.
        const reportedInTab = result.failure.kind === 'core' && result.failure.error.code === 'certificate_rejected';
        if (!reportedInTab) {
          this.failure = browserFailureMessage(result.failure);
        }
      }
      return result.ok;
    } finally {
      this.busy = false;
    }
  }

  /** Moves keyboard focus into a tab's page, in takeover. */
  async focusPage(tabId: string): Promise<void> {
    const outcome = await call(() => commands.browserFocus({ conversationId: this.conversationId, tabId }));
    if (!outcome.ok) {
      this.failure = browserFailureMessage(outcome.failure);
    }
  }

  #apply(message: ChannelMessage<BrowserStatus>): void {
    if (message.kind === 'item') {
      this.loading = false;
      this.status = message.item;
      return;
    }
    // A channel that fell behind starts again. One that the host ended, as
    // when the plugin stops or the page loads again, stays ended.
    if (message.reason === 'lagged') {
      void this.#open();
    }
  }

  async #open(): Promise<void> {
    this.stop();
    const generation = this.#generation;
    const channel = batchChannel<ChannelMessage<BrowserStatus>>(
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
    const outcome = await call(() => channels.browserFollow({ conversationId: this.conversationId }, channel));
    if (generation !== this.#generation) {
      if (outcome.ok) {
        void unsubscribe(outcome.data);
      }
      return;
    }
    if (outcome.ok) {
      this.#subscription = outcome.data;
      return;
    }
    this.loading = false;
    this.failure = browserFailureMessage(outcome.failure);
  }
}
