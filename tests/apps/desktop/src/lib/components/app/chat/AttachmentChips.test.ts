// The chips of a conversation's attachments below the message box (ADR-0046,
// decision 6), rendered on the server side: each chip detaches its
// attachment, except the chip of the attachment that the conversation
// belongs to, which stays (ADR-0051).
import type { Component } from 'svelte';
import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';

import AttachmentChips from '../../../../../../../../apps/desktop/src/lib/components/app/chat/AttachmentChips.svelte';
import type { Attachment, AttachmentRef } from '../../../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { setShellSlots, ShellSlots } from '../../../../../../../../apps/desktop/src/lib/state/shell-slots.svelte.js';

interface ChipsProps {
  readonly attachments: readonly Attachment[];
  readonly home?: AttachmentRef | null;
  readonly ondetach: (attachment: Attachment) => void;
}

/** The chips with the shell's slots in their context, as the composition root provides them. */
const Chips: Component<ChipsProps> = (...args) => {
  setShellSlots(new ShellSlots());
  return AttachmentChips(...args);
};

const PHYSICS: Attachment = { key: 'n1', label: 'Physics', provider: 'notebooks' };
const OPTICS: Attachment = { key: 'n2', label: 'Optics', provider: 'notebooks' };

describe('AttachmentChips', () => {
  it('detaches each attachment but the home', () => {
    const body = render(Chips, {
      props: { attachments: [PHYSICS, OPTICS], home: { key: 'n1', provider: 'notebooks' }, ondetach: () => undefined },
    }).body;

    expect(body).toContain('Physics');
    expect(body).toContain('data-home');
    expect(body).not.toContain('Detach Physics');
    expect(body).toContain('Detach Optics');
  });

  it('detaches every attachment of a conversation without a home', () => {
    const body = render(Chips, { props: { attachments: [PHYSICS], ondetach: () => undefined } }).body;

    expect(body).toContain('Detach Physics');
    expect(body).not.toContain('data-home');
  });
});
