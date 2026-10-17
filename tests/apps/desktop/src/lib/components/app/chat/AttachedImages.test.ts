// The images of the message that the owner writes, below the message box
// (docs/features/images.md, adding images), rendered on the server side: a
// spinner while an image is prepared, its thumbnail once it is ready, a
// button that removes it, and why an image was not added.
import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';

import AttachedImages from '../../../../../../../../apps/desktop/src/lib/components/app/chat/AttachedImages.svelte';
import { MessageImages } from '../../../../../../../../apps/desktop/src/lib/state/message-images.svelte.js';

describe('AttachedImages', () => {
  it('shows a chip for each image, with its thumbnail once ready, and a button that removes it', () => {
    const images = new MessageImages();
    images.begin('images', 'ready.png')?.ready({
      discard: () => undefined,
      height: 1,
      key: 'k',
      thumbnailUrl: 'data:image/jpeg;base64,AAAA',
      width: 1,
    });
    images.begin('images', 'waiting.png');

    const { body } = render(AttachedImages, { props: { images } });

    expect(body).toContain('data-image-chips');
    expect(body).toContain('src="data:image/jpeg;base64,AAAA"');
    expect(body).toMatch(/<span[^>]*data-content[^>]*>ready\.png<\/span>/u);
    expect(body).toContain('aria-label="Remove waiting.png"');
    expect(body).toContain('Preparing the image…');
  });

  it('says why an image was not added, and shows no list without images', () => {
    const images = new MessageImages();
    images.refuse('notes.txt is not an image that can be added to a message.');

    const { body } = render(AttachedImages, { props: { images } });

    expect(body).not.toContain('data-image-chips');
    expect(body).toMatch(/role="alert"[^>]*>|data-image-failures/u);
    expect(body).toContain('notes.txt is not an image that can be added to a message.');
  });
});
