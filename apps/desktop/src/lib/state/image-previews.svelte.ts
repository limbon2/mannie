// The thumbnails of the images in transcripts (ADR-0033, decision 4). The
// views of image parts carry no bytes, so each thumbnail loads once, when its
// part shows, and the most recent ones stay for when a conversation opens
// again. The full image loads each time the viewer opens it.
import { SvelteMap } from 'svelte/reactivity';

import { type ChatError, commands, type ImageSize } from '#lib/generated/bindings.js';
import type { Outcome } from '#lib/types/calls.js';
import type { ChatFailure } from '#lib/types/chat.js';
import { call } from '#lib/utils/calls.js';
import { imageDataUrl } from '#lib/utils/image-data.js';

/** The most thumbnails that stay loaded. */
const MAX_THUMBNAILS = 200;

/** A thumbnail as it loads. */
export type Preview =
  | { readonly failure: ChatFailure; readonly status: 'failed' }
  | { readonly status: 'loading' }
  | { readonly status: 'ready'; readonly url: string };

const LOADING: Preview = { status: 'loading' };

/** Reads an image of a part as a `data:` URL. */
export async function loadImage(
  conversationId: string,
  partId: string,
  size: ImageSize,
): Promise<Outcome<string, ChatError>> {
  const outcome = await call(() => commands.readImage({ conversationId, partId, size }));
  if (!outcome.ok) {
    return outcome;
  }
  return { data: imageDataUrl(outcome.data.format, outcome.data.data), ok: true };
}

class ImagePreviews {
  readonly #thumbnails = new SvelteMap<string, Preview>();

  /** The thumbnail of an image part: loading until `load` has it. */
  thumbnail(partId: string): Preview {
    return this.#thumbnails.get(partId) ?? LOADING;
  }

  /** Loads the thumbnail of an image part, unless it is loaded or loading. */
  load(conversationId: string, partId: string): void {
    if (this.#thumbnails.has(partId)) {
      return;
    }
    this.#thumbnails.set(partId, LOADING);
    void this.#fetch(conversationId, partId);
  }

  /** Loads a thumbnail again after it failed. */
  retry(conversationId: string, partId: string): void {
    this.#thumbnails.delete(partId);
    this.load(conversationId, partId);
  }

  async #fetch(conversationId: string, partId: string): Promise<void> {
    const outcome = await loadImage(conversationId, partId, 'thumbnail');
    const preview: Preview = outcome.ok
      ? { status: 'ready', url: outcome.data }
      : { failure: outcome.failure, status: 'failed' };
    this.#thumbnails.set(partId, preview);
    // A map keeps the order in which keys were added, so the oldest go first.
    for (const key of this.#thumbnails.keys()) {
      if (this.#thumbnails.size <= MAX_THUMBNAILS) {
        break;
      }
      this.#thumbnails.delete(key);
    }
  }
}

/** The thumbnails of the window's transcripts. */
export const imagePreviews = new ImagePreviews();
