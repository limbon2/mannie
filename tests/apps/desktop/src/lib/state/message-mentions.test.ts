// The mentions of the message that the owner writes (docs/features/chat/
// mentions.md): the "@" list that follows the caret and asks each kind once
// the owner pauses, kinds that answer one by one, answers that arrive too
// late, the groups that show their first entries while others show theirs,
// names with spaces, the choice of an entry, and the references that the
// message carries.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  MentionEntry,
  MentionGroup,
  MentionPlace,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import {
  COLLAPSED_ENTRIES,
  type MentionChoice,
  type MentionOption,
  MessageMentions,
  SUGGESTION_DELAY_MS,
} from '../../../../../../apps/desktop/src/lib/state/message-mentions.svelte.js';
import type { MentionKind } from '../../../../../../apps/desktop/src/lib/types/shell-slots.js';

const core = vi.hoisted(() => ({ listMentionProviders: vi.fn(), suggestMentions: vi.fn() }));

vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: core }));

const PLACE: MentionPlace = { kind: 'new_conversation', projectId: null };

/** What each provider lists for a query, by provider, in the core's order. */
type Lists = Partial<Record<string, (query: string) => MentionEntry[]>>;

/** The argument of a call of `suggestMentions`. */
interface Asked {
  readonly provider: string;
  readonly query: string;
}

function entry(key: string, extra: Partial<MentionEntry> = {}): MentionEntry {
  return { expands: false, key, label: key, token: `@${key}`, variant: 'file', warning: null, ...extra };
}

/** Entries named `prefix0`, `prefix1` and so on. */
function numbered(prefix: string, count: number): MentionEntry[] {
  return Array.from({ length: count }, (_value, index) => entry(`${prefix}${String(index)}`));
}

function group(provider: string, entries: MentionEntry[]): MentionGroup {
  return { entries, failure: null, incomplete: false, provider };
}

function answer(...groups: MentionGroup[]): { data: { groups: MentionGroup[] }; status: 'ok' } {
  return { data: { groups }, status: 'ok' };
}

/** The core offers the providers of `lists`, and each answers with what it lists for the query. */
function offer(lists: Lists): void {
  core.listMentionProviders.mockResolvedValue({ data: { providers: Object.keys(lists) }, status: 'ok' });
  core.suggestMentions.mockImplementation(async ({ provider, query }: Asked) => {
    const list = lists[provider];
    return Promise.resolve(answer(group(provider, list === undefined ? [] : list(query))));
  });
}

/** The entries of `entries` whose keys start with the query. */
function starting(...entries: MentionEntry[]): (query: string) => MentionEntry[] {
  return (query) => entries.filter((candidate) => candidate.key.startsWith(query));
}

/** The entries of `entries` whose labels hold the query, as a host's name does. */
function holding(...entries: MentionEntry[]): (query: string) => MentionEntry[] {
  return (query) => entries.filter((candidate) => candidate.label.includes(query));
}

/** An answer that arrives when the test hands it over. */
function later(): { readonly answer: Promise<unknown>; readonly give: (value: unknown) => void } {
  let give: (value: unknown) => void = () => undefined;
  const pending = new Promise((resolve) => {
    give = resolve;
  });
  return { answer: pending, give };
}

function mentionsWithoutKinds(): MessageMentions {
  return new MessageMentions(
    () => PLACE,
    () => undefined,
  );
}

/** The mentions of a message box whose kinds place their groups in `orders`. */
function mentionsWithKinds(orders: Record<string, number>): MessageMentions {
  const kinds = new Map(Object.entries(orders).map(([provider, order]) => [provider, { order }]));
  return new MessageMentions(
    () => PLACE,
    (provider) => kinds.get(provider) as MentionKind | undefined,
  );
}

/** Lets the delay pass and the core's answers arrive. */
async function settle(): Promise<void> {
  await vi.advanceTimersByTimeAsync(SUGGESTION_DELAY_MS);
  for (let step = 0; step < 10; step += 1) {
    await Promise.resolve();
  }
}

/** Types the characters of `text` after its first `typed`, key by key, as the message box follows them. */
function typeOn(mentions: MessageMentions, text: string, typed = 0): void {
  for (let length = typed + 1; length <= text.length; length += 1) {
    mentions.update(text.slice(0, length), length);
  }
}

function first(mentions: MessageMentions): MentionChoice {
  const option = mentions.options.at(0);
  if (option?.type !== 'entry') {
    throw new Error('the list shows an entry first');
  }
  return option.choice;
}

/** What each option shows: an entry's key, or how many entries the option of a group shows. */
function shown(mentions: MessageMentions): string[] {
  return mentions.options.map((option) =>
    option.type === 'entry' ? option.choice.entry.key : `+${String(option.hidden)} ${option.provider}`,
  );
}

function keyOf(option: MentionOption | undefined): string | undefined {
  return option?.type === 'entry' ? option.choice.entry.key : undefined;
}

beforeEach(() => {
  vi.useFakeTimers();
  core.listMentionProviders.mockReset();
  core.suggestMentions.mockReset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('MessageMentions', () => {
  it('asks each kind once the owner pauses, with what they typed after "@"', async () => {
    offer({ files: starting(entry('src/main.rs')), notes: () => [] });
    const mentions = mentionsWithoutKinds();

    mentions.update('Read @s', 7);
    mentions.update('Read @sr', 8);
    const whileWaiting = mentions.status;
    await settle();

    expect(whileWaiting).toBe('loading');
    expect(core.listMentionProviders).toHaveBeenCalledTimes(1);
    expect(core.listMentionProviders).toHaveBeenCalledWith({ place: PLACE });
    expect(core.suggestMentions.mock.calls).toEqual([
      [{ place: PLACE, provider: 'files', query: 'sr' }],
      [{ place: PLACE, provider: 'notes', query: 'sr' }],
    ]);
    expect(mentions.status).toBe('ready');
    expect(keyOf(mentions.active)).toBe('src/main.rs');
  });

  it('shows each kind as soon as it answers', async () => {
    const slow = later();
    core.listMentionProviders.mockResolvedValue({ data: { providers: ['files', 'git'] }, status: 'ok' });
    core.suggestMentions.mockImplementation(async ({ provider }: Asked) =>
      provider === 'git' ? slow.answer : Promise.resolve(answer(group('files', [entry('a.rs')]))),
    );
    const mentions = mentionsWithoutKinds();

    mentions.update('@a', 2);
    await settle();
    const beforeGit = { looking: mentions.looking, shown: shown(mentions), status: mentions.status };
    slow.give(answer(group('git', [entry('a1b2c3d', { variant: 'commit' })])));
    await settle();

    expect(beforeGit).toEqual({ looking: true, shown: ['a.rs'], status: 'ready' });
    expect(shown(mentions)).toEqual(['a.rs', 'a1b2c3d']);
    expect(mentions.looking).toBe(false);
  });

  it('drops an answer that a later query replaced', async () => {
    const early = later();
    core.listMentionProviders.mockResolvedValue({ data: { providers: ['files'] }, status: 'ok' });
    core.suggestMentions
      .mockImplementationOnce(async () => early.answer)
      .mockResolvedValueOnce(answer(group('files', [entry('second.rs')])));
    const mentions = mentionsWithoutKinds();

    mentions.update('@f', 2);
    await settle();
    mentions.update('@se', 3);
    await settle();
    early.give(answer(group('files', [entry('first.rs')])));
    await settle();

    expect(shown(mentions)).toEqual(['second.rs']);
  });

  it('closes outside a token, and an answer on its way changes nothing then', async () => {
    offer({ files: starting(entry('a.rs')) });
    const mentions = mentionsWithoutKinds();

    mentions.update('@a', 2);
    mentions.update('a', 1);
    await settle();

    expect(mentions.open).toBe(false);
    expect(mentions.status).toBe('closed');
    expect(core.suggestMentions).not.toHaveBeenCalled();
  });

  it('shows why the core listed nothing', async () => {
    core.listMentionProviders.mockResolvedValue({
      error: { code: 'project_not_found', description: 'refused' },
      status: 'error',
    });
    const mentions = mentionsWithoutKinds();

    mentions.update('@', 1);
    await settle();

    expect(mentions.status).toBe('failed');
    expect(mentions.failure).toEqual({
      error: { code: 'project_not_found', description: 'refused' },
      kind: 'core',
    });
    expect(mentions.settled).toBe(true);
  });

  it('says when the place offers nothing to mention', async () => {
    offer({});
    const mentions = mentionsWithoutKinds();

    mentions.update('@', 1);
    await settle();

    expect(mentions.status).toBe('ready');
    expect(mentions.offersNothing).toBe(true);
    expect(mentions.looking).toBe(false);
    expect(await mentions.whenAnswered()).toBeUndefined();
  });

  it('inserts a chosen entry and sends a reference for each token of the text', async () => {
    offer({ files: () => [entry('a.rs'), entry('b.rs')] });
    const mentions = mentionsWithoutKinds();

    mentions.update('Compare @', 9);
    await settle();
    const inserted = mentions.choose('Compare @', first(mentions));
    const text = `${inserted.text}with nothing`;

    expect(inserted).toEqual({ caret: 14, text: 'Compare @a.rs ' });
    expect(mentions.open).toBe(false);
    expect(mentions.refs(text)).toEqual([{ key: 'a.rs', provider: 'files' }]);
    expect(mentions.inText(text).map((choice) => choice.entry.label)).toEqual(['a.rs']);
  });

  it('counts a typed token that the list showed, and forgets a token that was deleted', async () => {
    offer({ files: () => [entry('a.rs'), entry('b.rs')] });
    const mentions = mentionsWithoutKinds();

    mentions.update('@b', 2);
    await settle();
    const typed = mentions.refs('Look at @b.rs and @c.rs');
    const deleted = mentions.refs('Look at and @c.rs');

    expect(typed).toEqual([{ key: 'b.rs', provider: 'files' }]);
    expect(deleted).toEqual([]);
  });

  it('keeps the list open inside a folder that expands', async () => {
    offer({
      files: (query) =>
        query === 'docs/'
          ? [entry('docs/guide.md')]
          : [entry('docs', { expands: true, token: '@docs/', variant: 'folder' })],
    });
    const mentions = mentionsWithoutKinds();

    mentions.update('@do', 3);
    await settle();
    const inserted = mentions.choose('@do', first(mentions));
    await settle();

    expect(inserted).toEqual({ caret: 6, text: '@docs/' });
    expect(mentions.open).toBe(true);
    expect(core.suggestMentions).toHaveBeenLastCalledWith({ place: PLACE, provider: 'files', query: 'docs/' });
    expect(first(mentions).entry.key).toBe('docs/guide.md');
  });

  it('moves the highlight around the ends and orders the groups by their kinds', async () => {
    offer({ files: () => [entry('a.rs'), entry('b.rs')], notes: () => [entry('note')] });
    const mentions = mentionsWithKinds({ files: 10, notes: 0 });

    mentions.update('@', 1);
    await settle();
    mentions.move(-1);
    const wrapped = keyOf(mentions.active);
    mentions.move(1);

    expect(mentions.shown.map((shownGroup) => shownGroup.group.provider)).toEqual(['notes', 'files']);
    expect(wrapped).toBe('b.rs');
    expect(keyOf(mentions.active)).toBe('note');
  });

  it('keeps the highlight on the entry that the owner moved to while other kinds answer', async () => {
    const slow = later();
    core.listMentionProviders.mockResolvedValue({ data: { providers: ['files', 'notes'] }, status: 'ok' });
    core.suggestMentions.mockImplementation(async ({ provider }: Asked) =>
      provider === 'notes' ? slow.answer : Promise.resolve(answer(group('files', [entry('a.rs'), entry('b.rs')]))),
    );
    const mentions = mentionsWithKinds({ files: 10, notes: 0 });

    mentions.update('@', 1);
    await settle();
    mentions.move(1);
    slow.give(answer(group('notes', [entry('note')])));
    await settle();

    expect(shown(mentions)).toEqual(['note', 'a.rs', 'b.rs']);
    expect(keyOf(mentions.active)).toBe('b.rs');
    expect(mentions.highlighted).toBe(2);
  });

  it('shows the first entries of each kind while another kind shows entries, and the rest on request', async () => {
    offer({ files: () => numbered('file', 8), notes: () => numbered('note', 2) });
    const mentions = mentionsWithKinds({ files: 0, notes: 10 });

    mentions.update('@', 1);
    await settle();
    const collapsed = shown(mentions);
    mentions.highlight(COLLAPSED_ENTRIES);
    const more = mentions.active;
    mentions.expand('files');

    expect(collapsed).toEqual(['file0', 'file1', 'file2', 'file3', 'file4', '+3 files', 'note0', 'note1']);
    expect(more).toEqual({ hidden: 3, provider: 'files', type: 'more' });
    expect(shown(mentions)).toEqual([...numbered('file', 8).map((shownEntry) => shownEntry.key), 'note0', 'note1']);
    expect(keyOf(mentions.active)).toBe('file5');
  });

  it('shows every entry of the only kind with entries once the other kinds answered', async () => {
    const slow = later();
    core.listMentionProviders.mockResolvedValue({ data: { providers: ['files', 'notes'] }, status: 'ok' });
    core.suggestMentions.mockImplementation(async ({ provider }: Asked) =>
      provider === 'notes' ? slow.answer : Promise.resolve(answer(group('files', numbered('file', 8)))),
    );
    const mentions = mentionsWithoutKinds();

    mentions.update('@', 1);
    await settle();
    const whileNotesLook = shown(mentions).length;
    slow.give(answer(group('notes', [])));
    await settle();

    expect(whileNotesLook).toBe(COLLAPSED_ENTRIES + 1);
    expect(shown(mentions)).toHaveLength(8);
    expect(mentions.shown[0]?.whole).toBe(true);
  });

  it('keeps the list closed after Escape until the owner types in the token or leaves it', async () => {
    offer({ files: starting(entry('a.rs'), entry('ab.rs')) });
    const mentions = mentionsWithoutKinds();

    mentions.update('@a', 2);
    await settle();
    mentions.dismiss();
    // The keyup of Escape reads the same token.
    mentions.update('@a', 2);
    const afterEscape = mentions.open;
    mentions.update('@ab', 3);
    const afterTyping = mentions.open;
    mentions.dismiss();
    mentions.update('@ab x', 5);
    const afterLeaving = mentions.open;
    mentions.update('@ab', 3);

    expect(afterEscape).toBe(false);
    expect(afterTyping).toBe(true);
    expect(afterLeaving).toBe(false);
    expect(mentions.open).toBe(true);
  });

  it('lets an Enter before the answer choose from the answer for what the owner typed', async () => {
    offer({
      files: (query) =>
        query === 'docs/'
          ? [entry('docs/guide.md')]
          : [entry('docs', { expands: true, token: '@docs/', variant: 'folder' }), entry('docs/old.md')],
    });
    const mentions = mentionsWithoutKinds();

    mentions.update('@do', 3);
    await settle();
    mentions.choose('@do', first(mentions));
    const settledInsideTheFolder = mentions.settled;
    const shownInsideTheFolder = mentions.options.length;
    const chosen = mentions.whenAnswered();
    await settle();

    expect(settledInsideTheFolder).toBe(false);
    expect(shownInsideTheFolder).toBe(0);
    expect(keyOf(await chosen)).toBe('docs/guide.md');
  });

  it('lets an early Enter take the first entry once every kind before it answered', async () => {
    const slow = later();
    core.listMentionProviders.mockResolvedValue({ data: { providers: ['files', 'notes'] }, status: 'ok' });
    core.suggestMentions.mockImplementation(async ({ provider }: Asked) =>
      provider === 'notes' ? slow.answer : Promise.resolve(answer(group('files', [entry('a.rs')]))),
    );
    const mentions = mentionsWithKinds({ files: 10, notes: 0 });

    mentions.update('@a', 2);
    const chosen = mentions.whenAnswered();
    await settle();
    const settledBeforeNotes = mentions.settled;
    slow.give(answer(group('notes', [])));
    await settle();

    expect(settledBeforeNotes).toBe(false);
    expect(keyOf(await chosen)).toBe('a.rs');
  });

  it('lets Enter take an entry that the owner moved to while another kind still looks', async () => {
    const slow = later();
    core.listMentionProviders.mockResolvedValue({ data: { providers: ['files', 'notes'] }, status: 'ok' });
    core.suggestMentions.mockImplementation(async ({ provider }: Asked) =>
      provider === 'notes' ? slow.answer : Promise.resolve(answer(group('files', [entry('a.rs'), entry('b.rs')]))),
    );
    const mentions = mentionsWithKinds({ files: 10, notes: 0 });

    mentions.update('@', 1);
    await settle();
    mentions.move(1);

    expect(mentions.settled).toBe(true);
    expect(keyOf(await mentions.whenAnswered())).toBe('b.rs');
  });

  it('waits for the kinds before the first entry when the entry that the owner moved to is gone', async () => {
    const slow = later();
    core.listMentionProviders.mockResolvedValue({ data: { providers: ['files', 'notes'] }, status: 'ok' });
    core.suggestMentions.mockImplementation(async ({ provider, query }: Asked) => {
      if (provider === 'notes') {
        return query === 'x' ? slow.answer : Promise.resolve(answer(group('notes', [])));
      }
      const files = query === 'x' ? [entry('x.rs')] : [entry('a.rs'), entry('b.rs')];
      return Promise.resolve(answer(group('files', files)));
    });
    const mentions = mentionsWithKinds({ files: 10, notes: 0 });

    mentions.update('@', 1);
    await settle();
    mentions.update('@x', 2);
    // The list still shows the entries for "@" while it asks about "@x".
    mentions.move(1);
    const moved = keyOf(mentions.active);
    await settle();
    const settledWithoutNotes = mentions.settled;
    slow.give(answer(group('notes', [])));
    await settle();

    expect(moved).toBe('b.rs');
    expect(keyOf(mentions.active)).toBe('x.rs');
    expect(settledWithoutNotes).toBe(false);
    expect(mentions.settled).toBe(true);
  });

  it('gives an early Enter nothing once the owner types on', async () => {
    offer({ files: starting(entry('a.rs')) });
    const mentions = mentionsWithoutKinds();

    mentions.update('@a', 2);
    const chosen = mentions.whenAnswered();
    mentions.update('@ab', 3);
    await settle();

    expect(await chosen).toBeUndefined();
    expect(mentions.settled).toBe(true);
  });

  it('follows a name with a space while the list shows it', async () => {
    const host = entry('host-1', { label: 'build box', token: '@"build box"', variant: 'host' });
    offer({ files: starting(entry('build.rs')), hosts: holding(host) });
    const mentions = mentionsWithoutKinds();

    typeOn(mentions, 'Ask @build');
    await settle();
    typeOn(mentions, 'Ask @build b', 'Ask @build'.length);
    await settle();

    expect(mentions.open).toBe(true);
    expect(core.suggestMentions).toHaveBeenLastCalledWith({ place: PLACE, provider: 'hosts', query: 'build b' });
    expect(shown(mentions)).toEqual(['host-1']);
    const inserted = mentions.choose('Ask @build b', first(mentions));
    expect(inserted.text).toBe('Ask @"build box" ');
    mentions.update(inserted.text, inserted.caret);
    expect(mentions.open).toBe(false);
  });

  it('decides about a space that comes before the first answer once the answers arrive', async () => {
    const host = entry('host-1', { label: 'build box', token: '@"build box"', variant: 'host' });
    offer({ files: () => [], hosts: holding(host) });
    const mentions = mentionsWithoutKinds();

    typeOn(mentions, '@build b');
    const whileWaiting = { open: mentions.open, settled: mentions.settled };
    const chosen = mentions.whenAnswered();
    await settle();
    await settle();

    expect(whileWaiting).toEqual({ open: true, settled: false });
    expect(keyOf(await chosen)).toBe('host-1');
    expect(core.suggestMentions).toHaveBeenLastCalledWith({ place: PLACE, provider: 'hosts', query: 'build b' });
  });

  it('ends with a token at the space after it, and closes when no entry holds the query', async () => {
    offer({ files: starting(entry('src/main.rs'), entry('build.rs')) });
    const mentions = mentionsWithoutKinds();

    typeOn(mentions, '@src/main.rs');
    await settle();
    const calls = core.suggestMentions.mock.calls.length;
    mentions.update('@src/main.rs ', 13);
    const afterWholeToken = mentions.open;
    mentions.update('@src/main.rs and', 16);
    typeOn(mentions, 'and @bu');
    await settle();
    mentions.update('and @bu x', 9);

    expect(afterWholeToken).toBe(false);
    expect(mentions.open).toBe(false);
    expect(core.suggestMentions.mock.calls.length).toBe(calls + 1);
    expect(mentions.refs('@src/main.rs and')).toEqual([{ key: 'src/main.rs', provider: 'files' }]);
  });

  it('forgets everything once the message is sent', async () => {
    offer({ files: starting(entry('a.rs')) });
    const mentions = mentionsWithoutKinds();

    mentions.update('@a', 2);
    await settle();
    mentions.choose('@a', first(mentions));
    mentions.reset();

    expect(mentions.refs('@a.rs ')).toEqual([]);
    expect(mentions.open).toBe(false);
  });
});
