// The core's questions about closing the window and quitting (docs/features/
// window-and-tray.md, closing and quitting). The window subscribes once when
// it starts; the core sends a question that waits again on each subscription,
// for example after a reload. Without a window that answers within five
// seconds, the core quits on its own.
import {
  type CloseChoice,
  commands,
  type WindowError,
  type WindowRequest,
  type WindowRequestUpdate,
} from '#lib/generated/bindings.js';
import { windowSettings } from '#lib/state/window-settings.svelte.js';
import type { Failure } from '#lib/types/calls.js';
import { batchChannel, call } from '#lib/utils/calls.js';

/** A question of the core and the ID that its answer names. */
export interface WindowQuestion {
  readonly request: WindowRequest;
  readonly requestId: number;
}

/** The question that waits for the owner, and the outcome of the answer. */
export class WindowRequests {
  /** Whether an answer is on its way to the core. */
  answering = $state(false);
  /** Why the latest answer failed. */
  failure: Failure<WindowError> | null = $state(null);
  /** Whether the question's dialog is open. */
  open = $state(false);
  /** The latest question, which stays while its dialog closes. */
  question: WindowQuestion | null = $state.raw(null);
  /** Counts subscriptions, so that the updates of a replaced one change nothing. */
  #generation = 0;
  #started = false;

  /** Answers the first-close question. The core saves the choice, then hides the window or quits. */
  async answerClose(choice: CloseChoice): Promise<void> {
    const answered = await this.#answer((requestId) => commands.answerCloseQuestion({ choice, requestId }));
    if (answered) {
      // The settings section shows the saved choice when the window opens again.
      windowSettings.closeAction = choice;
    }
  }

  /** Quits although replies are being written. */
  async confirmQuit(): Promise<void> {
    await this.#answer((requestId) => commands.confirmQuit({ requestId }));
  }

  /** Closes the question's dialog without an answer: the window stays open and nothing changes. */
  async dismiss(): Promise<void> {
    const question = this.question;
    this.open = false;
    if (question === null) {
      return;
    }
    // A question that no longer waits needs no answer, so a refusal changes nothing.
    await call(() => commands.dismissWindowRequest({ requestId: question.requestId }));
  }

  /** Starts receiving the core's questions. Later calls change nothing. */
  start(): void {
    if (this.#started) {
      return;
    }
    this.#started = true;
    void this.#subscribe();
  }

  /**
   * Sends an answer with `send`, and returns whether the core accepted it. The
   * dialog closes when it did and shows the reason when it did not.
   */
  async #answer(send: (requestId: number) => ReturnType<typeof commands.confirmQuit>): Promise<boolean> {
    const question = this.question;
    if (question === null) {
      return false;
    }
    this.answering = true;
    this.failure = null;
    const outcome = await call(() => send(question.requestId));
    this.answering = false;
    if (!outcome.ok) {
      this.failure = outcome.failure;
      return false;
    }
    if (this.question === question) {
      this.open = false;
    }
    return true;
  }

  #apply(update: WindowRequestUpdate): void {
    switch (update.type) {
      case 'cleared': {
        if (this.question?.requestId === update.requestId) {
          this.open = false;
        }
        return;
      }
      case 'pending': {
        this.question = { request: update.request, requestId: update.requestId };
        this.failure = null;
        this.answering = false;
        this.open = true;
      }
    }
  }

  async #subscribe(): Promise<void> {
    this.#generation += 1;
    const generation = this.#generation;
    const channel = batchChannel<WindowRequestUpdate>(
      (updates) => {
        if (generation !== this.#generation) {
          return;
        }
        for (const update of updates) {
          this.#apply(update);
        }
      },
      () => {
        // Lost batches: subscribing again gets the question that waits.
        if (generation === this.#generation) {
          void this.#subscribe();
        }
      },
    );
    try {
      await commands.subscribeWindowRequests(channel);
    } catch {
      // A core that does not answer cannot ask either. When it asks later
      // and no window receives the question, it quits after five seconds.
    }
  }
}

/** The window's questions from the core. */
export const windowRequests = new WindowRequests();
