// The files that the Images plugin takes from a paste or a drop, and their
// bytes as base64 for the core (docs/features/images.md, adding images).
import { describe, expect, it } from 'vitest';

import { imageDataUrl } from '../../../../../../apps/desktop/src/lib/utils/image-data.js';
import { readAsBase64, takesImageFile } from '../../../../../../apps/desktop/src/lib/utils/images-files.js';

function file(name: string, type: string, bytes: Uint8Array<ArrayBuffer> = new Uint8Array([0])): File {
  return new File([bytes], name, { type });
}

describe('takesImageFile', () => {
  it('takes PNG, JPEG, GIF and WebP by their type, or by their name without one', () => {
    expect(takesImageFile(file('a.png', 'image/png'))).toBe(true);
    expect(takesImageFile(file('a', 'image/webp'))).toBe(true);
    expect(takesImageFile(file('PHOTO.JPG', ''))).toBe(true);
    expect(takesImageFile(file('a.gif', ''))).toBe(true);
  });

  it('refuses other files, whatever their name says when they have a type', () => {
    expect(takesImageFile(file('a.svg', 'image/svg+xml'))).toBe(false);
    expect(takesImageFile(file('a.png', 'text/plain'))).toBe(false);
    expect(takesImageFile(file('notes.txt', ''))).toBe(false);
  });
});

describe('readAsBase64', () => {
  it('encodes a file’s bytes, also across the chunks it reads them in', async () => {
    const bytes = new Uint8Array(100_000).map((_byte, index) => index % 256);

    const encoded = await readAsBase64(file('a.png', 'image/png', bytes));

    let binary = '';
    for (const byte of bytes) {
      binary += String.fromCodePoint(byte);
    }
    expect(encoded).toBe(btoa(binary));
    expect(await readAsBase64(file('b.png', 'image/png', new Uint8Array([0xff, 0x00, 0x7f])))).toBe('/wB/');
  });
});

describe('imageDataUrl', () => {
  it('names the media type of the format', () => {
    expect(imageDataUrl('png', 'AAAA')).toBe('data:image/png;base64,AAAA');
    expect(imageDataUrl('jpeg', 'BBBB')).toBe('data:image/jpeg;base64,BBBB');
  });
});
