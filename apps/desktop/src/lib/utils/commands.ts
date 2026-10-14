// The command palette's list: the available commands, grouped under their
// group's heading (ADR-0020, decision 2).
import type { ShellCommand } from '#lib/types/shell-slots.js';

import { sortedByOrder } from './order.js';

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
