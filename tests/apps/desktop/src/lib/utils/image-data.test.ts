// The bytes of images as the core sends them, as `data:` URLs that the
// webview shows (ADR-0033).
import { describe, expect, it } from 'vitest';

import { imageDataUrl } from '../../../../../../apps/desktop/src/lib/utils/image-data.js';

describe('imageDataUrl', () => {
  it('names the media type of the format', () => {
    expect(imageDataUrl('png', 'AAAA')).toBe('data:image/png;base64,AAAA');
    expect(imageDataUrl('jpeg', 'BBBB')).toBe('data:image/jpeg;base64,BBBB');
  });
});
