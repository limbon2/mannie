// The mentions of the message that the owner writes (docs/features/chat/
// mentions.md): the "@" list that follows the caret and asks the core once
// the owner pauses, answers that arrive too late, the choice of an entry,
// and the references that the message carries.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  MentionEntry,
  MentionGroup,
  MentionPlace,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import {
  type MentionChoice,
  MessageMentions,
  SUGGESTION_DELAY_MS,
} from '../../../../../../apps/desktop/src/lib/state/message-mentions.svelte.js';
import type { MentionKind } from '../../../../../../apps/desktop/src/lib/types/shell-slots.js';

const core = vi.hoisted(() => ({ suggestMentions: vi.fn() }));

vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: core }));

const PLACE: MentionPlace = { kind: 'new_conversation', projectId: null };

function entry(key: string, extra: Partial<MentionEntry> = {}): MentionEntry {
  return { expands: false, key, label: key, token: `@${key}`, variant: 'file', warning: null, ...extra };
}

function group(provider: string, ...entries: MentionEntry[]): MentionGroup {
  return { entries, failure: null, incomplete: false, provider };
}

function answer(...groups: MentionGroup[]): { data: { groups: MentionGroup[] }; status: 'ok' } {
  return { data: { groups }, status: 'ok' };
}

function mentionsWithoutKinds(): MessageMentions {
  return new MessageMentions(
    () => PLACE,
    () => undefined,
  );
}

/** Lets the delay pass and the core's answer arrive. */
async function settle(): Promise<void> {
  await vi.advanceTimersByTimeAsync(SUGGESTION_DELAY_MS);
  for (let step = 0; step < 5; step += 1) {
    await Promise.resolve();
  }
}

function first(mentions: MessageMentions): MentionChoice {
  const choice = mentions.choices.at(0);
  if (choice === undefined) {
    throw new Error('the list shows an entry');
  }
  return choice;
}

beforeEach(() => {
  vi.useFakeTimers();
  core.suggestMentions.mockReset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('MessageMentions', () => {
  it('asks the core once the owner pauses, with what they typed after "@"', async () => {
    core.suggestMentions.mockResolvedValue(answer(group('files', entry('src/main.rs'))));
    const mentions = mentionsWithoutKinds();

    mentions.update('Read @s', 7);
    mentions.update('Read @sr', 8);
    const whileWaiting = mentions.status;
    await settle();

    expect(whileWaiting).toBe('loading');
    expect(core.suggestMentions).toHaveBeenCalledTimes(1);
    expect(core.suggestMentions).toHaveBeenCalledWith({ place: PLACE, query: 'sr' });
    expect(mentions.status).toBe('ready');
    expect(mentions.active?.entry.key).toBe('src/main.rs');
  });

  it('drops an answer that a later query replaced', async () => {
    let answerFirst: (value: unknown) => void = () => undefined;
    core.suggestMentions
      .mockImplementationOnce(
        async () =>
          new Promise((resolve) => {
            answerFirst = resolve;
          }),
      )
      .mockResolvedValueOnce(answer(group('files', entry('second.rs'))));
    const mentions = mentionsWithoutKinds();

    mentions.update('@f', 2);
    await settle();
    mentions.update('@se', 3);
    await settle();
    answerFirst(answer(group('files', entry('first.rs'))));
    await settle();

    expect(mentions.choices.map((choice) => choice.entry.key)).toEqual(['second.rs']);
  });

  it('closes outside a token, and an answer on its way changes nothing then', async () => {
    core.suggestMentions.mockResolvedValue(answer(group('files', entry('a.rs'))));
    const mentions = mentionsWithoutKinds();

    mentions.update('@a', 2);
    mentions.update('@a b', 4);
    await settle();

    expect(mentions.open).toBe(false);
    expect(mentions.status).toBe('closed');
    expect(core.suggestMentions).not.toHaveBeenCalled();
  });

  it('shows why the core listed nothing', async () => {
    core.suggestMentions.mockResolvedValue({
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
  });

  it('inserts a chosen entry and sends a reference for each token of the text', async () => {
    core.suggestMentions.mockResolvedValue(answer(group('files', entry('a.rs'), entry('b.rs'))));
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
    core.suggestMentions.mockResolvedValue(answer(group('files', entry('a.rs'), entry('b.rs'))));
    const mentions = mentionsWithoutKinds();

    mentions.update('@b', 2);
    await settle();
    const typed = mentions.refs('Look at @b.rs and @c.rs');
    const deleted = mentions.refs('Look at and @c.rs');

    expect(typed).toEqual([{ key: 'b.rs', provider: 'files' }]);
    expect(deleted).toEqual([]);
  });

  it('keeps the list open inside a folder that expands', async () => {
    core.suggestMentions
      .mockResolvedValueOnce(
        answer(group('files', entry('docs', { expands: true, token: '@docs/', variant: 'folder' }))),
      )
      .mockResolvedValueOnce(answer(group('files', entry('docs/guide.md'))));
    const mentions = mentionsWithoutKinds();

    mentions.update('@do', 3);
    await settle();
    const inserted = mentions.choose('@do', first(mentions));
    await settle();

    expect(inserted).toEqual({ caret: 6, text: '@docs/' });
    expect(mentions.open).toBe(true);
    expect(core.suggestMentions).toHaveBeenLastCalledWith({ place: PLACE, query: 'docs/' });
    expect(first(mentions).entry.key).toBe('docs/guide.md');
  });

  it('moves the highlight around the ends and orders the groups by their kinds', async () => {
    core.suggestMentions.mockResolvedValue(
      answer(group('notes', entry('note')), group('files', entry('a.rs'), entry('b.rs'))),
    );
    const kinds = new Map<string, Pick<MentionKind, 'order'>>([
      ['files', { order: 0 }],
      ['notes', { order: 10 }],
    ]);
    const mentions = new MessageMentions(
      () => PLACE,
      (provider) => kinds.get(provider) as MentionKind | undefined,
    );

    mentions.update('@', 1);
    await settle();
    mentions.move(-1);
    const wrapped = mentions.active?.entry.key;
    mentions.move(1);

    expect(mentions.groups.map((shown) => shown.provider)).toEqual(['files', 'notes']);
    expect(wrapped).toBe('note');
    expect(mentions.active?.entry.key).toBe('a.rs');
  });

  it('keeps the list closed after Escape until the owner types in the token or leaves it', async () => {
    core.suggestMentions.mockResolvedValue(answer(group('files', entry('a.rs'))));
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
    mentions.update('@ab', 3);

    expect(afterEscape).toBe(false);
    expect(afterTyping).toBe(true);
    expect(mentions.open).toBe(true);
  });

  it('lets an Enter before the answer choose from the answer for what the owner typed', async () => {
    core.suggestMentions
      .mockResolvedValueOnce(
        answer(
          group('files', entry('docs', { expands: true, token: '@docs/', variant: 'folder' }), entry('docs/old.md')),
        ),
      )
      .mockResolvedValueOnce(answer(group('files', entry('docs/guide.md'))));
    const mentions = mentionsWithoutKinds();

    mentions.update('@do', 3);
    await settle();
    mentions.choose('@do', first(mentions));
    const settledInsideTheFolder = mentions.settled;
    const shownInsideTheFolder = mentions.groups.length;
    const chosen = mentions.whenAnswered();
    await settle();

    expect(settledInsideTheFolder).toBe(false);
    expect(shownInsideTheFolder).toBe(0);
    expect((await chosen)?.entry.key).toBe('docs/guide.md');
  });

  it('gives an early Enter nothing once the owner types on', async () => {
    core.suggestMentions.mockResolvedValue(answer(group('files', entry('a.rs'))));
    const mentions = mentionsWithoutKinds();

    mentions.update('@a', 2);
    const chosen = mentions.whenAnswered();
    mentions.update('@ab', 3);
    await settle();

    expect(await chosen).toBeUndefined();
    expect(mentions.settled).toBe(true);
  });

  it('forgets everything once the message is sent', async () => {
    core.suggestMentions.mockResolvedValue(answer(group('files', entry('a.rs'))));
    const mentions = mentionsWithoutKinds();

    mentions.update('@a', 2);
    await settle();
    mentions.choose('@a', first(mentions));
    mentions.reset();

    expect(mentions.refs('@a.rs ')).toEqual([]);
    expect(mentions.open).toBe(false);
  });
});
