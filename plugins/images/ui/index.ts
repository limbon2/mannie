// The Images plugin's UI (plugins/images/docs/README.md; ADR-0033): its image
// source in the message box, with the button that opens the file dialog,
// the files of a paste or a drop that it takes, and the words of its
// refusals. The entry shows only while Images is on (ADR-0030, decision 11);
// the images that messages hold show through the core's renderer either way.
// The plugin has no work in progress and no start failure of its own.
import { definePlugin } from '@mannie/plugin-sdk';

import AttachImagesButton from './components/AttachImagesButton.svelte';
import { imagesActions } from './state/images-actions.svelte.js';
import { takesImageFile } from './utils/images-files.js';
import { imagesRefusalMessage } from './utils/images-messages.js';

export default definePlugin({
  register({ id, slots }) {
    // The image source's ID is the plugin's, which the images it adds name.
    slots.imageSources.register({
      button: AttachImagesButton,
      id,
      order: 0,
      refusal: imagesRefusalMessage,
      take: (files, target) => void imagesActions.prepareFiles(files, target),
      takes: takesImageFile,
    });
  },
});
