import { describe, expect, it } from 'vitest';

import { Slot } from '../../../../../../apps/desktop/src/lib/state/shell-slots.svelte.js';

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
