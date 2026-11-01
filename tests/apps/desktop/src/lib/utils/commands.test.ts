import { describe, expect, it, vi } from 'vitest';

import type { NavigationEntry, ShellCommand } from '../../../../../../apps/desktop/src/lib/types/shell-slots.js';
import {
  commandOfKey,
  groupCommands,
  isAvailable,
  navigationCommands,
  type PressedKey,
} from '../../../../../../apps/desktop/src/lib/utils/commands.js';

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

describe('navigationCommands', () => {
  it('gives each entry of the activity bar a command under Views that selects it', () => {
    const entries: NavigationEntry[] = [
      { icon: () => ({}), id: 'chat', label: () => 'Chat', order: 0, view: 'chat' },
      { icon: () => ({}), id: 'notebooks', label: () => 'Notebooks', order: 20, view: 'notebooks' },
    ];
    const select = vi.fn();

    const commands = navigationCommands(entries, select);
    commands[1]?.run();

    expect(commands.map((entry) => entry.id)).toEqual(['shell.show.chat', 'shell.show.notebooks']);
    expect(commands.map((entry) => entry.label())).toEqual(['Show Chat', 'Show Notebooks']);
    expect(commands[0]?.group()).toBe('Views');
    expect(select).toHaveBeenCalledWith(entries[1]);
  });
});

describe('commandOfKey (ADR-0053, decision 8)', () => {
  const press = (code: string, extra: Partial<PressedKey> = {}): PressedKey => ({
    altKey: false,
    code,
    ctrlKey: true,
    metaKey: false,
    shiftKey: false,
    ...extra,
  });

  it('finds the available command whose key is pressed with Ctrl or Cmd', () => {
    let open = true;
    const terminal: ShellCommand = {
      ...command('terminal.open', 'Terminal', 0, () => open),
      key: { code: 'Backquote', label: () => 'Ctrl+`' },
    };
    const commands = [command('plain', 'General', 0), terminal];

    expect(commandOfKey(commands, press('Backquote'))?.id).toBe('terminal.open');
    expect(commandOfKey(commands, press('Backquote', { ctrlKey: false, metaKey: true }))?.id).toBe('terminal.open');
    expect(commandOfKey(commands, press('Backquote', { ctrlKey: false }))).toBeUndefined();
    expect(commandOfKey(commands, press('Backquote', { shiftKey: true }))).toBeUndefined();
    expect(commandOfKey(commands, press('KeyA'))).toBeUndefined();
    open = false;
    expect(commandOfKey(commands, press('Backquote'))).toBeUndefined();
  });
});

describe('isAvailable', () => {
  it('treats a command without a condition as available', () => {
    expect(isAvailable(command('always', 'General', 0))).toBe(true);
    expect(isAvailable(command('never', 'General', 0, () => false))).toBe(false);
  });
});
