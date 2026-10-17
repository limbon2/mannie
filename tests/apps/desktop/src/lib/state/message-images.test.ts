// The images of the message that the owner writes (ADR-0033, decision 7;
// docs/features/images.md, adding images): a chip while a source prepares an
// image, the references of the ready ones in order, at most ten, a removed
// image that its source drops, and sentences that go once the owner adds
// other images.
import { describe, expect, it, vi } from 'vitest';

import {
  MAX_MESSAGE_IMAGES,
  MessageImages,
} from '../../../../../../apps/desktop/src/lib/state/message-images.svelte.js';
import type { ReadyImage } from '../../../../../../apps/desktop/src/lib/types/shell-slots.js';

function ready(key: string, discard = vi.fn()): ReadyImage {
  return { discard, height: 1, key, thumbnailUrl: `data:image/png;base64,${key}`, width: 1 };
}

describe('MessageImages', () => {
  it('shows an image while it is prepared and sends the ready ones in order', () => {
    const images = new MessageImages();

    const first = images.begin('images', 'one.png');
    const second = images.begin('images', 'two.png');
    const waiting = { hasReady: images.hasReady, preparing: images.preparing };
    second?.ready(ready('b'));
    first?.ready(ready('a'));

    expect(waiting).toEqual({ hasReady: false, preparing: true });
    expect(images.images.map((image) => image.name)).toEqual(['one.png', 'two.png']);
    expect(images.preparing).toBe(false);
    expect(images.refs()).toEqual([
      { key: 'a', source: 'images' },
      { key: 'b', source: 'images' },
    ]);
  });

  it('takes at most ten images and says so', () => {
    const images = new MessageImages();
    for (let index = 0; index < MAX_MESSAGE_IMAGES; index += 1) {
      images.begin('images', `${String(index)}.png`)?.ready(ready(String(index)));
    }

    const eleventh = images.begin('images', 'eleven.png');

    expect(eleventh).toBeNull();
    expect(images.images).toHaveLength(10);
    expect(images.failures).toEqual(['A message holds at most 10 images.']);
  });

  it('drops a failed image with its sentence, which goes once the owner adds other images', () => {
    const images = new MessageImages();

    images.begin('images', 'broken.png')?.fail('broken.png is damaged.');
    const afterFailure = { failures: [...images.failures], images: images.images.length };
    images.begin('images', 'good.png');

    expect(afterFailure).toEqual({ failures: ['broken.png is damaged.'], images: 0 });
    expect(images.failures).toEqual([]);
  });

  it('lets the source drop a removed image, also one that became ready after its removal', () => {
    const images = new MessageImages();
    const removedReady = vi.fn();
    const removedWhilePrepared = vi.fn();
    images.begin('images', 'a.png')?.ready(ready('a', removedReady));
    const pending = images.begin('images', 'b.png');

    for (const image of [...images.images]) {
      images.remove(image.id);
    }
    pending?.ready(ready('b', removedWhilePrepared));

    expect(removedReady).toHaveBeenCalledOnce();
    expect(removedWhilePrepared).toHaveBeenCalledOnce();
    expect(images.images).toEqual([]);
    expect(images.refs()).toEqual([]);
  });

  it('empties itself after a sent message without letting the sources drop the images', () => {
    const images = new MessageImages();
    const discard = vi.fn();
    images.begin('images', 'a.png')?.ready(ready('a', discard));
    images.refuse('Something was not added.');

    images.reset();

    expect(images.images).toEqual([]);
    expect(images.failures).toEqual([]);
    expect(discard).not.toHaveBeenCalled();
  });
});
