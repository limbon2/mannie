// What the owner reads about background commands (docs/features/
// system-tools/background-commands.md, interface): a command's state, where
// it runs, the label of its output's panel, and the row of a wake-up. A
// command's text and its start call's activity are content, shown as the
// model wrote them.
import type {
  ArchiveRef,
  BackgroundCommandView,
  BackgroundEnding,
  CommandNoteView,
  ConversationId,
} from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';

import { formatDuration } from './durations.js';
import { firstLine } from './tools-summaries.js';

/** The side panel kind that shows a command's output. */
export const COMMAND_OUTPUT_PANEL = 'tools.command-output';

/** Whether the command still runs. */
export function isRunning(command: BackgroundCommandView): boolean {
  return command.ending === null;
}

/** The commands that run, then the ones that ended, each by number. */
export function runningFirst(commands: readonly BackgroundCommandView[]): {
  readonly ended: BackgroundCommandView[];
  readonly running: BackgroundCommandView[];
} {
  return {
    ended: commands.filter((command) => !isRunning(command)),
    running: commands.filter((command) => isRunning(command)),
  };
}

/**
 * How a command ended, in a few words, with the exit code or the signal that
 * its host reported. Exit codes are identifiers, such as -1073741819, and get
 * no grouping.
 */
export function endingLabel(ending: BackgroundEnding, exitCode: number | null, signal: string | null): string {
  switch (ending) {
    case 'exited': {
      return exitCode === null
        ? m.tools_background_state_finished()
        : m.tools_background_state_exited({ code: String(exitCode) });
    }
    case 'ended_by_host': {
      if (exitCode !== null) {
        return m.tools_background_state_ended_by_host_code({ code: String(exitCode) });
      }
      if (signal !== null) {
        return m.tools_background_state_ended_by_host_signal({ signal });
      }
      return m.tools_background_state_ended_by_host();
    }
    case 'interrupted': {
      return m.tools_background_state_interrupted();
    }
    case 'let_go': {
      return m.tools_background_state_let_go();
    }
    case 'plugin_off': {
      return m.tools_background_state_plugin_off();
    }
    case 'session_closed': {
      return m.tools_background_state_session_closed();
    }
    case 'stopped_by_agent': {
      return m.tools_background_state_stopped_by_agent();
    }
    case 'stopped_by_owner': {
      return m.tools_background_state_stopped_by_owner();
    }
    case 'task_ended': {
      return m.tools_background_state_task_ended();
    }
  }
}

/** A command's state: how long it has run, measured to `now`, or how it ended. */
export function commandState(command: BackgroundCommandView, now: number): string {
  if (command.ending === null) {
    // Whole seconds, so that the label changes once a second.
    const elapsed = Math.floor(Math.max(0, now - command.startedAt) / 1_000) * 1_000;
    return m.tools_background_state_running({ duration: formatDuration(elapsed) });
  }
  return endingLabel(command.ending, command.exitCode, command.signal);
}

/** Where a command runs: its working directory, or its host. */
export function commandPlace(command: BackgroundCommandView): string {
  const source = command.source;
  return source.kind === 'local'
    ? m.tools_background_place_local({ folder: source.workingDirectory })
    : m.tools_background_place_remote({ endpoint: source.endpoint, host: source.label });
}

/**
 * The label of a command's output panel and of its row in the toolbar's
 * menu: its number and what its start call said it does, or the first line
 * of its text when the call said nothing.
 */
export function commandLabel(command: BackgroundCommandView): string {
  const name = command.activity ?? firstLine(command.command);
  return m.tools_background_tab_label({ command: name, number: String(command.number) });
}

/** The argument of a command's output panel: its conversation and its number. */
export function commandPanelArgument(conversationId: ConversationId, number: number): string {
  return `${conversationId}.${String(number)}`;
}

/** The conversation and the number that a panel's argument names, or `null` for one that names none. */
export function parseCommandPanelArgument(argument: string): { conversationId: ConversationId; number: number } | null {
  const separator = argument.lastIndexOf('.');
  if (separator <= 0) {
    return null;
  }
  const number = Number(argument.slice(separator + 1));
  const isCommandNumber = Number.isSafeInteger(number) && number >= 1;
  if (!isCommandNumber) {
    return null;
  }
  return { conversationId: argument.slice(0, separator), number };
}

/** The command that a call started in the background, among the conversation's. */
export function startedCommand(
  commands: readonly BackgroundCommandView[],
  partId: string,
): BackgroundCommandView | undefined {
  return commands.find((command) => command.partId === partId);
}

/** The title of a wake-up's row: the commands whose ends woke the agent. */
export function noteTitle(note: CommandNoteView): string {
  const numbers = note.ends.map((end) => String(end.number));
  if (numbers.length > 1) {
    const list = new Intl.ListFormat(undefined, { style: 'long', type: 'conjunction' }).format(numbers);
    return m.tools_background_note_several({ numbers: list });
  }
  return m.tools_background_note_one({ number: numbers.at(0) ?? '' });
}

/** The archive of a command's output as the dialog of a whole output opens it, once the command ended. */
export function outputArchive(command: BackgroundCommandView): ArchiveRef | null {
  if (command.archiveId === null) {
    return null;
  }
  return { bytes: command.bytes, id: command.archiveId, lines: command.lines, name: 'output' };
}
