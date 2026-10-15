// The UI's composition root (docs/architecture/extension-points.md): a feature
// adds its entries through its registration alone, next to the shell's own.
import { describe, expect, it, vi } from 'vitest';

import { composeShell } from '../../../../../../apps/desktop/src/lib/registrations/compose-shell.js';
import type { ShellSlots } from '../../../../../../apps/desktop/src/lib/state/shell-slots.svelte.js';

// The shell's commands navigate and change the theme; neither runs here.
vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));
vi.mock('$app/state', () => ({ page: { url: new URL('http://localhost/') } }));
vi.mock('mode-watcher', () => ({ resetMode: vi.fn(), setMode: vi.fn() }));

/**
 * A feature as a later milestone adds one: a view, its navigation entry, a
 * command, a settings section, a sidebar panel, an overlay and a part
 * renderer.
 */
function registerExampleFeature(slots: ShellSlots): void {
  const component = () => ({});
  slots.views.register({ component, id: 'example', title: () => 'Example' });
  slots.sidebarPanels.register({ component, id: 'example.panel', order: 0 });
  slots.overlays.register({ component, id: 'example.overlay' });
  slots.partRenderers.register({ component, id: 'example_part' });
  slots.navigation.register({ icon: component, id: 'example', label: () => 'Example', order: 0, view: 'example' });
  slots.commands.register({
    group: () => 'Example',
    id: 'example.run',
    label: () => 'Run the example',
    order: 0,
    run: vi.fn(),
  });
  slots.settingsSections.register({
    component,
    description: () => 'Settings of the example',
    icon: component,
    id: 'example.settings',
    order: 10,
    title: () => 'Example',
  });
}

describe('composeShell', () => {
  it('registers the shell’s own settings section and commands', () => {
    const slots = composeShell([]);

    expect(slots.settingsSections.entries.map((section) => section.id)).toEqual(['shell.interface']);
    expect(slots.commands.entries.map((command) => command.id)).toEqual([
      'shell.open-settings',
      'shell.theme-light',
      'shell.theme-dark',
      'shell.theme-system',
    ]);
    expect(slots.views.entries).toEqual([]);
    expect(slots.navigation.entries).toEqual([]);
    expect(slots.sidebarPanels.entries).toEqual([]);
    expect(slots.overlays.entries.map((overlay) => overlay.id)).toEqual(['shell.link-confirmation']);
    expect(slots.partRenderers.entries).toEqual([]);
  });

  it('composes the app’s features beside the shell, with chat first', async () => {
    const { features } = await import('../../../../../../apps/desktop/src/lib/registrations/features.js');

    const slots = composeShell(features);

    expect(slots.views.entries.map((view) => view.id)).toEqual(['chat']);
    expect(slots.sidebarPanels.entries.map((panel) => panel.id)).toEqual(['chat.projects', 'chat.conversations']);
    expect(slots.overlays.entries.map((overlay) => overlay.id)).toEqual([
      'shell.link-confirmation',
      'chat.dialogs',
      'window.requests',
      'tools.dialogs',
    ]);
    expect(slots.partRenderers.entries.map((renderer) => renderer.id)).toEqual([
      'user_text',
      'text',
      'reasoning',
      'error',
      'tool_call',
    ]);
    expect(slots.settingsSections.entries.map((section) => section.id)).toEqual([
      'shell.interface',
      'chat.providers',
      'chat.instructions',
      'window.behavior',
      'tools.settings',
    ]);
    expect(slots.commands.entries.map((command) => command.id)).toEqual(
      expect.arrayContaining(['chat.add-project', 'chat.add-project-from-path', 'tools.show-permissions']),
    );
  });

  it('adds a feature’s entries beside the shell’s', () => {
    const slots = composeShell([registerExampleFeature]);

    expect(slots.views.find('example')).toBeDefined();
    expect(slots.navigation.find('example')?.view).toBe('example');
    expect(slots.commands.find('example.run')).toBeDefined();
    expect(slots.commands.find('shell.open-settings')).toBeDefined();
    expect(slots.sidebarPanels.find('example.panel')).toBeDefined();
    expect(slots.overlays.find('example.overlay')).toBeDefined();
    expect(slots.partRenderers.find('example_part')).toBeDefined();
    expect(slots.settingsSections.entries.map((section) => section.id)).toEqual([
      'shell.interface',
      'example.settings',
    ]);
  });

  it('refuses a feature that reuses an ID of the shell', () => {
    const clashing = (slots: ShellSlots): void => {
      slots.settingsSections.register({
        component: () => ({}),
        description: () => '',
        icon: () => ({}),
        id: 'shell.interface',
        order: 1,
        title: () => '',
      });
    };

    expect(() => composeShell([clashing])).toThrow('already has an entry with the ID shell.interface');
  });
});
