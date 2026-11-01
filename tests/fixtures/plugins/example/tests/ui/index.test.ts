// The plugin's entries in the shell (ADR-0030, decision 11): its group of
// the sidebar, its command and its settings section show only while the
// plugin is on, and the notes follow the plugin from its registration. The
// settings section of its feature `counts` shows only while the feature is
// on too, and the count follows the feature (ADR-0055, decision 10).
import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { describe, expect, it, vi } from 'vitest';

import plugin from '../../ui/index.js';

const notes = vi.hoisted(() => ({ follow: vi.fn() }));
const counts = vi.hoisted(() => ({ follow: vi.fn() }));

// The notes follow the plugin's channel and the count calls the feature's
// command, which these tests do not open.
vi.mock('../../ui/state/notes.svelte.js', () => ({ noteList: notes, SETTINGS_SECTION: 'example.settings' }));
vi.mock('../../ui/state/counts.svelte.js', () => ({
  COUNTS_FEATURE: 'counts',
  COUNTS_SECTION: 'example.counts',
  noteCount: counts,
}));
vi.mock('@mannie/plugin-sdk/navigation', () => ({ openSettings: vi.fn() }));

describe('the plugin', () => {
  it('registers its entries, which show only while it is on', () => {
    const context = new TestPluginContext('example');

    plugin.register(context);
    const whileOff = [context.shell.sidebarPanels.entries, context.shell.settingsSections.entries];
    context.turnOn();

    expect(whileOff).toEqual([[], []]);
    expect(context.shell.sidebarPanels.entries.map((panel) => panel.id)).toEqual(['example.panel']);
    expect(context.shell.commands.entries.map((command) => command.id)).toEqual(['example.add-note']);
    expect(context.shell.settingsSections.entries.map((section) => section.id)).toEqual(['example.settings']);
    expect(notes.follow).toHaveBeenCalledWith(context);
  });

  it('registers the settings section of its feature, which shows only while the feature is on too', () => {
    const context = new TestPluginContext('example', true);

    plugin.register(context);
    const whileOff = context.shell.settingsSections.entries.map((section) => section.id);
    context.feature('counts').turnOn();

    expect(whileOff).toEqual(['example.settings']);
    expect(context.shell.settingsSections.entries.map((section) => section.id)).toEqual([
      'example.settings',
      'example.counts',
    ]);
    expect(counts.follow).toHaveBeenCalledWith(context.feature('counts'));
  });

  it('has no start failure or work in progress of its own', () => {
    expect(plugin.failure).toBeUndefined();
    expect(plugin.offQuestion).toBeUndefined();
  });
});
