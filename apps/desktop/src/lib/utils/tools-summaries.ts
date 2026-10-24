// What a tool call's row says (docs/features/system-tools/tool-activity.md,
// tool calls in the reply): the line that Mannie builds about what the call
// does or did, its status, and its tool's icon. The line names paths and
// commands as the result reports them, or as the model wrote them while the
// call has no result.
import CirclePlayIcon from '@lucide/svelte/icons/circle-play';
import CircleStopIcon from '@lucide/svelte/icons/circle-stop';
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

/** The tool of a tool server that a plugin's call calls, with the server's name (ADR-0043). */
function accessExternalTool(accesses: readonly Access[]): { server: string; tool: string } | null {
  const access = accesses.find((candidate) => candidate.kind === 'external_tool');
  return access?.kind === 'external_tool' ? { server: access.label, tool: access.tool } : null;
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
      // A plugin's tool that runs a command on a remote host or calls a tool
      // server's tool is named by its access, once prepared, whatever the
      // plugin calls the tool.
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
    case 'awaiting_approval': {
      return { label: m.tools_status_awaiting_approval(), quiet: false, tone: 'attention' };
    }
    case 'canceled': {
      return { label: m.tools_status_canceled(), quiet: false, tone: 'ended' };
    }
    case 'completed': {
      const outcome = call.result?.outcome;
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
