import { hostRefusalMessage, ipcFailureMessage } from '@mannie/plugin-sdk/messages';
// Messages for the terminal plugin's codes (ADR-0019): the core words a call
// that got no answer and the host's refusals, and the plugin its own errors,
// its tools' failures, the terminals' states and its mentions.
import type { MentionSummary, ToolKindCall } from '@mannie/plugin-sdk/slots';

import type {
  EndingView,
  TerminalError,
  TerminalHostView,
  TerminalPhase,
  TerminalSummary,
} from '../generated/bindings.js';
import { m } from '../generated/messages.js';
import type { TerminalFailure } from '../types/terminal.js';

/** The plugin's own errors by their codes, which the type checks hold to the plugin's error type. */
const ERRORS: Record<TerminalError['code'], () => string> = {
  block_not_found: () => m.terminal_error_block_not_found(),
  conversation_not_found: () => m.terminal_error_conversation_not_found(),
  folder_invalid: () => m.terminal_error_folder_invalid(),
  host_not_found: () => m.terminal_error_host_not_found(),
  host_unavailable: () => m.terminal_error_host_unavailable(),
  open_failed: () => m.terminal_error_open_failed(),
  shell_not_listed: () => m.terminal_error_shell_not_listed(),
  storage_failed: () => m.terminal_error_storage_failed(),
  terminal_ended: () => m.terminal_error_terminal_ended(),
  terminal_not_found: () => m.terminal_error_terminal_not_found(),
  too_many: () => m.terminal_error_too_many(),
  unsupported: () => m.terminal_error_unsupported(),
};

/** What to tell the owner about a call of the plugin that failed, and what they can do. */
export function terminalFailureMessage(failure: TerminalFailure): string {
  if (failure.kind === 'ipc') {
    return ipcFailureMessage();
  }
  const { error } = failure;
  if (error.origin === 'plugin') {
    return ERRORS[error.code]();
  }
  return hostRefusalMessage(error, m.terminal_plugin_name());
}

/** The sentence of a start failure code of the plugin. */
export function terminalStartFailureMessage(code: string): string | null {
  return code === 'storage_failed' ? m.terminal_start_failed() : null;
}

/** What turning the plugin off ends, or `null` when nothing runs. */
export function terminalOffQuestion(items: readonly { code: string; count: number }[]): string | null {
  const running = items.find((item) => item.code === 'running_commands');
  return running === undefined ? null : m.terminal_off_question({ count: running.count });
}

/** The sentence of a failure code of `run_in_terminal`. */
export function toolFailure(code: string): string | null {
  switch (code) {
    case 'interrupted': {
      return m.terminal_tool_interrupted();
    }
    case 'owner_typing': {
      return m.terminal_tool_owner_typing();
    }
    case 'still_running': {
      return m.terminal_tool_still_running();
    }
    case 'terminal_busy': {
      return m.terminal_tool_busy();
    }
    case 'terminal_changed': {
      return m.terminal_tool_changed();
    }
    case 'terminal_ended': {
      return m.terminal_tool_ended();
    }
    case 'terminal_not_opened': {
      return m.terminal_tool_not_opened();
    }
    case 'terminal_plain': {
      return m.terminal_tool_plain();
    }
    default: {
      return null;
    }
  }
}

/** A whole number of the card's data, or `null`. */
function dataNumber(data: ToolKindCall['data'], key: string): number | null {
  const value = data?.[key];
  if (typeof value !== 'number') {
    return null;
  }
  return Number.isSafeInteger(value) ? value : null;
}

/** A text of the card's data, or `null`. */
function dataText(data: ToolKindCall['data'], key: string): string | null {
  const value = data?.[key];
  return typeof value === 'string' ? value : null;
}

/** The line of a call of `run_in_terminal` that ran, or `null` for the core's line from its access. */
export function runSummary(call: ToolKindCall): string | null {
  const command = dataText(call.data, 'command');
  const terminal = dataNumber(call.data, 'terminal');
  if (command === null) {
    return null;
  }
  if (terminal === null) {
    return null;
  }
  const exitCode = dataNumber(call.data, 'exitCode');
  const words = { command, terminal: String(terminal) };
  if (call.data?.interrupted === true) {
    return m.terminal_card_interrupted(words);
  }
  return exitCode === null ? m.terminal_card_ran(words) : m.terminal_card_exited({ ...words, code: String(exitCode) });
}

/** What the model got of a mention. */
export function mentionSummary(summary: MentionSummary): string | null {
  if (summary.code !== 'lines') {
    return null;
  }
  const total = summary.values?.total ?? 0;
  const shown = summary.values?.shown ?? total;
  return shown < total ? m.terminal_mention_some_lines({ shown, total }) : m.terminal_mention_lines({ count: total });
}

/** What an entry of the "@" list is, for screen readers. */
export function mentionVariantName(variant: string): string | null {
  switch (variant) {
    case 'command': {
      return m.terminal_mention_command();
    }
    case 'selection': {
      return m.terminal_mention_selection();
    }
    default: {
      return null;
    }
  }
}

/** The sentence of a refusal of a mention. */
export function mentionRefusal(code: string): string | null {
  return code === 'not_found' ? m.terminal_mention_not_found() : null;
}

/** Where a terminal runs, as the owner reads it. */
export function hostName(host: TerminalHostView): string {
  return host.kind === 'this_computer' ? m.terminal_this_computer() : host.label;
}

/** A terminal's name, such as "Terminal 2 · bash · build-box", without the shell while a host's login shell is not known. */
export function terminalName(summary: TerminalSummary): string {
  const host = hostName(summary.host);
  if (summary.shell === '') {
    return m.terminal_name_without_shell({ host, number: summary.number });
  }
  return m.terminal_name({ host, number: summary.number, shell: summary.shell });
}

/** What a terminal does, in a few words. */
export function phaseLabel(phase: TerminalPhase): string {
  switch (phase) {
    case 'ended': {
      return m.terminal_phase_ended();
    }
    case 'prompt': {
      return m.terminal_phase_prompt();
    }
    case 'running': {
      return m.terminal_phase_running();
    }
    case 'starting': {
      return m.terminal_phase_starting();
    }
  }
}

/** How a terminal ended, as a sentence. */
export function endingMessage(ending: EndingView): string {
  switch (ending.kind) {
    case 'closed': {
      return m.terminal_ended_closed();
    }
    case 'exited': {
      return m.terminal_ended_exited({ code: String(ending.code) });
    }
    case 'lost': {
      return m.terminal_ended_lost({ code: ending.code });
    }
    case 'stopped': {
      return m.terminal_ended_stopped();
    }
  }
}

/** Why a host does not open terminals now, as a few words. */
export function unavailableReason(code: string): string {
  return m.terminal_host_unavailable({ code });
}
