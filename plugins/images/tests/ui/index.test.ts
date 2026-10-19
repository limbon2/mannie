// The Images plugin's entry in the shell (plugins/images/docs/README.md; ADR-0030,
// decision 11): its source of images in the message box, under the plugin's
// ID, shows only while Images is on, and takes the image files of a paste or
// a drop.
import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { describe, expect, it } from 'vitest';

import plugin from '../../ui/index.js';

describe('the Images plugin', () => {
  it('registers its source of images, which shows only while Images is on', () => {
    const context = new TestPluginContext('images');

    plugin.register(context);
    const whileOff = context.shell.imageSources.entries;
    context.turnOn();
    const source = context.shell.imageSources.find('images');

    expect(whileOff).toEqual([]);
    expect(source?.id).toBe('images');
    expect(source?.takes(new File([new Uint8Array(1)], 'a.png', { type: 'image/png' }))).toBe(true);
    expect(source?.takes(new File([new Uint8Array(1)], 'notes.txt', { type: 'text/plain' }))).toBe(false);
  });

  it('has no start failure or work in progress of its own', () => {
    expect(plugin.failure).toBeUndefined();
    expect(plugin.offQuestion).toBeUndefined();
  });
});
