// The UI's composition root (docs/architecture/extension-points.md): a feature
// adds its entries through its registration alone, next to the shell's own.
import { describe, expect, it, vi } from 'vitest';

import { composeShell } from '../../../../../../apps/desktop/src/lib/registrations/compose-shell.js';
import type { ShellSlots } from '../../../../../../apps/desktop/src/lib/state/shell-slots.svelte.js';

// The shell's commands navigate and change the theme; neither runs here.
vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));
vi.mock('mode-watcher', () => ({ resetMode: vi.fn(), setMode: vi.fn() }));

/** A feature as a later milestone adds one: a view, its navigation entry, a command and a settings section. */
function registerExampleFeature(slots: ShellSlots): void {
  const component = () => ({});
  slots.views.register({ component, id: 'example', title: () => 'Example' });
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
  });

  it('adds a feature’s entries beside the shell’s', () => {
    const slots = composeShell([registerExampleFeature]);

    expect(slots.views.find('example')).toBeDefined();
    expect(slots.navigation.find('example')?.view).toBe('example');
    expect(slots.commands.find('example.run')).toBeDefined();
    expect(slots.commands.find('shell.open-settings')).toBeDefined();
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
        id: 'shell.interface',
        order: 1,
        title: () => '',
      });
    };

    expect(() => composeShell([clashing])).toThrow('already has an entry with the ID shell.interface');
  });
});
