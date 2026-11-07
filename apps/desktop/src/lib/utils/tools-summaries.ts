// What a tool call's row says (docs/features/system-tools/tool-activity.md,
// tool calls in the reply): the line that Mannie builds about what the call
// does or did, its status, and its tool's icon. The line names paths and
// commands as the result reports them, or as the model wrote them while the
// call has no result.
import BookOpenIcon from '@lucide/svelte/icons/book-open';
import CirclePlayIcon from '@lucide/svelte/icons/circle-play';
import CircleStopIcon from '@lucide/svelte/icons/circle-stop';
import ClipboardListIcon from '@lucide/svelte/icons/clipboard-list';
import CopyIcon from '@lucide/svelte/icons/copy';
import FileOutputIcon from '@lucide/svelte/icons/file-output';
import FilePenIcon from '@lucide/svelte/icons/file-pen';
import FilePenLineIcon from '@lucide/svelte/icons/file-pen-line';
import FileSearchIcon from '@lucide/svelte/icons/file-search';
import FileTextIcon from '@lucide/svelte/icons/file-text';
import FolderInputIcon from '@lucide/svelte/icons/folder-input';
import FolderOpenIcon from '@lucide/svelte/icons/folder-open';
import FolderPlusIcon from '@lucide/svelte/icons/folder-plus';
import InfoIcon from '@lucide/svelte/icons/info';
import MessageCircleQuestionMarkIcon from '@lucide/svelte/icons/message-circle-question-mark';
import MonitorIcon from '@lucide/svelte/icons/monitor';
import MoveIcon from '@lucide/svelte/icons/move';
import ScrollTextIcon from '@lucide/svelte/icons/scroll-text';
import SquareTerminalIcon from '@lucide/svelte/icons/square-terminal';
import TextSearchIcon from '@lucide/svelte/icons/text-search';
import Trash2Icon from '@lucide/svelte/icons/trash-2';
import WrenchIcon from '@lucide/svelte/icons/wrench';
import type { Component } from 'svelte';

import type { Access, ToolCallView, ToolOutcome } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';

import { formatDuration } from './durations.js';
import { formatBytes } from './tools-messages.js';

/** The tool that runs commands. */
export const RUN_COMMAND = 'run_command';

/** The tool that starts commands in the background (ADR-0041). */
export const START_COMMAND = 'start_command';

/** How a status looks: it asks for the owner, works, ended well, failed, or ended without running. */
export type CallTone = 'attention' | 'busy' | 'done' | 'ended' | 'failed';

/** A call's status as its row shows it. */
export interface CallStatus {
  readonly label: string;
  /**
   * Whether the row leaves the status to its icon: the call is prepared,
   * waits to run, runs without a timer, or ended well. Screen readers still
   * hear the label.
   */
  readonly quiet: boolean;
  readonly tone: CallTone;
}

type Arguments = Readonly<Record<string, unknown>>;

const ICONS: Readonly<Record<string, Component>> = {
  ask_owner: MessageCircleQuestionMarkIcon,
  save_plan: ClipboardListIcon,
  copy_path: CopyIcon,
  create_directory: FolderPlusIcon,
  delete_path: Trash2Icon,
  edit_file: FilePenLineIcon,
  file_info: InfoIcon,
  find_files: FileSearchIcon,
  list_directory: FolderOpenIcon,
  move_path: MoveIcon,
  move_to_project: FolderInputIcon,
  read_file: FileTextIcon,
  read_command_output: ScrollTextIcon,
  read_skill: BookOpenIcon,
  read_tool_output: FileOutputIcon,
  run_command: SquareTerminalIcon,
  search_text: TextSearchIcon,
  start_command: CirclePlayIcon,
  stop_command: CircleStopIcon,
  system_info: MonitorIcon,
  write_file: FilePenIcon,
};

function isArguments(value: unknown): value is Arguments {
  const isObject = typeof value === 'object' && value !== null;
  return isObject && !Array.isArray(value);
}

/** The call's arguments as an object, or an empty one while they are cut or not an object. */
export function callArguments(call: ToolCallView): Arguments {
  if (call.argumentsCut) {
    return {};
  }
  try {
    const parsed: unknown = JSON.parse(call.arguments);
    return isArguments(parsed) ? parsed : {};
  } catch {
    return {};
  }
}

function argumentText(values: Arguments, key: string): string | null {
  const value = values[key];
  if (typeof value !== 'string') {
    return null;
  }
  return value === '' ? null : value;
}

/** A whole number that the model passed, such as a command's number. */
function argumentNumber(values: Arguments, key: string): number | null {
  const value = values[key];
  const isWhole = typeof value === 'number' && Number.isSafeInteger(value);
  return isWhole ? value : null;
}

function accessPath(accesses: readonly Access[], index: number): string | null {
  const access = accesses.at(index);
  return access?.kind === 'path' ? access.path : null;
}

function accessProject(accesses: readonly Access[]): string | null {
  const access = accesses.find((candidate) => candidate.kind === 'project');
  return access?.kind === 'project' ? access.name : null;
}

function accessCommand(accesses: readonly Access[]): string | null {
  const access = accesses.find((candidate) => candidate.kind === 'command');
  return access?.kind === 'command' ? access.command : null;
}

/**
 * The command on a remote host that a plugin's call runs, with the host's
 * name and whether it runs in the background (ADR-0036; ADR-0041, decision 11).
 */
function accessRemoteCommand(
  accesses: readonly Access[],
): { background: boolean; command: string; host: string } | null {
  const access = accesses.find((candidate) => candidate.kind === 'remote_command');
  return access?.kind === 'remote_command'
    ? { background: access.background ?? false, command: access.command, host: access.label }
    : null;
}

/**
 * The command that a plugin's call types into a terminal, with the terminal's
 * number, or `null` for one that the call opens, and the remote host's name,
 * or `null` on this machine (ADR-0053, decision 5).
 */
function accessTerminalCommand(
  accesses: readonly Access[],
): { command: string; host: string | null; terminal: number | null } | null {
  const access = accesses.find((candidate) => candidate.kind === 'terminal_command');
  if (access?.kind !== 'terminal_command') {
    return null;
  }
  const host = access.host.kind === 'remote' ? access.host.label : null;
  return { command: access.command, host, terminal: access.terminal };
}

/** How a call that types a command into a terminal is named, by where the terminal runs and whether the call opens it. */
function terminalCommandSummary(typed: { command: string; host: string | null; terminal: number | null }): string {
  const command = firstLine(typed.command);
  if (typed.terminal === null) {
    return typed.host === null
      ? m.tools_call_terminal_command_new_here({ command })
      : m.tools_call_terminal_command_new({ command, host: typed.host });
  }
  const terminal = String(typed.terminal);
  return typed.host === null
    ? m.tools_call_terminal_command_here({ command, terminal })
    : m.tools_call_terminal_command({ command, host: typed.host, terminal });
}

/** The tool of a tool server that a plugin's call calls, with the server's name (ADR-0043). */
function accessExternalTool(accesses: readonly Access[]): { server: string; tool: string } | null {
  const access = accesses.find((candidate) => candidate.kind === 'external_tool');
  return access?.kind === 'external_tool' ? { server: access.label, tool: access.tool } : null;
}

/**
 * How a call on a registered database is named, by the strongest of its
 * statements, with the database's name (ADR-0056, decision 7).
 */
function databaseSummary(accesses: readonly Access[]): string | null {
  const statements = accesses.filter((candidate) => candidate.kind === 'database');
  const first = statements.at(0);
  if (first === undefined) {
    return null;
  }
  const database = first.name;
  if (statements.some((statement) => statement.class === 'schema')) {
    return m.tools_call_database_schema({ database });
  }
  if (statements.some((statement) => statement.class === 'write')) {
    return m.tools_call_database_write({ database });
  }
  return m.tools_call_database_read({ database });
}

/** A folder's last name, such as `app` for `D:\Projects\app`, or the whole path for a root. */
export function folderName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/u, '');
  const name = trimmed.split(/[\\/]/u).at(-1) ?? '';
  return name === '' ? path : name;
}

/** The first line of a command, which stands for the whole command in one line. */
export function firstLine(text: string): string {
  const lines = text.trim().split(/\r?\n/u);
  const first = lines[0] ?? '';
  return lines.length > 1 ? `${first} …` : first;
}

/** The icon of a tool, and a generic one for a tool that this version does not know. */
export function toolIcon(tool: string): Component {
  return ICONS[tool] ?? WrenchIcon;
}

/** What a tool does, for a call whose target is not known yet. */
function toolLabel(tool: string): string {
  switch (tool) {
    case 'ask_owner': {
      return m.tools_tool_ask_owner();
    }
    case 'save_plan': {
      return m.tools_tool_save_plan();
    }
    case 'copy_path': {
      return m.tools_tool_copy_path();
    }
    case 'create_directory': {
      return m.tools_tool_create_directory();
    }
    case 'delete_path': {
      return m.tools_tool_delete_path();
    }
    case 'edit_file': {
      return m.tools_tool_edit_file();
    }
    case 'file_info': {
      return m.tools_tool_file_info();
    }
    case 'find_files': {
      return m.tools_tool_find_files();
    }
    case 'list_directory': {
      return m.tools_tool_list_directory();
    }
    case 'move_path': {
      return m.tools_tool_move_path();
    }
    case 'move_to_project': {
      return m.tools_tool_move_to_project();
    }
    case 'read_command_output': {
      return m.tools_tool_read_command_output();
    }
    case 'read_file': {
      return m.tools_tool_read_file();
    }
    case 'read_skill': {
      return m.tools_tool_read_skill();
    }
    case 'read_tool_output': {
      return m.tools_call_read_tool_output();
    }
    case RUN_COMMAND: {
      return m.tools_tool_run_command();
    }
    case 'search_text': {
      return m.tools_tool_search_text();
    }
    case START_COMMAND: {
      return m.tools_tool_start_command();
    }
    case 'stop_command': {
      return m.tools_tool_stop_command();
    }
    case 'system_info': {
      return m.tools_call_system_info();
    }
    case 'write_file': {
      return m.tools_tool_write_file();
    }
    default: {
      return m.tools_call_other({ tool });
    }
  }
}

/** What the call does, from its arguments or, once prepared, from its accesses. */
function intentSummary(call: ToolCallView): string {
  const values = callArguments(call);
  const path = argumentText(values, 'path') ?? accessPath(call.accesses, 0);
  const source = argumentText(values, 'source') ?? accessPath(call.accesses, 0);
  const destination = argumentText(values, 'destination') ?? accessPath(call.accesses, 1);
  const command = argumentText(values, 'command') ?? accessCommand(call.accesses);
  const pattern = argumentText(values, 'pattern');
  const query = argumentText(values, 'query');
  const commandNumber = argumentNumber(values, 'command');
  const hasEnds = source !== null && destination !== null;
  switch (call.tool) {
    case 'copy_path': {
      return hasEnds ? m.tools_call_copy_path({ destination, source }) : toolLabel(call.tool);
    }
    case 'create_directory': {
      return path === null ? toolLabel(call.tool) : m.tools_call_create_directory({ path });
    }
    case 'delete_path': {
      return path === null ? toolLabel(call.tool) : m.tools_call_delete_path({ path });
    }
    case 'edit_file': {
      return path === null ? toolLabel(call.tool) : m.tools_call_edit_file({ path });
    }
    case 'file_info': {
      return path === null ? toolLabel(call.tool) : m.tools_call_file_info({ path });
    }
    case 'find_files': {
      if (pattern === null) {
        return toolLabel(call.tool);
      }
      return path === null
        ? m.tools_call_find_files_workspace({ pattern })
        : m.tools_call_find_files({ path, pattern });
    }
    case 'list_directory': {
      return path === null ? m.tools_call_list_workspace() : m.tools_call_list_directory({ path });
    }
    case 'move_path': {
      return hasEnds ? m.tools_call_move_path({ destination, source }) : toolLabel(call.tool);
    }
    case 'move_to_project': {
      const project = accessProject(call.accesses) ?? argumentText(values, 'project');
      return project === null ? toolLabel(call.tool) : m.tools_call_move_to_project({ project });
    }
    case 'read_command_output': {
      return commandNumber === null
        ? toolLabel(call.tool)
        : m.tools_call_read_command_output({ number: commandNumber });
    }
    case 'read_file': {
      return path === null ? toolLabel(call.tool) : m.tools_call_read_file({ path });
    }
    case 'read_skill': {
      return skillIntent(values);
    }
    case RUN_COMMAND: {
      return command === null ? toolLabel(call.tool) : m.tools_call_run_command({ command: firstLine(command) });
    }
    case 'search_text': {
      if (query === null) {
        return toolLabel(call.tool);
      }
      return path === null ? m.tools_call_search_text_workspace({ query }) : m.tools_call_search_text({ path, query });
    }
    case START_COMMAND: {
      return command === null ? toolLabel(call.tool) : m.tools_call_start_command({ command: firstLine(command) });
    }
    case 'stop_command': {
      return commandNumber === null ? toolLabel(call.tool) : m.tools_call_stop_command({ number: commandNumber });
    }
    case 'write_file': {
      return path === null ? toolLabel(call.tool) : m.tools_call_write_file({ path });
    }
    default: {
      // A plugin's tool that runs a command on a remote host, types one into
      // a terminal, works on a registered database or calls a tool server's
      // tool is named by its access, once prepared, whatever the plugin calls
      // the tool.
      const typed = accessTerminalCommand(call.accesses);
      if (typed !== null) {
        return terminalCommandSummary(typed);
      }
      const database = databaseSummary(call.accesses);
      if (database !== null) {
        return database;
      }
      const remote = accessRemoteCommand(call.accesses);
      if (remote !== null) {
        const words = { command: firstLine(remote.command), host: remote.host };
        return remote.background ? m.tools_call_remote_command_background(words) : m.tools_call_remote_command(words);
      }
      const external = accessExternalTool(call.accesses);
      return external === null ? toolLabel(call.tool) : m.tools_call_external_tool(external);
    }
  }
}

/** What a call of `read_skill` reads: a skill's instructions, or one of its files (ADR-0061). */
function skillIntent(values: Arguments): string {
  const name = argumentText(values, 'name');
  if (name === null) {
    return toolLabel('read_skill');
  }
  const file = argumentText(values, 'path');
  return file === null ? m.tools_call_read_skill({ name }) : m.tools_call_read_skill_file({ name, path: file });
}

function commandSummary(outcome: Extract<ToolOutcome, { kind: 'command_ran' }>): string {
  const command = firstLine(outcome.command);
  const duration = formatDuration(outcome.durationMs);
  switch (outcome.ending) {
    case 'exited': {
      // Exit codes are identifiers, such as -1073741819, and get no grouping.
      return outcome.exitCode === null
        ? m.tools_done_command_finished({ command, duration })
        : m.tools_done_command_exited({ code: String(outcome.exitCode), command, duration });
    }
    case 'interrupted': {
      return m.tools_done_command_interrupted({ command });
    }
    case 'stopped': {
      return m.tools_done_command_stopped({ command, duration });
    }
    case 'timed_out': {
      return m.tools_done_command_timed_out({ command, duration });
    }
  }
}

/** What a finished call did, or `null` when the intent says more: it failed, it is a plugin's tool, or this version cannot read its outcome. */
function outcomeSummary(outcome: ToolOutcome): string | null {
  switch (outcome.kind) {
    case 'command_ran': {
      return commandSummary(outcome);
    }
    case 'command_started': {
      return m.tools_done_command_started({ number: outcome.number });
    }
    case 'command_output_read': {
      return outcome.lastLine === 0
        ? m.tools_done_command_output_none({ number: outcome.number })
        : m.tools_done_command_output_read({
            first: outcome.firstLine,
            last: outcome.lastLine,
            number: outcome.number,
          });
    }
    case 'command_stopped': {
      return m.tools_done_background_stopped({ number: outcome.number });
    }
    case 'copied': {
      return m.tools_done_copied({ destination: outcome.destination, files: outcome.files, source: outcome.source });
    }
    case 'deleted': {
      return m.tools_done_deleted({ path: outcome.path });
    }
    case 'described': {
      return m.tools_call_file_info({ path: outcome.path });
    }
    case 'edited': {
      return m.tools_done_edited({ path: outcome.path, replacements: outcome.replacements });
    }
    case 'failed':
    case 'unknown': {
      return null;
    }
    case 'folder_created': {
      return m.tools_done_folder_created({ path: outcome.path });
    }
    case 'found': {
      return m.tools_done_found({ path: outcome.path, shown: outcome.shown });
    }
    case 'listed': {
      return m.tools_done_listed({ path: outcome.path, shown: outcome.shown, total: outcome.total });
    }
    case 'moved': {
      return m.tools_done_moved({ destination: outcome.destination, source: outcome.source });
    }
    case 'moved_to_project': {
      return m.tools_done_moved_to_project({ project: outcome.name });
    }
    case 'output_read': {
      return m.tools_done_output_read({ first: outcome.firstLine, last: outcome.lastLine });
    }
    case 'plugin': {
      // SDK 1's plugin tools report no outcome of their own, so the call is named by its tool.
      return null;
    }
    case 'plan_kept': {
      return m.tools_done_plan_kept();
    }
    case 'plan_saved': {
      return m.tools_done_plan_saved({ title: outcome.title });
    }
    case 'question_answered': {
      return m.tools_done_question_answered({ count: outcome.answers.length });
    }
    case 'question_declined': {
      return m.tools_done_question_declined();
    }
    case 'read': {
      return outcome.totalLines === 0
        ? m.tools_done_read_empty({ path: outcome.path })
        : m.tools_done_read({
            first: outcome.firstLine,
            last: outcome.lastLine,
            path: outcome.path,
            total: outcome.totalLines,
          });
    }
    case 'searched': {
      return m.tools_done_searched({ path: outcome.path, shown: outcome.shown });
    }
    case 'skill_read': {
      return outcome.path === 'SKILL.md'
        ? m.tools_done_skill_read({ name: outcome.skill })
        : m.tools_done_skill_file_read({ name: outcome.skill, path: outcome.path });
    }
    case 'system_info': {
      return m.tools_call_system_info();
    }
    case 'written': {
      const size = formatBytes(outcome.bytes);
      return outcome.created
        ? m.tools_done_created_file({ path: outcome.path, size })
        : m.tools_done_replaced_file({ path: outcome.path, size });
    }
  }
}

/** One line about what a call does, or did once it finished. */
export function callSummary(call: ToolCallView): string {
  const outcome = call.result?.outcome;
  const done = outcome === undefined ? null : outcomeSummary(outcome);
  return done ?? intentSummary(call);
}

/** The call's status. A command that runs says how long it has run, measured to `now`. */
export function callStatus(call: ToolCallView, now: number): CallStatus {
  switch (call.status) {
    case 'awaiting_answer': {
      return { label: m.tools_status_awaiting_answer(), quiet: false, tone: 'attention' };
    }
    case 'awaiting_approval': {
      return { label: m.tools_status_awaiting_approval(), quiet: false, tone: 'attention' };
    }
    case 'canceled': {
      return { label: m.tools_status_canceled(), quiet: false, tone: 'ended' };
    }
    case 'completed': {
      const outcome = call.result?.outcome;
      if (outcome?.kind === 'question_declined') {
        return { label: m.tools_status_question_declined(), quiet: false, tone: 'ended' };
      }
      // A plugin's call that failed names its code in its outcome (ADR-0039).
      const pluginFailed = outcome?.kind === 'plugin' && (outcome.failure ?? null) !== null;
      const failed = outcome?.kind === 'failed' || pluginFailed;
      return failed
        ? { label: m.tools_status_failed(), quiet: false, tone: 'failed' }
        : { label: m.tools_status_done(), quiet: true, tone: 'done' };
    }
    case 'denied': {
      return { label: m.tools_status_denied(), quiet: false, tone: 'ended' };
    }
    case 'executing':
    case 'running': {
      // Only a command says how long it has run.
      const startedAt = call.tool === RUN_COMMAND ? call.startedAt : null;
      if (startedAt === null) {
        return { label: m.tools_status_running(), quiet: true, tone: 'busy' };
      }
      // Whole seconds, so that the label changes once a second.
      const elapsed = Math.floor(Math.max(0, now - startedAt) / 1_000) * 1_000;
      return {
        label: m.tools_status_running_for({ duration: formatDuration(elapsed) }),
        quiet: false,
        tone: 'busy',
      };
    }
    case 'not_run': {
      return { label: m.tools_status_not_run(), quiet: false, tone: 'ended' };
    }
    case 'pending': {
      return { label: m.tools_status_pending(), quiet: true, tone: 'busy' };
    }
    case 'receiving': {
      return { label: m.tools_status_receiving(), quiet: true, tone: 'busy' };
    }
    case 'uncertain': {
      return { label: m.tools_status_uncertain(), quiet: false, tone: 'failed' };
    }
  }
}
