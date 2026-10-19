// The images of the message that the owner writes (ADR-0033, decision 7;
// plugins/images/docs/README.md, adding images). Image sources add them through
// `begin`, each shows as a chip while its source prepares it, and the message
// sends the references of the ready ones. Sending keeps them when it fails,
// and the core lets the sources drop them once the message is stored.
import type { ImageRef } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import type { ImageTarget, PendingImage, ReadyImage } from '#lib/types/shell-slots.js';

/** The most images of one message, as the core's `MAX_IMAGES` says. */
export const MAX_MESSAGE_IMAGES = 10;

/** An image of the message, as its chip shows it. */
export interface MessageImage {
  /** The chip's ID within the message box. */
  readonly id: number;
  /** The image's name. */
  readonly name: string;
  /** The prepared image, or `null` while its source prepares it. */
  readonly ready: ReadyImage | null;
  /** The source's ID. */
  readonly source: string;
}

/** The images of one message box. */
export class MessageImages implements ImageTarget {
  #failures: string[] = $state([]);
  #images: MessageImage[] = $state([]);
  #nextId = 0;

  /** Why images could not be added, newest last. */
  get failures(): readonly string[] {
    return this.#failures;
  }

  /** The images, in the order the owner added them. */
  get images(): readonly MessageImage[] {
    return this.#images;
  }

  /** Whether a source still prepares an image, which the message waits for. */
  get preparing(): boolean {
    return this.#images.some((image) => image.ready === null);
  }

  /** Whether the message has an image to send. */
  get hasReady(): boolean {
    return this.#images.some((image) => image.ready !== null);
  }

  begin(source: string, name: string): PendingImage | null {
    // The sentences of the previous images go once the owner adds others.
    if (!this.preparing) {
      this.#failures = [];
    }
    if (this.#images.length >= MAX_MESSAGE_IMAGES) {
      this.refuse(m.chat_image_too_many({ max: MAX_MESSAGE_IMAGES }));
      return null;
    }
    const id = this.#nextId;
    this.#nextId += 1;
    this.#images.push({ id, name, ready: null, source });
    return {
      fail: (sentence) => {
        this.#images = this.#images.filter((image) => image.id !== id);
        this.refuse(sentence);
      },
      ready: (prepared) => {
        const index = this.#images.findIndex((image) => image.id === id);
        // An image that the owner removed while it was prepared goes at once.
        if (index === -1) {
          prepared.discard();
          return;
        }
        this.#images[index] = { id, name, ready: prepared, source };
      },
    };
  }

  refuse(sentence: string): void {
    this.#failures.push(sentence);
  }

  /** Removes an image from the message, and lets its source drop it. */
  remove(id: number): void {
    const removed = this.#images.find((image) => image.id === id);
    this.#images = this.#images.filter((image) => image.id !== id);
    removed?.ready?.discard();
  }

  /** The references of the ready images, in order. */
  refs(): ImageRef[] {
    return this.#images.flatMap((image) =>
      image.ready === null ? [] : [{ key: image.ready.key, source: image.source }],
    );
  }

  /** Empties the message box's images once the message was sent; the core lets their sources drop them. */
  reset(): void {
    this.#images = [];
    this.#failures = [];
  }
}
