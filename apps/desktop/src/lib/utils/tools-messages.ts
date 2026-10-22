// Messages for the codes of the system tools feature (ADR-0019): the
// commands' errors, the failures of tool calls, and the parts of an approval
// request. The UI shows the catalog's message for a code and never the core's
// description. Paths and commands are content: they enter a message as its
// arguments and render as text.
import type {
  AccessMode,
  ApprovalReason,
  Decision,
  DiscardedWork,
  EntryCount,
  GrantScope,
  Interruption,
  NotRunReason,
  PathRefusal,
  ProgramEffect,
  ProgramEffectKey,
  Refusal,
  ToolFailure,
} from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import { pluginName } from '#lib/state/plugin-catalog.js';
import type { ToolsFailure } from '#lib/types/tools.js';

const BYTE_UNITS = ['byte', 'kilobyte', 'megabyte', 'gigabyte'] as const;

/** A size in bytes, such as "512 bytes" or "1.2 kB", in the owner's language. */
export function formatBytes(bytes: number): string {
  const whole = Math.max(0, bytes);
  // Each unit is a thousand of the one before; sizes beyond the last unit stay in it.
  const thousands = whole < 1_000 ? 0 : Math.floor(Math.log10(whole) / 3);
  const unit = Math.min(thousands, BYTE_UNITS.length - 1);
  const value = whole / 1_000 ** unit;
  const isBytes = unit === 0;
  return new Intl.NumberFormat(undefined, {
    maximumFractionDigits: isBytes ? 0 : 1,
    style: 'unit',
    unit: BYTE_UNITS[unit],
    unitDisplay: isBytes ? 'long' : 'short',
  }).format(value);
}

/** Why Mannie refused a path, as a sentence. */
export function pathRefusalMessage(reason: PathRefusal): string {
  switch (reason) {
    case 'alternate_stream': {
      return m.tools_path_refusal_alternate_stream();
    }
    case 'device_path': {
      return m.tools_path_refusal_device_path();
    }
    case 'drive_relative': {
      return m.tools_path_refusal_drive_relative();
    }
    case 'empty': {
      return m.tools_path_refusal_empty();
    }
    case 'network_path': {
      return m.tools_path_refusal_network_path();
    }
    case 'not_absolute': {
      return m.tools_path_refusal_not_absolute();
    }
    case 'reserved_name': {
      return m.tools_path_refusal_reserved_name();
    }
    case 'too_long': {
      return m.tools_path_refusal_too_long();
    }
    case 'trailing_dot_or_space': {
      return m.tools_path_refusal_trailing_dot_or_space();
    }
  }
}

/** What to tell the owner about a command of the feature that failed. */
export function toolsFailureMessage(failure: ToolsFailure): string {
  if (failure.kind === 'ipc') {
    return m.shell_error_ipc_failed();
  }
  const { error } = failure;
  switch (error.code) {
    case 'approval_changed': {
      return m.tools_error_approval_changed();
    }
    case 'approval_not_pending': {
      return m.tools_error_approval_not_pending();
    }
    case 'conversation_not_found': {
      return m.tools_error_conversation_not_found();
    }
    case 'environment_name_invalid': {
      return m.tools_error_environment_name_invalid({ name: error.name });
    }
    case 'environment_name_secret': {
      return m.tools_error_environment_name_secret({ name: error.name });
    }
    case 'full_access_follows_parent': {
      return m.tools_error_full_access_follows_parent();
    }
    case 'full_access_refused': {
      return m.tools_error_full_access_refused();
    }
    case 'grant_not_found': {
      return m.tools_error_grant_not_found();
    }
    case 'grant_not_possible': {
      return m.tools_error_grant_not_possible();
    }
    case 'not_in_a_project': {
      return m.tools_error_not_in_a_project();
    }
    case 'output_not_found': {
      return m.tools_error_output_not_found();
    }
    case 'background_command_not_found': {
      return m.tools_error_background_command_not_found();
    }
    case 'plugin_off': {
      return m.tools_error_plugin_off({ plugin: pluginName(error.plugin) });
    }
    case 'rule_not_found': {
      return m.tools_error_rule_not_found();
    }
    case 'rule_not_possible': {
      return m.tools_error_rule_not_possible();
    }
    case 'rule_prefix_invalid': {
      return m.tools_error_rule_prefix_invalid();
    }
    case 'shutting_down': {
      return m.tools_error_shutting_down();
    }
    case 'storage_failed': {
      return m.tools_error_storage_failed();
    }
    case 'store_unavailable': {
      return m.shell_error_store_unavailable();
    }
    case 'task_target_unreachable': {
      return m.tools_error_task_target_unreachable({ target: error.target });
    }
    case 'too_many_environment_names': {
      return m.tools_error_too_many_environment_names({ max: error.max });
    }
    // 0.2.0's error of a worker that cannot be reached; this version names
    // the target instead.
    case 'worker_unreachable': {
      return m.tools_error_task_unreachable();
    }
    case 'written_by_newer_version': {
      return m.tools_error_written_by_newer_version();
    }
  }
}

/** Why a tool call failed, as the owner reads it in the call's card. */
export function toolFailureMessage(failure: ToolFailure): string {
  switch (failure.code) {
    case 'already_exists': {
      return m.tools_failure_already_exists({ path: failure.path });
    }
    case 'already_in_project': {
      return m.tools_failure_already_in_project({ project: failure.name });
    }
    case 'binary_file': {
      return m.tools_failure_binary_file({ path: failure.path });
    }
    case 'command_not_found': {
      return m.tools_failure_command_not_found({ number: failure.number });
    }
    case 'different_volume': {
      return m.tools_failure_different_volume();
    }
    case 'edit_ambiguous': {
      return m.tools_failure_edit_ambiguous({ edit: failure.edit, matches: failure.matches });
    }
    case 'edit_not_found': {
      return m.tools_failure_edit_not_found({ edit: failure.edit });
    }
    case 'edits_overlap': {
      return m.tools_failure_edits_overlap({ first: failure.first, second: failure.second });
    }
    case 'folder_not_empty': {
      return m.tools_failure_folder_not_empty({ path: failure.path });
    }
    case 'in_use': {
      return m.tools_failure_in_use({ path: failure.path });
    }
    case 'invalid_pattern': {
      return m.tools_failure_invalid_pattern();
    }
    case 'io_error': {
      return m.tools_failure_io_error();
    }
    case 'link_in_source': {
      return m.tools_failure_link_in_source({ path: failure.path });
    }
    case 'no_workspace': {
      return m.tools_failure_no_workspace();
    }
    case 'not_a_file': {
      return m.tools_failure_not_a_file({ path: failure.path });
    }
    case 'not_a_folder': {
      return m.tools_failure_not_a_folder({ path: failure.path });
    }
    case 'not_found': {
      return m.tools_failure_not_found({ path: failure.path });
    }
    case 'output_not_found': {
      return m.tools_failure_output_not_found();
    }
    case 'path_changed': {
      return m.tools_failure_path_changed();
    }
    case 'path_refused': {
      return m.tools_failure_path_refused({ reason: pathRefusalMessage(failure.reason) });
    }
    case 'path_through_link': {
      return m.tools_failure_path_through_link({ resolved: failure.resolved });
    }
    case 'permission_denied': {
      return m.tools_failure_permission_denied({ path: failure.path });
    }
    case 'plugin_invalid': {
      return m.tools_failure_plugin_invalid({ plugin: pluginName(failure.plugin) });
    }
    case 'project_not_found': {
      return m.tools_failure_project_not_found({ project: failure.project });
    }
    case 'revision_mismatch': {
      return m.tools_failure_revision_mismatch();
    }
    case 'shell_missing': {
      return m.tools_failure_shell_missing({ path: failure.path });
    }
    case 'spawn_failed': {
      return m.tools_failure_spawn_failed();
    }
    case 'too_large': {
      return m.tools_failure_too_large({ limit: formatBytes(failure.limitBytes) });
    }
    case 'too_many_background_commands': {
      return failure.scope === 'conversation'
        ? m.tools_failure_too_many_commands_in_conversation({ limit: failure.limit })
        : m.tools_failure_too_many_commands_on_machine({ limit: failure.limit });
    }
    case 'too_many_entries': {
      return m.tools_failure_too_many_entries({ limit: failure.limit });
    }
    case 'unsupported_encoding': {
      return m.tools_failure_unsupported_encoding({ path: failure.path });
    }
  }
}

/** Why a tool call never started. */
export function notRunMessage(reason: NotRunReason): string {
  switch (reason.reason) {
    case 'cut_off': {
      return m.tools_not_run_cut_off();
    }
    case 'earlier_call_denied': {
      return m.tools_not_run_earlier_call_denied();
    }
    case 'invalid_arguments': {
      return m.tools_not_run_invalid_arguments();
    }
    case 'over_budget': {
      return m.tools_not_run_over_budget();
    }
    case 'plugin_off': {
      return m.tools_not_run_plugin_off({ plugin: pluginName(reason.plugin) });
    }
    case 'refused': {
      return refusalMessage(reason.refusal);
    }
    case 'tools_off': {
      return m.tools_not_run_tools_off();
    }
    case 'unknown_tool': {
      return m.tools_not_run_unknown_tool();
    }
  }
}

/** What the ceiling of the computer that runs a task forbids. */
export function refusalMessage(refusal: Refusal): string {
  switch (refusal) {
    case 'no_programs_on_host': {
      return m.tools_refusal_no_programs_on_host();
    }
    case 'no_projects_on_host': {
      return m.tools_refusal_no_projects_on_host();
    }
    case 'no_remote_commands': {
      return m.tools_refusal_no_remote_commands();
    }
    case 'read_only_host': {
      return m.tools_refusal_read_only_host();
    }
  }
}

/** Why the policy asks the owner about an operation. */
export function approvalReasonMessage(reason: ApprovalReason): string {
  switch (reason) {
    case 'change_in_workspace': {
      return m.tools_reason_change_in_workspace();
    }
    case 'command': {
      return m.tools_reason_command();
    }
    case 'host_asks': {
      return m.tools_reason_host_asks();
    }
    case 'mannie_folder': {
      return m.tools_reason_mannie_folder();
    }
    case 'move_to_project': {
      return m.tools_reason_move_to_project();
    }
    case 'no_workspace': {
      return m.tools_reason_no_workspace();
    }
    case 'outside_allowed_folders': {
      return m.tools_reason_outside_allowed_folders();
    }
    case 'outside_workspace': {
      return m.tools_reason_outside_workspace();
    }
    case 'program': {
      return m.tools_reason_program();
    }
    case 'remote_command': {
      return m.tools_reason_remote_command();
    }
  }
}

/** What allowed or denied an operation. */
export function decisionMessage(decision: Decision): string {
  switch (decision) {
    case 'allowed_always': {
      return m.tools_decision_allowed_always();
    }
    case 'allowed_folder': {
      return m.tools_decision_allowed_folder();
    }
    case 'allowed_for_conversation': {
      return m.tools_decision_allowed_for_conversation();
    }
    case 'allowed_for_project': {
      return m.tools_decision_allowed_for_project();
    }
    case 'allowed_once': {
      return m.tools_decision_allowed_once();
    }
    case 'always_grant': {
      return m.tools_decision_always_grant();
    }
    case 'denied': {
      return m.tools_decision_denied();
    }
    case 'full_access': {
      return m.tools_decision_full_access();
    }
    case 'grant': {
      return m.tools_decision_grant();
    }
    case 'host_full_access': {
      return m.tools_decision_host_full_access();
    }
    case 'host_rule': {
      return m.tools_decision_host_rule();
    }
    case 'owner_named_project': {
      return m.tools_decision_owner_named_project();
    }
    case 'project_grant': {
      return m.tools_decision_project_grant();
    }
    case 'rule': {
      return m.tools_decision_rule();
    }
    case 'rule_saved': {
      return m.tools_decision_rule_saved();
    }
    case 'workspace': {
      return m.tools_decision_workspace();
    }
  }
}

/** How an operation uses a path, as a short label. */
export function accessModeLabel(mode: AccessMode): string {
  switch (mode) {
    case 'create': {
      return m.tools_mode_create();
    }
    case 'delete': {
      return m.tools_mode_delete();
    }
    case 'read': {
      return m.tools_mode_read();
    }
    case 'write': {
      return m.tools_mode_write();
    }
  }
}

/** What a recursive delete removes inside a folder, such as "3 files and 2 folders". */
export function entryCountMessage(entries: EntryCount): string {
  const counts = [m.tools_count_files({ count: entries.files }), m.tools_count_folders({ count: entries.folders })];
  return new Intl.ListFormat(undefined, { type: 'conjunction' }).format(counts);
}

/** Why an operation that was executing may have taken effect: Mannie quit, the owner stopped it, or its plugin went off. */
export function uncertainMessage(interruption: Interruption | null): string {
  if (interruption === null) {
    return m.tools_uncertain_description();
  }
  switch (interruption.reason) {
    case 'plugin_off': {
      return m.tools_uncertain_plugin_off_description({ plugin: pluginName(interruption.plugin) });
    }
    case 'stopped': {
      return m.tools_uncertain_stopped_description();
    }
  }
}

/**
 * The words of a run of an installed program as one line, such as
 * `git commit -m "Fix the parser"`, for the owner to read. The program
 * receives each word exactly, without a shell; the quotes only show where a
 * word with spaces, quotes or nothing in it begins and ends.
 */
export function programRunText(words: readonly string[]): string {
  return words
    .map((word) => {
      const needsQuotes = word === '' || /[\s"'\\]/u.test(word);
      return needsQuotes ? JSON.stringify(word) : word;
    })
    .join(' ');
}

/** What a run of an installed program does beyond the workspace, as a sentence, such as "Runs pre-commit from .git/hooks/pre-commit". */
export function programEffectMessage(effect: ProgramEffect): string {
  switch (effect.effect) {
    case 'discards_work': {
      return discardsMessage(effect.work, effect.count);
    }
    case 'network': {
      return m.tools_effect_network({ endpoint: effect.endpoint });
    }
    case 'rewrites_published': {
      const places = new Intl.ListFormat(undefined, { type: 'conjunction' }).format(effect.places);
      return m.tools_effect_rewrites_published({ count: effect.count, places });
    }
    case 'runs_folder_code': {
      return m.tools_effect_runs_folder_code({ name: effect.name, source: effect.source });
    }
  }
}

/** What a run discards, by its kind and how much of it. */
function discardsMessage(work: DiscardedWork, count: number): string {
  switch (work) {
    case 'commits': {
      return m.tools_effect_discards_commits({ count });
    }
    case 'stash_entries': {
      return m.tools_effect_discards_stash_entries({ count });
    }
    case 'uncommitted_changes': {
      return m.tools_effect_discards_uncommitted_changes({ count });
    }
  }
}

/**
 * The names of what a run discards, such as its files, with how many more
 * there are, or `null` for an effect that names nothing.
 */
export function programEffectItems(effect: ProgramEffect): string | null {
  if (effect.effect !== 'discards_work') {
    return null;
  }
  if (effect.items.length === 0) {
    return null;
  }
  const more = effect.count - effect.items.length;
  const items = more > 0 ? [...effect.items, m.tools_effect_more({ count: more })] : effect.items;
  return new Intl.ListFormat(undefined, { type: 'unit' }).format(items);
}

/** Whether a run keeps a snapshot of the work that it discards, from which the owner can take it back. */
export function keepsSnapshot(effect: ProgramEffect): boolean {
  return effect.effect === 'discards_work' ? effect.snapshot : false;
}

/** What a grant or a rule covers of discarded work, as a phrase that follows "when it". */
function discardedKeyPhrase(work: DiscardedWork): string {
  switch (work) {
    case 'commits': {
      return m.tools_effect_key_discards_commits();
    }
    case 'stash_entries': {
      return m.tools_effect_key_discards_stash_entries();
    }
    case 'uncommitted_changes': {
      return m.tools_effect_key_discards_uncommitted_changes();
    }
  }
}

/** What a grant or a rule covers of an effect, as a phrase that follows "when it", such as "runs pre-commit from .git/hooks/pre-commit". */
export function effectKeyPhrase(key: ProgramEffectKey): string {
  switch (key.effect) {
    case 'discards_work': {
      return discardedKeyPhrase(key.work);
    }
    case 'network': {
      return m.tools_effect_key_network({ endpoint: key.endpoint });
    }
    case 'rewrites_published': {
      return m.tools_effect_key_rewrites_published();
    }
    case 'runs_folder_code': {
      return m.tools_effect_key_runs_folder_code({ name: key.name, source: key.source });
    }
  }
}

/** The phrases of several effects as one, such as "runs pre-commit from … and connects to …". */
export function effectKeysText(keys: readonly ProgramEffectKey[]): string {
  return new Intl.ListFormat(undefined, { type: 'conjunction' }).format(keys.map((key) => effectKeyPhrase(key)));
}

/** What a grant allows, such as "Changing files in D:\Projects\app". */
export function grantScopeMessage(scope: GrantScope): string {
  if (scope.kind === 'command') {
    return m.tools_grant_command({ command: scope.command, folder: scope.workingDirectory });
  }
  if (scope.kind === 'remote_command') {
    return m.tools_grant_remote_command({ command: scope.command, endpoint: scope.endpoint, host: scope.label });
  }
  if (scope.kind === 'program') {
    const run = programRunText([scope.name, ...scope.arguments]);
    const folder = scope.workingDirectory;
    return scope.effects.length === 0
      ? m.tools_grant_program({ folder, run })
      : m.tools_grant_program_effects({ effects: effectKeysText(scope.effects), folder, run });
  }
  switch (scope.mode) {
    case 'create': {
      return m.tools_grant_create({ folder: scope.folder });
    }
    case 'delete': {
      return m.tools_grant_delete({ folder: scope.folder });
    }
    case 'read': {
      return m.tools_grant_read({ folder: scope.folder });
    }
    case 'write': {
      return m.tools_grant_write({ folder: scope.folder });
    }
  }
}
