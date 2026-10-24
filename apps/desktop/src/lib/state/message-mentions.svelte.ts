// The mentions of the message that the owner writes (docs/features/chat/
// mentions.md): the "@" list that follows the caret, the entries that the
// owner chose and that the list showed, and the references that the message
// carries when it is sent (ADR-0032, decision 1). Each message box has its
// own.
//
// The list asks each kind of mention on its own and shows each kind as soon
// as it answers, so that a slow kind, such as git in a large repository,
// holds back no other. While more than one kind shows entries, each shows
// its first few with an option that shows the rest, so that every kind can
// be reached without scrolling past the others.
import { SvelteMap, SvelteSet } from 'svelte/reactivity';

import {
  type ChatError,
  commands,
  type MentionEntry,
  type MentionGroup,
  type MentionPlace,
  type MentionRef,
} from '#lib/generated/bindings.js';
import type { Outcome } from '#lib/types/calls.js';
import type { ChatFailure } from '#lib/types/chat.js';
import { call } from '#lib/utils/calls.js';
import { type FindMentionKind, sortMentionProviders } from '#lib/utils/mention-kinds.js';
import { type Insertion, insertMention, type MentionQuery, mentionQueryAt, tokensInText } from '#lib/utils/mentions.js';

/** How long the list waits after a keystroke before it asks the core. */
export const SUGGESTION_DELAY_MS = 50;

/** The entries that a kind shows while another kind shows entries too, until the owner shows the rest. */
export const COLLAPSED_ENTRIES = 5;

/** The most entries that the list remembers having shown, for tokens that the owner types. */
const MAX_SEEN = 1000;

/** An entry of the list with its provider. */
export interface MentionChoice {
  readonly entry: MentionEntry;
  readonly provider: string;
}

/** An option of the list: an entry, or the option that shows the rest of a kind's entries. */
export type MentionOption =
  | { readonly choice: MentionChoice; readonly type: 'entry' }
  | { readonly hidden: number; readonly provider: string; readonly type: 'more' };

/** A kind's group as the list shows it. */
export interface ShownMentionGroup {
  /** The kind's latest answer, which may answer what the owner typed before. */
  readonly group: MentionGroup;
  /** The group's options: the entries that it shows, then the option that shows the rest, if it hides some. */
  readonly options: readonly MentionOption[];
  /** Whether the group shows every entry of its answer. */
  readonly whole: boolean;
}

/** Where the list is. */
export type MentionListStatus = 'closed' | 'failed' | 'loading' | 'ready';

/** What Enter would choose while that depends on an answer that has not arrived. */
const PENDING = Symbol('pending');

/** The provider of an option. */
function providerOf(option: MentionOption): string {
  return option.type === 'entry' ? option.choice.provider : option.provider;
}

/** What tells an option apart from the others, also across the answers of one query. */
function optionKey(option: MentionOption): string {
  const key = option.type === 'entry' ? option.choice.entry.key : null;
  return JSON.stringify([option.type, providerOf(option), key]);
}

/** The mentions of one message box. */
export class MessageMentions {
  /** Why the core listed nothing. */
  failure: ChatFailure | null = $state(null);
  /** The token that the caret is in, or `null` while the list is closed. */
  query: MentionQuery | null = $state(null);

  /** Each kind's latest answer, by provider. */
  readonly #answers = new SvelteMap<string, MentionGroup>();
  /** The request that each kind's latest answer belongs to, by provider. */
  readonly #answeredFor = new SvelteMap<string, number>();
  /** The token whose list the owner closed with Escape, which stays closed until the owner types in it or leaves it. */
  #dismissed: MentionQuery | null = null;
  /** The kinds whose every entry the owner chose to show. */
  readonly #expanded = new SvelteSet<string>();
  readonly #findKind: FindMentionKind;
  /** The option that the owner highlighted, by its key, or `null` for the first one. */
  #highlight: string | null = $state(null);
  /** The providers that the place offers, in the order of their kinds, or `null` until the core says. */
  #offered: readonly string[] | null = $state(null);
  /** The core's answer of which providers the place offers, which every request of one list shares. */
  #offering: Promise<Outcome<readonly string[], ChatError>> | undefined;
  readonly #picked = new SvelteMap<string, MentionChoice>();
  readonly #place: () => MentionPlace;
  #request = 0;
  readonly #seen = new SvelteMap<string, MentionChoice>();
  /** Whether a query with white space waits for a kind that has not answered yet, before the list decides whether to follow it. */
  #spacedPending = false;
  #timer: ReturnType<typeof setTimeout> | undefined;
  /** Hands over what Enter chooses once the answer that an early Enter waits for arrives. */
  #waiting: ((option: MentionOption | undefined) => void) | undefined;

  /** The groups that the list shows, in the order of their kinds. */
  readonly shown: readonly ShownMentionGroup[] = $derived(this.#shownGroups());
  /** Every option of the list, group after group. */
  readonly options: readonly MentionOption[] = $derived(this.shown.flatMap((group) => group.options));
  /** The highlighted option, as its index among every option. */
  readonly highlighted: number = $derived.by(() => {
    const chosen = this.#highlight;
    const index = chosen === null ? -1 : this.options.findIndex((option) => optionKey(option) === chosen);
    return Math.max(index, 0);
  });

  /** The mentions of a message written in `place`, whose groups `findKind` orders. */
  constructor(place: () => MentionPlace, findKind: FindMentionKind) {
    this.#place = place;
    this.#findKind = findKind;
  }

  /** The highlighted option, if the list shows one. */
  get active(): MentionOption | undefined {
    return this.options.at(this.highlighted);
  }

  /** Whether a kind has not answered since the list opened or went into a folder. */
  get looking(): boolean {
    const offered = this.#offered;
    const isLooking = offered === null || offered.some((provider) => !this.#answers.has(provider));
    return isLooking;
  }

  /** Whether the place offers no kind of mention. */
  get offersNothing(): boolean {
    return this.#offered?.length === 0;
  }

  /** Whether the list shows. */
  get open(): boolean {
    return this.query !== null;
  }

  /** Whether Enter can choose now, from the answers for the token that the caret is in. */
  get settled(): boolean {
    return this.#determined() !== PENDING;
  }

  /** Where the list is: loading until a kind answers. */
  get status(): MentionListStatus {
    if (this.query === null) {
      return 'closed';
    }
    if (this.failure !== null) {
      return 'failed';
    }
    const offered = this.#offered;
    const waitsForAll = offered === null || (offered.length > 0 && this.#answers.size === 0);
    return waitsForAll ? 'loading' : 'ready';
  }

  /**
   * What Enter chooses once the list shows the answers for the token that the
   * caret is in, for an Enter or Tab that came before them, or `undefined`
   * when the answers have no entries or the owner typed on.
   */
  async whenAnswered(): Promise<MentionOption | undefined> {
    const determined = this.#determined();
    if (determined !== PENDING) {
      return determined;
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
    const at = this.query ?? { end: text.length, query: '', spaced: false, start: text.length };
    this.#picked.set(choice.entry.token, choice);
    const inserted = insertMention(text, at, choice.entry.token, choice.entry.expands);
    if (choice.entry.expands) {
      // What the list showed lies outside the folder now.
      this.#forgetAnswers();
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
    this.#settle(undefined);
    this.#spacedPending = false;
    this.query = null;
    this.failure = null;
    this.#offered = null;
    this.#offering = undefined;
    this.#forgetAnswers();
  }

  /** Shows every entry of a kind, and highlights the first entry that was hidden. */
  expand(provider: string): void {
    this.#expanded.add(provider);
    const entry = this.#answers.get(provider)?.entries[COLLAPSED_ENTRIES];
    if (entry !== undefined) {
      this.#highlight = optionKey({ choice: { entry, provider }, type: 'entry' });
    }
  }

  /** Highlights the option at `index`, as the arrows and the pointer do. */
  highlight(index: number): void {
    const option = this.options.at(index);
    this.#highlight = option === undefined ? null : optionKey(option);
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

  /** Moves the highlight by `step` options, around the ends. */
  move(step: number): void {
    const count = this.options.length;
    if (count === 0) {
      return;
    }
    this.highlight((((this.highlighted + step) % count) + count) % count);
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
    const found = mentionQueryAt(text, caret);
    if (found === null) {
      this.#dismissed = null;
      if (this.open) {
        this.close();
      }
      return;
    }
    if (found.spaced) {
      this.#followSpaced(found);
      return;
    }
    if (this.#isDismissed(found)) {
      return;
    }
    this.#dismissed = null;
    const changed = this.query?.query !== found.query || this.query.start !== found.start;
    this.query = found;
    if (changed) {
      this.#spacedPending = false;
      // An early Enter waited for the answers of the token as it was.
      this.#settle(undefined);
      this.#ask(found.query);
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

  /**
   * Follows a token whose query holds white space, which never opens the
   * list. The list stays with the token while it shows an entry that the
   * query names with its space, such as a host called "build box", and
   * closes otherwise, since the owner wrote on after the token.
   */
  #followSpaced(found: MentionQuery): void {
    const sameToken = this.query?.start === found.start;
    if (!sameToken) {
      // The owner left a token whose list they closed.
      this.#dismissed = null;
      if (this.open) {
        this.close();
      }
      return;
    }
    const changed = this.query?.query !== found.query;
    this.query = found;
    if (changed) {
      // An early Enter waited for the answers of the token as it was.
      this.#settle(undefined);
      this.#decideSpaced();
    }
  }

  /** Asks about the query with white space, closes the list, or waits for a kind that has not answered yet. */
  #decideSpaced(): void {
    const query = this.query;
    if (query === null) {
      return;
    }
    const verdict = this.#spacedVerdict(query.query);
    this.#spacedPending = verdict === 'wait';
    if (verdict === 'follow') {
      this.#ask(query.query);
    } else if (verdict === 'end') {
      this.close();
    }
  }

  /**
   * Whether the list follows a query with white space: it ends with the
   * token when the text before the white space is a whole token that the
   * list shows, as in "@main.rs and", and follows it when an entry's label
   * holds the whole query without regard to case, as "build box" holds
   * "build b". Otherwise it waits for the kinds that have not answered yet,
   * and then ends.
   */
  #spacedVerdict(query: string): 'end' | 'follow' | 'wait' {
    const entries = [...this.#answers.values()].flatMap((group) => group.entries);
    const typed = `@${query.trimEnd()}`;
    if (entries.some((entry) => entry.token === typed)) {
      return 'end';
    }
    const wanted = query.toLowerCase();
    if (entries.some((entry) => entry.label.toLowerCase().includes(wanted))) {
      return 'follow';
    }
    const waits = this.failure === null && this.looking;
    return waits ? 'wait' : 'end';
  }

  #ask(query: string): void {
    clearTimeout(this.#timer);
    this.#request += 1;
    this.#highlight = null;
    const request = this.#request;
    this.#timer = setTimeout(() => {
      void this.#fetch(query, request);
    }, SUGGESTION_DELAY_MS);
  }

  async #fetch(query: string, request: number): Promise<void> {
    const offered = await this.#offeredProviders();
    // A later keystroke asked again, or the list closed, since.
    if (request !== this.#request) {
      return;
    }
    if (!offered.ok) {
      this.#fail(offered.failure);
      return;
    }
    if (offered.data.length === 0) {
      this.#afterAnswer();
      return;
    }
    await Promise.all(offered.data.map(async (provider) => this.#fetchKind(provider, query, request)));
  }

  /** The providers that the place offers, which the core tells once for each list. */
  async #offeredProviders(): Promise<Outcome<readonly string[], ChatError>> {
    if (this.#offered !== null) {
      return { data: this.#offered, ok: true };
    }
    const offering = this.#offering ?? this.#askOffered();
    this.#offering = offering;
    const outcome = await offering;
    // The list closed since, and a new one asks again.
    if (this.#offering !== offering) {
      return outcome;
    }
    if (outcome.ok) {
      this.#offered ??= outcome.data;
    } else {
      // The next keystroke asks again.
      this.#offering = undefined;
    }
    return outcome;
  }

  async #askOffered(): Promise<Outcome<readonly string[], ChatError>> {
    const outcome = await call(() => commands.listMentionProviders({ place: this.#place() }));
    if (!outcome.ok) {
      return outcome;
    }
    return { data: sortMentionProviders(outcome.data.providers, this.#findKind), ok: true };
  }

  async #fetchKind(provider: string, query: string, request: number): Promise<void> {
    const outcome = await call(() => commands.suggestMentions({ place: this.#place(), provider, query }));
    if (request !== this.#request) {
      return;
    }
    if (!outcome.ok) {
      this.#fail(outcome.failure);
      return;
    }
    // A provider that the place no longer offers answers with no group.
    const group = outcome.data.groups.find((candidate) => candidate.provider === provider) ?? {
      entries: [],
      failure: null,
      incomplete: false,
      provider,
    };
    this.failure = null;
    this.#answers.set(provider, group);
    this.#answeredFor.set(provider, request);
    for (const entry of group.entries) {
      this.#remember({ entry, provider });
    }
    this.#afterAnswer();
  }

  /** Decides a query with white space that waited for an answer, and hands over what an early Enter waits for. */
  #afterAnswer(): void {
    if (this.#spacedPending) {
      this.#decideSpaced();
    }
    if (this.#waiting !== undefined) {
      const determined = this.#determined();
      if (determined !== PENDING) {
        this.#settle(determined);
      }
    }
  }

  #fail(failure: ChatFailure): void {
    this.failure = failure;
    this.#spacedPending = false;
    this.#settle(undefined);
  }

  /**
   * What Enter chooses: the option that the owner highlighted, once its kind
   * answered what they typed, or else the first entry of the answers, once
   * every kind before it answered. `PENDING` until then.
   */
  #determined(): MentionOption | typeof PENDING | undefined {
    const isClosed = this.query === null;
    const hasFailed = this.failure !== null;
    if (isClosed || hasFailed) {
      return undefined;
    }
    const offered = this.#offered;
    if (offered === null) {
      return PENDING;
    }
    if (this.#spacedPending) {
      return PENDING;
    }
    const chosen = this.#chosenAndAnswered();
    if (chosen !== undefined) {
      return chosen;
    }
    for (const provider of offered) {
      if (!this.#hasAnswered(provider)) {
        return PENDING;
      }
      const first = this.options.find((option) => providerOf(option) === provider);
      if (first !== undefined) {
        return first;
      }
    }
    return undefined;
  }

  /**
   * The option that the owner highlighted, once its kind answered what they
   * typed last. An option that the list no longer shows was not chosen,
   * even though the highlight falls back to the first one.
   */
  #chosenAndAnswered(): MentionOption | undefined {
    const option = this.active;
    if (option === undefined) {
      return undefined;
    }
    if (optionKey(option) !== this.#highlight) {
      return undefined;
    }
    const answered = this.#hasAnswered(providerOf(option));
    return answered ? option : undefined;
  }

  /** Whether a kind answered what the owner typed last. */
  #hasAnswered(provider: string): boolean {
    return this.#answeredFor.get(provider) === this.#request;
  }

  #forgetAnswers(): void {
    this.#answers.clear();
    this.#answeredFor.clear();
    this.#expanded.clear();
    this.#highlight = null;
  }

  /**
   * The groups in the order of their kinds. While another kind shows entries
   * or has not answered yet, a group shows its first entries and an option
   * that shows the rest, unless the owner chose to show them.
   */
  #shownGroups(): ShownMentionGroup[] {
    const offered = this.#offered ?? [];
    const groups = offered.flatMap((provider) => {
      const group = this.#answers.get(provider);
      return group === undefined ? [] : [group];
    });
    const withEntries = groups.filter((group) => group.entries.length > 0).length;
    const waiting = offered.some((provider) => !this.#answers.has(provider));
    return groups.map((group) => {
      const hasEntries = group.entries.length > 0;
      const othersShowEntries = withEntries > (hasEntries ? 1 : 0);
      const crowded = othersShowEntries || waiting;
      const fits = group.entries.length <= COLLAPSED_ENTRIES;
      const whole = !crowded || fits || this.#expanded.has(group.provider);
      const entries = whole ? group.entries : group.entries.slice(0, COLLAPSED_ENTRIES);
      const options: MentionOption[] = entries.map((entry) => ({
        choice: { entry, provider: group.provider },
        type: 'entry',
      }));
      if (!whole) {
        options.push({ hidden: group.entries.length - entries.length, provider: group.provider, type: 'more' });
      }
      return { group, options, whole };
    });
  }

  /** Hands what Enter chooses to an early Enter that waits, if one does. */
  #settle(option: MentionOption | undefined): void {
    const waiting = this.#waiting;
    this.#waiting = undefined;
    waiting?.(option);
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
