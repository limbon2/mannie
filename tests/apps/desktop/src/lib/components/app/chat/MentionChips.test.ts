// The chips below the message box (docs/features/chat/mentions.md, mentions
// in the message), rendered on the server side: a chip for each mention with
// its label, the warning of a file that may hold secrets in the words of its
// kind, and generic words for a provider without a kind (ADR-0032,
// decision 4).
import type { Component } from 'svelte';
import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';

import MentionChips from '../../../../../../../../apps/desktop/src/lib/components/app/chat/MentionChips.svelte';
import type { MentionEntry } from '../../../../../../../../apps/desktop/src/lib/generated/bindings.js';
import type { MentionChoice } from '../../../../../../../../apps/desktop/src/lib/state/message-mentions.svelte.js';
import { setShellSlots, ShellSlots } from '../../../../../../../../apps/desktop/src/lib/state/shell-slots.svelte.js';
import {
  fileMentionIcon,
  fileMentionRefusal,
  fileMentionSummary,
  fileMentionVariantName,
  fileMentionWarning,
  FILES_PROVIDER,
} from '../../../../../../../../apps/desktop/src/lib/utils/file-mentions.js';

interface ChipsProps {
  readonly mentions: readonly MentionChoice[];
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

/** The chips with `slots` in their context, as the composition root provides them. */
function withSlots(slots: ShellSlots): Component<ChipsProps> {
  return (...args) => {
    setShellSlots(slots);
    return MentionChips(...args);
  };
}

function choice(provider: string, key: string, extra: Partial<MentionEntry> = {}): MentionChoice {
  return {
    entry: { expands: false, key, label: key, token: `@${key}`, variant: 'file', warning: null, ...extra },
    provider,
  };
}

/** The markup of the chips of `mentions`. */
function shown(slots: ShellSlots, mentions: MentionChoice[]): string {
  return render(withSlots(slots), { props: { mentions } }).body;
}

/** The labels of the chips, in order. */
function labels(body: string): string[] {
  return [...body.matchAll(/<span[^>]*data-content[^>]*>([^<]*)<\/span>/gu)].map((match) => match[1]);
}

describe('MentionChips', () => {
  it('shows a chip for each mention, and warns about a file that may hold secrets', () => {
    const body = shown(slotsWithFiles(), [
      choice(FILES_PROVIDER, 'src/main.rs'),
      choice(FILES_PROVIDER, '.env', { warning: 'secret' }),
    ]);

    expect(body).toMatch(/<ul[^>]*aria-label="Mentioned in this message"/u);
    expect(labels(body)).toEqual(['src/main.rs', '.env']);
    expect(body.match(/data-mention-warning/gu)).toHaveLength(1);
    expect(body).toContain('may hold secrets');
  });

  it('shows generic words for a provider without a kind', () => {
    const body = shown(new ShellSlots(), [choice('notes', 'plan', { variant: 'note', warning: 'private' })]);

    expect(labels(body)).toEqual(['plan']);
    expect(body).toContain('<span class="sr-only">Mention</span>');
    expect(body).toContain('check this before you send it');
  });
});
