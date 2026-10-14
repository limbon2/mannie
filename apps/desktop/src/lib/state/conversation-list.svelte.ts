// The conversation list (docs/features/chat/conversation-view.md, layout):
// the whole list from the core first, then each conversation that is
// created, changed or deleted. The window follows it for as long as it runs.
import {
  type ChatError,
  commands,
  type ConversationListUpdate,
  type ConversationSummary,
  type SubscriptionId,
} from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';
import { batchChannel, call } from '#lib/utils/calls.js';

/** The most recently changed first, as the core sorts them. */
function newestFirst(left: ConversationSummary, right: ConversationSummary): number {
  const byTime = right.updatedAt - left.updatedAt;
  if (byTime !== 0) {
    return byTime;
  }
  return right.id.localeCompare(left.id);
}

/** The window's conversation list. */
export class ConversationList {
  conversations: ConversationSummary[] = $state([]);
  /** Why the list could not be read, if it could not. */
  failure: Failure<ChatError> | null = $state(null);
  status: 'error' | 'loading' | 'ready' = $state('loading');
  /** Counts subscriptions, so that the updates of an ended one change nothing. */
  #generation = 0;
  #started = false;
  #subscription: SubscriptionId | null = null;

  /** The conversation with this ID, if the list has it. */
  find(id: string): ConversationSummary | undefined {
    return this.conversations.find((conversation) => conversation.id === id);
  }

  /** Follows the list again after a failure. */
  retry(): void {
    void this.#subscribe();
  }

  /** Starts following the list. Later calls change nothing. */
  start(): void {
    if (this.#started) {
      return;
    }
    this.#started = true;
    void this.#subscribe();
  }

  #apply(update: ConversationListUpdate): void {
    switch (update.type) {
      case 'ended': {
        // A list that fell behind follows the list again; at shutdown the
        // window is about to close.
        if (update.reason === 'lagged') {
          void this.#subscribe();
        }
        return;
      }
      case 'removed': {
        this.conversations = this.conversations.filter((conversation) => conversation.id !== update.id);
        return;
      }
      case 'snapshot': {
        this.conversations = [...update.conversations].sort(newestFirst);
        this.status = 'ready';
        this.failure = null;
        return;
      }
      case 'upserted': {
        const others = this.conversations.filter((conversation) => conversation.id !== update.conversation.id);
        this.conversations = [...others, update.conversation].sort(newestFirst);
      }
    }
  }

  async #subscribe(): Promise<void> {
    this.#generation += 1;
    const generation = this.#generation;
    const previous = this.#subscription;
    this.#subscription = null;
    if (previous !== null) {
      void commands.unsubscribe({ subscriptionId: previous });
    }
    if (this.conversations.length === 0) {
      this.status = 'loading';
    }
    const channel = batchChannel<ConversationListUpdate>(
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
    const outcome = await call(() => commands.subscribeConversations(channel));
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

/** The window's conversation list. */
export const conversationList = new ConversationList();
