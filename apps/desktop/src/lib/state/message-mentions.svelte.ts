// The mentions of the message that the owner writes (docs/features/chat/
// mentions.md): the "@" list that follows the caret, the entries that the
// owner chose and that the list showed, and the references that the message
// carries when it is sent (ADR-0032, decision 1). Each message box has its
// own.
import { SvelteMap } from 'svelte/reactivity';

import {
  commands,
  type MentionEntry,
  type MentionGroup,
  type MentionPlace,
  type MentionRef,
} from '#lib/generated/bindings.js';
import type { ChatFailure } from '#lib/types/chat.js';
import { call } from '#lib/utils/calls.js';
import { type FindMentionKind, sortMentionGroups } from '#lib/utils/mention-kinds.js';
import { type Insertion, insertMention, type MentionQuery, mentionQueryAt, tokensInText } from '#lib/utils/mentions.js';

/** How long the list waits after a keystroke before it asks the core. */
export const SUGGESTION_DELAY_MS = 100;

/** The most entries that the list remembers having shown, for tokens that the owner types. */
const MAX_SEEN = 1000;

/** An entry of the list with its provider. */
export interface MentionChoice {
  readonly entry: MentionEntry;
  readonly provider: string;
}

/** Where the list is. */
export type MentionListStatus = 'closed' | 'failed' | 'loading' | 'ready';

/** The mentions of one message box. */
export class MessageMentions {
  /** Why the core listed nothing. */
  failure: ChatFailure | null = $state(null);
  /** The groups of the latest answer, in the order of their kinds. */
  groups: readonly MentionGroup[] = $state([]);
  /** The highlighted entry, as its index among every group's entries. */
  highlighted = $state(0);
  /** The token that the caret is in, or `null` while the list is closed. */
  query: MentionQuery | null = $state(null);
  status: MentionListStatus = $state('closed');

  /** The token whose list the owner closed with Escape, which stays closed until the owner types in it or leaves it. */
  #dismissed: MentionQuery | null = null;
  /** The request whose answer the list shows. */
  #answered = 0;
  readonly #findKind: FindMentionKind;
  readonly #picked = new SvelteMap<string, MentionChoice>();
  readonly #place: () => MentionPlace;
  #request = 0;
  readonly #seen = new SvelteMap<string, MentionChoice>();
  #timer: ReturnType<typeof setTimeout> | undefined;
  /** Hands over the highlighted entry once the answer that an early Enter waits for arrives. */
  #waiting: ((choice: MentionChoice | undefined) => void) | undefined;

  /** The mentions of a message written in `place`, whose groups `findKind` orders. */
  constructor(place: () => MentionPlace, findKind: FindMentionKind) {
    this.#place = place;
    this.#findKind = findKind;
  }

  /** The highlighted entry, if the list shows one. */
  get active(): MentionChoice | undefined {
    return this.choices[this.highlighted];
  }

  /** Every entry of the list, group after group. */
  get choices(): MentionChoice[] {
    return this.groups.flatMap((group) => group.entries.map((entry) => ({ entry, provider: group.provider })));
  }

  /** Whether the list shows. */
  get open(): boolean {
    return this.query !== null;
  }

  /** Whether the list shows the answer for the token that the caret is in. */
  get settled(): boolean {
    return this.#answered === this.#request;
  }

  /**
   * The highlighted entry once the list shows the answer for the token that
   * the caret is in, for an Enter or Tab that came before the answer, or
   * `undefined` when the answer has no entries or the owner typed on.
   */
  async whenAnswered(): Promise<MentionChoice | undefined> {
    if (this.settled) {
      return this.active;
    }
    this.#settle(undefined);
    return new Promise((resolve) => {
      this.#waiting = resolve;
    });
  }

  /**
   * Puts the token of `choice` in place of the token that the caret is in.
   * An entry that expands, such as a folder, keeps the list open for what is
   * inside it.
   */
  choose(text: string, choice: MentionChoice): Insertion {
    const at = this.query ?? { end: text.length, query: '', start: text.length };
    this.#picked.set(choice.entry.token, choice);
    const inserted = insertMention(text, at, choice.entry.token, choice.entry.expands);
    if (choice.entry.expands) {
      // What the list showed lies outside the folder now.
      this.groups = [];
      this.update(inserted.text, inserted.caret);
    } else {
      this.close();
    }
    return inserted;
  }

  /** Closes the list for the token that the caret is in, until the owner types in it or moves to another one. */
  dismiss(): void {
    this.#dismissed = this.query;
    this.close();
  }

  /** Closes the list. An answer that is still on its way is dropped. */
  close(): void {
    clearTimeout(this.#timer);
    this.#request += 1;
    this.#answered = this.#request;
    this.#settle(undefined);
    this.query = null;
    this.groups = [];
    this.failure = null;
    this.highlighted = 0;
    this.status = 'closed';
  }

  /** The mentions that the text holds, in its order: tokens that the owner chose, or that the list showed. */
  inText(text: string): MentionChoice[] {
    const seenOnly = [...this.#seen.keys()].filter((token) => !this.#picked.has(token));
    const tokens = [...this.#picked.keys(), ...seenOnly];
    return tokensInText(text, tokens).flatMap((token) => {
      const choice = this.#picked.get(token) ?? this.#seen.get(token);
      return choice === undefined ? [] : [choice];
    });
  }

  /** Moves the highlight by `step` entries, around the ends. */
  move(step: number): void {
    const count = this.choices.length;
    if (count === 0) {
      return;
    }
    this.highlighted = (((this.highlighted + step) % count) + count) % count;
  }

  /** The references that a message with this text carries. */
  refs(text: string): MentionRef[] {
    return this.inText(text).map((choice) => ({ key: choice.entry.key, provider: choice.provider }));
  }

  /** Forgets everything once the message is sent. */
  reset(): void {
    this.close();
    this.#dismissed = null;
    this.#picked.clear();
    this.#seen.clear();
  }

  /** Follows the text and the caret: opens the list in a token, and closes it outside one. */
  update(text: string, caret: number): void {
    const query = mentionQueryAt(text, caret);
    if (query === null) {
      this.#dismissed = null;
      if (this.open) {
        this.close();
      }
      return;
    }
    if (this.#isDismissed(query)) {
      return;
    }
    this.#dismissed = null;
    const changed = this.query?.query !== query.query || this.query.start !== query.start;
    this.query = query;
    if (changed) {
      this.#ask(query.query);
    }
  }

  /** Whether the owner closed the list of this token and has not typed in it since. */
  #isDismissed(query: MentionQuery): boolean {
    const dismissed = this.#dismissed;
    if (dismissed === null) {
      return false;
    }
    const sameToken = dismissed.start === query.start;
    const sameQuery = dismissed.query === query.query;
    return sameToken && sameQuery;
  }

  #ask(query: string): void {
    clearTimeout(this.#timer);
    this.#request += 1;
    // An early Enter waited for the answer of the token as it was.
    this.#settle(undefined);
    const request = this.#request;
    if (this.groups.length === 0) {
      this.status = 'loading';
    }
    this.#timer = setTimeout(() => {
      void this.#fetch(query, request);
    }, SUGGESTION_DELAY_MS);
  }

  async #fetch(query: string, request: number): Promise<void> {
    const outcome = await call(() => commands.suggestMentions({ place: this.#place(), query }));
    // A later keystroke asked again, or the list closed, since.
    if (request !== this.#request) {
      return;
    }
    this.#answered = request;
    if (!outcome.ok) {
      this.groups = [];
      this.failure = outcome.failure;
      this.status = 'failed';
      this.#settle(undefined);
      return;
    }
    this.groups = sortMentionGroups(outcome.data.groups, this.#findKind);
    for (const choice of this.choices) {
      this.#remember(choice);
    }
    this.failure = null;
    this.highlighted = 0;
    this.status = 'ready';
    this.#settle(this.active);
  }

  /** Hands `choice` to an early Enter that waits, if one does. */
  #settle(choice: MentionChoice | undefined): void {
    const waiting = this.#waiting;
    this.#waiting = undefined;
    waiting?.(choice);
  }

  #remember(choice: MentionChoice): void {
    this.#seen.delete(choice.entry.token);
    this.#seen.set(choice.entry.token, choice);
    if (this.#seen.size > MAX_SEEN) {
      const oldest = this.#seen.keys().next().value;
      if (oldest !== undefined) {
        this.#seen.delete(oldest);
      }
    }
  }
}
