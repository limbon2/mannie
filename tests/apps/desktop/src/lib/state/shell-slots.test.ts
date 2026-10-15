import { describe, expect, it } from 'vitest';

import { ShellSlots, Slot } from '../../../../../../apps/desktop/src/lib/state/shell-slots.svelte.js';

interface Entry {
  readonly id: string;
  readonly order: number;
}

describe('Slot', () => {
  it('keeps the entries in the order they were registered', () => {
    const slot = new Slot<Entry>('test');

    slot.register({ id: 'second', order: 2 });
    slot.register({ id: 'first', order: 1 });

    expect(slot.entries.map((entry) => entry.id)).toEqual(['second', 'first']);
  });

  it('finds an entry by its ID', () => {
    const slot = new Slot<Entry>('test');
    slot.register({ id: 'settings', order: 0 });

    expect(slot.find('settings')).toEqual({ id: 'settings', order: 0 });
    expect(slot.find('missing')).toBeUndefined();
  });

  it('refuses a second entry with the same ID', () => {
    const slot = new Slot<Entry>('commands');
    slot.register({ id: 'shell.open-settings', order: 0 });

    expect(() => {
      slot.register({ id: 'shell.open-settings', order: 1 });
    }).toThrow('The commands slot already has an entry with the ID shell.open-settings.');
    expect(slot.entries).toHaveLength(1);
  });
});

describe('ShellSlots', () => {
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
