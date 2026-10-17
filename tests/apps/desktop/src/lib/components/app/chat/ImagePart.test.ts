// An image of the owner's message in the transcript (docs/features/images.md,
// the transcript), rendered on the server side: a placeholder in the image's
// proportions while its thumbnail loads, and nothing for a part of another
// kind. A server render runs no effect, so nothing loads here.
import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';

import ImagePart from '../../../../../../../../apps/desktop/src/lib/components/app/chat/ImagePart.svelte';
import type { PartView } from '../../../../../../../../apps/desktop/src/lib/generated/bindings.js';

function shown(content: PartView['content']): string {
  const part: PartView = {
    content,
    createdAt: 0,
    id: 'part-1',
    position: 1,
    runId: null,
    state: 'complete',
    turnId: 'turn-0',
  };
  const { body } = render(ImagePart, {
    props: { conversationId: '0199e0f2-0000-7000-8000-000000000001', latest: true, part },
  });
  return body;
}

describe('ImagePart', () => {
  it('holds the image’s place in its proportions while the thumbnail loads', () => {
    const body = shown({
      image: { bytes: 1_000, format: 'png', height: 600, name: 'shot.png', width: 800 },
      kind: 'image',
    });

    expect(body).toContain('data-image-part');
    expect(body).toContain('aspect-ratio: 800 / 600');
    expect(body).toContain('Loading the image…');
  });

  it('shows nothing for a part of another kind', () => {
    const body = shown({ kind: 'user_text', text: 'Look' });

    expect(body).not.toContain('data-image-part');
  });
});
