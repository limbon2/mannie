// The unsent messages of message boxes (ADR-0068, decision 7): a box that
// goes keeps its message for its conversation, the next box of that
// conversation takes it once, and an empty message keeps nothing.
import { describe, expect, it } from 'vitest';

import { messageBoxes } from '../../../../../../apps/desktop/src/lib/state/message-boxes.svelte.js';
import { MessageImages } from '../../../../../../apps/desktop/src/lib/state/message-images.svelte.js';

describe('unsent messages', () => {
  it('come back once to the next box of their conversation, and to no other', () => {
    const images = new MessageImages();
    messageBoxes.keepUnsent('conversation-a', { images, mentions: [], text: 'Half a thought' });

    const other = messageBoxes.takeUnsent('conversation-b');
    const taken = messageBoxes.takeUnsent('conversation-a');
    const again = messageBoxes.takeUnsent('conversation-a');

    expect(other).toBeUndefined();
    expect(taken).toEqual({ images, mentions: [], text: 'Half a thought' });
    expect(again).toBeUndefined();
  });

  it('keep nothing for an empty message, and an empty one replaces a kept one', () => {
    messageBoxes.keepUnsent('conversation-c', { images: new MessageImages(), mentions: [], text: 'Draft' });
    messageBoxes.keepUnsent('conversation-c', { images: new MessageImages(), mentions: [], text: '' });

    expect(messageBoxes.takeUnsent('conversation-c')).toBeUndefined();
  });

  it('keep a message that holds only images', () => {
    const images = new MessageImages();
    images.begin('files', 'diagram.png');
    messageBoxes.keepUnsent('conversation-d', { images, mentions: [], text: '' });

    expect(messageBoxes.takeUnsent('conversation-d')?.images.images).toHaveLength(1);
  });
});
