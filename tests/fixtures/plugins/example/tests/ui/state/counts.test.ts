// The count of the notes, which the plugin's optional feature `counts`
// answers: the window reads it once the plugin and the feature are on, reads
// it again on request, keeps the reason when the host refused it, and
// forgets it as the feature or the plugin turns off (ADR-0055).
import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { COUNTS_FEATURE, NoteCount } from '../../../ui/state/counts.svelte.js';

const core = vi.hoisted(() => ({ countNotes: vi.fn() }));

vi.mock('../../../ui/generated/bindings.js', () => ({ commands: { countNotes: core.countNotes } }));

/** A count that follows the feature of a plugin that is on, while the feature is off until the test turns it on. */
function followedCount(): { count: NoteCount; plugin: TestPluginContext } {
  const plugin = new TestPluginContext('example', true);
  const count = new NoteCount();
  count.follow(plugin.feature(COUNTS_FEATURE));
  return { count, plugin };
}

beforeEach(() => {
  core.countNotes.mockReset();
  core.countNotes.mockResolvedValue({ data: 2, status: 'ok' });
});

describe('the count of the notes', () => {
  it('is read once the feature turns on, and again on request', async () => {
    const { count, plugin } = followedCount();
    const readWhileOff = core.countNotes.mock.calls.length;

    plugin.feature(COUNTS_FEATURE).turnOn();
    await vi.waitFor(() => {
      expect(count.status).toBe('ready');
    });
    core.countNotes.mockResolvedValue({ data: 3, status: 'ok' });
    await count.refresh();

    expect(readWhileOff).toBe(0);
    expect(count.count).toBe(3);
    expect(core.countNotes).toHaveBeenCalledTimes(2);
  });

  it('keeps the reason when the host refused the count because the feature is off', async () => {
    core.countNotes.mockResolvedValue({
      error: { code: 'feature_off', description: '', feature: COUNTS_FEATURE, origin: 'host', plugin: 'example' },
      status: 'error',
    });
    const { count, plugin } = followedCount();

    plugin.feature(COUNTS_FEATURE).turnOn();
    await vi.waitFor(() => {
      expect(count.status).toBe('error');
    });

    expect(count.failure).toEqual({
      error: { code: 'feature_off', description: '', feature: COUNTS_FEATURE, origin: 'host', plugin: 'example' },
      kind: 'core',
    });
    expect(count.count).toBeNull();
  });

  it('is forgotten as the feature turns off, and as its plugin does, and read again when both are on', async () => {
    const { count, plugin } = followedCount();
    const counts = plugin.feature(COUNTS_FEATURE);
    counts.turnOn();
    await vi.waitFor(() => {
      expect(count.status).toBe('ready');
    });

    counts.turnOff();
    const afterFeatureOff = { count: count.count, status: count.status };
    counts.turnOn();
    await vi.waitFor(() => {
      expect(count.status).toBe('ready');
    });
    plugin.turnOff();
    const afterPluginOff = { count: count.count, status: count.status };
    plugin.turnOn();
    await vi.waitFor(() => {
      expect(count.status).toBe('ready');
    });

    expect(afterFeatureOff).toEqual({ count: null, status: 'loading' });
    expect(afterPluginOff).toEqual({ count: null, status: 'loading' });
    expect(core.countNotes).toHaveBeenCalledTimes(3);
  });

  it('is not read while the feature is on and its plugin is off', () => {
    const plugin = new TestPluginContext('example');
    const count = new NoteCount();
    count.follow(plugin.feature(COUNTS_FEATURE));

    plugin.feature(COUNTS_FEATURE).turnOn();

    expect(core.countNotes).not.toHaveBeenCalled();
    expect(plugin.feature(COUNTS_FEATURE).isOn()).toBe(false);
  });
});
