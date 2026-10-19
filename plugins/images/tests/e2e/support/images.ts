// Images for the end-to-end tests of the Images plugin (docs/features/
// images.md, acceptance): PNG files that the tests write, pastes and drops
// that WebDriver cannot make with the system's clipboard or mouse, and the
// images of the requests that reached the test provider.
import { crc32, deflateSync } from 'node:zlib';

import { COMPOSER } from '#e2e/chat.ts';
import type { RecordedRequest } from '#e2e/provider-server.ts';

/** A file as a paste or a drop carries it. */
export interface TestFile {
  readonly bytes: Buffer;
  readonly name: string;
  readonly type: string;
}

const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

function chunk(kind: string, data: Buffer): Buffer {
  const typed = Buffer.concat([Buffer.from(kind, 'ascii'), data]);
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const checksum = Buffer.alloc(4);
  checksum.writeUInt32BE(crc32(typed));
  return Buffer.concat([length, typed, checksum]);
}

/** A PNG of one color, which stays small at any size. */
export function solidPng(width: number, height: number, rgb: readonly [number, number, number]): Buffer {
  const row = Buffer.alloc(1 + width * 3);
  for (let x = 0; x < width; x += 1) {
    row.set(rgb, 1 + x * 3);
  }
  const pixels = Buffer.concat(Array.from({ length: height }, () => row));
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header.set([8, 2, 0, 0, 0], 8);
  return Buffer.concat([
    PNG_SIGNATURE,
    chunk('IHDR', header),
    chunk('IDAT', deflateSync(pixels)),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

/** A PNG file of one color, named `name`. */
export function pngFile(name: string, width: number, height: number): TestFile {
  return { bytes: solidPng(width, height, [40, 120, 200]), name, type: 'image/png' };
}

/** The width and height that a PNG's header says. */
export function pngSize(png: Buffer): { height: number; width: number } {
  return { height: png.readUInt32BE(20), width: png.readUInt32BE(16) };
}

interface EncodedFile {
  readonly data: string;
  readonly name: string;
  readonly type: string;
}

function encoded(files: readonly TestFile[]): EncodedFile[] {
  return files.map((file) => ({ data: file.bytes.toString('base64'), name: file.name, type: file.type }));
}

/**
 * Pastes files into the message box, as Ctrl+V of a screenshot does, with
 * `text` that the paste carries too. Returns whether the message box took the
 * paste, which then pastes nothing itself.
 */
export async function pasteFiles(files: readonly TestFile[], text = ''): Promise<boolean> {
  return browser.execute(
    (selector: string, pasted: EncodedFile[], pastedText: string) => {
      const transfer = new DataTransfer();
      for (const file of pasted) {
        const bytes = Uint8Array.from(atob(file.data), (character) => character.codePointAt(0) ?? 0);
        transfer.items.add(new File([bytes], file.name, { type: file.type }));
      }
      if (pastedText !== '') {
        transfer.setData('text/plain', pastedText);
      }
      const box = document.querySelector<HTMLTextAreaElement>(selector);
      if (box === null) {
        throw new Error('the message box is missing');
      }
      box.focus();
      const paste = new ClipboardEvent('paste', { bubbles: true, cancelable: true, clipboardData: transfer });
      return !box.dispatchEvent(paste);
    },
    COMPOSER,
    encoded(files),
    text,
  );
}

/**
 * Drags files over the element of `selector` and drops them there, as the
 * mouse would. Returns whether the page took the drop, so that the webview
 * opens nothing.
 */
export async function dropFiles(selector: string, files: readonly TestFile[]): Promise<boolean> {
  return browser.execute(
    (target: string, dropped: EncodedFile[]) => {
      const transfer = new DataTransfer();
      for (const file of dropped) {
        const bytes = Uint8Array.from(atob(file.data), (character) => character.codePointAt(0) ?? 0);
        transfer.items.add(new File([bytes], file.name, { type: file.type }));
      }
      const element = document.querySelector(target);
      if (element === null) {
        throw new Error(`${target} is missing`);
      }
      for (const type of ['dragenter', 'dragover']) {
        element.dispatchEvent(new DragEvent(type, { bubbles: true, cancelable: true, dataTransfer: transfer }));
      }
      const drop = new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer: transfer });
      return !element.dispatchEvent(drop);
    },
    selector,
    encoded(files),
  );
}

/** The names of the images in the message box, once each is ready. */
export async function readyChips(): Promise<string[]> {
  return browser.execute(() =>
    [...document.querySelectorAll('[data-image-chips] li')]
      .filter((chip) => chip.querySelector('img') !== null)
      .map((chip) => chip.querySelector('[data-content]')?.textContent.trim() ?? ''),
  );
}

/** Waits until the message box shows `count` ready images. */
export async function waitForReadyChips(count: number): Promise<string[]> {
  let names: string[] = [];
  await browser.waitUntil(
    async () => {
      names = await readyChips();
      return names.length === count;
    },
    { timeout: 30_000, timeoutMsg: `the message box did not show ${String(count)} ready images` },
  );
  return names;
}

/** The sentences below the message box about images that were not added. */
export async function imageFailures(): Promise<string[]> {
  return browser.execute(() =>
    [...document.querySelectorAll('[data-image-failures] li')].map((item) => item.textContent.trim()),
  );
}

/** The images of the latest request that holds any, in order, from either interface. */
export function requestImages(requests: readonly RecordedRequest[]): Buffer[] {
  const urls = requests
    .map((request) => imageUrls(request.body))
    .filter((found) => found.length > 0)
    .at(-1);
  return (urls ?? []).map((url) => Buffer.from(url.slice(url.indexOf(',') + 1), 'base64'));
}

/** The `data:` URLs of the images of a request body. */
export function imageUrls(body: Record<string, unknown> | null): string[] {
  if (body === null) {
    return [];
  }
  const entries = (body.messages ?? body.input ?? []) as { content?: unknown }[];
  const urls: string[] = [];
  for (const entry of entries) {
    if (!Array.isArray(entry.content)) {
      continue;
    }
    for (const part of entry.content as ContentPart[]) {
      const url = imageUrlOf(part);
      if (url !== null) {
        urls.push(url);
      }
    }
  }
  return urls;
}

/** A part of a message's content, of either interface. */
interface ContentPart {
  readonly image_url?: string | { readonly url?: string };
  readonly type?: string;
}

/** The `data:` URL of an image part: a string on the Responses API, an object on Chat Completions. */
function imageUrlOf(part: ContentPart): string | null {
  const url = part.image_url;
  if (url === undefined) {
    return null;
  }
  if (typeof url === 'string') {
    return part.type === 'input_image' ? url : null;
  }
  return part.type === 'image_url' ? (url.url ?? null) : null;
}
