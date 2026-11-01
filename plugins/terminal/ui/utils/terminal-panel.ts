// The terminal's side panel (ADR-0025; plugins/terminal/docs/README.md): its
// kind, and its argument, which names the conversation and the terminal's
// number, such as `0199e0f2-….2`. Cite keys name a terminal by its own ID and
// a command by its number, such as `3f9a2c1d.5`, and may end with a line
// fragment, such as `#L3-L4` (ADR-0054, decision 2).
import type { Reveal, RevealedLines, TerminalAddress } from '../types/terminal.js';

/** The side panel kind of a terminal. */
export const TERMINAL_PANEL = 'terminal.session';

/** The settings section of the plugin. */
export const SETTINGS_SECTION = 'terminal.settings';

/** A panel's argument for a terminal. */
export function panelArgument(address: TerminalAddress): string {
  return `${address.conversationId}.${String(address.terminal)}`;
}

/** The terminal that a panel's argument names, or `null` for an argument that the plugin never made. */
export function parsePanelArgument(argument: string): TerminalAddress | null {
  const dot = argument.lastIndexOf('.');
  if (dot <= 0) {
    return null;
  }
  const number = Number(argument.slice(dot + 1));
  if (!Number.isSafeInteger(number)) {
    return null;
  }
  if (number < 1) {
    return null;
  }
  return { conversationId: argument.slice(0, dot), terminal: number };
}

/** What a cite key names: a terminal's ID, a command and maybe lines. */
export interface CitedCommand {
  readonly reveal: Reveal;
  /** The terminal's own ID. */
  readonly session: string;
}

/** The lines that a fragment names, or `null` without lines or with a last before the first. */
function linesOf(first: string | undefined, last: string | undefined): RevealedLines | null {
  if (first === undefined) {
    return null;
  }
  const firstLine = Number(first);
  const lastLine = last === undefined ? firstLine : Number(last);
  return lastLine < firstLine ? null : { first: firstLine, last: lastLine };
}

const KEY = /^([0-9a-f]{8})\.([1-9]\d*)(?:#L([1-9]\d*)(?:-L([1-9]\d*))?)?$/u;

/** The command that a cite key names, or `null` for a key that names no command. */
export function parseCiteKey(key: string): CitedCommand | null {
  const match = KEY.exec(key);
  if (match === null) {
    return null;
  }
  const [, session = '', block = ''] = match;
  // The fragment's lines are optional groups, which may not have matched.
  const first: string | undefined = match[3];
  const last: string | undefined = match[4];
  return { reveal: { block: Number(block), lines: linesOf(first, last) }, session };
}
