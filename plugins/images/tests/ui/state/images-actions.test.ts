// What the Images plugin does for the message box (plugins/images/docs/README.md,
// adding images): the chosen files show as chips at once and are prepared
// one by one through the plugin's generated bindings, a pasted file crosses
// IPC as base64 unless it is too large, the plugin's error and the host's
// refusal end their chip with a sentence, and a removed image is dropped in
// the core.
import { MessageImages } from '@mannie/plugin-sdk/testing';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { PreparedImage } from '../../../ui/generated/bindings.js';
import { imagesActions } from '../../../ui/state/images-actions.svelte.js';

const core = vi.hoisted(() => ({
  discardPreparedImage: vi.fn(),
  pickImages: vi.fn(),
  prepareImage: vi.fn(),
  preparePickedImage: vi.fn(),
}));

vi.mock('../../../ui/generated/bindings.js', () => ({ commands: core }));

function prepared(key: string): PreparedImage {
  return {
    bytes: 100,
    format: 'png',
    height: 300,
    key,
    name: `${key}.png`,
    thumbnail: 'AAAA',
    thumbnailFormat: 'jpeg',
    width: 400,
  };
}

describe('imagesActions', () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  it('shows every chosen file at once and prepares them one by one', async () => {
    const images = new MessageImages();
    core.pickImages.mockResolvedValue({
      data: [
        { name: 'a.png', pickedId: 'p1' },
        { name: 'b.png', pickedId: 'p2' },
      ],
      status: 'ok',
    });
    let chipsWhilePreparing: string[] = [];
    core.preparePickedImage.mockImplementation(({ pickedId }: { pickedId: string }) => {
      chipsWhilePreparing = images.images.map((image) => image.name);
      return pickedId === 'p1'
        ? Promise.resolve({ data: prepared('k1'), status: 'ok' })
        : Promise.resolve({
            error: { code: 'unsupported_format', description: '', origin: 'plugin' },
            status: 'error',
          });
    });

    await imagesActions.pick(images);

    expect(core.pickImages).toHaveBeenCalledWith({ filterName: 'Images' });
    expect(chipsWhilePreparing).toEqual(['a.png', 'b.png']);
    expect(images.refs()).toEqual([{ key: 'k1', source: 'images' }]);
    expect(images.images[0]?.ready?.thumbnailUrl).toBe('data:image/jpeg;base64,AAAA');
    expect(images.failures).toEqual(['b.png is not a PNG, JPEG, GIF or WebP image.']);
  });

  it('says why the dialog could not open, also when the host refused the call', async () => {
    const images = new MessageImages();
    core.pickImages.mockResolvedValueOnce({
      error: { code: 'plugin_off', description: '', origin: 'host', plugin: 'images' },
      status: 'error',
    });
    core.pickImages.mockResolvedValueOnce({
      error: { code: 'dialog_failed', description: '', origin: 'plugin' },
      status: 'error',
    });

    await imagesActions.pick(images);
    await imagesActions.pick(images);

    expect(images.images).toEqual([]);
    expect(images.failures).toEqual([
      'Images is off. Turn it on in Settings, Plugins, then try again.',
      'The file dialog could not open. Paste or drop the images instead.',
    ]);
  });

  it('sends a pasted file as base64 and keeps a too large one in the webview', async () => {
    const images = new MessageImages();
    core.prepareImage.mockResolvedValue({ data: prepared('k2'), status: 'ok' });
    const pasted = new File([new Uint8Array([0xff, 0x00, 0x7f])], 'image.png', { type: 'image/png' });
    const large = new File([new Uint8Array(1)], 'large.png', { type: 'image/png' });
    Object.defineProperty(large, 'size', { value: 21 * 1024 * 1024 });

    await imagesActions.prepareFiles([pasted, large], images);

    expect(core.prepareImage).toHaveBeenCalledOnce();
    expect(core.prepareImage).toHaveBeenCalledWith({ data: '/wB/', name: 'image.png' });
    expect(images.refs()).toEqual([{ key: 'k2', source: 'images' }]);
    expect(images.failures).toEqual(['large.png is larger than 20 MiB, so it was not added.']);
  });

  it('drops an image in the core when the owner removes it', async () => {
    const images = new MessageImages();
    core.prepareImage.mockResolvedValue({ data: prepared('k3'), status: 'ok' });
    core.discardPreparedImage.mockResolvedValue({ data: null, status: 'ok' });
    await imagesActions.prepareFiles([new File([new Uint8Array(1)], 'a.png', { type: 'image/png' })], images);

    images.remove(images.images[0]?.id ?? -1);

    expect(core.discardPreparedImage).toHaveBeenCalledWith({ key: 'k3' });
  });
});
