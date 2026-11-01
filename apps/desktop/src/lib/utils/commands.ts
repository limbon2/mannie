// The command palette's list: the available commands, grouped under their
// group's heading (ADR-0020, decision 2), and a command for each entry of the
// activity bar (ADR-0045, decision 4). A command may have a key, which runs
// it while it is available (ADR-0053, decision 8).
import { m } from '#lib/paraglide/messages.js';
import type { NavigationEntry, ShellCommand } from '#lib/types/shell-slots.js';

import { sortedByOrder } from './order.js';

/** A command for each entry of the activity bar, which selects the entry, in the entries' order. */
export function navigationCommands(
  entries: readonly NavigationEntry[],
  select: (entry: NavigationEntry) => void,
): ShellCommand[] {
  return entries.map((entry) => ({
    group: m.shell_command_group_views,
    id: `shell.show.${entry.id}`,
    label: () => m.shell_command_show_view({ view: entry.label() }),
    order: entry.order,
    run: () => select(entry),
  }));
}

/** The commands of one heading, in their order. */
export interface CommandGroup {
  readonly commands: ShellCommand[];
  readonly heading: string;
}

/** Whether a command applies now. A command without a condition always does. */
export function isAvailable(command: ShellCommand): boolean {
  return command.available?.() ?? true;
}

/** What of a key press decides which command it runs. */
export type PressedKey = Pick<KeyboardEvent, 'altKey' | 'code' | 'ctrlKey' | 'metaKey' | 'shiftKey'>;

/** The available command whose key is pressed, with Ctrl or Cmd, if one is. */
export function commandOfKey(commands: readonly ShellCommand[], pressed: PressedKey): ShellCommand | undefined {
  const withCommandKey = pressed.ctrlKey || pressed.metaKey;
  if (!withCommandKey) {
    return undefined;
  }
  return commands.find((command) => {
    const { key } = command;
    if (key === undefined) {
      return false;
    }
    const sameCode = key.code === pressed.code;
    const sameShift = (key.shift ?? false) === pressed.shiftKey;
    const sameAlt = (key.alt ?? false) === pressed.altKey;
    return sameCode && sameShift && sameAlt && isAvailable(command);
  });
}

/** The available commands under their group's heading, in the order the groups first appear. */
export function groupCommands(commands: readonly ShellCommand[]): CommandGroup[] {
  const available = commands.filter((command) => isAvailable(command));
  const headings = [...new Set(available.map((command) => command.group()))];
  return headings.map((heading) => ({
    commands: sortedByOrder(available.filter((command) => command.group() === heading)),
    heading,
  }));
}
