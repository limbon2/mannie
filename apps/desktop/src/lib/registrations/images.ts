// The Images plugin's UI (docs/features/images.md; ADR-0033): its image
// source in the message box, with the button that opens the file dialog,
// the files of a paste or a drop that it takes, and the words of its
// refusals. The entry shows only while Images is on (ADR-0030, decision 11);
// the images that messages hold show through the core's renderer either way.
import AttachImagesButton from '#lib/components/app/images/AttachImagesButton.svelte';
import { m } from '#lib/paraglide/messages.js';
import { IMAGES_PLUGIN, imagesActions } from '#lib/state/images-actions.svelte.js';
import type { PluginRegistration } from '#lib/types/plugins.js';
import { takesImageFile } from '#lib/utils/images-files.js';
import { imagesRefusalMessage } from '#lib/utils/images-messages.js';

/** The Images plugin. It has no work in progress and no start failure of its own. */
export const imagesPlugin: PluginRegistration = {
  description: m.images_plugin_description,
  failure: () => null,
  id: IMAGES_PLUGIN,
  name: m.images_plugin_name,
  offQuestion: () => null,
  register(slots) {
    slots.imageSources.register({
      button: AttachImagesButton,
      id: IMAGES_PLUGIN,
      order: 0,
      refusal: imagesRefusalMessage,
      take: (files, target) => void imagesActions.prepareFiles(files, target),
      takes: takesImageFile,
    });
  },
};
