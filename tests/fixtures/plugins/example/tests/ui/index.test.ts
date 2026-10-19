// The plugin's entries in the shell (ADR-0030, decision 11): its group of
// the sidebar, its command and its settings section show only while the
// plugin is on, and the notes follow the plugin from its registration.
import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { describe, expect, it, vi } from 'vitest';

import plugin from '../../ui/index.js';

const notes = vi.hoisted(() => ({ follow: vi.fn() }));

// The notes follow the plugin's channel, which these tests do not open.
vi.mock('../../ui/state/notes.svelte.js', () => ({ noteList: notes, SETTINGS_SECTION: 'example.settings' }));
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

  it('has no start failure or work in progress of its own', () => {
    expect(plugin.failure).toBeUndefined();
    expect(plugin.offQuestion).toBeUndefined();
  });
});
