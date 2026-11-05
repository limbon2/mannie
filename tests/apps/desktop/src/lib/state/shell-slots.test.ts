// The registry behind the shell's slots (docs/architecture/extension-points.md,
// UI shell slots): entries in the order they were registered, no duplicate
// IDs, a plugin's entries only while the core reports the plugin as on
// (ADR-0030, decision 11), and a plugin's feature's entries only while the
// feature is on too (ADR-0055, decision 10).
import { describe, expect, it } from 'vitest';

import { ShellSlots, Slot } from '../../../../../../apps/desktop/src/lib/state/shell-slots.svelte.js';
import type { SlotOwner } from '../../../../../../apps/desktop/src/lib/types/shell-slots.js';

interface Entry {
  readonly id: string;
  readonly order: number;
}

function everyPluginOn(): boolean {
  return true;
}

/** The owner of a plugin's own entries. */
function owner(plugin: string): SlotOwner {
  return { feature: null, plugin };
}

/** The owner of the entries of a plugin's feature. */
function featureOwner(plugin: string, feature: string): SlotOwner {
  return { feature, plugin };
}

/**
 * Plugin states that the test changes: the plugins in `on` are on, and a
 * plugin's feature is on while `on` also holds `<plugin>/<feature>`.
 */
function pluginsOn(...plugins: string[]): { isOn: (owner: SlotOwner) => boolean; on: Set<string> } {
  const on = new Set(plugins);
  const isOn = ({ feature, plugin }: SlotOwner): boolean => {
    if (!on.has(plugin)) {
      return false;
    }
    return feature === null ? true : on.has(`${plugin}/${feature}`);
  };
  return { isOn, on };
}

describe('the commands slot (ADR-0053, decision 8)', () => {
  const keyed = (id: string, code: string, shift = false) => ({
    group: () => 'Terminal',
    id,
    key: { code, label: () => code, shift },
    label: () => id,
    order: 0,
    run: () => undefined,
  });

  it('refuses a key that the shell or another command holds', () => {
    const slots = new ShellSlots();
    slots.commands.register(keyed('terminal.open', 'Backquote'));

    expect(() => slots.commands.register(keyed('other.open', 'Backquote'))).toThrow(/terminal\.open/u);
    expect(() => slots.commands.register(keyed('palette', 'KeyK'))).toThrow(/shell binds/u);
    slots.commands.register(keyed('terminal.new', 'Backquote', true));
    slots.commands.register(keyed('find', 'KeyK', true));

    expect(slots.commands.entries.map((entry) => entry.id)).toEqual(['terminal.open', 'terminal.new', 'find']);
  });
});

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
    slot.register({ id: 'workers.panel', order: 1 }, owner('workers'));

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
    slot.register({ id: 'workers.panel', order: 1 }, owner('workers'));

    const whileOff = slot.ownerOff('workers.panel');
    plugins.on.add('workers');

    expect(whileOff).toEqual(owner('workers'));
    expect(slot.ownerOff('workers.panel')).toBeNull();
    expect(slot.ownerOff('core')).toBeNull();
    expect(slot.ownerOff('missing')).toBeNull();
  });

  it('lists a feature’s entry only while its plugin and the feature are on, and names both while it does not', () => {
    const plugins = pluginsOn();
    const slot = new Slot<Entry>('test', plugins.isOn);
    slot.register({ id: 'workers.panel', order: 0 }, owner('workers'));
    slot.register({ id: 'workers.databases', order: 1 }, featureOwner('workers', 'databases'));

    plugins.on.add('workers/databases');
    const whilePluginOff = slot.entries.map((entry) => entry.id);
    plugins.on.add('workers');
    const whileBothOn = slot.entries.map((entry) => entry.id);
    plugins.on.delete('workers/databases');

    expect(whilePluginOff).toEqual([]);
    expect(whileBothOn).toEqual(['workers.panel', 'workers.databases']);
    expect(slot.entries.map((entry) => entry.id)).toEqual(['workers.panel']);
    expect(slot.ownerOff('workers.databases')).toEqual(featureOwner('workers', 'databases'));
    expect(slot.ownerOff('workers.panel')).toBeNull();
  });

  it('counts a feature’s entries among its plugin’s', () => {
    const plugins = pluginsOn('workers', 'workers/databases');
    const slot = new Slot<Entry>('tool kinds', plugins.isOn);
    slot.register({ id: 'workers_task', order: 0 }, owner('workers'));
    slot.register({ id: 'query_database', order: 1 }, featureOwner('workers', 'databases'));

    expect(slot.findOf('query_database', 'workers')).toEqual({ id: 'query_database', order: 1 });
    expect(slot.entriesOf('workers').map((entry) => entry.id)).toEqual(['workers_task', 'query_database']);
    plugins.on.delete('workers/databases');
    expect(slot.findOf('query_database', 'workers')).toBeUndefined();
  });

  it('finds an entry for one plugin only when that plugin registered it and is on', () => {
    const plugins = pluginsOn('git');
    const slot = new Slot<Entry>('tool kinds', plugins.isOn);
    slot.register({ id: 'git_status', order: 0 }, owner('git'));
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

  it('lists the entries of one plugin while it is on', () => {
    const plugins = pluginsOn('mcp', 'git');
    const slot = new Slot<Entry>('tool kinds', plugins.isOn);
    slot.register({ id: 'mcp_tools', order: 0 }, owner('mcp'));
    slot.register({ id: 'git_status', order: 1 }, owner('git'));
    slot.register({ id: 'mcp_read_resource', order: 2 }, owner('mcp'));
    slot.register({ id: 'read_file', order: 3 });

    expect(slot.entriesOf('mcp').map((entry) => entry.id)).toEqual(['mcp_tools', 'mcp_read_resource']);
    plugins.on.delete('mcp');
    expect(slot.entriesOf('mcp')).toEqual([]);
  });

  it('refuses an ID that an entry of a plugin that is off has', () => {
    const slot = new Slot<Entry>('commands', pluginsOn().isOn);
    slot.register({ id: 'workers.pair', order: 0 }, owner('workers'));

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
    expect(slots.commands.ownerOff('workers.pair')).toEqual(owner('workers'));
  });

  it('registers a feature’s entries under its plugin and the feature', () => {
    const plugins = pluginsOn('workers');
    const slots = new ShellSlots(plugins.isOn);

    slots.forFeature('workers', 'databases').settingsSections.register({
      component: () => ({}),
      description: () => '',
      icon: () => ({}),
      id: 'workers.databases',
      order: 0,
      title: () => 'Databases',
    });
    const whileOff = slots.settingsSections.find('workers.databases');
    plugins.on.add('workers/databases');

    expect(whileOff).toBeUndefined();
    expect(slots.settingsSections.find('workers.databases')).toBeDefined();
    plugins.on.delete('workers/databases');
    expect(slots.settingsSections.ownerOff('workers.databases')).toEqual(featureOwner('workers', 'databases'));
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
