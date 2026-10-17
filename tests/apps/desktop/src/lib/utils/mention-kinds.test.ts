// How a mention shows through its kind, or with generic texts without one
// (ADR-0032, decision 4), and the order of the list's groups.
import type { Component } from 'svelte';
import { describe, expect, it } from 'vitest';

import type { MentionGroup } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import type { MentionKind } from '../../../../../../apps/desktop/src/lib/types/shell-slots.js';
import {
  mentionGroupHeading,
  mentionIcon,
  mentionRefusal,
  mentionSummary,
  mentionVariantName,
  mentionWarning,
  sortMentionGroups,
} from '../../../../../../apps/desktop/src/lib/utils/mention-kinds.js';

/** A stand-in for an icon component. */
const NOTE_ICON = (() => undefined) as unknown as Component;

const NOTES: MentionKind = {
  group: () => 'Notes',
  icon: () => NOTE_ICON,
  id: 'notes',
  order: 5,
  refusal: (code) => (code === 'gone' ? 'The note is gone.' : null),
  summary: (summary) => (summary.code === 'read' ? 'read' : null),
  variantName: () => 'Note',
  warning: () => 'private',
};

function group(provider: string): MentionGroup {
  return { entries: [], failure: null, incomplete: false, provider };
}

describe('the words of a mention', () => {
  it('come from the kind', () => {
    expect(mentionGroupHeading(NOTES)).toBe('Notes');
    expect(mentionIcon(NOTES, 'note')).toBe(NOTE_ICON);
    expect(mentionVariantName(NOTES, 'note')).toBe('Note');
    expect(mentionWarning(NOTES, 'secret')).toBe('private');
    expect(mentionRefusal(NOTES, 'gone')).toBe('The note is gone.');
    expect(mentionSummary(NOTES, { code: 'read' })).toBe('read');
  });

  it('are generic without a kind, or for a code that the kind does not know', () => {
    expect(mentionGroupHeading(undefined)).toBe('Other mentions');
    expect(mentionIcon(undefined, 'note')).not.toBe(NOTE_ICON);
    expect(mentionVariantName(undefined, 'note')).toBe('Mention');
    expect(mentionWarning(undefined, 'secret')).toBe('check this before you send it');
    expect(mentionRefusal(undefined, 'gone')).toBe('This mention is not available here.');
    expect(mentionRefusal(NOTES, 'other')).toBe('This mention is not available here.');
    expect(mentionSummary(undefined, { code: 'read' })).toBeNull();
  });
});

describe('sortMentionGroups', () => {
  it('orders the groups by their kinds, and groups without a kind last in their order', () => {
    const kinds = new Map([
      ['files', { ...NOTES, id: 'files', order: 0 }],
      ['notes', NOTES],
    ]);

    const sorted = sortMentionGroups([group('stray'), group('notes'), group('other'), group('files')], (provider) =>
      kinds.get(provider),
    );

    expect(sorted.map((entry) => entry.provider)).toEqual(['files', 'notes', 'stray', 'other']);
  });
});
