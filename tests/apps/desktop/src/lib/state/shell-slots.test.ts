// The registry behind the shell's slots (docs/architecture/extension-points.md,
// UI shell slots): entries in the order they were registered, no duplicate
// IDs, and a plugin's entries only while the core reports the plugin as on
// (ADR-0030, decision 11).
import { describe, expect, it } from 'vitest';

import { ShellSlots, Slot } from '../../../../../../apps/desktop/src/lib/state/shell-slots.svelte.js';

interface Entry {
  readonly id: string;
  readonly order: number;
}

function everyPluginOn(): boolean {
  return true;
}

/** Plugin states that the test changes: the plugins in `on` are on. */
function pluginsOn(...plugins: string[]): { isOn: (plugin: string) => boolean; on: Set<string> } {
  const on = new Set(plugins);
  return { isOn: (plugin) => on.has(plugin), on };
}

describe('Slot', () => {
  it('keeps the entries in the order they were registered', () => {
    const slot = new Slot<Entry>('test', everyPluginOn);

    slot.register({ id: 'second', order: 2 });
    slot.register({ id: 'first', order: 1 });

    expect(slot.entries.map((entry) => entry.id)).toEqual(['second', 'first']);
  });

  it('finds an entry by its ID', () => {
    const slot = new Slot<Entry>('test', everyPluginOn);
    slot.register({ id: 'settings', order: 0 });

    expect(slot.find('settings')).toEqual({ id: 'settings', order: 0 });
    expect(slot.find('missing')).toBeUndefined();
  });

  it('refuses a second entry with the same ID', () => {
    const slot = new Slot<Entry>('commands', everyPluginOn);
    slot.register({ id: 'shell.open-settings', order: 0 });

    expect(() => {
      slot.register({ id: 'shell.open-settings', order: 1 });
    }).toThrow('The commands slot already has an entry with the ID shell.open-settings.');
    expect(slot.entries).toHaveLength(1);
  });

  it('lists and finds a plugin’s entry only while the plugin is on, and the core’s always', () => {
    const plugins = pluginsOn();
    const slot = new Slot<Entry>('test', plugins.isOn);
    slot.register({ id: 'core', order: 0 });
    slot.register({ id: 'workers.panel', order: 1 }, 'workers');

    const whileOff = slot.entries.map((entry) => entry.id);
    const foundWhileOff = slot.find('workers.panel');
    plugins.on.add('workers');

    expect(whileOff).toEqual(['core']);
    expect(foundWhileOff).toBeUndefined();
    expect(slot.entries.map((entry) => entry.id)).toEqual(['core', 'workers.panel']);
    expect(slot.find('workers.panel')).toEqual({ id: 'workers.panel', order: 1 });
  });

  it('names the plugin of an entry that does not show because its plugin is off', () => {
    const plugins = pluginsOn();
    const slot = new Slot<Entry>('test', plugins.isOn);
    slot.register({ id: 'core', order: 0 });
    slot.register({ id: 'workers.panel', order: 1 }, 'workers');

    const whileOff = slot.pluginOff('workers.panel');
    plugins.on.add('workers');

    expect(whileOff).toBe('workers');
    expect(slot.pluginOff('workers.panel')).toBeNull();
    expect(slot.pluginOff('core')).toBeNull();
    expect(slot.pluginOff('missing')).toBeNull();
  });

  it('finds an entry for one plugin only when that plugin registered it and is on', () => {
    const plugins = pluginsOn('git');
    const slot = new Slot<Entry>('tool kinds', plugins.isOn);
    slot.register({ id: 'git_status', order: 0 }, 'git');
    slot.register({ id: 'read_file', order: 1 });

    expect(slot.findOf('git_status', 'git')).toEqual({ id: 'git_status', order: 0 });
    // Another plugin's call of a tool of the same name, and the core's entries,
    // are never described by it.
    expect(slot.findOf('git_status', 'shell')).toBeUndefined();
    expect(slot.findOf('read_file', 'git')).toBeUndefined();
    expect(slot.findOf('missing', 'git')).toBeUndefined();
    plugins.on.delete('git');
    expect(slot.findOf('git_status', 'git')).toBeUndefined();
  });

  it('refuses an ID that an entry of a plugin that is off has', () => {
    const slot = new Slot<Entry>('commands', pluginsOn().isOn);
    slot.register({ id: 'workers.pair', order: 0 }, 'workers');

    expect(() => {
      slot.register({ id: 'workers.pair', order: 1 });
    }).toThrow('The commands slot already has an entry with the ID workers.pair.');
  });
});

describe('ShellSlots', () => {
  it('registers a plugin’s entries under the plugin’s ID in every slot', () => {
    const plugins = pluginsOn();
    const slots = new ShellSlots(plugins.isOn);
    const component = () => ({});
    const workers = slots.forPlugin('workers');

    workers.sidebarPanels.register({ component, id: 'workers.panel', order: 5 });
    workers.commands.register({
      group: () => 'Workers',
      id: 'workers.pair',
      label: () => 'Pair',
      order: 0,
      run: () => undefined,
    });
    const whileOff = [slots.sidebarPanels.entries.length, slots.commands.entries.length];
    plugins.on.add('workers');

    expect(whileOff).toEqual([0, 0]);
    expect(slots.sidebarPanels.entries.map((panel) => panel.id)).toEqual(['workers.panel']);
    expect(slots.commands.find('workers.pair')).toBeDefined();
    plugins.on.delete('workers');
    expect(slots.commands.pluginOff('workers.pair')).toBe('workers');
  });

  it('lists a plugin’s kind of mention only while the plugin is on', () => {
    const plugins = pluginsOn();
    const slots = new ShellSlots(plugins.isOn);
    const kind = {
      group: () => 'Notes',
      icon: () => (() => ({})) as never,
      id: 'fake',
      order: 10,
      refusal: () => null,
      summary: () => null,
      variantName: () => null,
      warning: () => null,
    };

    slots.forPlugin('fake').mentionKinds.register(kind);
    const whileOff = slots.mentionKinds.find('fake');
    plugins.on.add('fake');

    expect(whileOff).toBeUndefined();
    expect(slots.mentionKinds.find('fake')).toBe(kind);
    expect(slots.mentionKinds.find('files')).toBeUndefined();
    expect(() => {
      slots.mentionKinds.register(kind);
    }).toThrow('The mention kinds slot already has an entry with the ID fake.');
  });

  it('lists a plugin’s source of images only while the plugin is on', () => {
    const plugins = pluginsOn();
    const slots = new ShellSlots(plugins.isOn);
    const source = {
      button: (() => ({})) as never,
      id: 'images',
      order: 0,
      refusal: () => null,
      take: () => undefined,
      takes: () => true,
    };

    slots.forPlugin('images').imageSources.register(source);
    const whileOff = slots.imageSources.entries;
    plugins.on.add('images');

    expect(whileOff).toEqual([]);
    expect(slots.imageSources.find('images')).toBe(source);
    expect(() => {
      slots.imageSources.register(source);
    }).toThrow('The image sources slot already has an entry with the ID images.');
  });

  it('has a slot for panels, overlays and part renderers that refuses duplicates', () => {
    const slots = new ShellSlots();
    const component = () => ({});

    slots.sidebarPanels.register({ component, id: 'fake.panel', order: 1 });
    slots.overlays.register({ component, id: 'fake.overlay' });
    slots.partRenderers.register({ component, id: 'fake_part' });

    expect(slots.sidebarPanels.find('fake.panel')?.order).toBe(1);
    expect(slots.overlays.find('fake.overlay')).toBeDefined();
    expect(() => {
      slots.partRenderers.register({ component, id: 'fake_part' });
    }).toThrow('The part renderers slot already has an entry with the ID fake_part.');
  });
});

describe('the conversation controls slot', () => {
  it('registers a control for a conversation’s toolbar by its ID, and refuses a duplicate', () => {
    const slots = new ShellSlots();
    const component = () => ({});

    slots.conversationControls.register({ component, id: 'fake.control', order: 2 });

    expect(slots.conversationControls.find('fake.control')?.order).toBe(2);
    expect(() => {
      slots.conversationControls.register({ component, id: 'fake.control', order: 3 });
    }).toThrow('The conversation controls slot already has an entry with the ID fake.control.');
  });
});

describe('the side panels slot', () => {
  it('registers a kind of side panel by its ID', () => {
    const slots = new ShellSlots();
    const component = () => ({});

    slots.sidePanels.register({ component, defaultLabel: () => 'Fake', icon: component, id: 'fake.panel' });

    expect(slots.sidePanels.find('fake.panel')?.defaultLabel()).toBe('Fake');
  });

  it('refuses a kind whose ID has a colon, which the address uses', () => {
    const slots = new ShellSlots();
    const component = () => ({});

    expect(() => {
      slots.sidePanels.register({ component, defaultLabel: () => 'Fake', icon: component, id: 'fake:panel' });
    }).toThrow('The side panel kind fake:panel has a colon, which its address uses.');
    expect(slots.sidePanels.entries).toEqual([]);
  });
});
