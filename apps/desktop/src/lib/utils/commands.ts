// The command palette's list: the available commands, grouped under their
// group's heading (ADR-0020, decision 2), and a command for each entry of the
// activity bar (ADR-0045, decision 4).
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

/** The available commands under their group's heading, in the order the groups first appear. */
export function groupCommands(commands: readonly ShellCommand[]): CommandGroup[] {
  const available = commands.filter((command) => isAvailable(command));
  const headings = [...new Set(available.map((command) => command.group()))];
  return headings.map((heading) => ({
    commands: sortedByOrder(available.filter((command) => command.group() === heading)),
    heading,
  }));
}
