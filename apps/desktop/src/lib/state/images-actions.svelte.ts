// What the Images plugin does for the message box (docs/features/images.md,
// adding images): the button's file dialog, and the files of a paste or a
// drop. Each image shows as a chip at once, and the core prepares them one
// by one.
import { commands, type ImagesError, type PreparedImage } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import type { Outcome } from '#lib/types/calls.js';
import type { ImageTarget, PendingImage } from '#lib/types/shell-slots.js';
import { call } from '#lib/utils/calls.js';
import { imageDataUrl } from '#lib/utils/image-data.js';
import { MAX_IMAGE_FILE_BYTES, readAsBase64 } from '#lib/utils/images-files.js';
import { imagesErrorMessage } from '#lib/utils/images-messages.js';

/** The plugin's ID, which its image source shares. */
export const IMAGES_PLUGIN = 'images';

/** Starts a chip for each name, up to the message's limit, which the message box then says. */
function beginAll<T extends { readonly name: string }>(
  items: readonly T[],
  target: ImageTarget,
): { item: T; pending: PendingImage }[] {
  const started: { item: T; pending: PendingImage }[] = [];
  for (const item of items) {
    const pending = target.begin(IMAGES_PLUGIN, item.name);
    if (pending === null) {
      break;
    }
    started.push({ item, pending });
  }
  return started;
}

/** Ends a chip with what the core prepared, or with why it could not. */
function finish(pending: PendingImage, name: string, outcome: Outcome<PreparedImage, ImagesError>): void {
  if (!outcome.ok) {
    pending.fail(imagesErrorMessage(outcome.failure, name));
    return;
  }
  const prepared = outcome.data;
  pending.ready({
    discard: () => void call(() => commands.discardPreparedImage({ key: prepared.key })),
    height: prepared.height,
    key: prepared.key,
    thumbnailUrl: imageDataUrl(prepared.thumbnailFormat, prepared.thumbnail),
    width: prepared.width,
  });
}

class ImagesActions {
  /** Opens the system's file dialog and adds the chosen images. */
  async pick(target: ImageTarget): Promise<void> {
    const picked = await call(() => commands.pickImages({ filterName: m.images_dialog_filter() }));
    if (!picked.ok) {
      target.refuse(imagesErrorMessage(picked.failure, ''));
      return;
    }
    for (const { item, pending } of beginAll(picked.data, target)) {
      const outcome = await call(() => commands.preparePickedImage({ pickedId: item.pickedId }));
      finish(pending, item.name, outcome);
    }
  }

  /** Adds the images of a paste or a drop. A file larger than the core reads never crosses IPC. */
  async prepareFiles(files: readonly File[], target: ImageTarget): Promise<void> {
    for (const { item: file, pending } of beginAll(files, target)) {
      if (file.size > MAX_IMAGE_FILE_BYTES) {
        finish(pending, file.name, {
          failure: { error: { code: 'too_large', maxBytes: MAX_IMAGE_FILE_BYTES }, kind: 'core' },
          ok: false,
        });
        continue;
      }
      let data: string;
      try {
        data = await readAsBase64(file);
      } catch {
        pending.fail(m.images_error_file_unreadable({ name: file.name }));
        continue;
      }
      const outcome = await call(() => commands.prepareImage({ data, name: file.name }));
      finish(pending, file.name, outcome);
    }
  }
}

/** The Images plugin's actions. */
export const imagesActions = new ImagesActions();
