import { describe, expect, it, vi } from 'vitest';

import type { ShellCommand } from '../../../../../../apps/desktop/src/lib/types/shell-slots.js';
import { groupCommands, isAvailable } from '../../../../../../apps/desktop/src/lib/utils/commands.js';

function command(id: string, group: string, order: number, available?: () => boolean): ShellCommand {
  return { available, group: () => group, id, label: () => id, order, run: vi.fn() };
}

describe('groupCommands', () => {
  it('groups the available commands under their headings in their order', () => {
    const commands = [
      command('settings', 'General', 1),
      command('compact', 'Conversation', 0, () => false),
      command('new', 'Conversation', 1, () => true),
      command('theme', 'Appearance', 0),
      command('palette', 'General', 0),
    ];

    const groups = groupCommands(commands);

    expect(groups.map((group) => group.heading)).toEqual(['General', 'Conversation', 'Appearance']);
    expect(groups[0]?.commands.map((entry) => entry.id)).toEqual(['palette', 'settings']);
    expect(groups[1]?.commands.map((entry) => entry.id)).toEqual(['new']);
  });

  it('leaves out a group whose commands are all unavailable', () => {
    const groups = groupCommands([command('compact', 'Conversation', 0, () => false)]);

    expect(groups).toEqual([]);
  });
});

describe('isAvailable', () => {
  it('treats a command without a condition as available', () => {
    expect(isAvailable(command('always', 'General', 0))).toBe(true);
    expect(isAvailable(command('never', 'General', 0, () => false))).toBe(false);
  });
});
