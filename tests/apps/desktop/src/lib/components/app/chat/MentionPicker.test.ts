// The "@" list above the message box (docs/features/chat/mentions.md, the
// list), rendered on the server side: each kind under its heading as soon as
// it answers, the option that shows the rest of a kind's entries, the line
// that says that a kind still looks for matches, and the list's empty state.
import type { Component } from 'svelte';
import { render } from 'svelte/server';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import MentionPicker from '../../../../../../../../apps/desktop/src/lib/components/app/chat/MentionPicker.svelte';
import type {
  MentionEntry,
  MentionGroup,
  MentionPlace,
} from '../../../../../../../../apps/desktop/src/lib/generated/bindings.js';
import {
  type MentionOption,
  MessageMentions,
  SUGGESTION_DELAY_MS,
} from '../../../../../../../../apps/desktop/src/lib/state/message-mentions.svelte.js';
import { setShellSlots, ShellSlots } from '../../../../../../../../apps/desktop/src/lib/state/shell-slots.svelte.js';
import {
  fileMentionIcon,
  fileMentionRefusal,
  fileMentionSummary,
  fileMentionVariantName,
  fileMentionWarning,
  FILES_PROVIDER,
} from '../../../../../../../../apps/desktop/src/lib/utils/file-mentions.js';

const core = vi.hoisted(() => ({ listMentionProviders: vi.fn(), suggestMentions: vi.fn() }));

vi.mock('../../../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: core }));

const PLACE: MentionPlace = { kind: 'new_conversation', projectId: null };

interface PickerProps {
  readonly listId: string;
  readonly mentions: MessageMentions;
  readonly onTake: (option: MentionOption) => void;
  readonly optionId: (index: number) => string;
}

/** The shell's slots with the core's kind of mentions, files and folders. */
function slotsWithFiles(): ShellSlots {
  const slots = new ShellSlots();
  slots.mentionKinds.register({
    group: () => 'Files and folders',
    icon: fileMentionIcon,
    id: FILES_PROVIDER,
    order: 0,
    refusal: fileMentionRefusal,
    summary: fileMentionSummary,
    variantName: fileMentionVariantName,
    warning: fileMentionWarning,
  });
  return slots;
}

/** The picker with `slots` in its context, as the composition root provides them. */
function withSlots(slots: ShellSlots): Component<PickerProps> {
  return (...args) => {
    setShellSlots(slots);
    return MentionPicker(...args);
  };
}

function entry(key: string): MentionEntry {
  return { expands: false, key, label: key, token: `@${key}`, variant: 'file', warning: null };
}

function numbered(prefix: string, count: number): MentionEntry[] {
  return Array.from({ length: count }, (_value, index) => entry(`${prefix}${String(index)}`));
}

function answer(provider: string, entries: MentionEntry[]): { data: { groups: MentionGroup[] }; status: 'ok' } {
  return { data: { groups: [{ entries, failure: null, incomplete: false, provider }] }, status: 'ok' };
}

/** An answer that never arrives, as from a kind that is still looking. */
async function silence(): Promise<never> {
  return new Promise<never>(() => {
    // The kind never answers.
  });
}

/** The core offers `providers`, and answers each with the entries of `entries`, or never for `silent` ones. */
function offer(providers: string[], entries: Partial<Record<string, MentionEntry[]>>, silent: string[] = []): void {
  core.listMentionProviders.mockResolvedValue({ data: { providers }, status: 'ok' });
  core.suggestMentions.mockImplementation(async ({ provider }: { provider: string }) => {
    if (silent.includes(provider)) {
      return silence();
    }
    return Promise.resolve(answer(provider, entries[provider] ?? []));
  });
}

/** The list of "@" in a message box, once the kinds that answer did. */
async function openList(): Promise<MessageMentions> {
  const mentions = new MessageMentions(
    () => PLACE,
    () => undefined,
  );
  mentions.update('@', 1);
  await vi.advanceTimersByTimeAsync(SUGGESTION_DELAY_MS);
  for (let step = 0; step < 10; step += 1) {
    await Promise.resolve();
  }
  return mentions;
}

/** The markup of the list. */
function shown(mentions: MessageMentions): string {
  const props: PickerProps = {
    listId: 'mentions',
    mentions,
    onTake: () => undefined,
    optionId: (index) => `option-${String(index)}`,
  };
  return render(withSlots(slotsWithFiles()), { props }).body;
}

/** The labels of the entries, in order. */
function labels(body: string): string[] {
  return [...body.matchAll(/<span[^>]*data-content[^>]*>([^<]*)<\/span>/gu)].map((match) => match[1]);
}

beforeEach(() => {
  vi.useFakeTimers();
  core.listMentionProviders.mockReset();
  core.suggestMentions.mockReset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('MentionPicker', () => {
  it('shows the first entries of each kind under its heading, with an option that shows the rest', async () => {
    offer([FILES_PROVIDER, 'notes'], { [FILES_PROVIDER]: numbered('file', 8), notes: [entry('note')] });

    const body = shown(await openList());

    expect(labels(body)).toEqual(['file0', 'file1', 'file2', 'file3', 'file4', 'note']);
    expect(body).toContain('aria-label="Files and folders"');
    expect(body).toContain('aria-label="Other mentions"');
    expect(body.match(/role="option"/gu)).toHaveLength(7);
    expect(body).toMatch(/data-mention-more[^>]*>\s*Show 3 more\s*</u);
    expect(body).toContain('6 matches.');
    expect(body).not.toContain('data-mention-looking');
  });

  it('shows a kind that answered while another still looks for matches', async () => {
    offer([FILES_PROVIDER, 'notes'], { [FILES_PROVIDER]: [entry('a.rs')] }, ['notes']);

    const body = shown(await openList());

    expect(labels(body)).toEqual(['a.rs']);
    expect(body).toContain('data-mention-looking');
    expect(body).toContain('Looking for matches…');
  });

  it('says that nothing matches once every kind answered without entries', async () => {
    offer([FILES_PROVIDER, 'notes'], {});

    const body = shown(await openList());

    expect(labels(body)).toEqual([]);
    expect(body).toContain('Nothing matches.');
    expect(body).not.toContain('role="listbox"');
  });
});
