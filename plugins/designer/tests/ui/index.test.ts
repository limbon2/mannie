import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { describe, expect, it, vi } from 'vitest';

import plugin from '../../ui/index.js';

const projects = vi.hoisted(() => ({ follow: vi.fn() }));

vi.mock('../../ui/state/projects.svelte.js', () => ({ projectLibrary: projects }));

describe('the plugin', () => {
  it('registers its entries, which show only while it is on', () => {
    const context = new TestPluginContext('designer');

    plugin.register(context);
    const whileOff = [
      context.shell.navigation.entries,
      context.shell.views.entries,
      context.shell.sidebarPanels.entries,
      context.shell.overlays.entries,
    ];
    context.turnOn();

    expect(whileOff).toEqual([[], [], [], []]);
    expect(context.shell.navigation.entries.map((entry) => entry.id)).toEqual(['designer']);
    expect(context.shell.views.entries.map((view) => view.id)).toEqual(['designer']);
    expect(context.shell.sidebarPanels.entries.map((panel) => panel.id)).toEqual(['designer.projects']);
    expect(context.shell.overlays.entries.map((overlay) => overlay.id)).toEqual(['designer.dialogs']);
    expect(projects.follow).toHaveBeenCalledWith(context);
  });

  it('shows its sidebar group under its own entry of the activity bar', () => {
    const context = new TestPluginContext('designer', true);

    plugin.register(context);

    expect(context.shell.sidebarPanels.entries.map((panel) => panel.activity)).toEqual(['designer']);
  });

  it('removes its shell entries when it turns off', () => {
    const context = new TestPluginContext('designer', true);

    plugin.register(context);
    context.turnOff();

    expect(context.shell.navigation.entries).toEqual([]);
    expect(context.shell.views.entries).toEqual([]);
    expect(context.shell.sidebarPanels.entries).toEqual([]);
    expect(context.shell.overlays.entries).toEqual([]);
  });
});
