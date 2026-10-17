// A mention below the owner's message (docs/features/chat/mentions.md, the
// transcript), rendered on the server side: its label and what the model got
// in the words of its kind, generic words for a provider without a kind, such
// as one of a plugin that is off (ADR-0032, decision 4), and nothing for a
// part of another kind.
import type { Component } from 'svelte';
import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';

import MentionPart from '../../../../../../../../apps/desktop/src/lib/components/app/chat/MentionPart.svelte';
import type { MentionView, PartView } from '../../../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { setShellSlots, ShellSlots } from '../../../../../../../../apps/desktop/src/lib/state/shell-slots.svelte.js';
import type { PartRendererProps } from '../../../../../../../../apps/desktop/src/lib/types/shell-slots.js';
import {
  fileMentionIcon,
  fileMentionRefusal,
  fileMentionSummary,
  fileMentionVariantName,
  fileMentionWarning,
  FILES_PROVIDER,
} from '../../../../../../../../apps/desktop/src/lib/utils/file-mentions.js';

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

/** The renderer with `slots` in its context, as the composition root provides them. */
function withSlots(slots: ShellSlots): Component<PartRendererProps> {
  return (...args) => {
    setShellSlots(slots);
    return MentionPart(...args);
  };
}

function mention(extra: Partial<MentionView> = {}): MentionView {
  return {
    key: 'src/main.rs',
    label: 'src/main.rs',
    provider: FILES_PROVIDER,
    summary: { code: 'lines', values: { first: 1, last: 120, total: 120 } },
    variant: 'file',
    ...extra,
  };
}

/** The markup of a part with `content`. */
function shown(slots: ShellSlots, content: PartView['content']): string {
  const part: PartView = {
    content,
    createdAt: 0,
    id: 'part-1',
    position: 1,
    runId: null,
    state: 'complete',
    turnId: 'turn-0',
  };
  const { body } = render(withSlots(slots), {
    props: { conversationId: '0199e0f2-0000-7000-8000-000000000001', latest: true, part },
  });
  return body;
}

describe('MentionPart', () => {
  it('shows the label and what the model got, in the words of the mention’s kind', () => {
    const body = shown(slotsWithFiles(), { kind: 'mention', mention: mention() });

    expect(body).toMatch(/<div[^>]*data-mention="files"/u);
    expect(body).toMatch(/<span[^>]*data-content[^>]*>src\/main\.rs<\/span>/u);
    expect(body).toContain('<span class="sr-only">File</span>');
    expect(body).toContain('120 lines');
  });

  it('shows the label with generic words for a provider without a kind', () => {
    const body = shown(new ShellSlots(), {
      kind: 'mention',
      mention: mention({ key: 'plan', label: 'Release plan', provider: 'notes', variant: 'note' }),
    });

    expect(body).toMatch(/<div[^>]*data-mention="notes"/u);
    expect(body).toMatch(/<span[^>]*data-content[^>]*>Release plan<\/span>/u);
    expect(body).toContain('<span class="sr-only">Mention</span>');
    expect(body).not.toContain('120 lines');
  });

  it('shows nothing for a part of another kind', () => {
    const body = shown(slotsWithFiles(), { kind: 'user_text', text: 'Read the notes' });

    expect(body).not.toContain('data-mention');
    expect(body).not.toContain('Read the notes');
  });
});
