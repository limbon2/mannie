// Messages for the codes of the Images plugin (ADR-0019; docs/features/
// images.md): its commands' errors, each about one image, the host's
// refusals of its commands, and the refusal of its image source when a
// message is sent.
import { formatCount } from '@mannie/plugin-sdk/format';
import type { PluginCallFailure } from '@mannie/plugin-sdk/ipc';
import { hostRefusalMessage, ipcFailureMessage } from '@mannie/plugin-sdk/messages';

import type { ImagesError } from '../generated/bindings.js';
import { m } from '../generated/messages.js';

const MEBIBYTE = 1024 * 1024;

/** What to tell the owner about an image named `name` that could not be added. */
export function imagesErrorMessage(failure: PluginCallFailure<ImagesError>, name: string): string {
  if (failure.kind === 'ipc') {
    return ipcFailureMessage();
  }
  const { error } = failure;
  if (error.origin === 'host') {
    return hostRefusalMessage(error, m.images_plugin_name());
  }
  switch (error.code) {
    case 'cannot_shrink': {
      return m.images_error_cannot_shrink({ name });
    }
    case 'dialog_failed': {
      return m.images_error_dialog_failed();
    }
    case 'file_unreadable': {
      return m.images_error_file_unreadable({ name });
    }
    case 'not_found': {
      return m.images_error_not_found({ name });
    }
    case 'too_large': {
      return m.images_error_too_large({ limit: formatCount(Math.floor(error.maxBytes / MEBIBYTE)), name });
    }
    case 'too_many_pixels': {
      return m.images_error_too_many_pixels({ maxMegapixels: formatCount(error.maxMegapixels), name });
    }
    case 'too_wide': {
      return m.images_error_too_wide({ maxSide: formatCount(error.maxSide), name });
    }
    case 'unreadable': {
      return m.images_error_unreadable({ name });
    }
    case 'unsupported_format': {
      return m.images_error_unsupported_format({ name });
    }
  }
}

/** Why the image source refused an image of a message, or `null` for a code it does not know. */
export function imagesRefusalMessage(code: string): string | null {
  return code === 'not_found' ? m.images_refusal_not_found() : null;
}
