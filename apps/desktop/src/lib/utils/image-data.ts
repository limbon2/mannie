// The bytes of images as the core sends them: base64 with their format
// (ADR-0033). The Content Security Policy allows images from `data:` URLs, so
// the webview shows them without any other access (ADR-0004).
import type { ImageFormat } from '#lib/generated/bindings.js';

const MEDIA_TYPES: Record<ImageFormat, string> = {
  jpeg: 'image/jpeg',
  png: 'image/png',
};

/** An image's bytes, as base64 in its format, as a `data:` URL. */
export function imageDataUrl(format: ImageFormat, base64: string): string {
  return `data:${MEDIA_TYPES[format]};base64,${base64}`;
}
