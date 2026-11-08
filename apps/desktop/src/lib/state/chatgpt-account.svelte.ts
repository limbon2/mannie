// The ChatGPT account (docs/features/chatgpt-plan.md): the account as the
// core reports it, first and after each change, and the owner's actions on
// it. The core keeps the tokens and opens OpenAI's sign-in page in the system
// browser, so the window never sees a token or the sign-in's URL (ADR-0062,
// decision 5). The page that the browser shows when a sign-in returns takes
// its texts from the catalog, since the core has none (ADR-0019).
import {
  type ChatGptAccountUpdate,
  type ChatGptAccountView,
  type ChatGptCallbackPage,
  type ChatGptError,
  type ChatGptModel,
  commands,
  type SignOutReport,
  type SubscriptionId,
} from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import { type LinkFailure, openLink } from '#lib/sdk/links.js';
import type { Outcome } from '#lib/types/calls.js';
import type { ChatGptFailure } from '#lib/types/chat.js';
import { batchChannel, call } from '#lib/utils/calls.js';

/**
 * ChatGPT's page where the owner manages the plan's usage, which OpenAI's
 * guidelines link "Manage usage" to (verified 2026-11-08).
 */
export const CHATGPT_USAGE_URL = 'https://chatgpt.com/settings/usage';

/** A command of the account that waits for the core's answer. */
export type ChatGptCommand = 'acknowledge' | 'cancel' | 'reopen' | 'sign_in' | 'sign_out';

/** The answer of a command of the account that returns nothing. */
type EmptyAnswer = Promise<{ data: null; status: 'ok' } | { error: ChatGptError; status: 'error' }>;

/** The page that the browser shows when a sign-in returns, in the owner's language. */
function callbackPage(): ChatGptCallbackPage {
  return {
    failed: m.chat_chatgpt_page_failed(),
    signedIn: m.chat_chatgpt_page_signed_in(),
    title: m.chat_chatgpt_page_title(),
  };
}

/** The window's ChatGPT account and the owner's actions on it. */
export class ChatGptAccount {
  /** The account, once the core reported it. */
  account: ChatGptAccountView | null = $state(null);
  /** Why the account could not be followed, if it could not. */
  failure: ChatGptFailure | null = $state(null);
  /** The command that waits for the core, which keeps the account's buttons off until it answers. */
  pending: ChatGptCommand | null = $state(null);
  /**
   * Why the latest sign-in did not start, such as a port that another
   * program uses. A sign-in that started and failed later says why in the
   * account's own sign-in step.
   */
  startFailure: ChatGptFailure | null = $state(null);
  status: 'error' | 'loading' | 'ready' = $state('loading');
  /** Counts subscriptions, so that the updates of an ended one change nothing. */
  #generation = 0;
  #started = false;
  #subscription: SubscriptionId | null = null;

  /** Whether the notice that Mannie uses the ChatGPT plan is due: after the first sign-in, until the owner saw it. */
  get noticeDue(): boolean {
    const account = this.account;
    if (account === null) {
      return false;
    }
    const signedIn = account.state === 'signed_in';
    return signedIn && !account.noticeSeen;
  }

  /** Whether the account is signed in, so that profiles on the ChatGPT plan can call their models. */
  get signedIn(): boolean {
    return this.account?.state === 'signed_in';
  }

  /** Why the latest sign-in failed, whether it did not start or ended in failure, or `null`. */
  get signInFailure(): ChatGptFailure | null {
    if (this.startFailure !== null) {
      return this.startFailure;
    }
    const signIn = this.account?.signIn;
    return signIn?.step === 'failed' ? { error: signIn.error, kind: 'core' } : null;
  }

  /** Whether a sign-in waits for the browser or checks the account. */
  get signingIn(): boolean {
    const step = this.account?.signIn.step;
    const waiting = step === 'waiting_for_browser';
    const finishing = step === 'finishing';
    return waiting || finishing;
  }

  /** Records that the owner saw the notice; returns why it could not, or `null`. */
  acknowledgeNotice(): Promise<ChatGptFailure | null> {
    return this.#send('acknowledge', () => commands.acknowledgeChatgptNotice());
  }

  /** Cancels the sign-in that runs; returns why it could not, or `null`. */
  cancel(): Promise<ChatGptFailure | null> {
    return this.#send('cancel', () => commands.cancelChatgptSignIn());
  }

  /** Opens ChatGPT's usage settings in the system browser; returns why they did not open, or `null`. */
  manageUsage(): Promise<LinkFailure | null> {
    return openLink(CHATGPT_USAGE_URL);
  }

  /** The models that the plan offers the signed-in account, in the order of OpenAI's catalog. */
  models(): Promise<Outcome<ChatGptModel[], ChatGptError>> {
    return call(() => commands.listChatgptModels());
  }

  /** Opens the page of the sign-in that waits for the browser again; returns why it could not, or `null`. */
  reopenPage(): Promise<ChatGptFailure | null> {
    return this.#send('reopen', () => commands.reopenChatgptSignInPage());
  }

  /** Follows the account again after a failure. */
  retry(): void {
    void this.#subscribe();
  }

  /**
   * Starts a sign-in, in which the core opens OpenAI's page in the system
   * browser, to the known account or, with `anotherAccount`, to a new one.
   * The account's updates show how it goes; returns why it did not start, or
   * `null`.
   */
  async signIn(anotherAccount: boolean): Promise<ChatGptFailure | null> {
    this.startFailure = null;
    const page = callbackPage();
    const failure = await this.#send('sign_in', () => commands.startChatgptSignIn({ anotherAccount, page }));
    this.startFailure = failure;
    return failure;
  }

  /** Signs out, and returns whether OpenAI confirmed it, or why signing out failed. */
  async signOut(): Promise<Outcome<SignOutReport, ChatGptError>> {
    this.pending = 'sign_out';
    const outcome = await call(() => commands.signOutChatgpt());
    this.#settle('sign_out');
    return outcome;
  }

  /** Starts following the account. Later calls change nothing. */
  start(): void {
    if (this.#started) {
      return;
    }
    this.#started = true;
    void this.#subscribe();
  }

  #apply(update: ChatGptAccountUpdate): void {
    switch (update.type) {
      case 'account': {
        this.account = update.account;
        this.status = 'ready';
        this.failure = null;
        return;
      }
      case 'ended': {
        // An account that fell behind follows again; at shutdown the window
        // is about to close.
        if (update.reason === 'lagged') {
          void this.#subscribe();
        }
      }
    }
  }

  /** Sends a command that answers with nothing, and returns why it failed, or `null`. */
  async #send(command: ChatGptCommand, send: () => EmptyAnswer): Promise<ChatGptFailure | null> {
    this.pending = command;
    const outcome = await call(send);
    this.#settle(command);
    return outcome.ok ? null : outcome.failure;
  }

  /** Ends the wait for a command, unless another command took its place meanwhile. */
  #settle(command: ChatGptCommand): void {
    if (this.pending === command) {
      this.pending = null;
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
    if (this.account === null) {
      this.status = 'loading';
    }
    const channel = batchChannel<ChatGptAccountUpdate>(
      (updates) => {
        if (generation !== this.#generation) {
          return;
        }
        for (const update of updates) {
          this.#apply(update);
        }
      },
      () => {
        if (generation === this.#generation) {
          void this.#subscribe();
        }
      },
    );
    const outcome = await call(() => commands.subscribeChatgptAccount(channel));
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

/** The window's ChatGPT account. */
export const chatgptAccount = new ChatGptAccount();
