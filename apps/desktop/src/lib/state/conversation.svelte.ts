// The conversation that the conversation view shows (docs/features/chat/
// conversations.md, transcript and events): it subscribes with its cursor,
// applies the updates, and subscribes again from the cursor when it falls
// behind or a batch is lost. The reply continues in the core whether or not
// the view follows it.
import {
  type ChatError,
  commands,
  type ConversationId,
  type ConversationUpdate,
  type RunView,
  type SubscriptionId,
} from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';
import { batchChannel, call } from '#lib/utils/calls.js';
import {
  activeRun,
  applyUpdate,
  emptyTranscript,
  endedRun,
  prependTurns,
  type TranscriptState,
} from '#lib/utils/conversation-reducer.js';

/** How many earlier turns one page loads. */
const EARLIER_TURNS_PAGE = 50;

/** One conversation, followed for as long as the view shows it. */
export class OpenConversation {
  /**
   * The latest reply that ended while the view followed the conversation,
   * which the view announces to screen readers. Replies that had ended before
   * the view opened are not announced.
   */
  endedRun: RunView | null = $state(null);
  /** Why the conversation could not be read, if it could not. */
  failure: Failure<ChatError> | null = $state(null);
  readonly id: ConversationId;
  /** Why earlier turns could not be loaded, if they could not. */
  loadEarlierFailure: Failure<ChatError> | null = $state(null);
  loadingEarlier = $state(false);
  /**
   * `deleted` when the conversation no longer exists, and `ended` when the core
   * stopped the subscription because Mannie quits.
   */
  status: 'deleted' | 'ended' | 'error' | 'loading' | 'ready' = $state('loading');
  transcript: TranscriptState = $state(emptyTranscript());
  #closed = false;
  /** Counts subscriptions, so that the updates of an ended one change nothing. */
  #generation = 0;
  #subscription: SubscriptionId | null = null;

  constructor(id: ConversationId) {
    this.id = id;
  }

  /** Stops following the conversation. */
  close(): void {
    this.#closed = true;
    this.#generation += 1;
    this.#unsubscribe();
  }

  /** Loads the page of turns before the first loaded one. */
  async loadEarlier(): Promise<void> {
    const first = this.transcript.turns.at(0);
    if (first === undefined) {
      return;
    }
    const cannotLoad = this.loadingEarlier || !this.transcript.hasEarlierTurns;
    if (cannotLoad) {
      return;
    }
    this.loadingEarlier = true;
    this.loadEarlierFailure = null;
    const outcome = await call(() =>
      commands.loadEarlierTurns({
        beforePosition: first.position,
        conversationId: this.id,
        limit: EARLIER_TURNS_PAGE,
      }),
    );
    this.loadingEarlier = false;
    if (outcome.ok) {
      prependTurns(this.transcript, outcome.data.turns, outcome.data.hasEarlierTurns);
    } else {
      this.loadEarlierFailure = outcome.failure;
    }
  }

  /** Starts following the conversation, from a snapshot. */
  open(): void {
    void this.#subscribe();
  }

  /** Follows the conversation again after a failure. */
  retry(): void {
    void this.#subscribe();
  }

  #apply(updates: readonly ConversationUpdate[]): void {
    const running = activeRun(this.transcript)?.id ?? null;
    for (const update of updates) {
      const continues = this.#applyOne(update);
      if (!continues) {
        break;
      }
    }
    if (running === null) {
      return;
    }
    const ended = endedRun(this.transcript, running);
    if (ended !== null) {
      this.endedRun = ended;
    }
  }

  /** Applies one update, and returns `false` when the subscription ended with it. */
  #applyOne(update: ConversationUpdate): boolean {
    const outcome = applyUpdate(this.transcript, update);
    switch (outcome) {
      case 'applied': {
        this.status = 'ready';
        this.failure = null;
        return true;
      }
      case 'deleted': {
        this.status = 'deleted';
        return false;
      }
      case 'resubscribe': {
        void this.#subscribe();
        return false;
      }
      case 'shutdown': {
        this.status = 'ended';
        return false;
      }
      case 'skipped': {
        return true;
      }
    }
  }

  async #subscribe(): Promise<void> {
    if (this.#closed) {
      return;
    }
    this.#generation += 1;
    const generation = this.#generation;
    this.#unsubscribe();
    const channel = batchChannel<ConversationUpdate>(
      (updates) => {
        if (generation === this.#generation) {
          this.#apply(updates);
        }
      },
      () => {
        if (generation === this.#generation) {
          void this.#subscribe();
        }
      },
    );
    const outcome = await call(() =>
      commands.subscribeConversation({ afterSeq: this.transcript.cursor, conversationId: this.id }, channel),
    );
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
    const isMissing = outcome.failure.kind === 'core' && outcome.failure.error.code === 'conversation_not_found';
    this.status = isMissing ? 'deleted' : 'error';
    this.failure = outcome.failure;
  }

  #unsubscribe(): void {
    const subscription = this.#subscription;
    this.#subscription = null;
    if (subscription !== null) {
      void commands.unsubscribe({ subscriptionId: subscription });
    }
  }
}
