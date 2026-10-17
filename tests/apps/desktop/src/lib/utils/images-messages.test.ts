// Every code of the Images plugin has a message of its own, which names the
// image it is about (ADR-0019).
import { describe, expect, it } from 'vitest';

import type { ImagesError } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { formatCount } from '../../../../../../apps/desktop/src/lib/utils/chat-errors.js';
import {
  imagesErrorMessage,
  imagesRefusalMessage,
} from '../../../../../../apps/desktop/src/lib/utils/images-messages.js';

const ERRORS: ImagesError[] = [
  { code: 'plugin_off' },
  { code: 'too_large', maxBytes: 20_971_520 },
  { code: 'too_wide', maxSide: 12_000 },
  { code: 'too_many_pixels', maxMegapixels: 64 },
  { code: 'cannot_shrink', maxBytes: 5_242_880 },
  { code: 'unsupported_format' },
  { code: 'unreadable' },
  { code: 'file_unreadable' },
  { code: 'not_found' },
  { code: 'dialog_failed' },
  { code: 'store_unavailable' },
];

describe('images messages', () => {
  it('say what each error means and name the image', () => {
    const messages = ERRORS.map((error) => imagesErrorMessage({ error, kind: 'core' }, 'shot.png'));

    for (const message of messages) {
      expect(message.trim()).not.toBe('');
    }
    expect(new Set(messages).size).toBe(messages.length);
    expect(imagesErrorMessage({ error: { code: 'unreadable' }, kind: 'core' }, 'shot.png')).toContain('shot.png');
    expect(imagesErrorMessage({ kind: 'ipc' }, 'shot.png')).not.toBe('');
  });

  it('give the limits in the owner’s format', () => {
    const tooLarge = imagesErrorMessage({ error: { code: 'too_large', maxBytes: 20_971_520 }, kind: 'core' }, 'a.png');
    const tooWide = imagesErrorMessage({ error: { code: 'too_wide', maxSide: 12_000 }, kind: 'core' }, 'a.png');

    expect(tooLarge).toContain('20 MiB');
    expect(tooWide).toContain(formatCount(12_000));
  });

  it('word the source’s refusal of a key it no longer holds, and nothing else', () => {
    expect(imagesRefusalMessage('not_found')).not.toBeNull();
    expect(imagesRefusalMessage('invalid_image')).toBeNull();
  });
});
