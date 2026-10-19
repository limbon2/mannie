// Every code of the Images plugin has a message of its own, which names the
// image it is about, and the host's refusals of its commands name the plugin
// (ADR-0019; ADR-0034, decision 9).
import { formatCount } from '@mannie/plugin-sdk/format';
import type { PluginCallFailure } from '@mannie/plugin-sdk/ipc';
import { describe, expect, it } from 'vitest';

import type { ImagesError } from '../../../ui/generated/bindings.js';
import { imagesErrorMessage, imagesRefusalMessage } from '../../../ui/utils/images-messages.js';

const ERRORS: ImagesError[] = [
  { code: 'too_large', maxBytes: 20_971_520 },
  { code: 'too_wide', maxSide: 12_000 },
  { code: 'too_many_pixels', maxMegapixels: 64 },
  { code: 'cannot_shrink', maxBytes: 5_242_880 },
  { code: 'unsupported_format' },
  { code: 'unreadable' },
  { code: 'file_unreadable' },
  { code: 'not_found' },
  { code: 'dialog_failed' },
];

/** The plugin's error as the generated bindings report it. */
function failed(error: ImagesError): PluginCallFailure<ImagesError> {
  return { error: { ...error, description: '', origin: 'plugin' }, kind: 'core' };
}

describe('images messages', () => {
  it('say what each error means and name the image', () => {
    const messages = ERRORS.map((error) => imagesErrorMessage(failed(error), 'shot.png'));

    for (const message of messages) {
      expect(message.trim()).not.toBe('');
    }
    expect(new Set(messages).size).toBe(messages.length);
    expect(imagesErrorMessage(failed({ code: 'unreadable' }), 'shot.png')).toContain('shot.png');
    expect(imagesErrorMessage({ kind: 'ipc' }, 'shot.png')).not.toBe('');
  });

  it('give the limits in the owner’s format', () => {
    const tooLarge = imagesErrorMessage(failed({ code: 'too_large', maxBytes: 20_971_520 }), 'a.png');
    const tooWide = imagesErrorMessage(failed({ code: 'too_wide', maxSide: 12_000 }), 'a.png');

    expect(tooLarge).toContain('20 MiB');
    expect(tooWide).toContain(formatCount(12_000));
  });

  it('word the host’s refusals with the plugin’s name', () => {
    const off = imagesErrorMessage(
      { error: { code: 'plugin_off', description: '', origin: 'host', plugin: 'images' }, kind: 'core' },
      'a.png',
    );
    const refused = imagesErrorMessage(
      { error: { code: 'not_allowed', description: 'refused', origin: 'host' }, kind: 'core' },
      'a.png',
    );

    expect(off).toBe('Images is off. Turn it on in Settings, Plugins, then try again.');
    expect(refused).toBe('The window may not ask Images for this. Report this as a bug.');
  });

  it('word the source’s refusal of a key it no longer holds, and nothing else', () => {
    expect(imagesRefusalMessage('not_found')).not.toBeNull();
    expect(imagesRefusalMessage('invalid_image')).toBeNull();
  });
});
