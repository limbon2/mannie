// The UI's composition root (docs/architecture/extension-points.md): a feature
// adds its entries through its registration alone, next to the shell's own,
// a plugin's entries show only while the core reports the plugin as on, and
// a plugin follows its own state through its context (ADR-0030, decision 11;
// ADR-0034, decision 11). The real plugins' entries are their own tests'.
import { describe, expect, it, vi } from 'vitest';

import {
  composeShell,
  type PluginFollowing,
} from '../../../../../../apps/desktop/src/lib/registrations/compose-shell.js';
import { findPlugin, pluginCatalog } from '../../../../../../apps/desktop/src/lib/state/plugin-catalog.js';
import type { ShellSlots } from '../../../../../../apps/desktop/src/lib/state/shell-slots.svelte.js';
import type { PluginEntry } from '../../../../../../apps/desktop/src/lib/types/plugins.js';

// The shell's commands navigate and change the theme; neither runs here.
vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));
vi.mock('$app/state', () => ({ page: { url: new URL('http://localhost/') } }));
vi.mock('mode-watcher', () => ({ resetMode: vi.fn(), setMode: vi.fn() }));

/**
 * A feature as a later milestone adds one: a view, its navigation entry, a
 * command, a settings section, a sidebar panel, an overlay, a part renderer,
 * a kind of side panel and a conversation control.
 */
function registerExampleFeature(slots: ShellSlots): void {
  const component = () => ({});
  slots.views.register({ component, id: 'example', title: () => 'Example' });
  slots.sidebarPanels.register({ component, id: 'example.panel', order: 0 });
  slots.overlays.register({ component, id: 'example.overlay' });
  slots.sidePanels.register({ component, defaultLabel: () => 'Example', icon: component, id: 'example.side' });
  slots.partRenderers.register({ component, id: 'example_part' });
  slots.conversationControls.register({ component, id: 'example.control', order: 0 });
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

/** A plugin as a later milestone adds one: a sidebar panel and a command. */
const EXAMPLE_PLUGIN: PluginEntry = {
  definition: {
    register({ slots }) {
      slots.sidebarPanels.register({ component: () => ({}), id: 'example.panel', order: 0 });
      slots.commands.register({
        group: () => 'Example',
        id: 'example.run',
        label: () => 'Run',
        order: 0,
        run: vi.fn(),
      });
    },
  },
  description: () => 'Adds an example.',
  id: 'example',
  name: () => 'Example',
};

/** Plugin states in which `isOn` tells which plugins are on, and that nothing follows. */
function following(isOn: (plugin: string) => boolean): PluginFollowing {
  return { isOn, whileOn: () => () => undefined };
}

describe('composeShell', () => {
  it('registers the shell’s own settings section and commands', () => {
    const slots = composeShell(
      [],
      [],
      following(() => true),
    );

    expect(slots.settingsSections.entries.map((section) => section.id)).toEqual(['shell.interface']);
    expect(slots.commands.entries.map((command) => command.id)).toEqual([
      'shell.open-settings',
      'shell.close-side-panel',
      'shell.theme-light',
      'shell.theme-dark',
      'shell.theme-system',
    ]);
    expect(slots.views.entries).toEqual([]);
    expect(slots.navigation.entries).toEqual([]);
    expect(slots.sidebarPanels.entries).toEqual([]);
    expect(slots.overlays.entries.map((overlay) => overlay.id)).toEqual(['shell.link-confirmation']);
    expect(slots.partRenderers.entries).toEqual([]);
    expect(slots.sidePanels.entries).toEqual([]);
    expect(slots.conversationControls.entries).toEqual([]);
    expect(slots.imageSources.entries).toEqual([]);
  });

  it('composes the app’s features beside the shell, with chat first', async () => {
    const { features } = await import('../../../../../../apps/desktop/src/lib/registrations/features.js');

    const slots = composeShell(
      features,
      [],
      following(() => true),
    );

    expect(pluginCatalog()).toEqual([]);
    expect(slots.views.entries.map((view) => view.id)).toEqual(['chat']);
    expect(slots.sidebarPanels.entries.map((panel) => panel.id)).toEqual(['chat.projects', 'chat.conversations']);
    expect(slots.sidePanels.entries.map((kind) => kind.id)).toEqual(['chat.conversation', 'tools.command-output']);
    expect(slots.conversationControls.entries.map((control) => control.id)).toEqual([
      'tools.full-access',
      'tools.background-commands',
      'child-agents.control',
      'export.conversation',
    ]);
    // Full access sits in the message box, and the rest in the conversation's
    // menu, whose button counts running commands and working child agents
    // (ADR-0042).
    expect(slots.conversationControls.entries.map((control) => [control.id, control.place])).toEqual([
      ['tools.full-access', 'message-box'],
      ['tools.background-commands', 'menu'],
      ['child-agents.control', 'menu'],
      ['export.conversation', 'menu'],
    ]);
    expect(
      slots.conversationControls.entries
        .filter((control) => control.running !== undefined)
        .map((control) => control.id),
    ).toEqual(['tools.background-commands', 'child-agents.control']);
    expect(slots.overlays.entries.map((overlay) => overlay.id)).toEqual([
      'shell.link-confirmation',
      'chat.dialogs',
      'window.requests',
      'tools.dialogs',
      'export.result',
    ]);
    expect(slots.partRenderers.entries.map((renderer) => renderer.id)).toEqual([
      'user_text',
      'text',
      'reasoning',
      'error',
      'mention',
      'image',
      'tool_call',
      'command_note',
      'task',
    ]);
    expect(slots.mentionKinds.entries.map((kind) => kind.id)).toEqual(['files']);
    expect(slots.imageSources.entries).toEqual([]);
    expect(slots.settingsSections.entries.map((section) => section.id)).toEqual([
      'shell.interface',
      'chat.providers',
      'chat.instructions',
      'window.behavior',
      'tools.settings',
      'child-agents.settings',
      'plugins.settings',
    ]);
    expect(slots.taskKinds.entries).toEqual([]);
    expect(slots.commands.entries.map((command) => command.id)).toEqual(
      expect.arrayContaining(['chat.add-project', 'chat.add-project-from-path', 'tools.show-permissions']),
    );
  });

  it('shows a plugin’s entries only while the core reports the plugin as on, without composing again', () => {
    const on = new Set<string>();

    const slots = composeShell(
      [],
      [EXAMPLE_PLUGIN],
      following((plugin) => on.has(plugin)),
    );
    const whileOff = [slots.sidebarPanels.find('example.panel'), slots.commands.find('example.run')];
    on.add('example');

    expect(whileOff).toEqual([undefined, undefined]);
    expect(slots.sidebarPanels.find('example.panel')).toBeDefined();
    expect(slots.commands.find('example.run')).toBeDefined();
    expect(pluginCatalog().map((plugin) => plugin.id)).toEqual(['example']);
  });

  it('lets a plugin follow its own state through its context', () => {
    const whileOn = vi.fn(() => () => undefined);
    const on = vi.fn();
    const off = vi.fn();
    const followingPlugin: PluginEntry = {
      ...EXAMPLE_PLUGIN,
      definition: {
        register(context) {
          context.whileOn(on, off);
        },
      },
    };

    composeShell([], [followingPlugin], { isOn: () => false, whileOn });

    expect(whileOn).toHaveBeenCalledWith('example', on, off);
  });

  it('names a plugin without UI on the Plugins page, without codes of its own', () => {
    const quiet: PluginEntry = {
      definition: null,
      description: () => 'Adds a service.',
      id: 'quiet',
      name: () => 'Quiet',
    };

    composeShell(
      [],
      [quiet],
      following(() => true),
    );
    const described = findPlugin('quiet');

    expect(described?.name()).toBe('Quiet');
    expect(described?.failure('storage_failed')).toBeNull();
    expect(described?.offQuestion([{ code: 'active_tasks', count: 1 }])).toBeNull();
  });

  it('adds a feature’s entries beside the shell’s', () => {
    const slots = composeShell(
      [registerExampleFeature],
      [],
      following(() => true),
    );

    expect(slots.views.find('example')).toBeDefined();
    expect(slots.navigation.find('example')?.view).toBe('example');
    expect(slots.commands.find('example.run')).toBeDefined();
    expect(slots.commands.find('shell.open-settings')).toBeDefined();
    expect(slots.sidebarPanels.find('example.panel')).toBeDefined();
    expect(slots.overlays.find('example.overlay')).toBeDefined();
    expect(slots.partRenderers.find('example_part')).toBeDefined();
    expect(slots.sidePanels.find('example.side')).toBeDefined();
    expect(slots.conversationControls.find('example.control')).toBeDefined();
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

    expect(() =>
      composeShell(
        [clashing],
        [],
        following(() => true),
      ),
    ).toThrow('already has an entry with the ID shell.interface');
  });
});
